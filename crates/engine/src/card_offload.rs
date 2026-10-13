//! Card offload (LINUX-PLAN L2.4): copying a camera card to a media drive with every file
//! verified, before anything is imported from it.
//!
//! - [`detect`] tells a camera card from any other drive by its folders (`DCIM`, `PRIVATE`,
//!   `XDROOT`, `CONTENTS`, `CLIP`; any letter case, since FAT / exFAT mounts differ).
//! - [`list`] walks the card without following links, skipping hidden files and the system
//!   folders other computers leave behind, with caps on depth and counts (a card is hostile
//!   input: a corrupt file system can loop or hold millions of entries).
//! - [`copy_verified`] copies one file to a temporary name next to its destination, checks the
//!   source didn't change size while it was read (a card pulled mid-copy), flushes the copy to
//!   disk, reads it back and compares CRC-32s before giving it its real name. A failed or
//!   cancelled copy leaves nothing behind; an identical file already there is kept and counted as
//!   copied, so an interrupted offload can simply be run again.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// Folders that mark a camera card (top level, compared ignoring case), with the layout's name.
pub const CARD_MARKERS: &[(&str, &str)] = &[("DCIM", "dcim"), ("PRIVATE", "private"), ("XDROOT", "xdcam"), ("CONTENTS", "p2"), ("CLIP", "clip")];

/// Deepest folder level [`list`] goes into (camera layouts use 4–5).
pub const MAX_DEPTH: usize = 12;
/// Most files [`list`] returns, and most directory entries it reads in all.
pub const MAX_FILES: usize = 20_000;
pub const MAX_ENTRIES: usize = 200_000;
/// Copy buffer.
const CHUNK: usize = 1 << 20;

/// Names never copied: other computers' bookkeeping (hidden files are skipped too).
const SKIPPED: &[&str] = &["System Volume Information", "$RECYCLE.BIN", "Thumbs.db", "desktop.ini"];

/// The layout of a camera card at `mount` (`dcim`, `private`, …), or `None` for any other drive.
pub fn detect(mount: &Path) -> Option<&'static str> {
    let entries = std::fs::read_dir(mount).ok()?;
    let mut found: Option<&'static str> = None;
    for e in entries.take(MAX_ENTRIES).flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(&(_, kind)) = CARD_MARKERS.iter().find(|(m, _)| m.eq_ignore_ascii_case(name)) else { continue };
        // a real folder, not a file or a link that happens to have the name
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            // the first marker in CARD_MARKERS order wins (a card with DCIM and PRIVATE is "dcim")
            let rank = |k: &str| CARD_MARKERS.iter().position(|(_, x)| *x == k).unwrap_or(usize::MAX);
            if found.is_none_or(|f| rank(kind) < rank(f)) {
                found = Some(kind);
            }
        }
    }
    found
}

/// A file on the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardFile {
    /// Where it is.
    pub src: PathBuf,
    /// Its path relative to the card (kept in the copy, so camera folder structures survive).
    pub rel: PathBuf,
    /// Its size when listed.
    pub size: u64,
}

/// What [`list`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub files: Vec<CardFile>,
    /// Total size of `files`.
    pub bytes: u64,
    /// True when a cap stopped the walk (some files weren't listed).
    pub truncated: bool,
}

/// The files on the card at `mount`, sorted by relative path. Links are never followed, hidden
/// files and [`SKIPPED`] names are left out, unreadable folders are skipped.
pub fn list(mount: &Path) -> std::io::Result<Listing> {
    list_capped(mount, MAX_FILES, MAX_ENTRIES)
}

