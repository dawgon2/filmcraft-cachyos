//! KDE Connect (LINUX-PLAN L2.10): send a file from FilmCraft to a paired phone, tablet or
//! computer (the Project panel's Send via KDE Connect, `kdeconnect.send`), and take the files a
//! phone shares with this computer (the share plugin's `shareReceived` signal becomes
//! `kdeconnect.received`: offered for import, put in the Inbox bin or ignored, as Settings ▸
//! Media says).
//!
//! FilmCraft never speaks the KDE Connect protocol (every implementation is GPL): it asks the
//! user's running `kdeconnectd` over the session D-Bus, the way Dolphin's "Send to" menu does.
//! The daemon is never started by FilmCraft (D-Bus activation would launch it on desktops where
//! the user doesn't run it): without it, or without a paired device in reach, there is simply
//! nothing to send to and the menu entry is hidden. GNOME's GSConnect has its own interface and
//! isn't covered.
//!
//! A worker thread with its own connection keeps the list of reachable devices fresh (at start,
//! every [`REFRESH`], and soon after the UI asks) and makes the share calls, so the UI never waits
//! on D-Bus. The daemon shows the transfer and its result in its own notification.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use filmcraft_ui_egui::desktop::{DesktopRequest, MAX_PHONES, Phone};
use zbus::blocking::{Connection, Proxy};

pub(crate) const DEST: &str = "org.kde.kdeconnect";
const DAEMON_PATH: &str = "/modules/kdeconnect";
const DAEMON_IFACE: &str = "org.kde.kdeconnect.daemon";
const DEVICE_IFACE: &str = "org.kde.kdeconnect.device";
const SHARE_IFACE: &str = "org.kde.kdeconnect.device.share";
const SHARE_PLUGIN: &str = "kdeconnect_share";

/// How often the device list is refreshed without being asked.
const REFRESH: Duration = Duration::from_secs(60);
/// The UI asking (an open menu asks every frame) refreshes at most this often.
const NUDGE_EVERY: Duration = Duration::from_secs(5);
/// Longest device id / name kept (KDE Connect's ids are 32–40 characters).
const MAX_ID: usize = 128;
const MAX_NAME: usize = 200;
/// Longest shared-file URL taken (a path is at most 4096 bytes, three per escaped byte).
const MAX_URL: usize = 16 * 1024;

enum Job {
    Refresh,
    Send { device: String, uri: String },
}

/// The host's KDE Connect side: a cached device list and a queue of share calls.
pub struct KdeConnect {
    devices: Arc<Mutex<Vec<Phone>>>,
    tx: Option<Sender<Job>>,
    nudged: Mutex<Option<Instant>>,
}

impl KdeConnect {
    /// Start the worker (it looks for the daemon at once) and the listener for shared files.
    pub fn start(shared: Arc<crate::desktop::Shared>) -> Self {
        let devices: Arc<Mutex<Vec<Phone>>> = Arc::default();
        let names = Arc::clone(&devices);
        let listener = std::thread::Builder::new().name("kdeconnect-share".into()).spawn(move || {
            let run = || {
                if let Err(e) = listen(&shared, &names) {
                    log::info!("KDE Connect: not listening for shared files: {e}");
                }
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
                log::warn!("KDE Connect: listening for shared files stopped");
            }
        });
        if let Err(e) = listener {
            log::info!("KDE Connect: not listening for shared files: {e}");
        }
        let (tx, rx) = channel();
        let list = Arc::clone(&devices);
        let spawned = std::thread::Builder::new().name("kdeconnect".into()).spawn(move || {
            let run = || {
                if let Err(e) = worker(rx, list) {
                    log::info!("KDE Connect unavailable: {e}");
                }
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
                log::warn!("KDE Connect integration stopped");
            }
        });
        let tx = match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::info!("KDE Connect unavailable: {e}");
                None
            }
        };
        Self { devices, tx, nudged: Mutex::new(None) }
    }

    /// The reachable devices last seen; asks for a refresh now and then.
    pub fn phones(&self) -> Vec<Phone> {
        let now = Instant::now();
        let mut nudged = self.nudged.lock().unwrap_or_else(PoisonError::into_inner);
        if nudged.is_none_or(|t| now.saturating_duration_since(t) >= NUDGE_EVERY) {
            *nudged = Some(now);
            if let Some(tx) = &self.tx {
                let _ = tx.send(Job::Refresh);
            }
        }
        drop(nudged);
        self.devices.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Queue sending `path` to `device` (one of [`Self::phones`]).
    pub fn send(&self, device: &str, path: &str) -> Result<(), String> {
        let known = self.devices.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|p| p.id == device);
        if !known {
            return Err(format!("no reachable device {device}"));
        }
        let file = std::path::Path::new(path);
        if !file.is_absolute() || !file.is_file() {
            return Err(format!("{path} is not a file"));
        }
        let uri = crate::desktop::file_uri(file);
        let tx = self.tx.as_ref().ok_or("KDE Connect is unavailable")?;
        tx.send(Job::Send { device: device.into(), uri }).map_err(|_| "KDE Connect is unavailable".to_string())
    }
}

