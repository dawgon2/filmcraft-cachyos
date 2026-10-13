//! Media controls (LINUX-PLAN L2.3): FilmCraft as an MPRIS player, so Plasma's media widget, the
//! lock screen, GNOME's media controls and the keyboard's media keys play, pause and step the
//! Program monitor.
//!
//! `org.mpris.MediaPlayer2.filmcraft` (`….filmcraft.instance<pid>` when another FilmCraft has
//! it) is owned only while a sequence is open; with nothing to play there is no player. The
//! methods queue ordinary commands for the UI (`playback.toggle`, `playback.stop`,
//! `playhead.stepForward` / `stepBack`, `playhead.set`), so they behave exactly like the
//! keyboard. Properties are read from the snapshot the UI publishes each frame; changes are
//! announced with `PropertiesChanged`. All of it runs on the desktop worker's thread and zbus's
//! own executor, never on the UI thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_ui_egui::desktop::{DesktopRequest, DesktopState, us_to_ticks};
use serde_json::json;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::desktop::Shared;

pub const PATH: &str = "/org/mpris/MediaPlayer2";
const NAME: &str = "org.mpris.MediaPlayer2.filmcraft";
const TRACK: &str = "/ai/storyteller/filmcraft/sequence";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What the UI last published (properties are read from here).
pub type Snapshot = Arc<Mutex<DesktopState>>;

/// The MPRIS `PlaybackStatus` of a state.
pub fn playback_status(s: &DesktopState) -> &'static str {
    if s.playing {
        "Playing"
    } else if s.sequence.is_some() {
        "Paused"
    } else {
        "Stopped"
    }
}

/// The request a player method turns into, given the current state (`None`: nothing to do).
pub fn request_for(method: &str, s: &DesktopState, arg_us: i64) -> Option<DesktopRequest> {
    let cmd = |id: &str, p: serde_json::Value| Some(DesktopRequest::Command(id.into(), p));
    let program = json!({"monitor": "program"});
    // switched off in Settings while a client still holds the old player
    if s.media_controls_off {
        return None;
    }
    match method {
        "PlayPause" => cmd("playback.toggle", program),
        "Play" if !s.playing => cmd("playback.toggle", program),
        "Pause" | "Stop" if s.playing => cmd("playback.stop", program),
        "Next" => cmd("playhead.stepForward", program),
        "Previous" => cmd("playhead.stepBack", program),
        "Seek" => {
            let to = s.position_us.saturating_add(arg_us).clamp(0, s.duration_us.max(0));
            cmd("playhead.set", json!({"time": us_to_ticks(to).0}))
        }
        // out of range: ignored, as the spec says
        "SetPosition" if (0..=s.duration_us).contains(&arg_us) => cmd("playhead.set", json!({"time": us_to_ticks(arg_us).0})),
        "Raise" => Some(DesktopRequest::Raise),
        _ => None,
    }
}

/// `Metadata` of a state.
pub fn metadata(s: &DesktopState) -> HashMap<&'static str, OwnedValue> {
    let mut m = HashMap::new();
    let mut put = |k: &'static str, v: Value<'_>| {
        if let Ok(v) = OwnedValue::try_from(v) {
            m.insert(k, v);
        }
    };
    if let Ok(track) = ObjectPath::try_from(TRACK) {
        put("mpris:trackid", Value::from(track));
    }
    put("mpris:length", Value::I64(s.duration_us.max(0)));
    put("xesam:title", Value::from(s.sequence.clone().unwrap_or_else(|| s.project.clone())));
    put("xesam:album", Value::from(s.project.clone()));
    m
}

/// The playhead jumped: `now` is more than half a second (or two frames) away from where clients
/// expect it, given the position `sent` announced `elapsed_us` ago and its rate. MPRIS clients
/// only re-read `Position` on `Seeked` or a status / rate change, so a jump needs `Seeked`.
pub fn jumped(sent: &DesktopState, elapsed_us: i64, now: &DesktopState) -> bool {
    if sent.sequence != now.sequence {
        return false;
    }
    let moved = if sent.playing { (rate(sent) * elapsed_us as f64) as i64 } else { 0 };
    let expected = sent.position_us.saturating_add(moved);
    let slack = now.frame_us.saturating_mul(2).max(500_000);
    now.position_us.saturating_sub(expected).saturating_abs() > slack
}

struct Root {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        self.shared.push(DesktopRequest::Raise);
    }
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "FilmCraft"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        crate::desktop::DESKTOP_ID
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    shared: Arc<Shared>,
    snapshot: Snapshot,
}

