//! Camera cards (LINUX-PLAN L2.4): notice a camera card when the desktop mounts it, and eject it
//! when its copy is done.
//!
//! **Noticing** reads the mount table (`/proc/self/mounts`) every two seconds and looks at what
//! the desktop mounts for removable drives (`/run/media/<user>/…` on most distributions,
//! `/media/…` on Debian / Ubuntu). A new mount that [`filmcraft_engine::card_offload::detect`]
//! calls a camera card becomes `card.detected` for the UI; when it goes away, `card.removed`. This
//! works the same on Plasma, GNOME, X11 and Wayland and needs no service.
//!
//! **Ejecting** asks UDisks2 on the system bus (the service file managers use) to unmount the card
//! and power its reader down, on its own thread; the answer comes back as `card.ejected`. Without
//! UDisks2 (some minimal systems, a Flatpak without `--system-talk-name=org.freedesktop.UDisks2`)
//! the UI says to eject from the file manager.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use filmcraft_ui_egui::desktop::DesktopRequest;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
/// How often the mount table is read.
const EVERY: Duration = Duration::from_secs(2);
/// Most of the mount table read (a few KB normally).
const MAX_TABLE: u64 = 1 << 20;
/// Most removable mounts followed at once.
const MAX_MOUNTS: usize = 64;
/// Longest mount path taken.
const MAX_PATH: usize = 4096;
/// Where desktops mount removable drives.
const ROOTS: &[&str] = &["/run/media/", "/media/"];

/// Start watching the mount table (a thread for the app's life; it ends when the app closes).
pub fn watch(shared: Arc<crate::desktop::Shared>) {
    let spawned = std::thread::Builder::new().name("card-watch".into()).spawn(move || {
        let run = || {
            let mut known = HashMap::new();
            while !shared.is_closing() {
                let table = read_table().unwrap_or_default();
                for r in step(&mut known, &removable_mounts(&table), &|m| filmcraft_engine::card_offload::detect(Path::new(m))) {
                    shared.push(r);
                }
                std::thread::sleep(EVERY);
            }
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
            log::warn!("camera card watch stopped");
        }
    });
    if let Err(e) = spawned {
        log::info!("camera cards won't be noticed: {e}");
    }
}

fn read_table() -> Option<String> {
    let mut s = String::new();
    std::fs::File::open("/proc/self/mounts").ok()?.take(MAX_TABLE).read_to_string(&mut s).ok()?;
    Some(s)
}

/// The mount points under [`ROOTS`] in a mount table (`/proc/self/mounts` format: the second
/// field, with spaces and other bytes as `\ooo` octal escapes).
pub fn removable_mounts(table: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in table.lines() {
        let Some(field) = line.split_ascii_whitespace().nth(1) else { continue };
        let Some(path) = unescape(field) else { continue };
        let under = ROOTS.iter().any(|r| path.strip_prefix(r).is_some_and(|rest| !rest.is_empty() && !rest.split('/').any(|c| c == "..")));
        if under && !out.contains(&path) {
            out.push(path);
            if out.len() >= MAX_MOUNTS {
                break;
            }
        }
    }
    out
}

/// `\040` → space, `\011` → tab…; `None` for a malformed escape, invalid UTF-8, a NUL or an
/// over-long path.
fn unescape(field: &str) -> Option<String> {
    if field.len() > MAX_PATH {
        return None;
    }
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        if c == b'\\' {
            let digits = b.get(i.saturating_add(1)..i.saturating_add(4))?;
            let mut v = 0u32;
            for d in digits {
                let d = char::from(*d).to_digit(8)?;
                v = v.checked_mul(8)?.checked_add(d)?;
            }
            out.push(u8::try_from(v).ok()?);
            i = i.saturating_add(4);
        } else {
            out.push(c);
            i = i.saturating_add(1);
        }
    }
    if out.contains(&0) {
        return None;
    }
    String::from_utf8(out).ok()
}

/// One look at the mounts: new camera cards are announced, announced ones that went away are
/// removed. `known` maps each mount seen to whether it was announced.
pub fn step(known: &mut HashMap<String, bool>, mounts: &[String], detect: &dyn Fn(&str) -> Option<&'static str>) -> Vec<DesktopRequest> {
    let mut out = Vec::new();
    for m in mounts {
        if known.contains_key(m) {
            continue;
        }
        let layout = detect(m);
        if let Some(layout) = layout {
            out.push(DesktopRequest::Command("card.detected".into(), serde_json::json!({"mount": m, "layout": layout})));
        }
        known.insert(m.clone(), layout.is_some());
    }
    let gone: Vec<String> = known.keys().filter(|k| !mounts.contains(*k)).cloned().collect();
    for m in gone {
        if known.remove(&m) == Some(true) {
            out.push(DesktopRequest::Command("card.removed".into(), serde_json::json!({"mount": m})));
        }
    }
    out
}