pub(crate) fn list_capped(mount: &Path, max_files: usize, max_entries: usize) -> std::io::Result<Listing> {
    // the mount itself must be readable; deeper folders are skipped when they aren't
    std::fs::read_dir(mount)?;
    let mut out = Listing::default();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(PathBuf::new(), 0)];
    while let Some((rel_dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(mount.join(&rel_dir)) else { continue };
        for e in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > max_entries {
                out.truncated = true;
                break;
            }
            let name = e.file_name();
            let skip = name.to_str().is_some_and(|n| n.starts_with('.') || SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(n)));
            if skip {
                continue;
            }
            // `DirEntry::file_type` doesn't follow links
            let Ok(t) = e.file_type() else { continue };
            let rel = rel_dir.join(&name);
            if t.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((rel, depth.saturating_add(1)));
                } else {
                    out.truncated = true;
                }
            } else if t.is_file() {
                if out.files.len() >= max_files {
                    out.truncated = true;
                    continue;
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.bytes = out.bytes.saturating_add(size);
                out.files.push(CardFile { src: e.path(), rel, size });
            }
        }
        if seen > max_entries {
            break;
        }
    }
    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// Why a copy failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// The caller asked to stop.
    Cancelled,
    /// The source ended early, grew or couldn't be read (a card pulled out mid-copy, a failing card).
    SourceChanged(String),
    /// Reading back the copy gave different bytes.
    Mismatch,
    /// A different file is already at the destination; it's left alone.
    Exists,
    /// Any other read / write error (a full or read-only drive…).
    Io(String),
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyError::Cancelled => write!(f, "cancelled"),
            CopyError::SourceChanged(e) => write!(f, "the card changed or couldn't be read while copying ({e})"),
            CopyError::Mismatch => write!(f, "the copy doesn't match the card (read back with different bytes)"),
            CopyError::Exists => write!(f, "a different file with that name is already there"),
            CopyError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A verified copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Copied {
    /// CRC-32 of the file.
    pub crc: u32,
    /// Bytes copied (0 when an identical file was already there).
    pub bytes: u64,
    /// True when an identical file was already at the destination.
    pub already: bool,
}

