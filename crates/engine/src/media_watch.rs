//! Settings ▸ Media ▸ Automatically refresh growing files: media files that change on disk while
//! the project is open are read again, whether they are still being written (a recording, an
//! ingest) or were replaced (a render from EffectCraft or After Effects written over the file a
//! clip uses).
//!
//! Each file media item's file is compared by size and modification time with what it was when
//! last seen. The desktop app takes the stamps every *Refresh growing files every* seconds and as
//! soon as its window comes back to the front, on a worker thread so a file on a slow network share
//! never holds up the interface ([`Session::start_media_scan`], [`Session::poll_media_scan`]);
//! `media.refreshChanged` takes them on the spot (CLI, agents).
//!
//! Refreshing opens the file again: the item gets the file's new [`MediaInfo`] (a longer
//! duration, another frame size) and identity (so relinking compares against the new file), the
//! media pool swaps in the new source (the old one still indexes the file as it was), and the
//! view is refreshed without an undo step or modifying a clean project ([`Session::bump_view`]):
//! the file changed, not the edit. A file seen for the first time is only remembered; one that
//! can't be opened (still being written) is tried again once it changes again, not on every look.
//!
//! [`MediaInfo`]: filmcraft_media::MediaInfo

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_project::{ItemId, ItemKind, MediaRef, Project};
use serde_json::{Value, json};

use crate::{Result, Session};

/// A file's size and modification time (nanoseconds since 1970).
pub type Stamp = (u64, u128);

/// Stamps of the project's media files taken on a worker thread.
pub type MediaScan = Arc<Mutex<Option<Vec<(String, Option<Stamp>)>>>>;

#[cfg(not(target_arch = "wasm32"))]
fn stamp(path: &str) -> Option<Stamp> {
    // a file on a share that stopped answering counts as not there (S1 item 3b), after a time limit
    let p = path.to_string();
    let meta = match crate::netfs::share_call(path, move || std::fs::metadata(p)) {
        Some(r) => r.ok()?,
        None => std::fs::metadata(path).ok()?,
    };
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos()))
}

#[cfg(target_arch = "wasm32")]
fn stamp(_: &str) -> Option<Stamp> {
    None
}

/// `{refreshed: [ids], online?: [ids], offline?: [ids]}` (the last two only when not empty).
fn scan_result(refreshed: &[u64], online: &[u64], offline: &[u64]) -> Value {
    let mut v = json!({"refreshed": refreshed});
    if !online.is_empty() {
        v["online"] = json!(online);
    }
    if !offline.is_empty() {
        v["offline"] = json!(offline);
    }
    v
}

