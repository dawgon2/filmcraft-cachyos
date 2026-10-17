//! Rate an Assistant answer and help improve the Assistant (fork AI5, AI-PLAN Phase AI5).
//!
//! Under every checked answer (Assistant panel and Ask box): 👍 / 👎, a "What should it have
//! said?" box (`coach.feedback`) and **Help improve the Assistant…**, which opens a dialog with the
//! exact text that would be shared (`coach.shareExample`, `route: preview`), the extras to tick,
//! the consent tick, and the two ways out: **Open on GitHub** (needs the tick) and **Save
//! example…** (doesn't). Nothing leaves the computer unless the user submits the GitHub page.
//!
//! Automation ids: `assistant.rate.{n}.up` / `.down` / `.correction` / `.share` in the panel,
//! `coach.ask.rate.up` / … in the Ask box ([`ASK_BOX_IDS`]); the dialog:
//! `share.preview`, `share.grade`, `share.picture`, `share.frame`, `share.consent`,
//! `share.github`, `share.save`, `share.close`.

use std::collections::BTreeMap;

use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::native_dialogs::FileDialog;

/// Ratings kept in the UI (the engine keeps as many answers).
const MAX_KEPT: usize = 64;
/// Longest correction typed (the engine keeps 2 000 chars).
const MAX_CORRECTION: usize = 2_000;

/// What the user said about one answer.
#[derive(Clone, Debug, Default)]
pub struct Feedback {
    /// `Some(true)` 👍, `Some(false)` 👎.
    pub rating: Option<bool>,
    pub correction: String,
}

/// The open "Help improve the Assistant" dialog.
#[derive(Clone, Debug, Default)]
pub struct ShareDialog {
    /// The answer's `coach.assist` id.
    pub id: u64,
    pub grade: bool,
    pub picture: bool,
    pub frame: bool,
    pub consent: bool,
    /// Exactly what is shared or saved (from `coach.shareExample`).
    pub preview: String,
    /// The last outcome (saved to …, opened, an error).
    pub status: String,
    /// The preview's check value: a share goes ahead only if it would still send exactly this.
    pub hash: String,
    /// The whole conversation the answer belongs to (`coach.assist` ids, oldest first), for the
    /// owner's training inbox.
    pub conversation: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ShareState {
    pub feedback: BTreeMap<u64, Feedback>,
    pub dialog: Option<ShareDialog>,
}

/// Sends the answer's rating and correction to the engine; the status line says why it failed.
fn send_feedback(app: &mut FilmcraftApp, id: u64) {
    let Some(f) = app.share.feedback.get(&id) else { return };
    let rating = match f.rating {
        Some(true) => "up",
        Some(false) => "down",
        None => "none",
    };
    let p = json!({ "id": id, "rating": rating, "correction": f.correction });
    if let Err(e) = app.session.execute("coach.feedback", p) {
        app.ui.status = e.to_string();
    }
}

/// The automation ids of one answer's row: 👍, 👎, the correction box, the share link.
pub struct RowIds<'a> {
    pub up: &'a str,
    pub down: &'a str,
    pub correction: &'a str,
    pub share: &'a str,
}

/// The Ask box's row ids.
pub const ASK_BOX_IDS: RowIds<'static> =
    RowIds { up: "coach.ask.rate.up", down: "coach.ask.rate.down", correction: "coach.ask.rate.correction", share: "coach.ask.rate.share" };

