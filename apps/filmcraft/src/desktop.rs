//! Desktop shell integration on Linux (LINUX-PLAN L1.7): export / render progress on the taskbar
//! entry, a notification with "Show in Folder" when a job finishes, keeping the screen awake while
//! playing and the computer from suspending while a job runs, and a single running instance per
//! data folder (a second launch hands its files over and exits; [`single_instance`]).
//!
//! Everything goes over the session D-Bus with zbus, through freedesktop interfaces every major
//! desktop implements, so it is tailored for KDE Plasma but not KDE-only:
//! - progress: the `com.canonical.Unity.LauncherEntry` `Update` signal (Plasma's task manager,
//!   Dock-style launchers; GNOME needs an extension and ignores it otherwise);
//! - notifications: `org.freedesktop.Notifications` (Plasma, GNOME, mako, dunst…), with a
//!   "Show in Folder" action answered through `org.freedesktop.FileManager1.ShowItems`
//!   (Dolphin, Nautilus, Nemo…; `xdg-open` on the folder otherwise);
//! - inhibit: the XDG portal's `org.freedesktop.portal.Inhibit` (idle while playing or a job
//!   runs, suspend while a job runs), else `org.freedesktop.ScreenSaver` for idle.
//!
//! When a service is missing the call fails quietly (one `info` line in the log) and the app
//! carries on. The UI thread never waits on D-Bus: [`LinuxDesktop::update`] only compares the
//! state with what was last sent and queues messages for the worker thread.
//!
//! [`single_instance`]: crate::single_instance

#[cfg(target_os = "linux")]
pub use linux::*;

/// The desktop file id (without `.desktop`) the taskbar and notifications refer to.
pub const DESKTOP_ID: &str = "ai.storyteller.filmcraft";

/// `file://` URI of an absolute path, percent-encoding everything but unreserved characters and
/// `/` (RFC 3986; what FileManager1 and `text/uri-list` expect).
pub fn file_uri(path: &std::path::Path) -> String {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut out = String::with_capacity(bytes.len().saturating_add(8));
    out.push_str("file://");
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// What the inhibit request should cover: (idle, suspend).
pub fn wanted_inhibit(state: &filmcraft_ui_egui::desktop::DesktopState) -> (bool, bool) {
    (state.playing || state.busy, state.busy)
}

/// Progress as the taskbar sees it: whole percent steps (no D-Bus traffic for tiny changes).
pub fn taskbar_progress(state: &filmcraft_ui_egui::desktop::DesktopState) -> Option<u8> {
    state.progress.filter(|p| p.is_finite()).map(|p| (p.clamp(0.0, 1.0) * 100.0).round() as u8)
}

/// The notification text for a finished job: (summary, body).
pub fn notification_text(job: &filmcraft_ui_egui::desktop::FinishedJob) -> (String, String) {
    use filmcraft_ui_egui::i18n::t;
    let label = t(&job.label).to_string();
    let file = job.path.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
    match (&job.error, file) {
        (Some(e), _) => (format!("{label}: {}", t("Failed")), truncate(e, 300)),
        (None, Some(f)) => (format!("{label}: {}", t("Finished")), f),
        (None, None) => (format!("{label}: {}", t("Finished")), String::new()),
    }
}

/// At most `max` characters (an error text from a decoder can be long).
fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use filmcraft_ui_egui::desktop::DesktopRequest;

/// Requests from D-Bus threads for the UI, the context that wakes it, and the single-instance
/// name kept while the app runs.
#[derive(Default)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Shared {
    requests: Mutex<Vec<DesktopRequest>>,
    wake: OnceLock<egui::Context>,
    holder: Mutex<Option<crate::single_instance::Holder>>,
    closing: AtomicBool,
}