/// The files of `p`'s file media items (made-offline ones aside).
pub fn media_files(p: &Project) -> BTreeSet<String> {
    p.items
        .values()
        .filter_map(|i| match &i.kind {
            ItemKind::Media(m) if !m.offline => match &m.media {
                MediaRef::File { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// `media.refreshChanged`: stamp the project's media files now and read the changed ones again.
pub(crate) fn refresh_changed(s: &mut Session, _: &Value) -> Result<Value> {
    let now = media_files(&s.project).into_iter().map(|p| (p.clone(), stamp(&p))).collect();
    s.apply_media_stamps(now)
}

impl Session {
    /// Start stamping the project's media files on a worker thread, unless a look is still
    /// running (or the build has no files to look at: the web app).
    pub fn start_media_scan(&mut self) {
        if self.media_scan.is_some() || cfg!(target_arch = "wasm32") {
            return;
        }
        let files = media_files(&self.project);
        if files.is_empty() {
            return;
        }
        let slot: MediaScan = Arc::default();
        let out = slot.clone();
        let spawned = std::thread::Builder::new().name("media-watch".into()).spawn(move || {
            let stamps = files.into_iter().map(|p| (p.clone(), stamp(&p))).collect();
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(stamps);
        });
        if spawned.is_ok() {
            self.media_scan = Some(slot);
        }
    }

    /// Apply a finished look (`None` while there is none, or it is still running).
    pub fn poll_media_scan(&mut self) -> Option<Result<Value>> {
        let stamps = self.media_scan.as_ref()?.lock().unwrap_or_else(PoisonError::into_inner).take()?;
        self.media_scan = None;
        Some(self.apply_media_stamps(stamps))
    }

    /// Compare the media files' stamps with the ones last seen and read the changed files
    /// again. Returns `{refreshed: [item ids]}`.
    fn apply_media_stamps(&mut self, now: Vec<(String, Option<Stamp>)>) -> Result<Value> {
        // Only the files the project uses now are remembered.
        let before: HashMap<String, Stamp> = std::mem::take(&mut self.media_stamps);
        let mut changed = BTreeSet::new();
        let (mut present, mut absent) = (BTreeSet::new(), BTreeSet::new());
        for (path, stamp) in now {
            match (before.get(&path), stamp) {
                // Not there right now (an app saving by delete and rename): keep the last stamp.
                (Some(old), None) => {
                    self.media_stamps.insert(path.clone(), *old);
                    absent.insert(path);
                }
                (old, Some(new)) => {
                    if old.is_some_and(|o| *o != new) {
                        changed.insert(path.clone());
                    }
                    self.media_stamps.insert(path.clone(), new);
                    present.insert(path);
                }
                (None, None) => {
                    absent.insert(path);
                }
            }
        }
        let (online, offline) = self.follow_presence(&present, &absent);
        if changed.is_empty() {
            return Ok(scan_result(&[], &online, &offline));
        }
        let mut fresh = Vec::new();
        let mut failed = BTreeSet::new();
        for (id, it) in &self.project.items {
            let Some(m) = it.as_media().filter(|m| !m.offline) else { continue };
            let MediaRef::File { path } = &m.media else { continue };
            if !changed.contains(path) || failed.contains(path) {
                continue;
            }
            // Opened the way import opens it, so the info is what a new import would get.
            match self.media.open_file(path, &*self.services) {
                Ok(src) => {
                    let mut info = src.info().clone();
                    if info.kind == filmcraft_media::MediaKind::Still
                        && let Some(v) = info.video.as_mut()
                    {
                        v.frame_rate = crate::settings::timebase_rate(&self.prefs.media.indeterminate_timebase);
                    }
                    let identity = crate::relink::identity_of(&*self.services, path).ok();
                    fresh.push((*id, path.clone(), info, identity, src));
                }
                Err(_) => {
                    failed.insert(path.clone());
                }
            }
        }
        // A file that can't be opened (still being written, or not media FilmCraft reads) keeps its
        // new stamp: it is tried again once it changes again, not on every look.
        if fresh.is_empty() {
            return Ok(scan_result(&[], &online, &offline));
        }
        let p = Arc::make_mut(&mut self.project);
        let mut refreshed: Vec<ItemId> = Vec::new();
        for (id, path, info, identity, src) in fresh {
            if let Some(m) = p.item_mut(id).and_then(|i| i.as_media_mut()) {
                m.info = info;
                if identity.is_some() {
                    m.identity = identity;
                }
            }
            self.media.insert_file(id, &path, src);
            refreshed.push(id);
        }
        self.bump_view();
        Ok(scan_result(&refreshed.iter().map(|i| i.0).collect::<Vec<_>>(), &online, &offline))
    }

    /// Clips whose file vanished go offline ("Media missing"), and clips that were missing come
    /// back online once their file is there again, e.g. a network share that dropped and returned
    /// (STORAGE-PLAN S1 item 3b): the cached "Media Offline" slate is dropped so the next look
    /// opens the file again. Works from the stamps the look took on its worker thread, so nothing
    /// here touches the disk. Returns the item ids that came back and went away.
    fn follow_presence(&mut self, present: &BTreeSet<String>, absent: &BTreeSet<String>) -> (Vec<u64>, Vec<u64>) {
        let (mut online, mut offline) = (Vec::new(), Vec::new());
        for (id, it) in &self.project.items {
            let Some(m) = it.as_media().filter(|m| !m.offline) else { continue };
            let MediaRef::File { path } = &m.media else { continue };
            // listed when the look found it gone, or when the project opened without it
            let listed = self.offline.missing.contains(id);
            if present.contains(path) && listed {
                online.push(*id);
            } else if absent.contains(path) && !listed {
                offline.push(*id);
            }
        }
        if online.is_empty() && offline.is_empty() {
            return (Vec::new(), Vec::new());
        }
        self.offline.missing.retain(|i| !online.contains(i));
        for id in &online {
            self.media.remove(*id);
        }
        self.offline.missing.extend(offline.iter().copied());
        self.bump_view();
        (online.iter().map(|i| i.0).collect(), offline.iter().map(|i| i.0).collect())
    }
}