/// Eject the card mounted at `mount` on a thread; the outcome arrives as `card.ejected`
/// (`mount`, `ok`, `error`).
pub fn eject(shared: Arc<crate::desktop::Shared>, mount: String) {
    let spawned = std::thread::Builder::new().name("card-eject".into()).spawn(move || {
        let run = || Connection::system().map_err(|e| e.to_string()).and_then(|c| eject_on(&c, &mount));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("internal error".into()));
        let error = r.as_ref().err().cloned();
        shared.push(DesktopRequest::Command("card.ejected".into(), serde_json::json!({"mount": mount, "ok": r.is_ok(), "error": error})));
    });
    if let Err(e) = spawned {
        log::info!("can't eject: {e}");
    }
}

type Managed = HashMap<OwnedObjectPath, HashMap<zbus::names::OwnedInterfaceName, HashMap<String, OwnedValue>>>;

/// Unmount the file system mounted at `mount` and power its drive down (when the drive can be;
/// a built-in card reader often can't, and that's fine: unmounted is safe to pull).
pub fn eject_on(conn: &Connection, mount: &str) -> Result<(), String> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination(UDISKS)
        .and_then(|b| b.path(UDISKS_PATH))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let objects: Managed = om.get_managed_objects().map_err(|e| format!("UDisks2 isn't available ({e}); eject the card from the file manager"))?;
    let (fs_path, drive) = find_mount(&objects, mount).ok_or_else(|| format!("UDisks2 doesn't know {mount}"))?;
    let none: HashMap<&str, Value<'_>> = HashMap::new();
    let fs = zbus::blocking::Proxy::new(conn, UDISKS, fs_path.as_str(), FILESYSTEM).map_err(|e| e.to_string())?;
    fs.call_method("Unmount", &(&none,)).map_err(|e| format!("couldn't unmount {mount}: {e}"))?;
    if let Some(drive) = drive {
        let can = objects
            .iter()
            .find(|(p, _)| p.as_str() == drive.as_str())
            .and_then(|(_, ifs)| ifs.iter().find(|(n, _)| n.as_str() == DRIVE))
            .and_then(|(_, props)| props.get("CanPowerOff"))
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false);
        if can {
            let d = zbus::blocking::Proxy::new(conn, UDISKS, drive.as_str(), DRIVE).map_err(|e| e.to_string())?;
            if let Err(e) = d.call_method("PowerOff", &(&none,)) {
                log::info!("card unmounted; powering the reader off failed: {e}");
            }
        }
    }
    Ok(())
}