impl Player {
    fn state(&self) -> DesktopState {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
    fn call(&self, method: &str, arg_us: i64) {
        if let Some(r) = request_for(method, &self.state(), arg_us) {
            self.shared.push(r);
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&self) {
        self.call("PlayPause", 0);
    }
    fn play(&self) {
        self.call("Play", 0);
    }
    fn pause(&self) {
        self.call("Pause", 0);
    }
    fn stop(&self) {
        self.call("Stop", 0);
    }
    fn next(&self) {
        self.call("Next", 0);
    }
    fn previous(&self) {
        self.call("Previous", 0);
    }
    fn seek(&self, offset: i64) {
        self.call("Seek", offset);
    }
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        if track_id.as_str() == TRACK {
            self.call("SetPosition", position);
        }
    }
    fn open_uri(&self, _uri: String) {}
    #[zbus(property)]
    fn playback_status(&self) -> String {
        playback_status(&self.state()).into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        rate(&self.state())
    }
    /// Read-write in the spec (`CanControl` is true); the speed follows the transport, so a
    /// client's request is accepted and ignored rather than answered with an error.
    #[zbus(property)]
    fn set_rate(&mut self, _rate: f64) {}
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        -8.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        8.0
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }
    /// Read-write in the spec (Plasma scrolls on the widget to change it); the mixer is
    /// FilmCraft's own, so the request is accepted and ignored.
    #[zbus(property)]
    fn set_volume(&mut self, _volume: f64) {}
    #[zbus(property)]
    fn metadata(&self) -> HashMap<&'static str, OwnedValue> {
        metadata(&self.state())
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        self.state().position_us
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state().sequence.is_some()
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state().sequence.is_some()
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state().sequence.is_some()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state().sequence.is_some()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.state().sequence.is_some()
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

/// The `Rate` property: the transport speed while playing, 1.0 otherwise.
fn rate(s: &DesktopState) -> f64 {
    if s.playing && s.speed.is_finite() && s.speed != 0.0 { s.speed } else { 1.0 }
}

/// The MPRIS player: owns its name while a sequence is open.
pub struct Mpris {
    conn: Option<Connection>,
    owned: Option<String>,
    shared: Arc<Shared>,
    snapshot: Snapshot,
    last: Option<DesktopState>,
}

impl Mpris {
    pub fn new(shared: Arc<Shared>, snapshot: Snapshot) -> Self {
        Self { conn: None, owned: None, shared, snapshot, last: None }
    }

