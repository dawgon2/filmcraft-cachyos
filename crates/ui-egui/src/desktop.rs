//! Desktop shell integration (LINUX-PLAN L1.7, L2.3, L2.7): what the host's desktop services
//! (taskbar progress, notifications, idle / suspend inhibit, single-instance hand-off, media keys)
//! need to know about the app, and what they ask it to do.
//!
//! The UI builds a small [`DesktopState`] every frame and hands it, with the jobs that finished
//! since the last frame, to [`HostHooks::desktop`](crate::HostHooks::desktop). The host (the
//! desktop app's `desktop` module: D-Bus on Linux) compares it with what it last published and
//! does its slow work on its own thread; it never blocks the frame. Requests coming back (files
//! from a second launch, a media key) are taken each frame and run as ordinary commands through
//! [`menus::invoke`](crate::menus::invoke), so everything stays a command.
//!
//! Without a host (tests, the web, macOS / Windows today) nothing here runs.

use std::collections::HashMap;

use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde::Serialize;

/// Jobs whose label starts with this are preview renders: the timeline shows them, no
/// notification.
const PREVIEW_RENDER: &str = "Rendering ";
/// A finished job is announced when the window is in the background, or when it ran this long
/// (seconds) even with the window in front (a long export the user walked away from).
pub const NOTIFY_AFTER_S: f64 = 30.0;

/// What the desktop sees of the app this frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopState {
    /// The Program monitor is playing (keeps the screen awake; MPRIS `PlaybackStatus`).
    pub playing: bool,
    /// Playback speed (negative = reverse; MPRIS `Rate`).
    pub speed: f64,
    /// A job runs (export, proxies, Project Manager…): keeps the computer from suspending.
    pub busy: bool,
    /// Progress of the running job, 0–1 (taskbar entry), `None` when none runs.
    pub progress: Option<f64>,
    /// The running job's label.
    pub job: Option<String>,
    /// Project name (MPRIS metadata).
    pub project: String,
    /// The active sequence's name, if one is open.
    pub sequence: Option<String>,
    /// The project file, once saved (the desktop's recent-files list).
    pub project_path: Option<String>,
    /// Playhead and sequence duration in microseconds (MPRIS `Position` / `mpris:length`).
    pub position_us: i64,
    pub duration_us: i64,
    /// Frame duration in microseconds (a "next" / "previous" step), 0 without a sequence.
    pub frame_us: i64,
    /// The app window has keyboard focus.
    pub focused: bool,
    /// Settings ▸ Playback ▸ Media keys and the desktop's media controls is off: no MPRIS
    /// player (`false`, the default, publishes one).
    pub media_controls_off: bool,
    /// Settings ▸ Playback ▸ Global shortcuts is on: Play / Pause and Add Marker are bound
    /// through the desktop's GlobalShortcuts portal (L1.8; off by default).
    pub global_shortcuts: bool,
}

/// A job that finished since the last frame (for a notification with "Show in Folder").
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedJob {
    pub id: u64,
    pub label: String,
    /// Finished without an error and was not cancelled.
    pub ok: bool,
    /// The file it wrote (exports), for "Show in Folder".
    pub path: Option<String>,
    /// The error, when it failed.
    pub error: Option<String>,
    /// How long it ran, in seconds (as far as the UI saw it).
    pub seconds: f64,
}

/// What the desktop asks the app to do (taken each frame, run on the UI thread).
#[derive(Clone, Debug, PartialEq)]
pub enum DesktopRequest {
    /// Files from a second launch (desktop file `%F`, "Open With" in a file manager): a project is
    /// opened, media is imported. The window is brought forward.
    Open(Vec<String>),
    /// Bring the window forward (a second launch without files).
    Raise,
    /// Run a command by id (media keys: `playback.toggle`, `playback.stop`, `playhead.stepForward`…).
    Command(String, serde_json::Value),
}

/// A paired, reachable device that files can be sent to (KDE Connect, L2.10).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Phone {
    /// The device id (KDE Connect's).
    pub id: String,
    /// Its name as the user set it ("Pixel 8", "my-laptop").
    pub name: String,
    /// `phone`, `tablet`, `desktop`, `laptop`, `tv`… (as the device reports it).
    pub kind: String,
}

/// The host's side (`HostHooks::desktop`).
pub trait DesktopHost {
    /// Called every frame; cheap: compare and hand off. `ctx` lets the host wake the UI when a
    /// request arrives.
    fn update(&mut self, ctx: &egui::Context, state: &DesktopState, finished: &[FinishedJob]);
    /// Requests that arrived since the last frame.
    fn take_requests(&mut self) -> Vec<DesktopRequest>;
    /// The app is closing (`on_exit`): clear the taskbar progress, release inhibitors and the
    /// single-instance name. May wait briefly.
    fn shutdown(&mut self) {}
    /// Paired devices that can be reached now (KDE Connect, L2.10): what the host last saw,
    /// cheap (no D-Bus call on the UI thread). Empty without KDE Connect.
    fn phones(&self) -> Vec<Phone> {
        Vec::new()
    }
    /// Send a file to one of [`Self::phones`] (by id). Queued: `Ok` means handed to the
    /// desktop service, which shows the transfer itself.
    fn send_to_phone(&mut self, _device: &str, _path: &str) -> Result<(), String> {
        Err("sending files to a device needs KDE Connect".into())
    }
    /// Eject the camera card mounted at `mount` (L2.4). Queued: the outcome arrives as the
    /// `card.ejected` command.
    fn eject_card(&mut self, _mount: &str) -> Result<(), String> {
        Err("eject the card from the file manager".into())
    }
}

