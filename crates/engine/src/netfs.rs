//! Is this path on a network share? (STORAGE-PLAN S1 item 1.)
//!
//! SMB / NFS shares mounted by Plasma (kio-fuse), fstab or autofs look like local folders, but
//! reads can stall or fail when the server goes away, renames aren't always atomic, and playback
//! is slower. FilmCraft tells the user where a clip or project lives so the Media panel and
//! Project Settings can say "network share" and later steps (local cache, timeouts, safe saves)
//! know which files need care.
//!
//! On Linux the answer comes from `/proc/self/mountinfo`: the mount whose mount point is the
//! longest prefix of the path wins (so a bind mount or a share mounted inside `/home` is found),
//! and its filesystem type says whether it is network storage. [`classify_mountinfo`] is a pure
//! function over the file's text, so hostile or truncated text gives [`StorageKind::Unknown`],
//! never a panic. Other systems (and the web build) answer `Unknown` for now.

use std::path::{Component, Path, PathBuf};

use filmcraft_project::{ItemKind, MediaRef};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, item_p};
use crate::{Result, Session};

/// Where a path's bytes live, as far as FilmCraft can tell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StorageKind {
    /// A local disk (or anything not known to be remote).
    #[serde(rename_all = "camelCase")]
    Local { fs_type: String },
    /// A network share; `fs_type` is the mount's filesystem type (`cifs`, `nfs4`,
    /// `fuse.kio-fuse`, …) and `source` the server / share it came from, as mounted.
    #[serde(rename_all = "camelCase")]
    Network { fs_type: String, source: String },
    /// The mount table couldn't be read or doesn't cover the path.
    Unknown,
}

impl StorageKind {
    /// True for a network share.
    pub fn is_network(&self) -> bool {
        matches!(self, StorageKind::Network { .. })
    }
}

/// Filesystem types that are network storage. FUSE types are matched by their subtype.
const NETWORK_FS: &[&str] = &[
    "cifs",
    "smb3",
    "smbfs",
    "nfs",
    "nfs4",
    "9p",
    "afs",
    "ceph",
    "glusterfs",
    "davfs",
    "fuse.kio-fuse",
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.gvfsd-fuse",
    "fuse.smbnetfs",
    "fuse.davfs2",
    "fuse.s3fs",
    "fuse.glusterfs",
];

/// The longest mountinfo text read (a real table is a few KB; thousands of container mounts
/// stay well under this).
#[cfg(target_os = "linux")]
const MAX_MOUNTINFO_BYTES: usize = 4 * 1024 * 1024;

/// The most mount lines looked at.
const MAX_MOUNT_LINES: usize = 100_000;

/// Whether a filesystem type is network storage.
pub fn is_network_fs(fs_type: &str) -> bool {
    NETWORK_FS.iter().any(|t| t.eq_ignore_ascii_case(fs_type))
}

/// Classifies `path` (absolute; `..` and `.` are resolved lexically) against the text of a
/// `/proc/<pid>/mountinfo` file. The mount with the longest mount point that contains the path
/// wins; among equal mount points the later line wins (it is mounted over the earlier one).
pub fn classify_mountinfo(mountinfo: &str, path: &Path) -> StorageKind {
    let Some(path) = normalize(path) else {
        return StorageKind::Unknown;
    };
    let mut best: Option<(usize, &str, &str)> = None;
    for line in mountinfo.lines().take(MAX_MOUNT_LINES) {
        let Some(mount) = parse_line(line) else {
            continue;
        };
        let Some(mount_point) = normalize(Path::new(&mount.mount_point)) else {
            continue;
        };
        if !path.starts_with(&mount_point) {
            continue;
        }
        let depth = mount_point.components().count();
        if best.is_none_or(|(d, _, _)| depth >= d) {
            best = Some((depth, mount.fs_type, mount.source));
        }
    }
    match best {
        Some((_, fs_type, source)) if is_network_fs(fs_type) => StorageKind::Network { fs_type: fs_type.to_string(), source: unescape(source) },
        Some((_, fs_type, _)) => StorageKind::Local { fs_type: fs_type.to_string() },
        None => StorageKind::Unknown,
    }
}