/// Requests waiting at most this many (a flood of second launches can't grow memory).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MAX_PENDING: usize = 64;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Shared {
    /// Queue a request for the UI and wake it.
    pub fn push(&self, r: DesktopRequest) {
        let mut q = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        if q.len() < MAX_PENDING {
            q.push(r);
        }
        drop(q);
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }
    pub(crate) fn take(&self) -> Vec<DesktopRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner))
    }
    /// Keep the single-instance name until [`Shared::close`].
    pub fn keep(&self, holder: Option<crate::single_instance::Holder>) {
        *self.holder.lock().unwrap_or_else(PoisonError::into_inner) = holder;
    }
    /// The app is closing: refuse further hand-offs (a later launch then starts its own window
    /// instead of losing its files) and release the name.
    pub fn close(&self) {
        self.closing.store(true, Ordering::Relaxed);
        drop(self.holder.lock().unwrap_or_else(PoisonError::into_inner).take());
    }
    /// See [`Shared::close`].
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Relaxed)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::{HashMap, VecDeque};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex, PoisonError};

    use filmcraft_ui_egui::desktop::{DesktopHost, DesktopRequest, DesktopState, FinishedJob, should_notify};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::{DESKTOP_ID, Shared, file_uri, notification_text, taskbar_progress, wanted_inhibit};

    /// How long closing the app waits for the worker to clear the taskbar and release the inhibitor.
    const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

    enum Msg {
        Inhibit {
            idle: bool,
            suspend: bool,
        },
        Progress(Option<u8>),
        Notify(FinishedJob),
        /// A project was opened or saved at this path (recent-files list).
        Recent(String),
        /// The app is closing: clear the taskbar progress, release the inhibitor, then answer.
        Shutdown(Sender<()>),
        /// Playback, sequence or project changed, or the playhead jumped (`true`: emit
        /// `Seeked`) (MPRIS player, L2.3).
        Mpris(Box<DesktopState>, bool),
    }

    /// The fields MPRIS announces (position is read on demand, not announced).
    fn mpris_key(s: &DesktopState) -> (bool, u64, Option<&str>, &str, i64, bool) {
        (s.playing, s.speed.to_bits(), s.sequence.as_deref(), s.project.as_str(), s.duration_us, s.media_controls_off)
    }

    /// The host hook (`HostHooks::desktop`).
    pub struct LinuxDesktop {
        tx: Sender<Msg>,
        shared: Arc<Shared>,
        inhibit: (bool, bool),
        progress: Option<u8>,
        project_path: Option<String>,
        /// What the MPRIS player reads its properties from.
        snapshot: crate::mpris::Snapshot,
        mpris_sent: Option<DesktopState>,
        /// When `mpris_sent` was sent (where clients think the playhead is now).
        mpris_sent_at: std::time::Instant,
        /// Settings ▸ Playback ▸ Global shortcuts (L1.8): its own thread, started when first on.
        global: crate::global_shortcuts::GlobalShortcuts,
        global_on: bool,
        /// Send via KDE Connect (L2.10): device list and share calls on its own thread.
        kdeconnect: crate::kdeconnect::KdeConnect,
    }

    impl LinuxDesktop {
        /// Start the worker on the session bus. `None` without a session bus (then nothing is
        /// published; requests from the single-instance service still arrive through `shared`).
        pub fn start(shared: Arc<Shared>) -> Self {
            let (tx, rx) = channel();
            let snapshot: crate::mpris::Snapshot = Arc::default();
            // Settings ▸ Playback ▸ Media keys and the desktop's media controls switches the player
            // on and off through `DesktopState::media_controls_off`
            let mpris = Some(crate::mpris::Mpris::new(Arc::clone(&shared), Arc::clone(&snapshot)));
            let spawned = std::thread::Builder::new().name("desktop-dbus".into()).spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(rx, mpris))).is_err() {
                    log::warn!("desktop integration stopped (taskbar progress, notifications, inhibit)");
                }
            });
            if let Err(e) = spawned {
                log::info!("desktop integration unavailable: {e}");
            }
            let this = Self {
                tx,
                shared: Arc::clone(&shared),
                inhibit: (false, false),
                progress: None,
                project_path: None,
                snapshot,
                mpris_sent: None,
                mpris_sent_at: std::time::Instant::now(),
                global: crate::global_shortcuts::GlobalShortcuts::new(Arc::clone(&shared)),
                global_on: false,
                kdeconnect: crate::kdeconnect::KdeConnect::start(Arc::clone(&shared)),
            };
            // camera cards (L2.4): watched for the app's life
            crate::cards::watch(shared);
            this
        }
    }

    impl DesktopHost for LinuxDesktop {
        fn update(&mut self, ctx: &egui::Context, state: &DesktopState, finished: &[FinishedJob]) {
            let _ = self.shared.wake.set(ctx.clone());
            let inhibit = wanted_inhibit(state);
            if inhibit != self.inhibit {
                self.inhibit = inhibit;
                let _ = self.tx.send(Msg::Inhibit { idle: inhibit.0, suspend: inhibit.1 });
            }
            let progress = taskbar_progress(state);
            if progress != self.progress {
                self.progress = progress;
                let _ = self.tx.send(Msg::Progress(progress));
            }
            if state.project_path != self.project_path {
                self.project_path = state.project_path.clone();
                if let Some(p) = &state.project_path {
                    let _ = self.tx.send(Msg::Recent(p.clone()));
                }
            }
            {
                let mut snap = self.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
                if *snap != *state {
                    *snap = state.clone();
                }
            }
            let elapsed_us = i64::try_from(self.mpris_sent_at.elapsed().as_micros()).unwrap_or(i64::MAX);
            let seeked = self.mpris_sent.as_ref().is_some_and(|m| crate::mpris::jumped(m, elapsed_us, state));
            if seeked || self.mpris_sent.as_ref().is_none_or(|m| mpris_key(m) != mpris_key(state)) {
                self.mpris_sent = Some(state.clone());
                self.mpris_sent_at = std::time::Instant::now();
                let _ = self.tx.send(Msg::Mpris(Box::new(state.clone()), seeked));
            }
            if state.global_shortcuts != self.global_on {
                self.global_on = state.global_shortcuts;
                self.global.set(state.global_shortcuts);
            }
            for job in finished.iter().filter(|j| should_notify(j, state.focused)) {
                let _ = self.tx.send(Msg::Notify(job.clone()));
            }
        }

        fn take_requests(&mut self) -> Vec<DesktopRequest> {
            self.shared.take()
        }

        fn phones(&self) -> Vec<filmcraft_ui_egui::desktop::Phone> {
            self.kdeconnect.phones()
        }

        fn send_to_phone(&mut self, device: &str, path: &str) -> Result<(), String> {
            self.kdeconnect.send(device, path)
        }

        fn eject_card(&mut self, mount: &str) -> Result<(), String> {
            crate::cards::eject(Arc::clone(&self.shared), mount.to_string());
            Ok(())
        }

        fn shutdown(&mut self) {
            self.shared.close();
            // the portal closes the session itself when the app disconnects; this frees the keys
            // at once
            self.global.set(false);
            let (done, wait) = channel();
            if self.tx.send(Msg::Shutdown(done)).is_ok() {
                let _ = wait.recv_timeout(SHUTDOWN_WAIT);
            }
        }
    }

    /// The application URIs the taskbar entry may be known by: ours, and the desktop file that
    /// launched us when a launcher says so (the fork's own `filmcraft-fork.desktop`).
    fn launcher_uris() -> Vec<String> {
        let mut uris = vec![format!("application://{DESKTOP_ID}.desktop")];
        // only when the launcher started *this* process (the variable is inherited by children)
        let ours = std::env::var("GIO_LAUNCHED_DESKTOP_FILE_PID").ok().and_then(|p| p.trim().parse::<u32>().ok()) == Some(std::process::id());
        if let Some(name) = std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE")
            .filter(|_| ours)
            .map(std::path::PathBuf::from)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .filter(|n| n.ends_with(".desktop") && !n.contains(char::is_whitespace))
        {
            let uri = format!("application://{name}");
            if !uris.contains(&uri) {
                uris.push(uri);
            }
        }
        uris
    }

    /// Notifications whose "Show in Folder" may still be clicked: id → file (newest last).
    type Shown = Arc<Mutex<VecDeque<(u32, String)>>>;
    const MAX_SHOWN: usize = 32;

    fn worker(rx: Receiver<Msg>, mut mpris: Option<crate::mpris::Mpris>) {
        let conn = match Connection::session() {
            Ok(c) => c,
            Err(e) => {
                log::info!("no session D-Bus: taskbar progress, notifications and inhibit are off ({e})");
                // drain so senders never block (they don't: the channel is unbounded) and exit
                return;
            }
        };
        let shown: Shown = Arc::default();
        listen_for_actions(&conn, Arc::clone(&shown));
        let uris = launcher_uris();
        let mut inhibit = Inhibitor::default();
        let mut warned = WarnOnce::default();
        let progress = |p: Option<u8>, warned: &mut WarnOnce| {
            for uri in &uris {
                let mut props: HashMap<&str, Value<'_>> = HashMap::new();
                props.insert("progress", Value::F64(f64::from(p.unwrap_or(0)) / 100.0));
                props.insert("progress-visible", Value::Bool(p.is_some()));
                if let Err(e) =
                    conn.emit_signal(None::<&str>, "/ai/storyteller/filmcraft/launcher", "com.canonical.Unity.LauncherEntry", "Update", &(uri.as_str(), props))
                {
                    warned.once("progress", &e);
                }
            }
        };
        let mut shown_progress = false;
        for msg in rx {
            match msg {
                Msg::Inhibit { idle, suspend } => inhibit.set(&conn, idle, suspend, &mut warned),
                Msg::Progress(p) => {
                    shown_progress = p.is_some();
                    progress(p, &mut warned);
                }
                Msg::Mpris(state, seeked) => {
                    if let Some(m) = mpris.as_mut()
                        && let Err(e) = m.update(&state, seeked)
                    {
                        warned.once("media controls (MPRIS)", &e);
                    }
                }
                Msg::Shutdown(done) => {
                    if let Some(m) = mpris.as_mut() {
                        m.release();
                    }
                    if shown_progress {
                        progress(None, &mut warned);
                    }
                    inhibit.set(&conn, false, false, &mut warned);
                    let _ = done.send(());
                    return;
                }
                Msg::Notify(job) => match notify(&conn, &job) {
                    Ok(id) => {
                        if let Some(path) = job.path.filter(|_| job.ok) {
                            let mut s = shown.lock().unwrap_or_else(PoisonError::into_inner);
                            s.push_back((id, path));
                            while s.len() > MAX_SHOWN {
                                s.pop_front();
                            }
                        }
                    }
                    Err(e) => warned.once("notifications", &e),
                },
                Msg::Recent(path) => {
                    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
                    if let Err(e) = crate::recent_files::add(std::path::Path::new(&path), &now) {
                        warned.once("recent files", &e);
                    }
                }
            }
        }
        // the app is closing: release the inhibitor
        inhibit.set(&conn, false, false, &mut warned);
    }

    /// One `info` line per service that doesn't answer.
    #[derive(Default)]
    struct WarnOnce(Vec<&'static str>);
    impl WarnOnce {
        fn once(&mut self, what: &'static str, e: &dyn std::fmt::Display) {
            if !self.0.contains(&what) {
                self.0.push(what);
                log::info!("desktop {what} unavailable: {e}");
            }
        }
    }

    fn notify(conn: &Connection, job: &FinishedJob) -> zbus::Result<u32> {
        use filmcraft_ui_egui::i18n::t;
        let proxy = Proxy::new(conn, "org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications")?;
        let (summary, body) = notification_text(job);
        let show = t("Show in Folder").to_string();
        let actions: Vec<&str> = if job.ok && job.path.is_some() { vec!["default", show.as_str(), "reveal", show.as_str()] } else { Vec::new() };
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(DESKTOP_ID));
        hints.insert("urgency", Value::U8(if job.ok { 1 } else { 2 }));
        proxy.call("Notify", &("FilmCraft", 0u32, DESKTOP_ID, summary.as_str(), body.as_str(), actions, hints, -1i32))
    }

    /// Follow `ActionInvoked` on its own thread: "Show in Folder" reveals the exported file.
    fn listen_for_actions(conn: &Connection, shown: Shown) {
        let Ok(proxy) = Proxy::new(conn, "org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications") else { return };
        let Ok(signals) = proxy.receive_signal("ActionInvoked") else { return };
        let conn = conn.clone();
        let _ = std::thread::Builder::new().name("desktop-actions".into()).spawn(move || {
            let _proxy = proxy;
            let run = || {
                for m in signals {
                    let Ok((id, action)) = m.body().deserialize::<(u32, String)>() else { continue };
                    if action != "default" && action != "reveal" {
                        continue;
                    }
                    let path = shown.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone());
                    if let Some(p) = path {
                        let _ = reveal_with(&conn, &p);
                    }
                }
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        });
    }

    /// Select `path` in the file manager (`org.freedesktop.FileManager1.ShowItems`), else open
    /// its folder with `xdg-open`. Runs on its own thread: the caller (the UI) never waits on
    /// D-Bus; failures are logged.
    pub fn reveal(path: &str) -> Result<(), String> {
        let shown_path = path.to_string();
        let path = path.to_string();
        std::thread::Builder::new()
            .name("reveal".into())
            .spawn(move || {
                let run = || {
                    let shown = zbus::blocking::connection::Builder::session()
                        .and_then(|b| b.method_timeout(std::time::Duration::from_secs(5)).build())
                        .map_err(|e| e.to_string())
                        .and_then(|c| reveal_with(&c, &path));
                    if shown.is_ok() {
                        return;
                    }
                    let p = std::path::Path::new(&path);
                    if let Err(e) = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn() {
                        log::warn!("can't show {path}: {e}");
                    }
                };
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
            })
            .map(|_| ())
            .map_err(|e| format!("can't show {shown_path}: {e}"))
    }

    fn reveal_with(conn: &Connection, path: &str) -> Result<(), String> {
        let p = std::path::Path::new(path);
        if !p.is_absolute() {
            return Err(format!("not an absolute path: {path}"));
        }
        let proxy =
            Proxy::new(conn, "org.freedesktop.FileManager1", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1").map_err(|e| e.to_string())?;
        proxy.call::<_, _, ()>("ShowItems", &(vec![file_uri(p)], "")).map_err(|e| e.to_string())
    }

    /// The current inhibit request: a portal request handle, or a ScreenSaver cookie.
    #[derive(Default)]
    struct Inhibitor {
        portal: Option<OwnedObjectPath>,
        screensaver: Option<u32>,
    }

    impl Inhibitor {
        fn set(&mut self, conn: &Connection, idle: bool, suspend: bool, warned: &mut WarnOnce) {
            self.release(conn);
            if !idle && !suspend {
                return;
            }
            // portal flags: 4 suspend, 8 idle
            let flags = if suspend { 4u32 } else { 0 } | if idle { 8 } else { 0 };
            let reason = filmcraft_ui_egui::i18n::t(if suspend { "Exporting" } else { "Playing" }).to_string();
            let portal =
                Proxy::new(conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Inhibit").and_then(|p| {
                    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
                    options.insert("reason", Value::from(reason.as_str()));
                    p.call::<_, _, OwnedObjectPath>("Inhibit", &("", flags, options))
                });
            match portal {
                Ok(handle) => self.portal = Some(handle),
                Err(e) => {
                    // no portal: the screensaver interface keeps the screen awake (not suspend)
                    let cookie = Proxy::new(conn, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver")
                        .and_then(|p| p.call::<_, _, u32>("Inhibit", &("FilmCraft", reason.as_str())));
                    match cookie {
                        Ok(c) => self.screensaver = Some(c),
                        Err(e2) => warned.once("inhibit", &format!("{e}; {e2}")),
                    }
                }
            }
        }

        fn release(&mut self, conn: &Connection) {
            if let Some(handle) = self.portal.take() {
                let _ = Proxy::new(conn, "org.freedesktop.portal.Desktop", handle.as_str(), "org.freedesktop.portal.Request")
                    .and_then(|p| p.call::<_, _, ()>("Close", &()));
            }
            if let Some(cookie) = self.screensaver.take() {
                let _ = Proxy::new(conn, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver")
                    .and_then(|p| p.call::<_, _, ()>("UnInhibit", &(cookie,)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_ui_egui::desktop::{DesktopState, FinishedJob};

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(file_uri(std::path::Path::new("/home/u/My Film #1/a.mov")), "file:///home/u/My%20Film%20%231/a.mov");
        assert_eq!(file_uri(std::path::Path::new("/tmp/é")), "file:///tmp/%C3%A9");
        assert_eq!(file_uri(std::path::Path::new("")), "file://");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let p = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/x/\xff\n"));
            assert_eq!(file_uri(p), "file:///x/%FF%0A");
        }
    }

    #[test]
    fn inhibit_and_progress_follow_the_state() {
        let mut s = DesktopState::default();
        assert_eq!(wanted_inhibit(&s), (false, false));
        s.playing = true;
        assert_eq!(wanted_inhibit(&s), (true, false), "playing keeps the screen on");
        s.playing = false;
        s.busy = true;
        assert_eq!(wanted_inhibit(&s), (true, true), "a job keeps the screen on and the PC awake");
        assert_eq!(taskbar_progress(&s), None);
        for (p, want) in [(0.0, 0), (0.004, 0), (0.5, 50), (1.0, 100), (7.0, 100), (-1.0, 0)] {
            s.progress = Some(p);
            assert_eq!(taskbar_progress(&s), Some(want), "{p}");
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            s.progress = Some(bad);
            assert_eq!(taskbar_progress(&s), None);
        }
    }

    #[test]
    fn notification_texts() {
        let job = |ok, path: Option<&str>, error: Option<&str>| FinishedJob {
            id: 1,
            label: "Export".into(),
            ok,
            path: path.map(Into::into),
            error: error.map(Into::into),
            seconds: 40.0,
        };
        let (s, b) = notification_text(&job(true, Some("/v/out.mp4"), None));
        assert!(s.starts_with("Export"), "{s}");
        assert_eq!(b, "out.mp4");
        let long = "e".repeat(5000);
        let (_, b) = notification_text(&job(false, None, Some(&long)));
        assert!(b.chars().count() <= 301, "error text is capped");
        let (_, b) = notification_text(&job(true, Some("/"), None));
        assert_eq!(b, "");
    }
}