/// Most devices listed or matched (a hostile bus can't make the menu or a reply huge).
pub const MAX_PHONES: usize = 32;

/// A file a paired phone shared with this computer (KDE Connect, L2.10), waiting for Import.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneShare {
    /// Where KDE Connect saved it.
    pub path: String,
    /// The device's name.
    pub device: String,
}

/// Most shared files kept waiting for Import (older ones are dropped first).
pub const MAX_PHONE_SHARES: usize = 200;
/// The bin files shared from a phone are imported into (made at the project's top level).
pub const INBOX_BIN: &str = "Inbox";

/// A camera card the desktop mounted (L2.4), offered for copying in the Project panel.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardOffer {
    /// Where it is mounted.
    pub mount: String,
    /// Its name (the mount folder's).
    pub label: String,
    /// Its layout (`dcim`, `private`, …; `filmcraft_engine::card_offload::detect`).
    pub layout: String,
    /// The copy job, once started.
    pub job: Option<u64>,
    /// An eject was asked for and hasn't answered yet.
    pub ejecting: bool,
}

/// Most cards offered at once (the oldest is dropped).
pub const MAX_CARD_OFFERS: usize = 8;

/// The `card.*` UI commands (camera cards, L2.4): `card.detected` / `card.removed` come from the
/// host's mount watch; `card.copy` starts the engine's `card.offload` into Settings ▸ Media ▸ Copy
/// camera cards to (or `destination`); `card.eject` asks the host to eject (`card.ejected` is its
/// answer); `card.dismiss` hides the offer.
fn route_cards(app: &mut crate::FilmcraftApp, id: &str, params: &serde_json::Value) -> Option<Result<serde_json::Value, String>> {
    let mount = || params.get("mount").and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| "give the card (mount)".to_string());
    Some(match id {
        "card.detected" => card_detected(app, params),
        "card.removed" | "card.dismiss" => mount().map(|m| {
            let before = app.ui.cards.len();
            app.ui.cards.retain(|c| c.mount != m);
            serde_json::json!({"removed": before - app.ui.cards.len()})
        }),
        "card.copy" => mount().and_then(|m| copy_card(app, &m, params)),
        "card.eject" => mount().and_then(|m| {
            let host = app.hooks.desktop.as_mut().ok_or("eject the card from the file manager")?;
            host.eject_card(&m)?;
            let label = app.ui.cards.iter_mut().find(|c| c.mount == m).map(|c| {
                c.ejecting = true;
                c.label.clone()
            });
            app.ui.status = tlf!("Ejecting {card}…", card = label.unwrap_or_else(|| m.clone())).to_string();
            Ok(serde_json::json!({"mount": m}))
        }),
        "card.ejected" => mount().map(|m| {
            let ok = params.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let label = app.ui.cards.iter().find(|c| c.mount == m).map(|c| c.label.clone()).unwrap_or_else(|| m.clone());
            if ok {
                app.ui.cards.retain(|c| c.mount != m);
                app.ui.status = tlf!("{card} can be removed now", card = label).to_string();
            } else {
                if let Some(c) = app.ui.cards.iter_mut().find(|c| c.mount == m) {
                    c.ejecting = false;
                }
                let error: String = params.get("error").and_then(|v| v.as_str()).unwrap_or("").chars().filter(|c| !c.is_control()).take(300).collect();
                app.ui.status = tlf!("Couldn't eject {card}: {error}", card = label, error = error).to_string();
            }
            serde_json::json!({"mount": m, "ok": ok})
        }),
        _ => return None,
    })
}

/// The desktop mounted a camera card: offered in the Project panel unless Settings ▸ Media ▸
/// Camera cards is Ignore. Checked again here (the command can come from anywhere).
fn card_detected(app: &mut crate::FilmcraftApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mount = params.get("mount").and_then(|v| v.as_str()).ok_or("give the card (mount)")?;
    let path = std::path::Path::new(mount);
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!("{mount} is not a folder"));
    }
    if app.session.prefs.media.camera_cards == "off" {
        return Ok(serde_json::json!({"ignored": "off in Settings"}));
    }
    let layout = filmcraft_engine::card_offload::detect(path).ok_or_else(|| format!("{mount} isn't a camera card"))?;
    let label: String = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().filter(|c| !c.is_control()).take(120).collect();
    let cards = &mut app.ui.cards;
    if !cards.iter().any(|c| c.mount == mount) {
        if cards.len() >= MAX_CARD_OFFERS {
            cards.remove(0);
        }
        cards.push(CardOffer { mount: mount.to_string(), label, layout: layout.to_string(), job: None, ejecting: false });
    }
    Ok(serde_json::json!({"offered": cards.len(), "layout": layout}))
}