/// Classifies a real path on this machine. Linux reads `/proc/self/mountinfo` (bounded); other
/// systems answer [`StorageKind::Unknown`]. Never touches the path itself, so a dead share can't
/// stall the call.
pub fn classify(path: &Path) -> StorageKind {
    match mount_table() {
        Some(table) => classify_mountinfo(&table, path),
        None => StorageKind::Unknown,
    }
}

/// This machine's mount table text (Linux only, at most 4 MB, reused for 2 s), so many paths
/// can be classified with one read.
pub fn mount_table() -> Option<String> {
    #[cfg(test)]
    if let Some(extra) = test_mounts::extra() {
        return Some(format!("{}\n{extra}", real_mount_table().unwrap_or_default()));
    }
    cached_mount_table()
}

/// How long a read mount table is reused (file calls classify every path; shares come and go).
const TABLE_TTL: std::time::Duration = std::time::Duration::from_secs(2);

/// [`real_mount_table`], read at most every [`TABLE_TTL`].
fn cached_mount_table() -> Option<String> {
    use std::sync::{Mutex, PoisonError};
    static CACHE: Mutex<Option<(web_time::Instant, Option<String>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, table)) = c.as_ref()
        && at.elapsed() < TABLE_TTL
    {
        return table.clone();
    }
    let table = real_mount_table();
    *c = Some((web_time::Instant::now(), table.clone()));
    table
}

fn real_mount_table() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let file = std::fs::File::open("/proc/self/mountinfo").ok()?;
        let mut bytes = Vec::new();
        let limit = u64::try_from(MAX_MOUNTINFO_BYTES).unwrap_or(u64::MAX);
        file.take(limit).read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// How long a share that just timed out is treated as down: its other files fail at once
/// instead of each waiting out the time limit (opening a project with 200 clips on a dead NAS).
pub const DOWN_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// Shares (by mount source) that timed out recently.
static DOWN: std::sync::Mutex<Vec<(String, web_time::Instant)>> = std::sync::Mutex::new(Vec::new());

fn share_down(source: &str) -> bool {
    let mut d = DOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    d.retain(|(_, at)| at.elapsed() < DOWN_FOR);
    d.iter().any(|(s, _)| s == source)
}

fn mark_down(source: &str) {
    let mut d = DOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    d.retain(|(s, at)| s != source && at.elapsed() < DOWN_FOR);
    if d.len() < 256 {
        d.push((source.to_string(), web_time::Instant::now()));
    }
}

/// Whether `path` is on a share that timed out in the last [`DOWN_FOR`].
pub fn share_unreachable(path: &str) -> bool {
    match classify(Path::new(path)) {
        StorageKind::Network { source, .. } => share_down(&source),
        _ => false,
    }
}

/// Runs a file call on `path` with the network time limit when `path` is on a share (`None`
/// when it isn't: the caller does the call itself). A share that timed out in the last
/// [`DOWN_FOR`] fails at once with `TimedOut`; a call that times out marks its share down.
pub fn share_call<T, F>(path: &str, f: F) -> Option<std::io::Result<T>>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    let StorageKind::Network { source, .. } = classify(Path::new(path)) else {
        return None;
    };
    if share_down(&source) {
        return Some(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("{path}: the network share {source} isn't answering"))));
    }
    let r = filmcraft_media::deadline::with_deadline(filmcraft_media::deadline::NETWORK_TIMEOUT, path, f);
    if r.as_ref().err().is_some_and(|e| e.kind() == std::io::ErrorKind::TimedOut) {
        mark_down(&source);
    }
    Some(r)
}

/// Tests: extra mount lines (appended to the real table) so a temp folder can pose as a share.
#[cfg(test)]
pub(crate) mod test_mounts {
    use std::sync::{Mutex, PoisonError};

    static EXTRA: Mutex<String> = Mutex::new(String::new());

    /// Mount `dir` as a cifs share for every later [`super::mount_table`] call in this process.
    pub fn add_share(dir: &std::path::Path) {
        let mut e = EXTRA.lock().unwrap_or_else(PoisonError::into_inner);
        let point = dir.to_string_lossy().replace(' ', "\\040");
        // a source of its own per share, so one test marking its share down leaves the others alone
        let n = e.lines().count();
        e.push_str(&format!("9999 1 0:99 / {point} rw shared:1 - cifs //test/share-{n} rw\n"));
    }

    pub(super) fn extra() -> Option<String> {
        let e = EXTRA.lock().unwrap_or_else(PoisonError::into_inner);
        (!e.is_empty()).then(|| e.clone())
    }
}

