//! Local copies of clips on network shares (STORAGE-PLAN S1 item 2).
//!
//! Playing 6.2K from a NAS over Wi-Fi stutters, and a share that drops takes the clip with it. So a
//! clip whose file is on a network share can be copied whole into the media cache folder
//! (`<media cache>/Network Copies/`), in the background; once the copy is complete playback reads
//! it instead of the share. A copy belongs to one version of one file: the key is the source path
//! with its size and modification time, so a file changed on the server is copied again.
//!
//! This module is the store: [`NetCache`] keeps `index.json` (hostile input: sizes come from the
//! file system, every number is bounded), finds a complete copy ([`NetCache::lookup`]), makes one
//! ([`NetCache::copy`]: read through the time-limited share reader, written to `*.part`, synced,
//! renamed) and keeps the folder under a size limit ([`NetCache::enforce`]: least recently used
//! first, and only files its own index lists, so whatever else is in the folder is left alone).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `mediaCache.networkCopies` choices.
pub const MODES: &[(&str, &str)] = &[("auto", "Automatic (clips of the open project)"), ("off", "Off")];

/// Default size limit (`mediaCache.networkCopiesGb`).
pub const DEFAULT_GB: u32 = 100;

/// The folder under the media cache that holds the copies.
pub const DIR_NAME: &str = "Network Copies";

const INDEX: &str = "index.json";
const MAX_INDEX_BYTES: u64 = 16 << 20;
const MAX_ENTRIES: usize = 100_000;
/// Copies are deleted down to this share of the limit, so a full cache doesn't evict on every copy.
const LOW_WATER_PERCENT: u64 = 90;
/// Read size while copying.
const CHUNK: usize = 4 << 20;

/// One copy in the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyEntry {
    /// The file on the share.
    pub source: String,
    pub size: u64,
    /// Modification time of the source, nanoseconds since 1970.
    pub mtime: u128,
    /// The copy's file name in the folder.
    pub file: String,
    /// When playback last used it (seconds since 1970).
    pub last_used: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: BTreeMap<String, CopyEntry>,
}

/// The local copies in one folder.
pub struct NetCache {
    dir: PathBuf,
    entries: BTreeMap<String, CopyEntry>,
    /// Bytes of the listed copies, kept up to date (taken from the file system when opened).
    used: u64,
}