/// Start copying the card at `mount` (the engine's `card.offload`, a background job).
fn copy_card(app: &mut crate::FilmcraftApp, mount: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let dest = params.get("destination").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| app.session.prefs.media.card_destination.clone());
    if dest.trim().is_empty() {
        return Err(tl!("Choose where camera cards are copied to (Settings ▸ Media ▸ Copy camera cards to)").to_string());
    }
    let r = app.session.execute("card.offload", serde_json::json!({"mount": mount, "destination": dest})).map_err(|e| e.to_string())?;
    if let Some(e) = r.get("error").and_then(|v| v.as_str()) {
        return Err(e.to_string());
    }
    let job = r.get("job").and_then(|v| v.as_u64());
    if let Some(c) = app.ui.cards.iter_mut().find(|c| c.mount == mount) {
        c.job = job;
    }
    Ok(r)
}

/// The `kdeconnect.*` commands (KDE Connect, L2.10): `kdeconnect.devices` lists the paired,
/// reachable devices; `kdeconnect.send` sends a file (`path`, or a project `item` whose media is a
/// file) to `device` (id or name; may be left out when exactly one device is reachable). `None`:
/// not one of them.
pub(crate) fn route(app: &mut crate::FilmcraftApp, id: &str, params: &serde_json::Value) -> Option<Result<serde_json::Value, String>> {
    match id {
        "kdeconnect.devices" => {
            let phones = app.hooks.desktop.as_ref().map(|h| h.phones()).unwrap_or_default();
            Some(Ok(serde_json::json!({"devices": phones.into_iter().take(MAX_PHONES).collect::<Vec<_>>()})))
        }
        "kdeconnect.send" => Some(send_to_phone(app, params)),
        "kdeconnect.received" => Some(phone_shared(app, params)),
        "kdeconnect.importReceived" => {
            let paths: Vec<String> = std::mem::take(&mut app.ui.phone_shares).into_iter().map(|s| s.path).collect();
            Some(import_into_inbox(app, paths))
        }
        "kdeconnect.dismissReceived" => {
            let n = std::mem::take(&mut app.ui.phone_shares).len();
            Some(Ok(serde_json::json!({"dismissed": n})))
        }
        _ => route_cards(app, id, params),
    }
}

fn send_to_phone(app: &mut crate::FilmcraftApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path = match (params.get("path").and_then(|v| v.as_str()), params.get("item").and_then(|v| v.as_u64())) {
        (Some(p), _) => p.to_string(),
        (None, Some(item)) => match app.session.project.item(filmcraft_project::ItemId(item)).map(|it| &it.kind) {
            Some(filmcraft_project::ItemKind::Media(m)) => match &m.media {
                filmcraft_project::MediaRef::File { path } => path.clone(),
                _ => return Err("that item isn't a file on disk".into()),
            },
            Some(_) => return Err("that item isn't a file on disk".into()),
            None => return Err(format!("no project item {item}")),
        },
        (None, None) => return Err("give the file to send (path) or a project item (item)".into()),
    };
    let file = std::path::Path::new(&path);
    if !file.is_absolute() || !file.is_file() {
        return Err(format!("{path} is not a file"));
    }
    let host = app.hooks.desktop.as_mut().ok_or("sending files to a device needs KDE Connect")?;
    let phones = host.phones();
    let phones = phones.get(..phones.len().min(MAX_PHONES)).unwrap_or_default();
    let device = match params.get("device").and_then(|v| v.as_str()) {
        Some(d) => phones.iter().find(|p| p.id == d).or_else(|| phones.iter().find(|p| p.name == d)).ok_or_else(|| format!("no reachable device {d}"))?,
        None => match phones {
            [one] => one,
            [] => return Err("no paired device is reachable (KDE Connect)".into()),
            _ => return Err("several devices are reachable: say which (device)".into()),
        },
    };
    host.send_to_phone(&device.id, &path)?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    app.ui.status = tlf!("Sending {name} to {device}", name = name, device = device.name).to_string();
    Ok(serde_json::json!({"device": device.id, "path": path}))
}