/// Copy `src` (`size` bytes when listed) to `dst`, verified. `progress` gets the bytes copied so
/// far in this file and returns `false` to cancel. Missing folders of `dst` are made.
pub fn copy_verified(src: &Path, size: u64, dst: &Path, progress: &mut dyn FnMut(u64) -> bool) -> std::result::Result<Copied, CopyError> {
    let io = |what: &str, p: &Path, e: std::io::Error| CopyError::Io(format!("{what} {}: {e}", p.display()));
    if let Ok(m) = std::fs::symlink_metadata(dst) {
        if !m.is_file() || m.len() != size {
            return Err(CopyError::Exists);
        }
        // the same name and size: the same file only if the bytes match (both reads report
        // progress, half each, and can be cancelled: a resumed offload re-reads the whole card)
        let a = crc_of(src, size, &mut |d| progress(d / 2))?;
        let b = match crc_of(dst, size, &mut |d| progress(size.saturating_sub(size.saturating_sub(d) / 2))) {
            Ok(b) => b,
            Err(CopyError::Cancelled) => return Err(CopyError::Cancelled),
            Err(_) => return Err(CopyError::Exists),
        };
        return if a == b { Ok(Copied { crc: a, bytes: 0, already: true }) } else { Err(CopyError::Exists) };
    }
    let dir = dst.parent().ok_or_else(|| CopyError::Io(format!("{} has no folder", dst.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io("making", dir, e))?;
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part = dir.join(format!(".{name}.filmcraft-part"));
    let r = copy_to_part(src, size, &part, progress);
    let crc = match r {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    // never replace a file that appeared meanwhile
    if std::fs::symlink_metadata(dst).is_ok() {
        let _ = std::fs::remove_file(&part);
        return Err(CopyError::Exists);
    }
    if let Err(e) = std::fs::rename(&part, dst) {
        let _ = std::fs::remove_file(&part);
        return Err(io("naming", dst, e));
    }
    Ok(Copied { crc, bytes: size, already: false })
}

/// Copy into `part`, flush it to disk and read it back; the CRC-32 when both sides agree.
fn copy_to_part(src: &Path, size: u64, part: &Path, progress: &mut dyn FnMut(u64) -> bool) -> std::result::Result<u32, CopyError> {
    let mut input = std::fs::File::open(src).map_err(|e| CopyError::SourceChanged(e.to_string()))?;
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(part).map_err(|e| CopyError::Io(format!("{}: {e}", part.display())))?;
    let mut buf = vec![0u8; CHUNK];
    let mut hash = crc32fast::Hasher::new();
    let mut done = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(|e| CopyError::SourceChanged(e.to_string()))?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).ok_or(CopyError::Mismatch)?;
        done = done.saturating_add(n as u64);
        if done > size {
            return Err(CopyError::SourceChanged("it grew".into()));
        }
        hash.update(chunk);
        out.write_all(chunk).map_err(|e| CopyError::Io(format!("writing {}: {e}", part.display())))?;
        if !progress(done) {
            return Err(CopyError::Cancelled);
        }
    }
    if done != size {
        return Err(CopyError::SourceChanged(format!("{done} of {size} bytes")));
    }
    out.sync_all().map_err(|e| CopyError::Io(format!("writing {}: {e}", part.display())))?;
    drop(out);
    let crc = hash.finalize();
    // read the copy back
    let back = crc_of(part, size, &mut |_| true).map_err(|e| match e {
        CopyError::SourceChanged(_) => CopyError::Mismatch,
        e => e,
    })?;
    if back != crc {
        return Err(CopyError::Mismatch);
    }
    Ok(crc)
}

/// CRC-32 of the file at `path`, which must be exactly `size` bytes.
pub fn crc_of(path: &Path, size: u64, progress: &mut dyn FnMut(u64) -> bool) -> std::result::Result<u32, CopyError> {
    let mut f = std::fs::File::open(path).map_err(|e| CopyError::SourceChanged(e.to_string()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut hash = crc32fast::Hasher::new();
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| CopyError::SourceChanged(e.to_string()))?;
        if n == 0 {
            break;
        }
        done = done.saturating_add(n as u64);
        if done > size {
            return Err(CopyError::SourceChanged("it grew".into()));
        }
        hash.update(buf.get(..n).ok_or(CopyError::Mismatch)?);
        if !progress(done) {
            return Err(CopyError::Cancelled);
        }
    }
    if done != size {
        return Err(CopyError::SourceChanged(format!("{done} of {size} bytes")));
    }
    Ok(hash.finalize())
}

// ---------- commands: card.inspect, card.offload ----------

/// Longest offload folder name kept.
const MAX_NAME: usize = 120;
/// The checksum list written next to the copied files.
pub const MANIFEST: &str = "offload-crc32.txt";

/// A running offload; [`poll`] imports its files when it finishes.
pub struct PendingOffload {
    pub job: u64,
    /// The bin the copies are imported into (`None`: don't import).
    pub bin: Option<String>,
    /// Verified copies, in card order.
    pub copied: Arc<Mutex<Vec<String>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An absolute, existing folder from `p[key]`.
fn folder_p(p: &Value, key: &str, cmd: &str) -> Result<PathBuf> {
    let s = str_p(p, key).filter(|s| !s.is_empty() && !s.contains('\0')).ok_or_else(|| bad(cmd, format!("give `{key}` (a folder)")))?;
    let path = PathBuf::from(s);
    if !path.is_absolute() || !path.is_dir() {
        return Err(bad(cmd, format!("{s} is not a folder")));
    }
    Ok(path)
}

/// A folder name from free text: no separators, no control characters, no `.` / `..`, capped.
pub fn clean_name(s: &str) -> Option<String> {
    let name: String = s.chars().filter(|c| !c.is_control() && *c != '/' && *c != '\\').take(MAX_NAME).collect();
    let name = name.trim().to_string();
    (!name.is_empty() && name != "." && name != "..").then_some(name)
}

fn inspect(_: &mut Session, p: &Value) -> Result<Value> {
    let mount = folder_p(p, "mount", "card.inspect")?;
    let l = list(&mount).map_err(|e| EngineError::Other(format!("{}: {e}", mount.display())))?;
    let media = l.files.iter().filter(|f| filmcraft_media::is_importable(&f.src)).count();
    Ok(json!({"layout": detect(&mount), "files": l.files.len(), "media": media, "bytes": l.bytes, "truncated": l.truncated}))
}

fn offload(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "card.offload";
    let mount = folder_p(p, "mount", CMD)?;
    let dest_root = folder_p(p, "destination", CMD)?;
    // never copy a card into itself (or onto the card): it would copy its own copies
    let (m, d) = (mount.canonicalize().unwrap_or_else(|_| mount.clone()), dest_root.canonicalize().unwrap_or_else(|_| dest_root.clone()));
    if d.starts_with(&m) {
        return Err(bad(CMD, "the destination is on the card; pick a folder on another drive"));
    }
    // default: the card's name and today's date, so the next card with the same name (cameras
    // reuse it) gets its own folder instead of meeting this card's file names
    let label = mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0);
    let offset = s.persistence.as_ref().map_or(0, |per| (per.local_offset)(now));
    let default_name = format!("{label} {}", crate::projects::iso_local(now, offset).get(..10).unwrap_or_default());
    let name = clean_name(str_p(p, "name").unwrap_or(&default_name)).or_else(|| clean_name("Card")).unwrap_or_default();
    let folder = dest_root.join(&name);
    // the copy's folder can't be the card either (the card's parent folder with the default name)
    if d.join(&name).starts_with(&m) || m.starts_with(d.join(&name)) {
        return Err(bad(CMD, "the copy would land on the card; pick another destination or name"));
    }
    let l = list(&mount).map_err(|e| EngineError::Other(format!("{}: {e}", mount.display())))?;
    if l.files.is_empty() {
        return Err(EngineError::Other(format!("there are no files on {}", mount.display())));
    }
    let import = bool_p(p, "import").unwrap_or(true);
    let n = l.files.len();
    let total = l.bytes;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1);
    let job = crate::Job { id, label: format!("Copy card {name} ({n} files)"), progress: Default::default(), result: Default::default() };
    job.progress.total.store(total, Ordering::Relaxed);
    let copied: Arc<Mutex<Vec<String>>> = Arc::default();
    let (prog, res, out, dir) = (job.progress.clone(), job.result.clone(), copied.clone(), folder.clone());
    let truncated = l.truncated;
    let run = move || {
        let t0 = web_time::Instant::now();
        let mut base = 0u64;
        let mut manifest = String::new();
        let mut conflicts = 0usize;
        let mut err = None;
        for (k, f) in l.files.iter().enumerate() {
            *lock(&prog.status) = format!("{} ({} of {n})", f.rel.display(), k.saturating_add(1));
            let dst = dir.join(&f.rel);
            let r = copy_verified(&f.src, f.size, &dst, &mut |done| {
                prog.done.store(base.saturating_add(done), Ordering::Relaxed);
                !prog.cancel.load(Ordering::Relaxed)
            });
            base = base.saturating_add(f.size);
            prog.done.store(base, Ordering::Relaxed);
            match r {
                Ok(c) => {
                    manifest.push_str(&format!("{:08x}  {}  {}\n", c.crc, f.size, f.rel.display()));
                    lock(&out).push(dst.to_string_lossy().into_owned());
                }
                Err(CopyError::Exists) => conflicts = conflicts.saturating_add(1),
                Err(CopyError::Cancelled) => {
                    err = Some("cancelled".to_string());
                    break;
                }
                Err(CopyError::SourceChanged(e)) => {
                    let ok = lock(&out).len();
                    err = Some(format!(
                        "the card was removed or couldn't be read at {} ({e}); {ok} of {n} files are copied and verified, run Copy Card again to finish",
                        f.rel.display()
                    ));
                    break;
                }
                Err(e) => {
                    err = Some(format!("{}: {e}", f.rel.display()));
                    break;
                }
            }
        }
        if !manifest.is_empty() {
            let _ = std::fs::write(dir.join(MANIFEST), &manifest);
        }
        let secs = t0.elapsed().as_secs_f64();
        let done = lock(&out).len();
        let mut status = format!("Copied and verified {done} of {n} files in {secs:.1}s");
        if conflicts > 0 {
            status.push_str(&format!("; {conflicts} left alone (a different file of the same name was already there)"));
        }
        if truncated {
            status.push_str("; the card has more files than FilmCraft lists, copy the rest by hand");
        }
        let r = match err {
            Some(e) => {
                *lock(&prog.error) = Some(e.clone());
                *lock(&prog.status) = e.clone();
                Err(e)
            }
            None => {
                *lock(&prog.status) = status;
                Ok(filmcraft_export::Report {
                    path: dir.to_string_lossy().into_owned(),
                    frames: 0,
                    seconds: secs,
                    bytes: base,
                    render_fps: 0.0,
                    extra_files: Vec::new(),
                })
            }
        };
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.card_jobs.push(PendingOffload { job: id, bin: import.then(|| name.clone()), copied });
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        std::thread::Builder::new().name("filmcraft-card-offload".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    let mut r = json!({"job": id, "files": n, "bytes": total, "folder": folder.to_string_lossy()});
    if let Some(Err(e)) = s.jobs.iter().find(|j| j.id == id).and_then(|j| lock(&j.result).clone()) {
        r["error"] = json!(e);
    }
    Ok(r)
}

/// Import the verified copies of finished offloads (unless cancelled) into their bins. Called
/// once per UI frame from [`Session::poll_persistence`] and after synchronous runs.
pub fn poll(s: &mut Session) {
    let mut i = 0;
    while i < s.card_jobs.len() {
        let Some(pj) = s.card_jobs.get(i) else { break };
        let job = s.jobs.iter().find(|j| j.id == pj.job);
        let finished = job.is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let cancelled = job.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        if !finished {
            i = i.saturating_add(1);
            continue;
        }
        let pj = s.card_jobs.remove(i);
        let Some(bin) = pj.bin.filter(|_| !cancelled) else { continue };
        let paths: Vec<String> = std::mem::take(&mut *lock(&pj.copied)).into_iter().filter(|p| filmcraft_media::is_importable(Path::new(p))).collect();
        if paths.is_empty() {
            continue;
        }
        if let Err(e) = import_into_bin(s, &bin, paths) {
            s.error_toast("card.offload", format!("Copy Card: {e}"));
        }
    }
}

/// Import `paths` into the top-level bin `name` (made when missing).
fn import_into_bin(s: &mut Session, name: &str, paths: Vec<String>) -> Result<Value> {
    let existing = s.project.root.children.iter().find_map(|e| match e {
        filmcraft_project::BinEntry::Bin(b) if b.name == name => Some(b.id.0),
        _ => None,
    });
    let bin = match existing {
        Some(b) => b,
        None => s
            .execute("file.newBin", json!({"name": name}))?
            .get("bin")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::Other("couldn't make the bin".into()))?,
    };
    s.execute("file.import", json!({"paths": paths, "bin": bin}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "card.inspect",
            label: "Inspect Card",
            menu: &[],
            shortcut: None,
            params: r#"{"mount":str}"#,
            enabled: always,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: "card.offload",
            label: "Copy Card…",
            menu: &[],
            shortcut: None,
            params: r#"{"mount":str,"destination":str,"name":str?,"import":bool=true,"wait":bool=false}"#,
            enabled: always,
            run: offload,
            journal: true,
        },
    ]
}