/// The key of a version of a source file (FNV-1a 64 over path, size and time: stable across runs).
pub fn key_of(source: &str, size: u64, mtime: u128) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in source.bytes().chain(size.to_le_bytes()).chain(mtime.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `<key>-<file name>`, with anything but letters, digits, `.`, `-` and `_` replaced, at most 80
/// characters of the name.
fn file_name_for(key: &str, source: &str) -> String {
    let name = Path::new(source).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let clean: String =
        name.chars().filter(|c| !c.is_control()).map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(80).collect();
    format!("{key}-{clean}")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A plain file name we made (no separators, no `..`), so an index entry can't name a file
/// elsewhere.
fn own_file(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.contains(['/', '\\', '\0']) && name != "." && name != ".." && !name.ends_with(".part")
}

impl NetCache {
    /// The copies in `dir` (made if missing). A missing or unreadable index starts empty; entries
    /// naming files that aren't plain names in the folder are dropped.
    pub fn open(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let entries = read_capped(&dir.join(INDEX))
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .map(|i| i.entries.into_iter().filter(|(_, e)| own_file(&e.file)).take(MAX_ENTRIES).collect())
            .unwrap_or_default();
        let mut c = Self { dir: dir.to_path_buf(), entries, used: 0 };
        c.used = c.used_bytes();
        c
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The complete local copy of this version of `source`, if there is one (the file is there
    /// with the source's size).
    pub fn lookup(&self, source: &str, size: u64, mtime: u128) -> Option<PathBuf> {
        let e = self.entries.get(&key_of(source, size, mtime))?;
        let path = self.dir.join(&e.file);
        let meta = std::fs::symlink_metadata(&path).ok()?;
        (meta.is_file() && meta.len() == size && e.source == source).then_some(path)
    }

    /// Whether a copy of this version is listed (no file system calls: for the interface).
    pub fn lookup_listed(&self, source: &str, size: u64, mtime: u128) -> bool {
        self.entries.get(&key_of(source, size, mtime)).is_some_and(|e| e.source == source)
    }

    /// The most recently copied complete version of `source` (for when the share doesn't answer
    /// and the current version can't be asked).
    pub fn latest_for(&self, source: &str) -> Option<PathBuf> {
        self.entries
            .values()
            .filter(|e| e.source == source)
            .filter(|e| std::fs::symlink_metadata(self.dir.join(&e.file)).is_ok_and(|m| m.is_file() && m.len() == e.size))
            .max_by_key(|e| (e.mtime, e.last_used))
            .map(|e| self.dir.join(&e.file))
    }

    /// Notes that playback used this copy (the size limit deletes the least recently used first).
    pub fn touch(&mut self, source: &str, size: u64, mtime: u128) {
        if let Some(e) = self.entries.get_mut(&key_of(source, size, mtime)) {
            e.last_used = now_secs();
        }
    }

    /// Copies `source` (this version: `size`, `mtime`) into the folder through `read` (a reader
    /// with the network time limit), checking `cancel` between chunks. The copy is written to a
    /// `.part` file, synced and renamed, so a half copy is never used; a failed copy leaves nothing.
    pub fn copy(&mut self, source: &str, size: u64, mtime: u128, read: &dyn filmcraft_media::ByteReader, cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
        let (key, entry) = copy_file(&self.dir, source, size, mtime, read, cancel, &|| Some((size, mtime)))?;
        let path = self.dir.join(&entry.file);
        self.adopt([(key, entry)].into_iter().collect());
        self.save();
        Ok(path)
    }

    /// The bytes of the copies this index lists (from the file system, not the index).
    pub fn used_bytes(&self) -> u64 {
        self.entries
            .values()
            .filter_map(|e| std::fs::symlink_metadata(self.dir.join(&e.file)).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .fold(0u64, u64::saturating_add)
    }

    /// Keeps the copies under `limit` bytes: above it, the least recently used are deleted down
    /// to 90 % of it. Copies whose source is in `keep` (clips of the open project) go last. Only
    /// files this index lists are ever deleted. Returns how many were deleted.
    pub fn enforce(&mut self, limit: u64, keep: &std::collections::HashSet<String>) -> usize {
        let mut used = self.used;
        if used <= limit {
            return 0;
        }
        let target = limit / 100 * LOW_WATER_PERCENT;
        let mut order: Vec<(bool, u64, String)> = self.entries.iter().map(|(k, e)| (keep.contains(&e.source), e.last_used, k.clone())).collect();
        order.sort();
        let mut n = 0;
        for (_, _, key) in order {
            if used <= target {
                break;
            }
            let Some(e) = self.entries.remove(&key) else { continue };
            let p = self.dir.join(&e.file);
            if std::fs::remove_file(&p).is_ok() || !p.exists() {
                used = used.saturating_sub(e.size);
                n += 1;
            } else {
                // still open (a decoder on Windows): kept listed, so it is counted and deleted later
                self.entries.insert(key, e);
            }
        }
        self.used = used;
        self.save();
        n
    }

    /// Takes entries made by a copy that ran outside this index's lock.
    fn adopt(&mut self, entries: BTreeMap<String, CopyEntry>) {
        for (k, e) in entries {
            if self.entries.len() < MAX_ENTRIES || self.entries.contains_key(&k) {
                let size = e.size;
                if let Some(old) = self.entries.insert(k, e) {
                    self.used = self.used.saturating_sub(old.size);
                }
                self.used = self.used.saturating_add(size);
            }
        }
    }

    /// Whether any copy of `source` is listed (no file system calls).
    pub fn has_source(&self, source: &str) -> bool {
        self.entries.values().any(|e| e.source == source)
    }

    /// The bytes the listed copies take (kept up to date, no file system calls).
    pub fn used(&self) -> u64 {
        self.used
    }

    /// Writes the index (atomically).
    pub fn save(&self) {
        let idx = Index { version: 1, entries: self.entries.clone() };
        let _ = filmcraft_format::atomic_write(&self.dir.join(INDEX), &serde_json::to_vec(&idx).unwrap_or_default());
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What the copier finished: `None` when there was nothing to copy.
type CopyResult = Option<(String, Result<PathBuf, String>)>;

/// A copy being made on a worker thread.
pub struct CopyJob {
    source: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::Arc<std::sync::Mutex<Option<CopyResult>>>,
}

struct CopyDone {
    source: String,
}

impl CopyJob {
    /// The file being copied, once the copier has picked one.
    pub fn source(&self) -> Option<String> {
        self.source.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Asks the copy to stop (it removes its partial file).
    pub fn stop(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl crate::Session {
    /// Starts copying the next clip of the open project that is on a network share and has no
    /// complete local copy yet, unless a copy is running, copies are off, or the size limit is
    /// reached. One file at a time, on a worker thread, read with the network time limit.
    /// Returns the file being copied.
    pub fn start_net_copy(&mut self) -> bool {
        if self.net_copy.is_some() || cfg!(target_arch = "wasm32") {
            return false;
        }
        if self.net_copy_quiet_until.is_some_and(|t| web_time::Instant::now() < t) {
            return false;
        }
        let Some(cache) = self.media.net_copies() else { return false };
        let limit = u64::from(self.prefs.media_cache.network_copies_gb).saturating_mul(1_000_000_000);
        let files: Vec<String> = crate::media_watch::media_files(&self.project)
            .into_iter()
            .filter(|f| !self.net_copy_failed.contains(f) && crate::netfs::classify(Path::new(f)).is_network())
            .collect();
        {
            // only the index under the lock: no network calls
            let mut c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let keep: std::collections::HashSet<String> = files.iter().cloned().collect();
            c.enforce(limit, &keep);
        }
        if files.is_empty() {
            return false;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = std::sync::Arc::new(std::sync::Mutex::new(None));
        let source = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (c2, d2, s2) = (cancel.clone(), done.clone(), source.clone());
        let (services, media) = (self.services.clone(), self.media.clone());
        let run = move || {
            let stop = || c2.load(std::sync::atomic::Ordering::Relaxed);
            let dir = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).dir().to_path_buf();
            // pick the first file without a complete copy of its current version (the share is
            // asked here, on the worker, never under the cache lock)
            let pick = files.into_iter().find_map(|f| {
                if stop() {
                    return None;
                }
                let (size, mtime) = services.file_stamp(&f)?;
                let c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                (c.lookup(&f, size, mtime).is_none() && c.used().saturating_add(size) <= limit).then_some((f, size, mtime))
            });
            let Some((src, size, mtime)) = pick else {
                *d2.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(None);
                return;
            };
            *s2.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(src.clone());
            let r = match services.reader(&src) {
                Some(Ok(reader)) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    copy_file(&dir, &src, size, mtime, &*reader, &stop, &|| services.file_stamp(&src))
                }))
                .unwrap_or_else(|_| Err("the copy failed".into())),
                Some(Err(e)) => Err(e.to_string()),
                None => Err("this build can't read files".into()),
            };
            // into the cache the pool holds now (the setting may have been switched meanwhile)
            let r = r.map(|(key, entry)| {
                let path = dir.join(&entry.file);
                if let Some(now) = media.net_copies() {
                    let mut c = now.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if c.dir() == dir {
                        c.adopt([(key, entry)].into_iter().collect());
                        c.save();
                    }
                }
                path
            });
            *d2.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Some((src, r)));
        };
        if std::thread::Builder::new().name("filmcraft-net-copy".into()).spawn(run).is_err() {
            return false;
        }
        self.net_copy = Some(CopyJob { source, cancel, done });
        true
    }

    /// Collects a finished copy: the clip is opened from the copy from now on (its cached source
    /// is dropped). Returns `(source, result)` once.
    pub fn poll_net_copy(&mut self) -> Option<(String, Result<PathBuf, String>)> {
        let finished = self.net_copy.as_ref()?.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
        self.net_copy = None;
        let Some((source, r)) = finished else {
            // nothing to copy: look again in a while
            self.net_copy_quiet_until = Some(web_time::Instant::now() + std::time::Duration::from_secs(30));
            return None;
        };
        let job = CopyDone { source };
        match &r {
            Ok(_) => {
                let ids: Vec<filmcraft_project::ItemId> = self
                    .project
                    .items
                    .iter()
                    .filter(|(_, i)| matches!(i.as_media().map(|m| &m.media), Some(filmcraft_project::MediaRef::File { path }) if *path == job.source))
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    self.media.remove(id);
                }
                self.bump_view();
            }
            // stopped for playback: tried again later
            Err(e) if e == "stopped" => {}
            Err(_) => {
                self.net_copy_failed.insert(job.source.clone());
            }
        }
        Some((job.source, r))
    }
}

/// Copies `source` into `dir` as the copy of this version (`*.part`, synced, renamed); the
/// index entry for it is returned, not written.
fn copy_file(
    dir: &Path,
    source: &str,
    size: u64,
    mtime: u128,
    read: &dyn filmcraft_media::ByteReader,
    cancel: &dyn Fn() -> bool,
    restat: &dyn Fn() -> Option<(u64, u128)>,
) -> Result<(String, CopyEntry), String> {
    if read.len() != size {
        return Err(format!("{source} changed while it was being copied"));
    }
    let key = key_of(source, size, mtime);
    let file = file_name_for(&key, source);
    let path = dir.join(&file);
    let part = dir.join(format!("{file}.part"));
    let r = (|| -> Result<(), String> {
        // a copy stopped for playback goes on where it stopped (the part is of this version: the
        // key is in its name)
        let have = std::fs::symlink_metadata(&part).ok().filter(|m| m.is_file()).map_or(0, |m| m.len());
        let mut at = if have <= size { have } else { 0 };
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(at > 0)
            .write(true)
            .truncate(at == 0)
            .open(&part)
            .map_err(|e| format!("{}: {e}", part.display()))?;
        let mut buf = vec![0u8; CHUNK];
        while at < size {
            if cancel() {
                return Err("stopped".into());
            }
            let n = usize::try_from((size - at).min(CHUNK as u64)).unwrap_or(CHUNK);
            let chunk = buf.get_mut(..n).ok_or("copy buffer")?;
            read.read_at(at, chunk).map_err(|e| format!("{source}: {e}"))?;
            out.write_all(chunk).map_err(|e| format!("{}: {e}", part.display()))?;
            at = at.saturating_add(n as u64);
        }
        out.sync_all().map_err(|e| format!("{}: {e}", part.display()))?;
        drop(out);
        // a file rewritten on the server while it was copied (same size) is not kept as this version
        if restat() != Some((size, mtime)) {
            return Err(format!("{source} changed while it was being copied"));
        }
        std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))
    })();
    match r {
        Ok(()) => {}
        // kept to go on from later
        Err(e) if e == "stopped" => return Err(e),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    Ok((key, CopyEntry { source: source.into(), size, mtime, file, last_used: now_secs() }))
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut b = Vec::new();
    f.take(MAX_INDEX_BYTES).read_to_end(&mut b).ok()?;
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_media::reader::MemReader;
    use std::sync::Arc;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fc-netcache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn data(n: usize, seed: u8) -> Arc<[u8]> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect::<Vec<_>>().into()
    }

    #[test]
    fn copies_are_found_by_version_and_survive_reopening() {
        let d = tmp("copy");
        let mut c = NetCache::open(&d);
        let bytes = data(10_000, 1);
        let src = "/mnt/nas/Shoot/A001 clip.mov";
        let p = c.copy(src, 10_000, 42, &MemReader(bytes.clone()), &|| false).unwrap_or_default();
        assert_eq!(std::fs::read(&p).ok().as_deref(), Some(&bytes[..]));
        assert_eq!(c.lookup(src, 10_000, 42), Some(p.clone()));
        assert_eq!(c.lookup(src, 10_000, 43), None, "a changed source is another version");
        let again = NetCache::open(&d);
        assert_eq!(again.lookup(src, 10_000, 42), Some(p));
        assert!(!std::fs::read_dir(&d).map(|r| r.flatten().any(|e| e.path().to_string_lossy().ends_with(".part"))).unwrap_or(true));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failed_or_stopped_copy_leaves_nothing() {
        let d = tmp("fail");
        let mut c = NetCache::open(&d);
        // the reader is shorter than the size the stamp said
        assert!(c.copy("/s/a.mov", 100, 1, &MemReader(data(50, 0)), &|| false).is_err());
        assert!(c.is_empty());
        let files: Vec<_> = std::fs::read_dir(&d).map(|r| r.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(files.iter().all(|f| f == INDEX), "{files:?}");
        // a stopped copy keeps its part and goes on from it next time
        assert!(c.copy("/s/b.mov", 50, 1, &MemReader(data(50, 0)), &|| true).is_err());
        assert!(c.is_empty() && c.lookup("/s/b.mov", 50, 1).is_none());
        let part = d.join(format!("{}.part", file_name_for(&key_of("/s/b.mov", 50, 1), "/s/b.mov")));
        let full = data(50, 0);
        let _ = std::fs::write(&part, &full[..20]);
        // the reader would give zeros for the first 20 bytes: a resumed copy never reads them again
        let mut tail = full.to_vec();
        tail[..20].fill(0);
        let p = c.copy("/s/b.mov", 50, 1, &MemReader(tail.into()), &|| false).unwrap_or_default();
        assert_eq!(std::fs::read(p).ok().as_deref(), Some(&full[..]), "resumed from byte 20");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_size_limit_deletes_least_recently_used_and_keeps_open_clips_longest() {
        let d = tmp("limit");
        let mut c = NetCache::open(&d);
        for (i, src) in ["/s/old.mov", "/s/open.mov", "/s/new.mov"].iter().enumerate() {
            let _ = c.copy(src, 1000, 1, &MemReader(data(1000, i as u8)), &|| false);
            if let Some(e) = c.entries.get_mut(&key_of(src, 1000, 1)) {
                e.last_used = i as u64; // old.mov least recent
            }
        }
        let stranger = d.join("not-ours.bin");
        let _ = std::fs::write(&stranger, [0u8; 5000]);
        let keep: std::collections::HashSet<String> = ["/s/open.mov".to_string()].into();
        assert_eq!(c.enforce(10_000, &keep), 0, "under the limit");
        assert_eq!(c.enforce(2000, &keep), 2);
        assert!(c.lookup("/s/open.mov", 1000, 1).is_some(), "the open project's clip goes last");
        assert!(c.lookup("/s/old.mov", 1000, 1).is_none() && c.lookup("/s/new.mov", 1000, 1).is_none());
        assert!(stranger.exists(), "files the index doesn't list are never deleted");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn hostile_indexes_are_survived() {
        let d = tmp("hostile");
        let _ = std::fs::create_dir_all(&d);
        let outside = tmp("hostile-outside");
        let _ = std::fs::create_dir_all(&outside);
        let victim = outside.join("victim.txt");
        let _ = std::fs::write(&victim, b"keep me");
        let evil = format!(
            r#"{{"version":1,"entries":{{"k":{{"source":"/s","size":7,"mtime":1,"file":"../{}/victim.txt","lastUsed":0}},"z":{{"source":"/s","size":1,"mtime":1,"file":"..","lastUsed":0}}}}}}"#,
            outside.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let _ = std::fs::write(d.join(INDEX), evil);
        let mut c = NetCache::open(&d);
        assert!(c.is_empty(), "entries naming files outside the folder are dropped");
        c.enforce(0, &Default::default());
        assert!(victim.exists());
        for junk in ["", "[", "{\"entries\":5}", "null"] {
            let _ = std::fs::write(d.join(INDEX), junk);
            assert!(NetCache::open(&d).is_empty());
        }
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn copy_names_are_plain_and_bounded() {
        let n = file_name_for("abc", "/x/../we ird\u{0}/na:me*?.mov");
        assert!(own_file(&n), "{n}");
        assert!(file_name_for("k", &"a".repeat(1000)).len() <= 100);
        assert_ne!(key_of("/a", 1, 1), key_of("/a", 1, 2));
    }

    #[test]
    fn the_session_copies_share_clips_one_at_a_time_and_switches_them_over() {
        use crate::media_test_util::{make_movie, session_with, tmp_dir};
        let root = tmp_dir("netcache-session");
        let share = root.join("share");
        let _ = std::fs::create_dir_all(&share);
        crate::netfs::test_mounts::add_share(&share);
        let (a, b) = (share.join("a.mov"), share.join("b.mov"));
        make_movie(&a, filmcraft_media::DemoScene::Aurora, 64, 36, 3);
        make_movie(&b, filmcraft_media::DemoScene::Dunes, 64, 36, 3);
        let (mut s, _, _) = session_with(&[&a, &b]);
        assert!(!s.start_net_copy(), "copies off: nothing");
        let cache = Arc::new(std::sync::Mutex::new(NetCache::open(&root.join("copies"))));
        s.media.set_net_copies(Some(cache.clone()));
        let mut copied = Vec::new();
        for _ in 0..2 {
            assert!(s.start_net_copy());
            assert!(!s.start_net_copy(), "one at a time");
            let t0 = web_time::Instant::now();
            let r = loop {
                if let Some(r) = s.poll_net_copy() {
                    break r;
                }
                assert!(t0.elapsed() < std::time::Duration::from_secs(20));
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert!(r.1.is_ok(), "{r:?}");
            copied.push(r.0);
        }
        copied.sort();
        assert_eq!(copied, vec![a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()]);
        // nothing left: the copier finds nothing and rests
        assert!(s.start_net_copy());
        let t0 = web_time::Instant::now();
        while s.net_copy.is_some() {
            assert!(s.poll_net_copy().is_none());
            assert!(t0.elapsed() < std::time::Duration::from_secs(20));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!s.start_net_copy(), "resting after finding nothing");
        assert_eq!(cache.lock().map(|c| c.len()).unwrap_or(0), 2);
        assert_eq!(NetCache::open(&root.join("copies")).len(), 2, "the index on disk has both");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_source_rewritten_during_the_copy_is_not_kept() {
        let d = tmp("restat");
        let _ = std::fs::create_dir_all(&d);
        let r = copy_file(&d, "/s/c.mov", 30, 5, &MemReader(data(30, 3)), &|| false, &|| Some((30, 6)));
        assert!(r.err().unwrap_or_default().contains("changed"));
        let files: Vec<_> = std::fs::read_dir(&d).map(|r| r.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(files.is_empty(), "{files:?}");
        let _ = std::fs::remove_dir_all(d);
    }
}