/// A paired phone shared a file (`path`, from `device`): offered for import, imported into the
/// Inbox bin or ignored, as Settings ▸ Media ▸ Files shared from a phone says. Only media files
/// FilmCraft can import are taken; anything else (a PDF, a link) is left to KDE Connect.
fn phone_shared(app: &mut crate::FilmcraftApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path = params.get("path").and_then(|v| v.as_str()).ok_or("give the shared file (path)")?;
    let file = std::path::Path::new(path);
    if !file.is_absolute() || !file.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if !filmcraft_media::is_importable(file) {
        return Ok(serde_json::json!({"ignored": "not a media file"}));
    }
    let device: String = params.get("device").and_then(|v| v.as_str()).unwrap_or("").chars().filter(|c| !c.is_control()).take(200).collect();
    let device = if device.trim().is_empty() { "KDE Connect".to_string() } else { device };
    match app.session.prefs.media.phone_shares.as_str() {
        "off" => Ok(serde_json::json!({"ignored": "off in Settings"})),
        "inbox" => import_into_inbox(app, vec![path.to_string()]),
        _ => {
            let shares = &mut app.ui.phone_shares;
            if !shares.iter().any(|s| s.path == path) {
                if shares.len() >= MAX_PHONE_SHARES {
                    shares.remove(0);
                }
                shares.push(PhoneShare { path: path.to_string(), device });
            }
            Ok(serde_json::json!({"waiting": shares.len()}))
        }
    }
}

/// Import `paths` into the project's Inbox bin (made when missing).
fn import_into_inbox(app: &mut crate::FilmcraftApp, paths: Vec<String>) -> Result<serde_json::Value, String> {
    if paths.is_empty() {
        return Ok(serde_json::json!({"items": []}));
    }
    let existing = app.session.project.root.children.iter().find_map(|e| match e {
        filmcraft_project::BinEntry::Bin(b) if b.name == INBOX_BIN => Some(b.id.0),
        _ => None,
    });
    let bin = match existing {
        Some(b) => b,
        None => {
            let r = app.session.execute("file.newBin", serde_json::json!({"name": INBOX_BIN})).map_err(|e| e.to_string())?;
            r.get("bin").and_then(|v| v.as_u64()).ok_or("couldn't make the Inbox bin")?
        }
    };
    let n = paths.len();
    let r = app.session.execute("file.import", serde_json::json!({"paths": paths, "bin": bin})).map_err(|e| e.to_string())?;
    app.ui.status = tlf!("Imported {n} file(s) shared from a phone into the Inbox bin", n = n).to_string();
    Ok(r)
}

/// Remembers which jobs were seen running, to report each one once when it finishes.
#[derive(Default, Debug)]
pub struct JobWatch {
    /// Job id → when (egui time, s) it was first seen running.
    running: HashMap<u64, f64>,
}

impl JobWatch {
    /// Jobs seen running earlier that have finished now (each reported once). Jobs that were
    /// already finished when first seen are not reported (nothing ran while we watched).
    pub fn finished(&mut self, jobs: &[filmcraft_engine::Job], now: f64) -> Vec<FinishedJob> {
        use std::sync::atomic::Ordering;
        let mut out = Vec::new();
        for j in jobs {
            let finished = j.progress.finished.load(Ordering::Relaxed);
            match (finished, self.running.get(&j.id).copied()) {
                (false, None) => {
                    self.running.insert(j.id, now);
                }
                (true, Some(started)) => {
                    self.running.remove(&j.id);
                    if j.label.starts_with(PREVIEW_RENDER) {
                        continue;
                    }
                    let cancelled = j.progress.cancel.load(Ordering::Relaxed);
                    let result = j.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                    let progress_error = j.progress.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                    let (path, error) = match result {
                        Some(Ok(r)) => (Some(r.path).filter(|p| !p.is_empty()), progress_error),
                        Some(Err(e)) => (None, Some(e)),
                        None => (None, progress_error),
                    };
                    let seconds = if (now - started).is_finite() { (now - started).max(0.0) } else { 0.0 };
                    out.push(FinishedJob { id: j.id, label: j.label.clone(), ok: !cancelled && error.is_none(), path, error, seconds });
                }
                _ => {}
            }
        }
        // jobs removed from the list while running: forget them
        self.running.retain(|id, _| jobs.iter().any(|j| j.id == *id));
        out
    }
}

/// Should a finished job be announced (notification)? Cancelled jobs never are.
pub fn should_notify(job: &FinishedJob, focused: bool) -> bool {
    let cancelled = !job.ok && job.error.is_none();
    !cancelled && (!focused || job.seconds >= NOTIFY_AFTER_S)
}

/// Ticks → microseconds, saturating (MPRIS times are `i64` µs).
pub fn ticks_to_us(t: Tick) -> i64 {
    let us = i128::from(t.0).saturating_mul(1_000_000) / i128::from(TICKS_PER_SECOND);
    i64::try_from(us).unwrap_or(if us < 0 { i64::MIN } else { i64::MAX })
}

/// Microseconds → ticks, saturating.
pub fn us_to_ticks(us: i64) -> Tick {
    let t = i128::from(us).saturating_mul(i128::from(TICKS_PER_SECOND)) / 1_000_000;
    Tick(i64::try_from(t).unwrap_or(if t < 0 { i64::MIN } else { i64::MAX }))
}