/// Where the local copy of the last save of a project on a network share is kept (STORAGE-PLAN
/// S1 item 4: renames over SMB aren't always atomic, so a local copy always exists):
/// `<data dir>/network-backups/<file stem>-<16 hex digits of the full path>.fcproj`.
pub fn backup_path(data_dir: &Path, project: &str) -> PathBuf {
    // FNV-1a: stable across runs and builds (std's hasher isn't).
    let hash = project.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    let stem = Path::new(project).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
    data_dir.join(BACKUP_DIR).join(format!("{stem}-{hash:016x}.fcproj"))
}

/// The folder under the data directory that holds [`backup_path`] copies.
pub const BACKUP_DIR: &str = "network-backups";

/// The `media.storage` command: where the project file and each file media item live.
pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.storage",
        label: "Show Media Storage",
        menu: &[],
        shortcut: None,
        params: r#"{"item":id?}"#,
        enabled: always,
        run: storage,
        journal: false,
    }]
}

/// `media.storage`: `{project: StorageKind|null, items: [{item, path, storage}], networkItems}`
/// for every file media item (or just `item`). Reads only the mount table, never the files, so
/// it answers at once even while a share is down.
fn storage(s: &mut Session, p: &Value) -> Result<Value> {
    let only = item_p(p, "item");
    if let Some(id) = only
        && !s.project.items.contains_key(&id)
    {
        return Err(bad("media.storage", format!("no item {}", id.0)));
    }
    let table = mount_table().unwrap_or_default();
    let kind = |path: &str| classify_mountinfo(&table, Path::new(path));
    let project = s.path.as_deref().map(kind);
    let mut items = Vec::new();
    let mut network = 0usize;
    for (id, item) in &s.project.items {
        if only.is_some_and(|o| o != *id) {
            continue;
        }
        let ItemKind::Media(m) = &item.kind else {
            continue;
        };
        let MediaRef::File { path } = &m.media else {
            continue;
        };
        let storage = kind(path);
        if storage.is_network() {
            network = network.saturating_add(1);
        }
        items.push(json!({"item": id.0, "path": path, "storage": storage}));
    }
    Ok(json!({"project": project, "items": items, "networkItems": network}))
}

struct MountLine<'a> {
    mount_point: String,
    fs_type: &'a str,
    source: &'a str,
}

/// One mountinfo line: `id parent major:minor root mount_point options [optional…] - fstype
/// source super_options`. The optional fields end at a lone `-`.
fn parse_line(line: &str) -> Option<MountLine<'_>> {
    let mut fields = line.split(' ');
    let mount_point = fields.nth(4)?;
    if !mount_point.starts_with('/') {
        return None;
    }
    let mut rest = fields.skip(1);
    rest.by_ref().find(|f| *f == "-")?;
    let fs_type = rest.next().filter(|t| !t.is_empty())?;
    let source = rest.next().unwrap_or("");
    Some(MountLine { mount_point: unescape(mount_point), fs_type, source })
}

