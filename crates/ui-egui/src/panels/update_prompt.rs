//! "Update available" (fork UPD, FORK-PLAN §1a.77): a small window in the bottom-right corner
//! when the once-a-day check (`app.update.*`) finds a newer release, with its notes and
//! **Update**, **Later** and **Don't check for updates**. The first time it also says what the
//! check sends. While installing it shows the progress; once installed, **Restart Now**.
//! Builds that can't update themselves say so and offer the release page instead.
//!
//! Automation ids: `update.notes`, `update.install`, `update.later`, `update.optOut`,
//! `update.restart`, `update.releasePage`, `update.close`.

use egui::RichText;
use serde_json::{Value, json};

use crate::FilmcraftApp;

/// How often an open app asks again whether a check is due (the check itself is once a day).
const AUTO_EVERY_SECS: f64 = 3600.0;

/// Once a frame: start the daily check when due, collect results, and draw the prompt.
pub fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    if app.session.updater.channel.is_none() {
        return;
    }
    let now = ctx.input(|i| i.time);
    if app.update_next_auto <= now {
        app.update_next_auto = now + AUTO_EVERY_SECS;
        // not while automation drives the app (tests and agents check explicitly)
        if !app.control_input_only {
            app.session.auto_update_check();
        }
    }
    let busy = app.session.updater.status.state == "checking" || app.session.updater.status.install == "installing";
    app.session.poll_update();
    app.session.poll_update_install();
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    let st = &app.session.updater.status;
    let show = app.session.updater.prompt() || st.install == "installing" || st.install == "installed" || st.install == "failed";
    if show && !app.update_closed {
        show_window(app, ctx);
    }
}

enum Action {
    Install,
    Later,
    OptOut,
    Restart,
    ReleasePage,
    Close,
}

fn show_window(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let t = app.tokens;
    let v: Value = app.session.execute("app.update.status", json!({})).unwrap_or_default();
    let cur = v["current"]["tag"].as_str().unwrap_or("this build").trim_start_matches("fork-").to_string();
    let new = v["latest"]["tag"].as_str().unwrap_or("").trim_start_matches("fork-").to_string();
    let notes = v["latest"]["notes"].as_str().unwrap_or("").to_string();
    let install = v["install"].as_str().unwrap_or("none").to_string();
    let manual = v["manual"].as_str().unwrap_or("").to_string();
    let mut action = None;
    egui::Window::new(tl!("Update available"))
        .id(egui::Id::new("update-prompt"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0])
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(tlf!("Update available: {current} → {new}", current = cur, new = new)).strong().color(t.text));
            if !notes.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("update-notes").max_height(180.0).show(ui, |ui| {
                    let r = ui.add(egui::Label::new(RichText::new(&notes).small().color(t.text_dim)).wrap());
                    app.auto.add_response("update.notes", &r, "Release notes");
                });
            }
            if v["explained"] != true {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(tl!("What the check sends: one request to GitHub (api.github.com) asking for the newest release. GitHub sees your IP address; nothing else is sent: no usage data, no identifiers.")).small().color(t.text_dim)).wrap());
            }
            ui.add_space(6.0);
            match install.as_str() {
                "installing" => {
                    let (done, total) = (v["done"].as_f64().unwrap_or(0.0), v["total"].as_f64().unwrap_or(0.0));
                    let frac = if total > 0.0 { (done / total).clamp(0.0, 1.0) } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac as f32).show_percentage());
                    ui.label(RichText::new(tl!("Downloading and checking the new build. You can keep working.")).small().color(t.text_dim));
                }
                "installed" => {
                    ui.add(egui::Label::new(tl!("Installed. Revolution starts the new build when you restart it; the old build is kept, so you can roll back in Settings ▸ Updates.")).wrap());
                    ui.horizontal(|ui| {
                        let r = ui.button(tl!("Restart Now"));
                        app.auto.add_response("update.restart", &r, "Restart Now");
                        if r.clicked() {
                            action = Some(Action::Restart);
                        }
                        let r = ui.button(tl!("Later"));
                        app.auto.add_response("update.close", &r, "Later");
                        if r.clicked() {
                            action = Some(Action::Close);
                        }
                    });
                }
                _ => {
                    if install == "failed" {
                        let msg = v["message"].as_str().unwrap_or("").to_string();
                        ui.add(egui::Label::new(RichText::new(tlf!("The update didn't install (nothing was changed): {error}", error = msg)).color(t.text)).wrap());
                    }
                    if !manual.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&manual).small().color(t.text_dim)).wrap());
                    }
                    ui.horizontal(|ui| {
                        if manual.is_empty() {
                            let r = ui.button(tl!("Update"));
                            app.auto.add_response("update.install", &r, "Update");
                            if r.clicked() {
                                action = Some(Action::Install);
                            }
                        } else {
                            let r = ui.button(tl!("Open the Release Page"));
                            app.auto.add_response("update.releasePage", &r, "Open the Release Page");
                            if r.clicked() {
                                action = Some(Action::ReleasePage);
                            }
                        }
                        let r = ui.button(tl!("Later"));
                        app.auto.add_response("update.later", &r, "Later");
                        if r.clicked() {
                            action = Some(Action::Later);
                        }
                        let r = ui.button(tl!("Don't check for updates"));
                        app.auto.add_response("update.optOut", &r, "Don't check for updates");
                        if r.clicked() {
                            action = Some(Action::OptOut);
                        }
                    });
                }
            }
        });
    let Some(a) = action else { return };
    // any answer to the first prompt counts as having read what the check sends
    let _ = app.session.execute("app.update.status", json!({"dismiss": "explained"}));
    match a {
        Action::Install => {
            if let Err(e) = app.session.execute("app.update.install", json!({})) {
                app.ui.status = e.to_string();
            }
        }
        Action::Later => {
            let _ = app.session.execute("app.update.status", json!({"dismiss": "later"}));
        }
        Action::OptOut => {
            let _ = app.session.execute("app.update.setEnabled", json!({"enabled": false}));
            app.ui.status = tl!("Revolution won't check for updates. Turn it back on in Settings ▸ Updates.").to_string();
        }
        Action::Restart => restart(app, ctx),
        Action::ReleasePage => {
            if let Some(ch) = app.session.updater.channel {
                let url = format!("https://github.com/{}/releases/latest", ch.repo);
                crate::links::open(app, ctx, &url);
            }
            let _ = app.session.execute("app.update.status", json!({"dismiss": "later"}));
        }
        Action::Close => app.update_closed = true,
    }
}

/// Restart Now: the new build starts once this one has closed (unsaved work is asked about as on
/// any close; the single-instance lock is released first).
fn restart(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let program = app.session.updater.status.installed.clone();
    if program.is_empty() {
        return;
    }
    app.restart_into = Some(std::path::PathBuf::from(program));
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

/// Called from `on_exit`: starts the build Restart Now chose, with the same arguments.
pub fn spawn_restart(app: &mut FilmcraftApp) {
    let restart = app.restart_into.take();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(p) = restart {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Err(e) = std::process::Command::new(&p).args(args).spawn() {
            log::warn!("restarting into {}: {e}", p.display());
        }
    }
    #[cfg(target_arch = "wasm32")]
    drop(restart);
}