impl crate::FilmcraftApp {
    /// What the desktop sees now.
    pub fn desktop_state(&self, focused: bool) -> DesktopState {
        use std::sync::atomic::Ordering;
        let running = self.session.jobs.iter().rev().find(|j| !j.progress.finished.load(Ordering::Relaxed));
        let seq = self.session.active_sequence();
        let frame_us = seq.map(|q| ticks_to_us(q.settings.frame_rate.frame_duration())).unwrap_or(0);
        DesktopState {
            playing: self.playback.playing,
            speed: if self.playback.playing && self.playback.speed.is_finite() { self.playback.speed } else { 0.0 },
            busy: running.is_some(),
            progress: running.map(|j| f64::from(j.progress.fraction().clamp(0.0, 1.0))),
            job: running.map(|j| j.label.clone()),
            project: self.session.project.name.clone(),
            sequence: self.session.state.active_sequence.and_then(|id| self.session.project.item(id)).map(|it| it.name.clone()),
            project_path: self.session.path.clone(),
            position_us: ticks_to_us(self.session.playhead()).max(0),
            duration_us: seq.map(|q| ticks_to_us(q.duration())).unwrap_or(0).max(0),
            frame_us: frame_us.max(0),
            focused,
            media_controls_off: !self.session.prefs.playback.desktop_media_controls,
            global_shortcuts: self.session.prefs.playback.desktop_global_shortcuts,
        }
    }

    /// Once per frame (`logic`): publish the state to the desktop host and run its requests.
    pub(crate) fn desktop_tick(&mut self, ctx: &egui::Context) {
        let Some(mut host) = self.hooks.desktop.take() else { return };
        let now = ctx.input(|i| i.time);
        let finished = self.desktop_jobs.finished(&self.session.jobs, now);
        let focused = self.native_window_focused.unwrap_or(true);
        host.update(ctx, &self.desktop_state(focused), &finished);
        let requests = host.take_requests();
        self.hooks.desktop = Some(host);
        for r in requests {
            self.run_desktop_request(ctx, r);
        }
    }

    /// Run one desktop request as commands (errors go to the status bar, never panic).
    pub fn run_desktop_request(&mut self, ctx: &egui::Context, request: DesktopRequest) {
        match request {
            DesktopRequest::Open(files) => {
                let (projects, media): (Vec<String>, Vec<String>) =
                    files.into_iter().filter(|f| !f.trim().is_empty()).partition(|f| f.to_ascii_lowercase().ends_with(".fcproj"));
                // one project at a time: the last one named wins, as a file manager's double-click.
                // Unsaved changes are never replaced from outside: the user saves or closes first.
                if let Some(p) = projects.last() {
                    if self.session.is_dirty() {
                        let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                        let msg = tlf!("Save or close the current project first, then open {name} again", name = name).to_string();
                        self.toast = Some((msg.clone(), ctx.input(|i| i.time)));
                        self.ui.status = msg;
                    } else {
                        let _ = crate::menus::invoke(self, ctx, "file.open", serde_json::json!({"path": p}));
                    }
                }
                if !media.is_empty() {
                    let _ = crate::menus::invoke(self, ctx, "file.import", serde_json::json!({"paths": media}));
                }
                raise(ctx);
            }
            DesktopRequest::Raise => raise(ctx),
            DesktopRequest::Command(id, params) => {
                let _ = crate::menus::invoke(self, ctx, &id, params);
            }
        }
        ctx.request_repaint();
    }
}