/// The file system object mounted at `mount` and its drive.
fn find_mount(objects: &Managed, mount: &str) -> Option<(OwnedObjectPath, Option<OwnedObjectPath>)> {
    for (path, ifs) in objects {
        let Some((_, fs)) = ifs.iter().find(|(n, _)| n.as_str() == FILESYSTEM) else { continue };
        let Some(points) = fs.get("MountPoints").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<Vec<u8>>::try_from(v).ok()) else { continue };
        // `ay` paths end in a NUL
        let hit = points.iter().any(|p| p.strip_suffix(&[0]).unwrap_or(p) == mount.as_bytes());
        if !hit {
            continue;
        }
        let drive = ifs
            .iter()
            .find(|(n, _)| n.as_str() == BLOCK)
            .and_then(|(_, b)| b.get("Drive"))
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
            .filter(|d| d.as_str() != "/");
        return Some((path.clone(), drive));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn removable_mounts_are_read_from_the_mount_table() {
        let table = "\
/dev/nvme0n1p2 / btrfs rw 0 0
/dev/sdb1 /run/media/user/FUJI\\040X-H2S exfat rw,nosuid 0 0
/dev/sdc1 /media/user/EOS_DIGITAL vfat rw 0 0
/dev/sdc1 /media/user/EOS_DIGITAL vfat rw 0 0
/dev/sdd1 /run/media/ vfat rw 0 0
/dev/sde1 /run/media/u/../../etc vfat rw 0 0
/dev/sdf1 /run/media/u/bad\\9 vfat rw 0 0
/dev/sdf2 /run/media/u/nul\\000 vfat rw 0 0
/dev/sdf3 /run/media/u/latin\\351 vfat rw 0 0
/dev/sdf4 /run/media/u/cut\\04 vfat rw 0 0
/dev/sdg1 /mnt/backup ext4 rw 0 0
garbage
";
        assert_eq!(removable_mounts(table), ["/run/media/user/FUJI X-H2S", "/media/user/EOS_DIGITAL"]);
        assert!(removable_mounts("").is_empty());
        let many: String = (0..1000).map(|i| format!("/dev/x /media/u/c{i} vfat rw 0 0\n")).collect();
        assert_eq!(removable_mounts(&many).len(), MAX_MOUNTS);
        let long = format!("/dev/x /media/{} vfat rw 0 0", "a".repeat(MAX_PATH));
        assert!(removable_mounts(&long).is_empty());
    }

    #[test]
    fn cards_are_announced_once_and_removed_when_unmounted() {
        let detect = |m: &str| if m.ends_with("CARD") { Some("dcim") } else { None };
        let mut known = HashMap::new();
        let mounts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let r = step(&mut known, &mounts(&["/media/u/CARD", "/media/u/USB"]), &detect);
        assert_eq!(r, [DesktopRequest::Command("card.detected".into(), serde_json::json!({"mount": "/media/u/CARD", "layout": "dcim"}))]);
        assert!(step(&mut known, &mounts(&["/media/u/CARD", "/media/u/USB"]), &detect).is_empty(), "once");
        let r = step(&mut known, &[], &detect);
        assert_eq!(r, [DesktopRequest::Command("card.removed".into(), serde_json::json!({"mount": "/media/u/CARD"}))], "the USB stick was never announced");
        assert!(known.is_empty());
        assert_eq!(step(&mut known, &mounts(&["/media/u/CARD"]), &detect).len(), 1, "plugged in again");
    }

    /// A stand-in UDisks2: one card file system on a drive that can power off, recording calls.
    struct MockFs {
        calls: Arc<Mutex<Vec<String>>>,
        points: Vec<Vec<u8>>,
    }

    #[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
    impl MockFs {
        fn unmount(&self, _options: HashMap<String, OwnedValue>) {
            self.calls.lock().unwrap().push("Unmount".into());
        }
        #[zbus(property)]
        fn mount_points(&self) -> Vec<Vec<u8>> {
            self.points.clone()
        }
    }

    struct MockBlock {
        drive: OwnedObjectPath,
    }

    #[zbus::interface(name = "org.freedesktop.UDisks2.Block")]
    impl MockBlock {
        #[zbus(property)]
        fn drive(&self) -> OwnedObjectPath {
            self.drive.clone()
        }
    }

    struct MockDrive {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.freedesktop.UDisks2.Drive")]
    impl MockDrive {
        fn power_off(&self, _options: HashMap<String, OwnedValue>) {
            self.calls.lock().unwrap().push("PowerOff".into());
        }
        #[zbus(property)]
        fn can_power_off(&self) -> bool {
            true
        }
    }

    /// On a private session bus (`dbus-run-session`; skipped without one) standing in for the
    /// system bus: no UDisks2 → an error, never a panic; with one, the card's file system is found
    /// by its mount point, unmounted, and its drive powered off; an unknown mount is an error.
    #[test]
    fn ejects_through_udisks() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("skipped: no session bus");
            return;
        }
        let Ok(conn) = Connection::session() else {
            eprintln!("skipped: session bus not reachable");
            return;
        };
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).unwrap();
        if dbus.name_has_owner(zbus::names::BusName::try_from(UDISKS).unwrap()).unwrap() {
            eprintln!("skipped: a UDisks2 is on this bus (run under dbus-run-session)");
            return;
        }
        assert!(eject_on(&conn, "/media/u/CARD").is_err(), "no UDisks2");

        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let service = Connection::session().unwrap();
        let os = service.object_server();
        os.at(UDISKS_PATH, zbus::fdo::ObjectManager).unwrap();
        let fs = "/org/freedesktop/UDisks2/block_devices/sdb1";
        let drive = "/org/freedesktop/UDisks2/drives/Card_Reader";
        os.at(fs, MockFs { calls: Arc::clone(&calls), points: vec![b"/media/u/CARD X\0".to_vec()] }).unwrap();
        os.at(fs, MockBlock { drive: OwnedObjectPath::try_from(drive).unwrap() }).unwrap();
        os.at(drive, MockDrive { calls: Arc::clone(&calls) }).unwrap();
        service.request_name(UDISKS).unwrap();

        assert!(eject_on(&conn, "/media/u/OTHER").unwrap_err().contains("doesn't know"));
        assert!(calls.lock().unwrap().is_empty());
        eject_on(&conn, "/media/u/CARD X").unwrap();
        assert_eq!(*calls.lock().unwrap(), ["Unmount", "PowerOff"]);
        service.release_name(UDISKS).unwrap();
    }
}