    /// Follow the published state: own or release the name, announce changed properties, and
    /// `Seeked` when the playhead `jumped`.
    pub fn update(&mut self, s: &DesktopState, seeked: bool) -> Result<(), String> {
        if s.sequence.is_none() || s.media_controls_off {
            self.release();
            return Ok(());
        }
        if self.owned.is_none() {
            self.claim()?;
            self.last = Some(s.clone());
            return Ok(());
        }
        let Some(conn) = self.conn.as_ref() else { return Ok(()) };
        let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
        let last = self.last.clone().unwrap_or_default();
        if playback_status(&last) != playback_status(s) {
            changed.insert("PlaybackStatus", Value::from(playback_status(s)));
        }
        if rate(&last) != rate(s) {
            changed.insert("Rate", Value::F64(rate(s)));
        }
        if last.sequence != s.sequence || last.project != s.project || last.duration_us != s.duration_us {
            let meta: HashMap<&str, OwnedValue> = metadata(s);
            if let Ok(v) = OwnedValue::try_from(Value::from(meta)) {
                changed.insert("Metadata", v.into());
            }
        }
        self.last = Some(s.clone());
        // properties first: a failed `Seeked` must not lose a status / rate change
        let props = if changed.is_empty() {
            Ok(())
        } else {
            conn.emit_signal(None::<&str>, PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged", &(PLAYER, changed, Vec::<&str>::new()))
                .map_err(|e| e.to_string())
        };
        let seek = if seeked { conn.emit_signal(None::<&str>, PATH, PLAYER, "Seeked", &(s.position_us.max(0),)).map_err(|e| e.to_string()) } else { Ok(()) };
        props.and(seek)
    }

    fn claim(&mut self) -> Result<(), String> {
        let conn = match self.conn.take() {
            Some(c) => c,
            None => {
                let c = Connection::session().map_err(|e| e.to_string())?;
                c.object_server().at(PATH, Root { shared: Arc::clone(&self.shared) }).map_err(|e| e.to_string())?;
                c.object_server().at(PATH, Player { shared: Arc::clone(&self.shared), snapshot: Arc::clone(&self.snapshot) }).map_err(|e| e.to_string())?;
                c
            }
        };
        let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
        let mut result = Err("no name".to_string());
        for name in [NAME.to_string(), format!("{NAME}.instance{}", std::process::id())] {
            match conn.request_name_with_flags(name.as_str(), flags) {
                Ok(zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner) => {
                    self.owned = Some(name);
                    result = Ok(());
                    break;
                }
                Ok(other) => result = Err(format!("{name}: {other:?}")),
                Err(e) => result = Err(e.to_string()),
            }
        }
        self.conn = Some(conn);
        result
    }

    /// Give the name up (nothing open, or the app closing).
    pub fn release(&mut self) {
        if let (Some(conn), Some(name)) = (self.conn.as_ref(), self.owned.take()) {
            let _ = conn.release_name(name.as_str());
        }
        self.last = None;
    }

    /// The name currently owned (tests).
    #[cfg(test)]
    pub fn owned(&self) -> Option<&str> {
        self.owned.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq() -> DesktopState {
        DesktopState { sequence: Some("Cut 1".into()), project: "Film".into(), duration_us: 10_000_000, position_us: 2_000_000, ..DesktopState::default() }
    }

    #[test]
    fn methods_become_commands() {
        let mut s = seq();
        let id = |r: Option<DesktopRequest>| match r {
            Some(DesktopRequest::Command(id, p)) => format!("{id} {p}"),
            Some(DesktopRequest::Raise) => "raise".into(),
            Some(DesktopRequest::Open(_)) => "open".into(),
            None => "none".into(),
        };
        assert!(id(request_for("PlayPause", &s, 0)).starts_with("playback.toggle"));
        assert!(id(request_for("Play", &s, 0)).starts_with("playback.toggle"));
        assert_eq!(id(request_for("Pause", &s, 0)), "none", "already paused");
        s.playing = true;
        assert_eq!(id(request_for("Play", &s, 0)), "none", "already playing");
        assert!(id(request_for("Pause", &s, 0)).starts_with("playback.stop"));
        assert!(id(request_for("Next", &s, 0)).starts_with("playhead.stepForward"));
        assert!(id(request_for("Previous", &s, 0)).starts_with("playhead.stepBack"));
        assert_eq!(id(request_for("Raise", &s, 0)), "raise");
        assert_eq!(id(request_for("Bogus", &s, 0)), "none");
        // Settings ▸ Playback ▸ Media keys and the desktop's media controls off: nothing runs
        s.media_controls_off = true;
        for method in ["PlayPause", "Play", "Pause", "Stop", "Next", "Previous", "Raise"] {
            assert_eq!(id(request_for(method, &s, 0)), "none", "{method}");
        }
    }

    #[test]
    fn rate_is_the_speed_while_playing_and_one_otherwise() {
        let mut s = seq();
        s.speed = 2.0;
        assert_eq!(rate(&s), 1.0, "paused");
        s.playing = true;
        assert_eq!(rate(&s), 2.0);
        s.speed = f64::NAN;
        assert_eq!(rate(&s), 1.0, "hostile speed");
        s.speed = 0.0;
        assert_eq!(rate(&s), 1.0);
    }

    #[test]
    fn seeking_is_clamped_and_set_position_checks_the_range() {
        let s = seq();
        let time = |r: Option<DesktopRequest>| match r {
            Some(DesktopRequest::Command(_, p)) => p["time"].as_i64(),
            _ => None,
        };
        let tps = filmcraft_time::TICKS_PER_SECOND;
        assert_eq!(time(request_for("Seek", &s, 1_000_000)), Some(3 * tps));
        assert_eq!(time(request_for("Seek", &s, -60_000_000)), Some(0));
        assert_eq!(time(request_for("Seek", &s, i64::MAX)), Some(10 * tps), "saturates, clamps to the end");
        assert_eq!(time(request_for("Seek", &s, i64::MIN)), Some(0));
        assert_eq!(time(request_for("SetPosition", &s, 5_000_000)), Some(5 * tps));
        assert_eq!(time(request_for("SetPosition", &s, -1)), None);
        assert_eq!(time(request_for("SetPosition", &s, 10_000_001)), None);
    }

    #[test]
    fn a_jump_is_a_seek_and_playing_on_is_not() {
        let mut sent = seq();
        let mut now = seq();
        assert!(!jumped(&sent, 3_000_000, &now), "paused, still");
        now.position_us = 2_400_000;
        assert!(!jumped(&sent, 0, &now), "within half a second");
        now.position_us = 9_000_000;
        assert!(jumped(&sent, 0, &now), "clicked further on while paused");
        sent.playing = true;
        sent.speed = 1.0;
        now.position_us = 5_000_000;
        assert!(!jumped(&sent, 3_000_000, &now), "played on at 1x");
        assert!(jumped(&sent, 0, &now), "jumped while playing");
        sent.speed = -2.0;
        now.position_us = 0;
        assert!(!jumped(&sent, 1_000_000, &now), "reverse 2x");
        // hostile values: no overflow
        sent.position_us = i64::MAX;
        sent.speed = f64::NAN;
        now.position_us = i64::MIN;
        now.frame_us = i64::MAX;
        let _ = jumped(&sent, i64::MAX, &now);
        now.sequence = Some("Other".into());
        assert!(!jumped(&seq(), 0, &now), "another sequence is a track change, not a seek");
    }

    #[test]
    fn status_and_metadata() {
        let mut s = DesktopState::default();
        assert_eq!(playback_status(&s), "Stopped");
        s = seq();
        assert_eq!(playback_status(&s), "Paused");
        s.playing = true;
        assert_eq!(playback_status(&s), "Playing");
        let m = metadata(&s);
        assert!(m.contains_key("mpris:trackid") && m.contains_key("mpris:length"));
        assert_eq!(String::try_from(m["xesam:title"].clone()).unwrap(), "Cut 1");
        s.duration_us = -5;
        assert_eq!(i64::try_from(metadata(&s)["mpris:length"].clone()).unwrap(), 0);
    }

    /// On a private session bus (`dbus-run-session`; skipped without one): the player appears
    /// with a sequence, answers properties and methods, and goes away without one.
    #[test]
    fn player_on_the_bus() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("skipped: no session bus");
            return;
        }
        let Ok(client) = Connection::session() else {
            eprintln!("skipped: session bus not reachable");
            return;
        };
        let shared = Arc::new(Shared::default());
        let snapshot: Snapshot = Arc::new(Mutex::new(seq()));
        let mut m = Mpris::new(Arc::clone(&shared), Arc::clone(&snapshot));
        m.update(&seq(), false).unwrap();
        let name = m.owned().expect("owns an MPRIS name").to_string();
        let player = zbus::blocking::Proxy::new(&client, name.as_str(), PATH, PLAYER).unwrap();
        let status: String = player.get_property("PlaybackStatus").unwrap();
        assert_eq!(status, "Paused");
        let pos: i64 = player.get_property("Position").unwrap();
        assert_eq!(pos, 2_000_000);
        player.call::<_, _, ()>("PlayPause", &()).unwrap();
        player.call::<_, _, ()>("Seek", &(1_000_000i64,)).unwrap();
        let root = zbus::blocking::Proxy::new(&client, name.as_str(), PATH, "org.mpris.MediaPlayer2").unwrap();
        let identity: String = root.get_property("Identity").unwrap();
        assert_eq!(identity, "FilmCraft");
        let got = shared.take();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(matches!(&got[0], DesktopRequest::Command(id, _) if id == "playback.toggle"));
        assert!(matches!(&got[1], DesktopRequest::Command(id, _) if id == "playhead.set"));
        // playing: a status change is announced without error
        let mut playing = seq();
        playing.playing = true;
        *snapshot.lock().unwrap() = playing.clone();
        m.update(&playing, true).unwrap();
        let dbus = zbus::blocking::fdo::DBusProxy::new(&client).unwrap();
        let has = |name: &str| dbus.name_has_owner(zbus::names::BusName::try_from(name).unwrap()).unwrap();
        // switched off in Settings: the player leaves the bus, and comes back when switched on
        let off = DesktopState { media_controls_off: true, ..seq() };
        m.update(&off, false).unwrap();
        assert!(m.owned().is_none());
        assert!(!has(&name), "released while off");
        m.update(&seq(), false).unwrap();
        let name = m.owned().expect("owns an MPRIS name again").to_string();
        assert!(has(&name));
        // nothing open: the player leaves the bus
        m.update(&DesktopState::default(), false).unwrap();
        assert!(m.owned().is_none());
        assert!(!has(&name), "released");
    }
}