/// The 👍 / 👎 row under answer `id`, the correction box once rated, and the share link.
pub fn rate_row(app: &mut FilmcraftApp, ui: &mut egui::Ui, id: u64, ids: &RowIds) {
    if !app.share.feedback.contains_key(&id) {
        while app.share.feedback.len() >= MAX_KEPT {
            let Some(oldest) = app.share.feedback.keys().next().copied() else { break };
            app.share.feedback.remove(&oldest);
        }
    }
    let mut fb = app.share.feedback.get(&id).cloned().unwrap_or_default();
    let mut changed = false;
    let mut share = false;
    // wraps: the Assistant panel can be docked narrow
    ui.horizontal_wrapped(|ui| {
        for (up, glyph, key, tip) in [(true, "👍", ids.up, tl!("Helpful")), (false, "👎", ids.down, tl!("Not helpful"))] {
            let on = fb.rating == Some(up);
            let r = ui.add(egui::Button::new(RichText::new(glyph).size(13.0)).selected(on).min_size(vec2(26.0, 20.0))).on_hover_text(tip);
            app.auto.add_response(key, &r, tip);
            if r.clicked() {
                fb.rating = if on { None } else { Some(up) };
                changed = true;
            }
        }
        let r = ui.link(RichText::new(tl!("Help improve the Assistant…")).small());
        app.auto.add_response(ids.share, &r, "Help improve the Assistant…");
        share = r.clicked();
    });
    if fb.rating.is_some() {
        let r = ui.add(
            egui::TextEdit::multiline(&mut fb.correction)
                .hint_text(tl!("What should it have said? (optional)"))
                .desired_rows(2)
                .char_limit(MAX_CORRECTION)
                .desired_width(f32::INFINITY),
        );
        app.auto.add_response(ids.correction, &r, "What should it have said?");
        changed |= r.changed();
    }
    if changed || !app.share.feedback.contains_key(&id) {
        app.share.feedback.insert(id, fb);
    }
    if changed {
        send_feedback(app, id);
    }
    if share {
        open(app, id);
    }
}

/// Opens the dialog for answer `id`, nothing ticked.
pub fn open(app: &mut FilmcraftApp, id: u64) {
    let panel: Vec<u64> = app.ui.assistant.turns.iter().filter_map(|t| t.result.as_ref().and_then(|r| r["id"].as_u64())).collect();
    let conversation = if panel.contains(&id) { panel } else { vec![id] };
    app.share.dialog = Some(ShareDialog { id, conversation, ..Default::default() });
    refresh(app);
}

fn params(d: &ShareDialog, route: &str) -> Value {
    json!({ "id": d.id, "route": route, "grade": d.grade, "picture": d.picture, "frame": d.frame, "consent": d.consent, "expect": d.hash })
}

/// Rebuilds the preview after a tick changed.
fn refresh(app: &mut FilmcraftApp) {
    let Some(d) = app.share.dialog.as_ref() else { return };
    let r = app.session.execute("coach.shareExample", params(d, "preview"));
    if let Some(d) = app.share.dialog.as_mut() {
        match r {
            Ok(v) => {
                d.preview = v["text"].as_str().unwrap_or("").to_string();
                d.hash = v["hash"].as_str().unwrap_or("").to_string();
            }
            Err(e) => d.status = e.to_string(),
        }
    }
}

enum Action {
    Refresh,
    GitHub,
    Save,
    Inbox,
    Close,
}

