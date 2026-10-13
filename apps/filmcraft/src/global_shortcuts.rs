//! Global shortcuts (LINUX-PLAN L1.8): Play / Stop and Add Marker keys that work while another
//! window has focus (a script, notes, a browser with the client's comments), through the XDG
//! desktop portal's `org.freedesktop.portal.GlobalShortcuts` (KDE Plasma 6, GNOME ≥ 48, and
//! other portals that implement it).
//!
//! Off by default (Settings ▸ Playback ▸ Global shortcuts). Switched on, a session is created and
//! the two shortcuts are bound; the desktop asks the user once which keys to use (FilmCraft
//! suggests Ctrl+Alt+Shift+Space and Ctrl+Alt+Shift+M, which nothing else uses by default).
//! An activation queues the same command the keyboard runs (`playback.toggle` /
//! `markers.add` on the Program monitor). Switched off, the session is closed and the keys are
//! free again. Without the portal the switch does nothing (one `info` line in the log).
//!
//! The portal answers through `Request.Response` signals, possibly only after the user has
//! answered a dialog, so the session lives on its own thread and connection; the UI and the
//! desktop worker never wait on it.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_ui_egui::desktop::DesktopRequest;
use serde_json::json;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::desktop::Shared;

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// The shortcuts: (id, label for the desktop's dialog, suggested keys in the XDG shortcuts
/// format).
pub const SHORTCUTS: [(&str, &str, &str); 2] = [("play-stop", "Play/Stop", "CTRL+ALT+SHIFT+space"), ("add-marker", "Add Marker", "CTRL+ALT+SHIFT+m")];

/// The request an activated shortcut turns into (`None`: not one of ours).
pub fn request_for(shortcut: &str) -> Option<DesktopRequest> {
    let program = json!({"monitor": "program"});
    match shortcut {
        "play-stop" => Some(DesktopRequest::Command("playback.toggle".into(), program)),
        "add-marker" => Some(DesktopRequest::Command("markers.add".into(), json!({"target": "program"}))),
        _ => None,
    }
}

/// The object path the portal uses for a request or session (`kind`) created by the connection
/// `unique` (`:1.42`) with `token` (the portal spec's handle-token rule).
pub fn portal_path(kind: &str, unique: &str, token: &str) -> String {
    let sender: String = unique.trim_start_matches(':').chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let token: String = token.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    format!("{PATH}/{kind}/{sender}/{token}")
}

/// The global-shortcuts switch: starts the session thread the first time it is switched on.
pub struct GlobalShortcuts {
    shared: Arc<Shared>,
    ctl: Option<Sender<bool>>,
}

impl GlobalShortcuts {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared, ctl: None }
    }

    /// Follow Settings ▸ Playback ▸ Global shortcuts.
    pub fn set(&mut self, on: bool) {
        if self.ctl.is_none() {
            if !on {
                return;
            }
            let (tx, rx) = channel();
            let shared = Arc::clone(&self.shared);
            let spawned = std::thread::Builder::new().name("global-shortcuts".into()).spawn(move || {
                let run = || {
                    if let Err(e) = run(rx, shared) {
                        log::info!("desktop global shortcuts unavailable: {e}");
                    }
                };
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
                    log::warn!("global shortcuts stopped");
                }
            });
            match spawned {
                Ok(_) => self.ctl = Some(tx),
                Err(e) => {
                    log::info!("desktop global shortcuts unavailable: {e}");
                    return;
                }
            }
        }
        if let Some(tx) = &self.ctl {
            let _ = tx.send(on);
        }
    }
}