/// A device id that is safe in an object path (KDE Connect uses letters, digits and `_`).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn device_path(id: &str) -> String {
    format!("{DAEMON_PATH}/devices/{id}")
}

fn proxy<'a>(conn: &Connection, path: String, iface: &'a str) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(conn).destination(DEST)?.path(path)?.interface(iface)?.cache_properties(zbus::proxy::CacheProperties::No).build()
}

fn worker(rx: Receiver<Job>, devices: Arc<Mutex<Vec<Phone>>>) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).map_err(|e| e.to_string())?;
    let publish = |list: Vec<Phone>| *devices.lock().unwrap_or_else(PoisonError::into_inner) = list;
    publish(list_devices(&conn, &dbus));
    let mut last = Instant::now();
    loop {
        let job = match rx.recv_timeout(REFRESH.saturating_sub(last.elapsed())) {
            Ok(job) => Some(job),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        match job {
            Some(Job::Send { device, uri }) => {
                if let Err(e) =
                    proxy(&conn, format!("{}/share", device_path(&device)), SHARE_IFACE).and_then(|p| p.call::<_, _, ()>("shareUrl", &(uri.as_str(),)))
                {
                    log::warn!("KDE Connect: sending a file to {device} failed: {e}");
                }
            }
            Some(Job::Refresh) | None => {
                publish(list_devices(&conn, &dbus));
                last = Instant::now();
            }
        }
    }
}

/// Wait for the share plugin's `shareReceived(url)` from the KDE Connect daemon (only from the
/// owner of its bus name, from any device) and hand each file to the UI. Blocks for the app's
/// life; nothing arrives while KDE Connect isn't running.
fn listen(shared: &crate::desktop::Shared, devices: &Mutex<Vec<Phone>>) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DEST)
        .and_then(|b| b.interface(SHARE_IFACE))
        .and_then(|b| b.member("shareReceived"))
        .map_err(|e| e.to_string())?
        .build();
    let messages = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)).map_err(|e| e.to_string())?;
    for msg in messages {
        if shared.is_closing() {
            break;
        }
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let Some(id) = header.path().and_then(|p| share_device(p.as_str())) else { continue };
        let Ok(url) = msg.body().deserialize::<String>() else { continue };
        if let Some(r) = received(id, &url, devices) {
            shared.push(r);
        }
    }
    Ok(())
}

/// The device id in a share plugin's object path (`/modules/kdeconnect/devices/<id>/share`).
fn share_device(path: &str) -> Option<&str> {
    let id = path.strip_prefix(DAEMON_PATH)?.strip_prefix("/devices/")?.strip_suffix("/share")?;
    valid_id(id).then_some(id)
}

/// The request for a file `id` shared (`url`), named after the device when it's listed.
fn received(id: &str, url: &str, devices: &Mutex<Vec<Phone>>) -> Option<DesktopRequest> {
    let path = local_path(url)?;
    let device = devices.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|p| p.id == id).map(|p| p.name.clone());
    let device = device.unwrap_or_else(|| id.to_string());
    Some(DesktopRequest::Command("kdeconnect.received".into(), serde_json::json!({"path": path, "device": device})))
}