/// Undoes the kernel's octal escapes (`\040` space, `\011` tab, `\012` newline, `\134`
/// backslash). Anything malformed is kept as written.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'\\'
            && let Some(digits) = bytes.get(i + 1..i + 4)
            && digits.iter().all(|d| (b'0'..=b'7').contains(d))
        {
            let value = digits.iter().fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
            if let Ok(byte) = u8::try_from(value) {
                out.push(byte);
                i += 4;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lexically resolves `.` and `..` in an absolute path; relative paths give `None`.
fn normalize(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
22 1 0:21 / / rw,relatime shared:1 - btrfs /dev/nvme0n1p4 rw,ssd
23 22 0:22 / /proc rw,nosuid shared:2 - proc proc rw
40 22 0:35 / /home rw,relatime shared:20 - btrfs /dev/nvme0n1p4 rw,subvol=/@home
51 40 0:44 / /home/user/Server rw,relatime shared:30 - cifs //nas/media rw,vers=3.1.1
52 40 0:45 / /mnt/nfs rw,relatime shared:31 - nfs4 nas:/export rw
60 22 0:50 / /run/user/1000/kio-fuse-AbCd rw,nosuid,nodev,relatime shared:40 - fuse.kio-fuse kio-fuse rw,user_id=1000
61 40 0:51 / /home/user/My\\040Share rw,relatime shared:41 - fuse.sshfs user@dell:/home rw
62 51 0:52 /local /home/user/Server/scratch rw,relatime shared:42 - ext4 /dev/sda1 rw
";

    fn kind(path: &str) -> StorageKind {
        classify_mountinfo(TABLE, Path::new(path))
    }

    #[test]
    fn local_paths_are_local() {
        assert_eq!(kind("/home/user/Videos/a.mov"), StorageKind::Local { fs_type: "btrfs".into() });
        assert_eq!(kind("/usr/bin"), StorageKind::Local { fs_type: "btrfs".into() });
    }

    #[test]
    fn smb_nfs_and_kio_fuse_are_network() {
        assert_eq!(kind("/home/user/Server/clips/a.mov"), StorageKind::Network { fs_type: "cifs".into(), source: "//nas/media".into() });
        assert!(kind("/mnt/nfs/x").is_network());
        assert!(kind("/run/user/1000/kio-fuse-AbCd/smb/nas/media/a.mov").is_network());
    }

    #[test]
    fn escaped_spaces_in_mount_points() {
        assert_eq!(kind("/home/user/My Share/clip.mp4"), StorageKind::Network { fs_type: "fuse.sshfs".into(), source: "user@dell:/home".into() });
        // "My" alone is not inside "My Share".
        assert!(!kind("/home/user/My/clip.mp4").is_network());
    }

    #[test]
    fn longest_prefix_wins_including_local_mounts_inside_shares() {
        assert_eq!(kind("/home/user/Server/scratch/x"), StorageKind::Local { fs_type: "ext4".into() });
        // A sibling whose name starts with the mount point's name isn't inside it.
        assert!(!kind("/home/user/ServerOld/x").is_network());
    }

    #[test]
    fn later_mount_over_the_same_point_wins() {
        let table = "1 0 0:1 / / rw - ext4 /dev/a rw\n2 1 0:2 / /m rw - ext4 /dev/b rw\n3 1 0:3 / /m rw - cifs //s/x rw\n";
        assert!(classify_mountinfo(table, Path::new("/m/f")).is_network());
    }

    #[test]
    fn dot_dot_is_resolved() {
        assert!(kind("/home/user/Videos/../Server/a.mov").is_network());
        assert!(!kind("/home/user/Server/../Videos/a.mov").is_network());
    }

    #[test]
    fn hostile_text_never_panics() {
        for table in [
            "",
            "garbage",
            "1 2 3 4",
            "1 2 3 4 /x rw - ",
            "1 2 3 4 /x rw shared:1",
            "1 2 3 4 relative rw - cifs //s rw",
            "\\\\\\ \\0 \\777 \\04",
            "1 2 3 / /\\777 rw - cifs //s\\9 rw",
            "\u{0}\u{ffff} - - - - - -",
        ] {
            let _ = classify_mountinfo(table, Path::new("/x/y"));
            let _ = classify_mountinfo(table, Path::new(""));
        }
        let truncated = &TABLE[..TABLE.len() / 2];
        let _ = classify_mountinfo(truncated, Path::new("/home/user/Server/a"));
        assert_eq!(classify_mountinfo("1 2 3 4 /x rw - ", Path::new("/x/y")), StorageKind::Unknown);
    }

    #[test]
    fn relative_paths_and_empty_tables_are_unknown() {
        assert_eq!(kind("clips/a.mov"), StorageKind::Unknown);
        assert_eq!(classify_mountinfo("", Path::new("/a")), StorageKind::Unknown);
    }

    #[test]
    fn unescape_handles_malformed_escapes() {
        assert_eq!(unescape("a\\040b"), "a b");
        assert_eq!(unescape("a\\134b"), "a\\b");
        assert_eq!(unescape("a\\04"), "a\\04");
        assert_eq!(unescape("a\\777"), "a\\777");
        assert_eq!(unescape("\\"), "\\");
    }

    #[test]
    fn the_real_table_classifies_the_root() {
        // Linux: `/` is always mounted; elsewhere the answer is Unknown.
        let root = classify(Path::new("/"));
        if cfg!(target_os = "linux") {
            assert!(matches!(root, StorageKind::Local { .. } | StorageKind::Network { .. }));
        } else {
            assert_eq!(root, StorageKind::Unknown);
        }
    }

    #[test]
    fn serde_shape() {
        let v = serde_json::to_value(StorageKind::Network { fs_type: "cifs".into(), source: "//nas/m".into() }).unwrap_or_default();
        assert_eq!(v["kind"], "network");
        assert_eq!(v["fsType"], "cifs");
    }

    #[test]
    fn media_storage_command_reports_items_and_project() {
        use crate::media_test_util::{make_movie, session_with, tmp_dir};
        let dir = tmp_dir("netfs-storage");
        let movie = dir.join("a.mov");
        make_movie(&movie, filmcraft_media::DemoScene::Plasma, 64, 36, 2);
        let (mut s, items, _) = session_with(&[&movie]);
        s.path = Some(dir.join("p.fcproj").to_string_lossy().into_owned());
        let r = s.execute("media.storage", json!({})).unwrap_or_default();
        let listed = r["items"].as_array().cloned().unwrap_or_default();
        assert_eq!(listed.len(), 1, "{r}");
        assert_eq!(listed[0]["item"], json!(items[0].0));
        assert!(r["networkItems"].is_u64(), "{r}");
        if cfg!(target_os = "linux") {
            assert!(r["project"]["kind"].is_string(), "{r}");
        }
        let one = s.execute("media.storage", json!({"item": items[0].0})).unwrap_or_default();
        assert_eq!(one["items"].as_array().map(Vec::len), Some(1));
        assert!(s.execute("media.storage", json!({"item": 999_999})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_on_a_share_keep_a_local_copy_and_say_so_when_they_fail() {
        use crate::media_test_util::tmp_dir;
        let root = tmp_dir("netfs-save");
        let share = root.join("share space");
        let data = root.join("data");
        std::fs::create_dir_all(&share).unwrap_or_default();
        test_mounts::add_share(&share);
        assert!(classify(&share.join("p.fcproj")).is_network());
        let mut s = crate::Session { prefs_path: Some(data.join("preferences.json")), ..Default::default() };
        let path = share.join("p.fcproj").to_string_lossy().into_owned();
        let r = s.execute("file.saveAs", json!({"path": path})).unwrap_or_default();
        let copy = r["localCopy"].as_str().unwrap_or_default().to_string();
        assert_eq!(copy, backup_path(&data, &path).to_string_lossy(), "{r}");
        assert_eq!(std::fs::read(&copy).ok(), std::fs::read(&path).ok());
        // a share that refuses the write: the error says where the save is kept
        let gone = share.join("missing dir").join("q.fcproj").to_string_lossy().into_owned();
        let e = s.execute("file.saveCopy", json!({"path": gone})).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("kept on this computer"), "{e}");
        assert!(backup_path(&data, &gone).exists());
        // a local save makes no copy
        let local = root.join("local.fcproj").to_string_lossy().into_owned();
        let r = s.execute("file.saveCopy", json!({"path": local})).unwrap_or_default();
        assert!(r["localCopy"].is_null(), "{r}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn backup_paths_are_stable_and_distinct() {
        let d = Path::new("/data");
        assert_eq!(backup_path(d, "/mnt/a/p.fcproj"), backup_path(d, "/mnt/a/p.fcproj"));
        assert_ne!(backup_path(d, "/mnt/a/p.fcproj"), backup_path(d, "/mnt/b/p.fcproj"));
        assert!(backup_path(d, "/mnt/a/p.fcproj").starts_with("/data/network-backups"));
        let _ = backup_path(d, "");
    }

    #[test]
    fn clips_on_a_share_are_read_through_the_time_limited_reader() {
        use crate::media_test_util::{frame_rgba, make_movie, session_with, tmp_dir};
        let root = tmp_dir("netfs-read");
        let share = root.join("share");
        std::fs::create_dir_all(&share).unwrap_or_default();
        test_mounts::add_share(&share);
        let movie = share.join("a.mov");
        make_movie(&movie, filmcraft_media::DemoScene::Plasma, 64, 36, 3);
        let (mut s, _, _) = session_with(&[&movie]);
        let (w, h, px) = frame_rgba(&mut s, 1, 1.0);
        assert_eq!((w, h), (64, 36));
        assert!(px.iter().any(|&b| b != 0), "decoded through the share reader");
        let r = crate::Services::reader(&crate::FsServices, &movie.to_string_lossy());
        assert!(matches!(r, Some(Ok(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    /// S1 real-share acceptance: a save on a share whose server froze (requests hang instead of
    /// failing) hung forever, because `FsServices::write_file` had no time limit. Writes on a
    /// share now go through `share_call`: a share that timed out fails the write at once (before,
    /// this wrote the file regardless of the share being down).
    #[test]
    fn writes_on_a_share_have_the_network_time_limit() {
        use crate::media_test_util::tmp_dir;
        let root = tmp_dir("netfs-write");
        let share = root.join("share");
        std::fs::create_dir_all(&share).unwrap_or_default();
        test_mounts::add_share(&share);
        let path = share.join("p.fcproj").to_string_lossy().into_owned();
        assert!(crate::Services::write_file(&crate::FsServices, &path, b"one").is_ok());
        assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"one"[..]));
        let StorageKind::Network { source, .. } = classify(&share) else {
            panic!("the test share isn't a share");
        };
        mark_down(&source);
        let e = crate::Services::write_file(&crate::FsServices, &path, b"two").err().map(|e| e.kind());
        assert_eq!(e, Some(std::io::ErrorKind::TimedOut));
        assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"one"[..]), "nothing written while the share is down");
        // a local write is not limited
        let local = root.join("local.fcproj").to_string_lossy().into_owned();
        assert!(crate::Services::write_file(&crate::FsServices, &local, b"x").is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_share_that_timed_out_fails_fast_for_a_while() {
        let source = "//netfs-test-down/share";
        assert!(!share_down(source));
        mark_down(source);
        assert!(share_down(source));
        assert!(!share_down("//another/share"));
        // local paths are never routed through share_call
        assert!(share_call("/proc/self/status", || Ok(1)).is_none_or(|r| r.is_ok()));
    }

    #[test]
    fn a_clip_with_a_local_copy_plays_from_it_when_the_share_is_gone() {
        use crate::media_test_util::{frame_rgba, make_movie, psnr, session_with, tmp_dir};
        let root = tmp_dir("netfs-copy");
        let share = root.join("share");
        std::fs::create_dir_all(&share).unwrap_or_default();
        test_mounts::add_share(&share);
        let movie = share.join("a.mov");
        make_movie(&movie, filmcraft_media::DemoScene::Aurora, 64, 36, 4);
        let (mut s, items, _) = session_with(&[&movie]);
        let good = frame_rgba(&mut s, 2, 1.0).2;
        let cache = std::sync::Arc::new(std::sync::Mutex::new(crate::net_cache::NetCache::open(&root.join("copies"))));
        s.media.set_net_copies(Some(cache.clone()));
        let path = movie.to_string_lossy().into_owned();
        let (size, mtime) = crate::Services::file_stamp(&crate::FsServices, &path).unwrap_or_default();
        let reader = filmcraft_media::reader::FileReader::open(&movie).ok();
        let copied = reader.map(|r| cache.lock().map(|mut c| c.copy(&path, size, mtime, &r, &|| false).is_ok()).unwrap_or(false));
        assert_eq!(copied, Some(true));
        // the file goes away from a working share: offline like any missing file
        std::fs::rename(&movie, share.join("gone.mov")).unwrap_or_default();
        s.media.remove(items[0]);
        let gone = frame_rgba(&mut s, 2, 1.0).2;
        assert!(psnr(&gone, &good) < 30.0, "a missing file isn't hidden by its old copy");
        // the share stops answering: the clip is read from the copy
        if let StorageKind::Network { source, .. } = classify(&movie) {
            mark_down(&source);
        }
        s.media.remove(items[0]);
        let from_copy = frame_rgba(&mut s, 2, 1.0).2;
        assert!(psnr(&from_copy, &good) > 40.0, "the copy's picture, not the offline slate");
        std::fs::rename(share.join("gone.mov"), &movie).unwrap_or_default();
        // a different file back under the same name is another version: the old copy isn't used
        make_movie(&movie, filmcraft_media::DemoScene::CityNight, 64, 36, 4);
        assert!(
            cache
                .lock()
                .map(|c| {
                    let (sz, mt) = crate::Services::file_stamp(&crate::FsServices, &path).unwrap_or_default();
                    c.lookup(&path, sz, mt).is_none()
                })
                .unwrap_or(false)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