/// The session thread: opens / closes the portal session as the switch says, until the app
/// closes (the sender is dropped).
fn run(rx: Receiver<bool>, shared: Arc<Shared>) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    register(&conn);
    let active: Arc<Mutex<Option<String>>> = Arc::default();
    listen(&conn, Arc::clone(&active), shared)?;
    let mut serial = 0u32;
    let mut failed = false;
    while let Ok(first) = rx.recv() {
        // only the latest position of the switch matters
        let want = rx.try_iter().last().unwrap_or(first);
        let current = active.lock().unwrap_or_else(PoisonError::into_inner).clone();
        match (want, current) {
            (true, None) => match open(&conn, &mut serial) {
                Ok(session) => *active.lock().unwrap_or_else(PoisonError::into_inner) = Some(session),
                Err(e) if !failed => {
                    failed = true;
                    log::info!("desktop global shortcuts unavailable: {e}");
                }
                Err(_) => {}
            },
            (false, Some(session)) => {
                *active.lock().unwrap_or_else(PoisonError::into_inner) = None;
                close(&conn, &session);
            }
            _ => {}
        }
    }
    if let Some(session) = active.lock().unwrap_or_else(PoisonError::into_inner).take() {
        close(&conn, &session);
    }
    Ok(())
}

/// Outside a sandbox the portal can't tell which app is calling; tell it (portal ≥ 1.19's
/// host registry; older portals don't have it and use the process's own name).
fn register(conn: &Connection) {
    if std::path::Path::new("/.flatpak-info").exists() {
        return;
    }
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _ = Proxy::new(conn, DEST, PATH, "org.freedesktop.host.portal.Registry")
        .and_then(|p| p.call::<_, _, ()>("Register", &(crate::desktop::DESKTOP_ID, options)));
}

/// Create a session and bind the shortcuts; the session's handle.
fn open(conn: &Connection, serial: &mut u32) -> Result<String, String> {
    let unique = conn.unique_name().map(|n| n.to_string()).ok_or("no unique bus name")?;
    let proxy = Proxy::new(conn, DEST, PATH, IFACE).map_err(|e| e.to_string())?;
    let mut token = || {
        *serial = serial.wrapping_add(1);
        format!("filmcraft{}_{serial}", std::process::id())
    };
    let (request_token, session_token) = (token(), token());
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(request_token.as_str()));
    options.insert("session_handle_token", Value::from(session_token.as_str()));
    let results = request(conn, &unique, &request_token, || proxy.call("CreateSession", &(options,)))?;
    let session = results
        .get("session_handle")
        .and_then(|v| String::try_from(v.clone()).ok().or_else(|| OwnedObjectPath::try_from(v.clone()).ok().map(|p| p.to_string())))
        .unwrap_or_else(|| portal_path("session", &unique, &session_token));
    let session_path = ObjectPath::try_from(session.as_str()).map_err(|e| e.to_string())?;
    let labels: Vec<(&str, String, &str)> = SHORTCUTS.iter().map(|(id, label, keys)| (*id, filmcraft_ui_egui::i18n::t(label).to_string(), *keys)).collect();
    let shortcuts: Vec<(&str, HashMap<&str, Value<'_>>)> = labels
        .iter()
        .map(|(id, label, keys)| {
            let mut props: HashMap<&str, Value<'_>> = HashMap::new();
            props.insert("description", Value::from(label.as_str()));
            props.insert("preferred_trigger", Value::from(*keys));
            (*id, props)
        })
        .collect();
    let bind_token = token();
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(bind_token.as_str()));
    let bound = request(conn, &unique, &bind_token, || proxy.call("BindShortcuts", &(&session_path, shortcuts, "", options)));
    if let Err(e) = bound {
        close(conn, &session);
        return Err(e);
    }
    Ok(session)
}