/// The path of a local `file://` URL (`file:///…` or `file://localhost/…`); anything else, a
/// relative path, a NUL or invalid UTF-8 is refused.
pub fn local_path(url: &str) -> Option<String> {
    if url.len() > MAX_URL {
        return None;
    }
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = crate::recent_files::percent_decode(rest);
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The paired, reachable devices that accept files; empty when the daemon isn't running.
fn list_devices(conn: &Connection, dbus: &zbus::blocking::fdo::DBusProxy<'_>) -> Vec<Phone> {
    let running = zbus::names::BusName::try_from(DEST).ok().and_then(|name| dbus.name_has_owner(name).ok()).unwrap_or(false);
    if !running {
        return Vec::new();
    }
    let ids: Vec<String> = match proxy(conn, DAEMON_PATH.into(), DAEMON_IFACE).and_then(|p| p.call("devices", &(true, true))) {
        Ok(ids) => ids,
        Err(e) => {
            log::info!("KDE Connect: no device list: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for id in ids.into_iter().filter(|id| valid_id(id)).take(MAX_PHONES) {
        let Ok(dev) = proxy(conn, device_path(&id), DEVICE_IFACE) else { continue };
        if !dev.call::<_, _, bool>("hasPlugin", &(SHARE_PLUGIN,)).unwrap_or(false) {
            continue;
        }
        let name: String = dev.get_property("name").unwrap_or_default();
        let kind: String = dev.get_property("type").unwrap_or_default();
        let name = if name.trim().is_empty() { id.clone() } else { name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect() };
        let kind = kind.chars().filter(char::is_ascii_alphanumeric).take(32).collect();
        out.push(Phone { id, name, kind });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_are_checked_before_they_reach_an_object_path() {
        assert!(valid_id("d4ed104f18b44317a16f2c10ce79933d"));
        assert!(valid_id("154bd09a_ac68_4b02_a042_a38d66767d55"));
        for bad in ["", "a/b", "../x", "a b", "é", "a-b", &"x".repeat(MAX_ID + 1)] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert!(zbus::zvariant::ObjectPath::try_from(device_path("d4ed_1")).is_ok());
    }

    #[test]
    fn shared_urls_and_paths_are_checked() {
        assert_eq!(local_path("file:///home/u/Downloads/cut%20%231.mp4").as_deref(), Some("/home/u/Downloads/cut #1.mp4"));
        assert_eq!(local_path("file://localhost/a.mov").as_deref(), Some("/a.mov"));
        for bad in [
            "",
            "https://x/a.mp4",
            "file://host/a.mp4",
            "file://a.mp4",
            "/a.mp4",
            "file:///a%00b",
            "file:///%ff%fe",
            &format!("file:///{}", "a".repeat(MAX_URL)),
        ] {
            assert_eq!(local_path(bad), None, "{bad}");
        }
        assert_eq!(share_device("/modules/kdeconnect/devices/abc_1/share"), Some("abc_1"));
        for bad in
            ["", "/modules/kdeconnect/devices/abc/sftp", "/modules/kdeconnect/devices/a/b/share", "/modules/kdeconnect/devices//share", "/x/devices/abc/share"]
        {
            assert_eq!(share_device(bad), None, "{bad}");
        }
        let devices = Mutex::new(vec![Phone { id: "abc".into(), name: "Pixel".into(), kind: "phone".into() }]);
        let name = |r: Option<DesktopRequest>| match r {
            Some(DesktopRequest::Command(id, p)) if id == "kdeconnect.received" => p["device"].as_str().map(str::to_string),
            _ => None,
        };
        assert_eq!(name(received("abc", "file:///a.mp4", &devices)).as_deref(), Some("Pixel"));
        assert_eq!(name(received("zzz", "file:///a.mp4", &devices)).as_deref(), Some("zzz"), "not listed: the id");
        assert!(received("abc", "content://a", &devices).is_none());
    }

    /// A stand-in daemon with two devices (one without the share plugin) that records shares.
    struct MockDaemon {
        ids: Vec<String>,
    }

    #[zbus::interface(name = "org.kde.kdeconnect.daemon")]
    impl MockDaemon {
        #[zbus(name = "devices")]
        fn devices(&self, only_reachable: bool, only_paired: bool) -> Vec<String> {
            assert!(only_reachable && only_paired, "asks for paired, reachable devices");
            self.ids.clone()
        }
    }

    struct MockDevice {
        name: String,
        share: bool,
    }

    #[zbus::interface(name = "org.kde.kdeconnect.device")]
    impl MockDevice {
        #[zbus(name = "hasPlugin")]
        fn has_plugin(&self, plugin: String) -> bool {
            self.share && plugin == SHARE_PLUGIN
        }
        #[zbus(property, name = "name")]
        fn name(&self) -> String {
            self.name.clone()
        }
        #[zbus(property, name = "type")]
        fn kind(&self) -> String {
            "phone".into()
        }
    }

    struct MockShare {
        shared: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.kde.kdeconnect.device.share")]
    impl MockShare {
        #[zbus(name = "shareUrl")]
        fn share_url(&self, url: String) {
            self.shared.lock().unwrap().push(url);
        }
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// On a private session bus (`dbus-run-session`; skipped without one): no daemon, no devices;
    /// with one, the devices that accept files are listed (hostile ids and names dropped or
    /// cleaned) and a send reaches the device's share plugin as a `file://` URI.
    #[test]
    fn sends_through_a_kdeconnect_daemon() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("skipped: no session bus");
            return;
        }
        let Ok(daemon) = Connection::session() else {
            eprintln!("skipped: session bus not reachable");
            return;
        };
        let dbus = zbus::blocking::fdo::DBusProxy::new(&daemon).unwrap();
        if dbus.name_has_owner(zbus::names::BusName::try_from(DEST).unwrap()).unwrap() {
            eprintln!("skipped: a real KDE Connect is on this bus (run under dbus-run-session)");
            return;
        }
        let queue = Arc::new(crate::desktop::Shared::default());
        let kc = KdeConnect::start(Arc::clone(&queue));
        assert!(kc.phones().is_empty(), "no daemon: nothing");

        let dir = std::env::temp_dir().join(format!("filmcraft-kdeconnect-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cut #1.mp4");
        std::fs::write(&file, b"x").unwrap();
        let file = file.to_string_lossy().into_owned();
        assert!(kc.send("abc", &file).is_err(), "unknown device");

        let shared = Arc::new(Mutex::new(Vec::new()));
        let ids = vec!["abc".to_string(), "noshare".to_string(), "../evil".to_string(), "x".repeat(10_000)];
        let server = daemon.object_server();
        server.at(DAEMON_PATH, MockDaemon { ids }).unwrap();
        server.at(device_path("abc"), MockDevice { name: "Pixel\n\u{7}8".into(), share: true }).unwrap();
        server.at(device_path("noshare"), MockDevice { name: "Watch".into(), share: false }).unwrap();
        server.at(format!("{}/share", device_path("abc")), MockShare { shared: Arc::clone(&shared) }).unwrap();
        daemon.request_name(DEST).unwrap();

        // the first ask refreshes at once; later asks within NUDGE_EVERY only read the cache
        let mut phones = Vec::new();
        wait_for("the device list", || {
            *kc.nudged.lock().unwrap() = None;
            phones = kc.phones();
            !phones.is_empty()
        });
        assert_eq!(phones, [Phone { id: "abc".into(), name: "Pixel8".into(), kind: "phone".into() }]);

        kc.send("abc", &file).unwrap();
        assert!(kc.send("abc", &dir.to_string_lossy()).is_err(), "a folder");
        assert!(kc.send("abc", "relative.mp4").is_err());
        assert!(kc.send("noshare", &file).is_err(), "not listed");
        wait_for("the share", || !shared.lock().unwrap().is_empty());
        let urls = shared.lock().unwrap().clone();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].starts_with("file:///") && urls[0].ends_with("/cut%20%231.mp4"), "{urls:?}");

        // a phone shares a file: only the daemon's signal reaches the UI, named after the device
        let impostor = Connection::session().unwrap();
        let share_path = format!("{}/share", device_path("abc"));
        impostor.emit_signal(None::<zbus::names::BusName<'_>>, share_path.as_str(), SHARE_IFACE, "shareReceived", &("file:///evil.mp4",)).unwrap();
        daemon
            .emit_signal(None::<zbus::names::BusName<'_>>, share_path.as_str(), SHARE_IFACE, "shareReceived", &("file:///home/u/Downloads/clip%201.mp4",))
            .unwrap();
        let mut got = Vec::new();
        wait_for("the shared file", || {
            got.extend(queue.take());
            !got.is_empty()
        });
        std::thread::sleep(Duration::from_millis(200));
        got.extend(queue.take());
        assert_eq!(
            got,
            [DesktopRequest::Command("kdeconnect.received".into(), serde_json::json!({"path": "/home/u/Downloads/clip 1.mp4", "device": "Pixel8"}))]
        );

        // the daemon goes away: the list empties on the next refresh
        daemon.release_name(DEST).unwrap();
        wait_for("the list to empty", || {
            *kc.nudged.lock().unwrap() = None;
            kc.phones().is_empty()
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