/// Ask the compositor to bring the window forward. On Wayland a window can't take focus by
/// itself: the taskbar entry asks for attention instead (Plasma highlights it).
fn raise(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    fn job(id: u64, label: &str) -> filmcraft_engine::Job {
        filmcraft_engine::Job { id, label: label.into(), progress: Default::default(), result: Default::default() }
    }

    fn report(path: &str) -> filmcraft_export::Report {
        filmcraft_export::Report { path: path.into(), frames: 1, seconds: 1.0, bytes: 1, render_fps: 1.0, extra_files: Vec::new() }
    }

    #[test]
    fn finished_jobs_are_reported_once_with_their_file() {
        let mut w = JobWatch::default();
        let jobs = vec![job(1, "Export"), job(2, "Rendering video previews"), job(3, "Proxies")];
        assert!(w.finished(&jobs, 0.0).is_empty());
        jobs[0].progress.finished.store(true, Ordering::Relaxed);
        *jobs[0].result.lock().unwrap() = Some(Ok(report("/tmp/out.mp4")));
        jobs[1].progress.finished.store(true, Ordering::Relaxed);
        let f = w.finished(&jobs, 42.0);
        assert_eq!(f.len(), 1, "preview renders are not announced: {f:?}");
        assert_eq!(f[0].id, 1);
        assert!(f[0].ok);
        assert_eq!(f[0].path.as_deref(), Some("/tmp/out.mp4"));
        assert_eq!(f[0].seconds, 42.0);
        // never twice
        assert!(w.finished(&jobs, 50.0).is_empty());
        // a cancelled job
        jobs[2].progress.finished.store(true, Ordering::Relaxed);
        jobs[2].progress.cancel.store(true, Ordering::Relaxed);
        let f = w.finished(&jobs, 60.0);
        assert_eq!(f.len(), 1);
        assert!(!f[0].ok && f[0].error.is_none(), "cancelled: {f:?}");
        assert!(!should_notify(&f[0], false), "a cancelled job is never announced");
        // a failed one
        let jobs = vec![job(4, "Export")];
        assert!(w.finished(&jobs, 0.0).is_empty());
        jobs[0].progress.finished.store(true, Ordering::Relaxed);
        *jobs[0].result.lock().unwrap() = Some(Err("disk full".into()));
        let f = w.finished(&jobs, 1.0);
        assert_eq!(f[0].error.as_deref(), Some("disk full"));
        assert!(!f[0].ok);
        assert!(should_notify(&f[0], false));
    }

    #[test]
    fn jobs_already_finished_or_removed_are_not_reported() {
        let mut w = JobWatch::default();
        let done = job(1, "Export");
        done.progress.finished.store(true, Ordering::Relaxed);
        assert!(w.finished(&[done], 0.0).is_empty(), "finished before we watched");
        let running = job(2, "Export");
        assert!(w.finished(std::slice::from_ref(&running), 0.0).is_empty());
        assert!(w.finished(&[], 1.0).is_empty(), "removed while running: forgotten");
        running.progress.finished.store(true, Ordering::Relaxed);
        assert!(w.finished(&[running], 2.0).is_empty(), "re-seen only after it finished");
    }

    #[test]
    fn hostile_times_and_poisoned_locks_never_panic() {
        let mut w = JobWatch::default();
        let j = job(7, "Export");
        assert!(w.finished(std::slice::from_ref(&j), f64::NAN).is_empty());
        j.progress.finished.store(true, Ordering::Relaxed);
        let r = Arc::clone(&j.result);
        let _ = std::thread::spawn(move || {
            let _g = r.lock().unwrap();
            panic!("poison the result lock");
        })
        .join();
        let f = w.finished(&[j], f64::INFINITY);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].seconds, 0.0);
    }

    #[test]
    fn notification_rule() {
        let j = |ok: bool, seconds: f64| FinishedJob { id: 1, label: "Export".into(), ok, path: None, error: None, seconds };
        assert!(should_notify(&j(true, 1.0), false), "background window: always");
        assert!(!should_notify(&j(true, 1.0), true), "short job with the window in front: no");
        assert!(should_notify(&j(true, NOTIFY_AFTER_S), true), "long job: yes");
    }

    #[test]
    fn microsecond_conversions_saturate() {
        assert_eq!(ticks_to_us(Tick(TICKS_PER_SECOND)), 1_000_000);
        assert_eq!(us_to_ticks(1_000_000), Tick(TICKS_PER_SECOND));
        assert!(ticks_to_us(Tick(i64::MAX)) > 0);
        assert!(ticks_to_us(Tick(i64::MIN)) < 0);
        assert_eq!(us_to_ticks(i64::MAX), Tick(i64::MAX));
        assert_eq!(us_to_ticks(i64::MIN), Tick(i64::MIN));
    }

    /// Settings ▸ Playback ▸ Media keys and the desktop's media controls reaches the desktop
    /// host, in both workflows.
    #[test]
    fn the_media_controls_setting_reaches_the_desktop_state() {
        use filmcraft_engine::settings::Workflow;
        for workflow in [Workflow::Premiere, Workflow::Resolve] {
            let mut session = filmcraft_engine::Session::default();
            session.prefs.workflow = workflow;
            let mut app = crate::FilmcraftApp::new(session);
            assert!(!app.desktop_state(true).media_controls_off, "{workflow:?}: on by default");
            app.session.execute("prefs.set", serde_json::json!({"key": "playback.desktopMediaControls", "value": false})).unwrap();
            assert!(app.desktop_state(true).media_controls_off, "{workflow:?}");
        }
    }

    /// Settings ▸ Playback ▸ Global shortcuts reaches the desktop host (off by default), in both
    /// workflows.
    #[test]
    fn the_global_shortcuts_setting_reaches_the_desktop_state() {
        use filmcraft_engine::settings::Workflow;
        for workflow in [Workflow::Premiere, Workflow::Resolve] {
            let mut session = filmcraft_engine::Session::default();
            session.prefs.workflow = workflow;
            let mut app = crate::FilmcraftApp::new(session);
            assert!(!app.desktop_state(true).global_shortcuts, "{workflow:?}: off by default");
            app.session.execute("prefs.set", serde_json::json!({"key": "playback.desktopGlobalShortcuts", "value": true})).unwrap();
            assert!(app.desktop_state(true).global_shortcuts, "{workflow:?}");
        }
    }

    /// A host with fixed devices that records what it was asked to send.
    struct FakeHost {
        phones: Vec<Phone>,
        sent: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl DesktopHost for FakeHost {
        fn update(&mut self, _ctx: &egui::Context, _state: &DesktopState, _finished: &[FinishedJob]) {}
        fn take_requests(&mut self) -> Vec<DesktopRequest> {
            Vec::new()
        }
        fn phones(&self) -> Vec<Phone> {
            self.phones.clone()
        }
        fn send_to_phone(&mut self, device: &str, path: &str) -> Result<(), String> {
            self.sent.lock().unwrap().push((device.into(), path.into()));
            Ok(())
        }
    }

    fn phone(id: &str, name: &str) -> Phone {
        Phone { id: id.into(), name: name.into(), kind: "phone".into() }
    }

    /// `kdeconnect.devices` / `kdeconnect.send` (L2.10) in both workflows: by path or project
    /// item, by device id or name, the only device by default; hostile parameters are errors,
    /// never a panic, and nothing is sent for them.
    #[test]
    fn kdeconnect_commands_send_files_to_a_reachable_device() {
        use filmcraft_engine::settings::Workflow;
        use serde_json::json;
        let dir = std::env::temp_dir().join(format!("filmcraft-kdeconnect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("export one.mp4");
        std::fs::write(&file, b"x").unwrap();
        let file = file.to_string_lossy().into_owned();
        for workflow in [Workflow::Premiere, Workflow::Resolve] {
            let mut session = filmcraft_engine::Session::default();
            session.prefs.workflow = workflow;
            session.execute("file.openDemoProject", json!({})).unwrap();
            let mut p = (*session.project).clone();
            let mut media = p.items.values().find_map(|i| i.as_media()).unwrap().clone();
            media.media = filmcraft_project::MediaRef::File { path: file.clone() };
            let item = p.add_item("Clip", filmcraft_project::Label::Iris, filmcraft_project::ItemKind::Media(media), None);
            let generator = p.items.values().find(|i| i.as_media().is_some_and(|m| !matches!(m.media, filmcraft_project::MediaRef::File { .. }))).map(|i| i.id);
            session.project = Arc::new(p);
            let ctx = egui::Context::default();
            let mut app = crate::FilmcraftApp::new(session);

            // no host (the web, macOS / Windows today): nothing listed, sending is an error
            let r = crate::menus::invoke(&mut app, &ctx, "kdeconnect.devices", json!({})).unwrap();
            assert_eq!(r, json!({"devices": []}), "{workflow:?}");
            assert!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"path": file})).is_err());

            let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
            app.hooks.desktop = Some(Box::new(FakeHost { phones: vec![phone("abc_1", "Pixel")], sent: Arc::clone(&sent) }));
            let r = crate::menus::invoke(&mut app, &ctx, "kdeconnect.devices", json!({})).unwrap();
            assert_eq!(r["devices"][0]["name"], "Pixel", "{workflow:?}");
            crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"path": file})).unwrap();
            crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"item": item.0, "device": "Pixel"})).unwrap();
            assert!(app.ui.status.contains("Pixel"), "{}", app.ui.status);

            let mut hostile = vec![
                json!({}),
                json!({"path": ""}),
                json!({"path": "relative.mp4"}),
                json!({"path": dir.to_string_lossy()}),
                json!({"path": "/no/such/file.mp4"}),
                json!({"path": 7}),
                json!({"item": u64::MAX}),
                json!({"item": -1}),
                json!({"path": file, "device": "nobody"}),
                json!({"path": file, "device": "x".repeat(100_000)}),
                json!({"path": format!("{file}\0")}),
            ];
            if let Some(g) = generator {
                hostile.push(json!({"item": g.0}));
            }
            for params in hostile {
                assert!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", params.clone()).is_err(), "{workflow:?}: {params}");
            }
            assert_eq!(*sent.lock().unwrap(), [("abc_1".to_string(), file.clone()), ("abc_1".to_string(), file.clone())]);

            // several devices: which one must be said
            let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
            let many: Vec<Phone> = (0..1000).map(|n| phone(&format!("d{n}"), &format!("Phone {n}"))).collect();
            app.hooks.desktop = Some(Box::new(FakeHost { phones: many, sent: Arc::clone(&sent) }));
            assert!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"path": file})).is_err());
            let r = crate::menus::invoke(&mut app, &ctx, "kdeconnect.devices", json!({})).unwrap();
            assert_eq!(r["devices"].as_array().map(Vec::len), Some(MAX_PHONES), "capped");
            crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"path": file, "device": "d3"})).unwrap();
            assert!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.send", json!({"path": file, "device": "d999"})).is_err(), "past the cap");
            assert_eq!(sent.lock().unwrap().len(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `card.*` (L2.4): hostile parameters are errors, never a panic; only real camera cards are
    /// offered, at most [`MAX_CARD_OFFERS`]; Ignore in Settings offers nothing; no destination or
    /// no host is an error with a way out.
    #[test]
    fn camera_card_commands_are_checked() {
        use serde_json::json;
        let dir = std::env::temp_dir().join(format!("filmcraft-card-ui-{}", std::process::id()));
        let usb = dir.join("USB");
        std::fs::create_dir_all(&usb).unwrap();
        let ctx = egui::Context::default();
        let mut app = crate::FilmcraftApp::new(filmcraft_engine::Session::default());
        let usb_s = usb.to_string_lossy().into_owned();
        for params in [json!({}), json!({"mount": 1}), json!({"mount": "CARD"}), json!({"mount": "/no/such/card"}), json!({"mount": usb_s})] {
            assert!(crate::menus::invoke(&mut app, &ctx, "card.detected", params.clone()).is_err(), "{params}");
        }
        for n in 0..(MAX_CARD_OFFERS + 3) {
            let card = dir.join(format!("CARD{n}"));
            std::fs::create_dir_all(card.join("DCIM")).unwrap();
            crate::menus::invoke(&mut app, &ctx, "card.detected", json!({"mount": card.to_string_lossy()})).unwrap();
            crate::menus::invoke(&mut app, &ctx, "card.detected", json!({"mount": card.to_string_lossy()})).unwrap();
        }
        assert_eq!(app.ui.cards.len(), MAX_CARD_OFFERS);
        assert!(app.ui.cards.first().is_some_and(|c| c.label == "CARD3" && c.layout == "dcim"), "oldest dropped first");
        let last = dir.join(format!("CARD{}", MAX_CARD_OFFERS + 2)).to_string_lossy().into_owned();
        let e = crate::menus::invoke(&mut app, &ctx, "card.copy", json!({"mount": last})).unwrap_err();
        assert!(e.contains("Copy camera cards to"), "{e}");
        assert!(crate::menus::invoke(&mut app, &ctx, "card.copy", json!({})).is_err());
        assert!(crate::menus::invoke(&mut app, &ctx, "card.eject", json!({"mount": last})).is_err(), "no desktop host");
        let r = crate::menus::invoke(&mut app, &ctx, "card.ejected", json!({"mount": last, "ok": false, "error": "busy\u{7}\n".repeat(500)})).unwrap();
        assert_eq!(r["ok"], false);
        assert!(app.ui.status.chars().count() < 400 && !app.ui.status.chars().any(char::is_control));
        assert_eq!(app.ui.cards.len(), MAX_CARD_OFFERS, "a failed eject keeps the offer");
        crate::menus::invoke(&mut app, &ctx, "card.ejected", json!({"mount": last, "ok": true})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "card.removed", json!({"mount": dir.join("CARD3").to_string_lossy()})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "card.dismiss", json!({"mount": "/not/offered"})).unwrap();
        assert_eq!(app.ui.cards.len(), MAX_CARD_OFFERS - 2);
        app.session.prefs.media.camera_cards = "off".into();
        let r = crate::menus::invoke(&mut app, &ctx, "card.detected", json!({"mount": last})).unwrap();
        assert!(r.get("ignored").is_some(), "{r}");
        assert_eq!(app.ui.cards.len(), MAX_CARD_OFFERS - 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `kdeconnect.received` (L2.10): hostile parameters are errors or ignored, never a panic;
    /// files FilmCraft can't import aren't offered; the waiting list is capped.
    #[test]
    fn files_shared_from_a_phone_are_checked() {
        use serde_json::json;
        let dir = std::env::temp_dir().join(format!("filmcraft-phone-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("notes.pdf");
        std::fs::write(&pdf, b"x").unwrap();
        let ctx = egui::Context::default();
        let mut app = crate::FilmcraftApp::new(filmcraft_engine::Session::default());
        for params in [
            json!({}),
            json!({"path": 7}),
            json!({"path": ""}),
            json!({"path": "a.mp4"}),
            json!({"path": "/no/such.mp4"}),
            json!({"path": dir.to_string_lossy()}),
        ] {
            assert!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.received", params.clone()).is_err(), "{params}");
        }
        let r = crate::menus::invoke(&mut app, &ctx, "kdeconnect.received", json!({"path": pdf.to_string_lossy()})).unwrap();
        assert!(r.get("ignored").is_some(), "{r}");
        for n in 0..(MAX_PHONE_SHARES + 20) {
            let f = dir.join(format!("{n}.mp4"));
            std::fs::write(&f, b"x").unwrap();
            let device = if n == MAX_PHONE_SHARES + 19 { "\u{7}\n".to_string() } else { "d".repeat(10_000) };
            crate::menus::invoke(&mut app, &ctx, "kdeconnect.received", json!({"path": f.to_string_lossy(), "device": device})).unwrap();
        }
        assert_eq!(app.ui.phone_shares.len(), MAX_PHONE_SHARES);
        assert!(app.ui.phone_shares.iter().all(|s| s.device.chars().count() <= 200 && !s.device.chars().any(char::is_control)));
        assert!(app.ui.phone_shares[0].path.ends_with("/20.mp4"), "oldest dropped first");
        let r = crate::menus::invoke(&mut app, &ctx, "kdeconnect.dismissReceived", json!({})).unwrap();
        assert_eq!(r["dismissed"], MAX_PHONE_SHARES);
        assert_eq!(crate::menus::invoke(&mut app, &ctx, "kdeconnect.importReceived", json!({})).unwrap(), json!({"items": []}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