/// Make a portal call that answers with a `Request` and wait for its `Response`.
fn request(conn: &Connection, unique: &str, token: &str, call: impl FnOnce() -> zbus::Result<OwnedObjectPath>) -> Result<HashMap<String, OwnedValue>, String> {
    let expected = portal_path("request", unique, token);
    // listen before calling: the response may come before the call returns
    let req = Proxy::new(conn, DEST, expected.clone(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let mut responses = req.receive_signal("Response").map_err(|e| e.to_string())?;
    let handle = call().map_err(|e| e.to_string())?;
    // a portal older than handle tokens answers on a path of its own
    let other = if handle.as_str() == expected {
        None
    } else {
        Some(Proxy::new(conn, DEST, handle.to_string(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?)
    };
    let mut other_responses = match &other {
        Some(p) => Some(p.receive_signal("Response").map_err(|e| e.to_string())?),
        None => None,
    };
    let msg = match other_responses.as_mut() {
        Some(r) => r.next(),
        None => responses.next(),
    }
    .ok_or("the portal went away")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("cancelled by the user".into()),
        n => Err(format!("refused by the portal ({n})")),
    }
}

fn close(conn: &Connection, session: &str) {
    let _ = Proxy::new(conn, DEST, session, "org.freedesktop.portal.Session").and_then(|p| p.call::<_, _, ()>("Close", &()));
}

/// Follow `Activated` on its own thread: an activation of the open session queues its command.
fn listen(conn: &Connection, active: Arc<Mutex<Option<String>>>, shared: Arc<Shared>) -> Result<(), String> {
    let proxy = Proxy::new(conn, DEST, PATH, IFACE).map_err(|e| e.to_string())?;
    let signals = proxy.receive_signal("Activated").map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("global-shortcuts-activated".into())
        .spawn(move || {
            let _proxy = proxy;
            let run = || {
                for m in signals {
                    let Ok((session, id, _time, _options)) = m.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>() else {
                        continue;
                    };
                    let ours = active.lock().unwrap_or_else(PoisonError::into_inner).as_deref() == Some(session.as_str());
                    if let Some(r) = request_for(&id).filter(|_| ours) {
                        shared.push(r);
                    }
                }
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn shortcuts_become_commands() {
        let id = |r: Option<DesktopRequest>| match r {
            Some(DesktopRequest::Command(id, p)) => format!("{id} {p}"),
            _ => "none".into(),
        };
        assert!(id(request_for("play-stop")).starts_with("playback.toggle"));
        assert!(id(request_for("add-marker")).starts_with("markers.add"));
        for bogus in ["", "PLAY-STOP", "play-stop\0", "x".repeat(10_000).as_str()] {
            assert_eq!(id(request_for(bogus)), "none");
        }
        assert!(SHORTCUTS.iter().all(|(id, _, _)| request_for(id).is_some()), "every bound shortcut does something");
    }

    #[test]
    fn portal_paths_follow_the_spec() {
        assert_eq!(portal_path("request", ":1.42", "filmcraft7_1"), "/org/freedesktop/portal/desktop/request/1_42/filmcraft7_1");
        assert_eq!(portal_path("session", ":1.7", "t"), "/org/freedesktop/portal/desktop/session/1_7/t");
        // hostile names still make a valid object path
        let p = portal_path("request", ":1.é/x y", "a/b-c");
        assert!(ObjectPath::try_from(p.as_str()).is_ok(), "{p}");
    }

    /// A stand-in for the portal: answers CreateSession / BindShortcuts at once and counts
    /// sessions closed.
    struct MockPortal {
        closed: Arc<AtomicUsize>,
        bound: Arc<Mutex<Vec<String>>>,
        sessions: Arc<Mutex<Vec<String>>>,
    }

    struct MockSession {
        closed: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Session")]
    impl MockSession {
        fn close(&self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn token(options: &HashMap<String, OwnedValue>, key: &str) -> String {
        options.get(key).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
    }

    #[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
    impl MockPortal {
        async fn create_session(
            &self,
            options: HashMap<String, OwnedValue>,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &zbus::Connection,
            #[zbus(object_server)] server: &zbus::ObjectServer,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let fail = |e: &dyn std::fmt::Display| zbus::fdo::Error::Failed(e.to_string());
            let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
            let req = portal_path("request", &sender, &token(&options, "handle_token"));
            let session = portal_path("session", &sender, &token(&options, "session_handle_token"));
            self.sessions.lock().unwrap().push(session.clone());
            server.at(session.as_str(), MockSession { closed: Arc::clone(&self.closed) }).await.map_err(|e| fail(&e))?;
            let mut results: HashMap<&str, Value<'_>> = HashMap::new();
            results.insert("session_handle", Value::from(session.as_str()));
            conn.emit_signal(Some(sender.as_str()), req.as_str(), "org.freedesktop.portal.Request", "Response", &(0u32, results))
                .await
                .map_err(|e| fail(&e))?;
            OwnedObjectPath::try_from(req).map_err(|e| fail(&e))
        }

        async fn bind_shortcuts(
            &self,
            _session: OwnedObjectPath,
            shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
            _parent: String,
            options: HashMap<String, OwnedValue>,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &zbus::Connection,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let fail = |e: &dyn std::fmt::Display| zbus::fdo::Error::Failed(e.to_string());
            self.bound.lock().unwrap().extend(shortcuts.into_iter().map(|(id, _)| id));
            let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
            let req = portal_path("request", &sender, &token(&options, "handle_token"));
            let results: HashMap<&str, Value<'_>> = HashMap::new();
            conn.emit_signal(Some(sender.as_str()), req.as_str(), "org.freedesktop.portal.Request", "Response", &(0u32, results))
                .await
                .map_err(|e| fail(&e))?;
            OwnedObjectPath::try_from(req).map_err(|e| fail(&e))
        }
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = std::time::Instant::now();
        while !ok() {
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// On a private session bus (`dbus-run-session`; skipped without one): switched on, the
    /// shortcuts are bound and an activation queues its command; switched off, the session is
    /// closed and a late activation of it does nothing.
    #[test]
    fn shortcuts_through_a_portal() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("skipped: no session bus");
            return;
        }
        let Ok(portal) = Connection::session() else {
            eprintln!("skipped: session bus not reachable");
            return;
        };
        let dbus = zbus::blocking::fdo::DBusProxy::new(&portal).unwrap();
        if dbus.name_has_owner(zbus::names::BusName::try_from(DEST).unwrap()).unwrap() {
            eprintln!("skipped: a real portal is on this bus (run under dbus-run-session)");
            return;
        }
        let closed = Arc::new(AtomicUsize::new(0));
        let bound = Arc::new(Mutex::new(Vec::new()));
        let sessions = Arc::new(Mutex::new(Vec::new()));
        portal.object_server().at(PATH, MockPortal { closed: Arc::clone(&closed), bound: Arc::clone(&bound), sessions: Arc::clone(&sessions) }).unwrap();
        portal.request_name(DEST).unwrap();

        let shared = Arc::new(Shared::default());
        let mut g = GlobalShortcuts::new(Arc::clone(&shared));
        g.set(false);
        assert!(g.ctl.is_none(), "nothing starts while off");
        g.set(true);
        wait_for("the shortcuts to be bound", || bound.lock().unwrap().len() == 2);
        assert_eq!(*bound.lock().unwrap(), ["play-stop", "add-marker"]);

        let activate = |session: &str, id: &str| {
            let options: HashMap<&str, Value<'_>> = HashMap::new();
            portal.emit_signal(None::<&str>, PATH, IFACE, "Activated", &(ObjectPath::try_from(session).unwrap(), id, 0u64, options)).unwrap();
        };
        let session = sessions.lock().unwrap().first().cloned().expect("a session");
        // the session counts as open once BindShortcuts has answered: repeat until it lands
        let mut got = Vec::new();
        wait_for("the activation", || {
            activate(&session, "bogus");
            activate(&session, "add-marker");
            std::thread::sleep(std::time::Duration::from_millis(30));
            got.extend(shared.take());
            !got.is_empty()
        });
        assert!(got.iter().all(|r| matches!(r, DesktopRequest::Command(id, _) if id == "markers.add")), "{got:?}");
        // a session that isn't ours does nothing
        activate("/org/freedesktop/portal/desktop/session/1_999/other", "play-stop");

        // let repeated activations still on the way arrive
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(shared.take().iter().all(|r| matches!(r, DesktopRequest::Command(id, _) if id == "markers.add")), "not the other session's");

        g.set(false);
        wait_for("the session to close", || closed.load(Ordering::SeqCst) == 1);
        activate(&session, "play-stop");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(shared.take().is_empty(), "nothing after switching off");

        // on again: a new session
        g.set(true);
        wait_for("the shortcuts to be bound again", || bound.lock().unwrap().len() == 4);
    }
}