/// The dialog, when open (called every frame from [`crate::panels::dialogs::show`]).
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.share.dialog.clone() else { return };
    let t = app.tokens;
    let mut action = None;
    let r = egui::Modal::new(egui::Id::new("share-example-modal")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(RichText::new(tl!("Help improve the Assistant")).size(17.0).strong().color(t.text));
        ui.add_space(4.0);
        ui.label(
            RichText::new(tl!(
                "This is exactly what would be shared or saved. Names of your project, clips, files and folders, your user name and network addresses are replaced."
            ))
            .small()
            .color(t.text_dim),
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("share-preview").max_height(260.0).show(ui, |ui| {
            let mut text = d.preview.as_str();
            let r = ui.add(egui::TextEdit::multiline(&mut text).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            app.auto.add_response("share.preview", &r, "Preview");
        });
        ui.add_space(6.0);
        for (key, label, value) in [
            ("share.grade", tl!("Include the grade's settings"), &mut d.grade),
            ("share.picture", tl!("Include the measured picture facts"), &mut d.picture),
            ("share.frame", tl!("Include a small frame from my footage (saved as an image)"), &mut d.frame),
        ] {
            let r = ui.checkbox(value, label);
            app.auto.add_response(key, &r, label);
            if r.changed() {
                action = Some(Action::Refresh);
            }
        }
        // a changed example needs a fresh agreement
        if matches!(action, Some(Action::Refresh)) {
            d.consent = false;
        }
        ui.add_space(6.0);
        let r = ui.checkbox(
            &mut d.consent,
            tl!(
                "I agree: it will be public on GitHub and used to train and improve Revolution's Assistant, including models released openly. It holds nothing private."
            ),
        );
        app.auto.add_response("share.consent", &r, "I agree");
        if !d.status.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&d.status).small().color(t.text_dim)).wrap());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let r = ui.add_enabled(d.consent, egui::Button::new(tl!("Open on GitHub")));
            let r = r.on_disabled_hover_text(tl!("Tick that you agree first."));
            app.auto.add_response("share.github", &r, "Open on GitHub");
            if r.clicked() {
                action = Some(Action::GitHub);
            }
            let r = ui.button(tl!("Save example…")).on_hover_text(tl!("Save the same text (and the frame, if ticked) to a folder. No agreement needed."));
            app.auto.add_response("share.save", &r, "Save example…");
            if r.clicked() {
                action = Some(Action::Save);
            }
            // FORK-ONLY (AI5 item 8): the owner's local training inbox, no consent (never leaves the computer)
            if filmcraft_engine::coach_share::INBOX {
                let r = ui.button("Send to my training inbox").on_hover_text("The whole conversation, unstripped, to this computer's assistant-inbox folder.");
                app.auto.add_response("share.inbox", &r, "Send to my training inbox");
                if r.clicked() {
                    action = Some(Action::Inbox);
                }
            }
            let r = ui.button(tl!("Close"));
            app.auto.add_response("share.close", &r, "Close");
            if r.clicked() {
                action = Some(Action::Close);
            }
        });
    });
    if r.should_close() {
        action = Some(Action::Close);
    }
    app.share.dialog = Some(d.clone());
    match action {
        Some(Action::Refresh) => refresh(app),
        Some(Action::Close) => app.share.dialog = None,
        Some(Action::GitHub) => match app.session.execute("coach.shareExample", params(&d, "github")) {
            Ok(v) => {
                let url = v["url"].as_str().unwrap_or("").to_string();
                crate::links::open(app, ctx, &url);
                let status = if v["tooLong"] == true {
                    tl!("Too long for a link: the GitHub page asks for the file. Use Save example… and drag the file into the issue.")
                } else if d.frame {
                    tl!("Opened on GitHub. A frame can't go in a link: use Save example… and drag the image into the issue.")
                } else {
                    tl!("Opened on GitHub. Check it there and submit it with your account.")
                };
                set_status(app, status);
            }
            Err(e) => set_status(app, &e.to_string()),
        },
        Some(Action::Inbox) => {
            let mut p = params(&d, "inbox");
            p["conversation"] = json!(d.conversation);
            match app.session.execute("coach.shareExample", p) {
                Ok(v) => set_status(app, &format!("Sent to the training inbox: {}", v["path"].as_str().unwrap_or(""))),
                Err(e) => set_status(app, &e.to_string()),
            }
        }
        Some(Action::Save) => {
            let p = params(&d, "file");
            app.pick_ui(FileDialog::Folder { at: None }, move |app, paths| {
                let Some(folder) = paths.into_iter().next() else { return };
                let mut p = p;
                p["folder"] = json!(folder);
                match app.session.execute("coach.shareExample", p) {
                    Ok(v) => {
                        let path = v["path"].as_str().unwrap_or("").to_string();
                        set_status(app, &tlf!("Saved as {path}", path));
                    }
                    Err(e) => set_status(app, &e.to_string()),
                }
            });
        }
        None => {}
    }
}

fn set_status(app: &mut FilmcraftApp, s: &str) {
    if let Some(d) = app.share.dialog.as_mut() {
        d.status = s.to_string();
    } else {
        app.ui.status = s.to_string();
    }
}
