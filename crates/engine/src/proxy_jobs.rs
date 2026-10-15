//! Automatic proxies (fork 6.2): half-size proxies made in the background for the clips playback
//! cannot decode fast enough.
//!
//! - **Only in the desktop app.** Nothing is queued on its own unless the frontend set
//!   [`AutoProxies::enabled`] (the desktop app does at startup): a CLI or headless session that
//!   exits would leave half-written proxies behind. `media.createProxies {preset: "auto"}` still
//!   queues on request.
//! - **Which clips.** On import and project open, 4:2:2 long-GOP clips (H.265 / H.264 4:2:2:
//!   software decode) always get a proxy, and 4K-and-larger long-GOP clips do while Settings ▸
//!   Media ▸ "Create proxies automatically…" (`media.autoProxies`) is on (the default). Nothing is
//!   queued while Settings ▸ Media ▸ Proxies is Off. [`auto_reason`] decides.
//! - **How.** Each proxy is a [`BackgroundJob`] on the session's background worker
//!   ([`crate::background`]): it drives the stepped export ([`Exporter::step`]) of a one-clip
//!   sequence ([`crate::proxies::transcode_project`]) with the automatic preset
//!   ([`crate::proxies::auto_proxy_preset`]: H.265 Main 10 half size on VA-API, else ProRes 422
//!   Proxy half size). The worker waits while the user plays, scrubs or exports.
//! - **Where.** `<stem>_Proxy.<fingerprint16>.mov` in [`crate::proxies::proxy_dir`]: the name
//!   carries the original's content fingerprint ("Clip ID", [`crate::relink`]), so a proxy made from
//!   another file is recognised. The file is written as `….partial.mov` and renamed when complete;
//!   a cancelled or failed job deletes its partial file. A complete file already on disk (a
//!   re-import, another project) is checked and attached without encoding again.
//! - **Finishing.** [`poll`] (every UI frame through `Session::poll_persistence`) attaches finished
//!   proxies **outside the undo history** ([`set_links`]: the project and every undo / redo
//!   snapshot get the link, so undo, redo and the redo stack are untouched), to every item on the
//!   same file (one job per output file), requeues an H.265 job that failed
//!   once with [`FALLBACK_PROXY_PRESET`] (VA-API encoding is not always there, see
//!   [`crate::proxies::hevc_encode_available`]), and records failures for the badges.
//! - **Identity.** After a relink or a project open, an automatic proxy whose fingerprint differs
//!   from the original's is detached and a new one queued.
//! - **Deleting files.** Only partial files and proxies this module wrote are deleted, never a
//!   path a project item uses as its original media (a proxy imported as a clip), and every delete
//!   is logged. A scratch folder from the project file is used only when it is absolute.
//! - Export always reads the originals (`MediaPool::full_res_provider`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_export::{ExportSettings, Exporter, Progress, Step};
use filmcraft_media::{MediaInfo, MediaKind, SharedSource, VideoStreamInfo};
use filmcraft_project::{ItemId, MediaRef, Project};
use serde::Serialize;
use serde_json::{Value, json};

use crate::autosave::MediaPrefs;
use crate::background::{BackgroundJob, JobId, JobKind, JobSpec, JobStep};
use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::media_pool::{MediaPool, ProxyMode};
use crate::proxies::{FALLBACK_PROXY_PRESET, HEVC_HALF, Preset};
use crate::{EngineError, Result, Services, Session};

/// The badge text of a 4:2:2 H.265 clip (FORK-PLAN §5 Phase 6 item 1).
pub const BADGE_422: &str = "4:2:2: software decode, proxies recommended";

/// Why a clip gets an automatic proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// 4:2:2 long-GOP: decoded in software (always proxied unless proxies are off).
    Chroma422,
    /// 4K or larger long-GOP (with `media.autoProxies` on).
    Large,
}

/// The stream's chroma is 4:2:2 (from the pixel-format label, e.g. "YUV 4:2:2 10-bit").
pub fn is_422(v: &VideoStreamInfo) -> bool {
    let p = v.pixel_format.to_ascii_lowercase();
    p.contains("4:2:2") || p.contains("422")
}

/// The codec predicts across frames (decoding a frame may mean decoding a GOP): H.264, H.265,
/// AV1, VP8/9, MPEG. ProRes, DNx, APV, JPEG and other intra codecs are not.
pub fn long_gop(v: &VideoStreamInfo) -> bool {
    let c = v.codec.to_ascii_lowercase();
    ["hevc", "h.265", "h265", "h.264", "h264", "avc", "av1", "vp9", "vp8", "mpeg"].iter().any(|k| c.starts_with(k))
}

/// The stream is H.265 (HEVC).
pub fn is_hevc(v: &VideoStreamInfo) -> bool {
    let c = v.codec.to_ascii_lowercase();
    c.starts_with("hevc") || c.starts_with("h.265") || c.starts_with("h265")
}

/// At least as many pixels as UHD (3840 × 2160).
pub fn large(v: &VideoStreamInfo) -> bool {
    u64::from(v.width).saturating_mul(u64::from(v.height)) >= 3840 * 2160
}

/// The stream is H.264 (AVC).
pub fn is_h264(v: &VideoStreamInfo) -> bool {
    let c = v.codec.to_ascii_lowercase();
    c.starts_with("h.264") || c.starts_with("h264") || c.starts_with("avc")
}

/// H.264 High 4:2:2 / High 10 decode in software here: no AMD / Intel / NVIDIA video engine has
/// those profiles (VA-API, NVDEC, Media Foundation). Not claimed under VideoToolbox, whose Apple
/// silicon may take them.
fn h264_pro_profiles_in_software() -> bool {
    filmcraft_codecs::hw::hw_backend() != Some("VideoToolbox")
}

/// The bit depth in the pixel-format label ("YUV 4:2:0 10-bit" → 10), if it names one.
pub fn bit_depth(v: &VideoStreamInfo) -> Option<u32> {
    let p = v.pixel_format.to_ascii_lowercase();
    let head = p.get(..p.find("-bit")?)?;
    let (start, _) = head.char_indices().rev().take_while(|(_, c)| c.is_ascii_digit()).last()?;
    head.get(start..)?.parse().ok()
}

/// Whether a 4:2:2 clip shows "4:2:2: software decode, proxies recommended": it decodes in
/// software. H.265: no 4:2:2 hardware decoder, or it failed this session (fork 6.1b); not while
/// the hardware probe is still running (no warning that might vanish a moment later). H.264
/// High 4:2:2: always, except under VideoToolbox (fork H3.2, [`h264_pro_profiles_in_software`]).
pub fn software_422(info: &MediaInfo) -> bool {
    let Some(v) = info.video.as_ref() else { return false };
    is_422(v) && ((is_h264(v) && h264_pro_profiles_in_software()) || (is_hevc(v) && filmcraft_codecs::hw::hevc_422_in_hardware() == Some(false)))
}

/// Whether a 4:2:0 H.264 clip of more than 8 bits (High 10) shows "H.264 10-bit: software
/// decode": FilmCraft's own decoder takes it (fork H2.2, H3.2; [`h264_pro_profiles_in_software`]).
/// 4:2:2 ones get the 4:2:2 badge instead ([`software_422`]).
pub fn software_h264_high_bit_depth(info: &MediaInfo) -> bool {
    info.video.as_ref().is_some_and(|v| is_h264(v) && !is_422(v) && bit_depth(v).is_some_and(|b| b > 8)) && h264_pro_profiles_in_software()
}

/// Hardware 4:2:2 decoding of this H.265 clip's kind failed earlier in this session (the badge
/// says so; fork 6.1b). Never for H.264, which has no 4:2:2 hardware path to fail.
pub fn hardware_422_failed(info: &MediaInfo) -> bool {
    info.video.as_ref().is_some_and(is_hevc) && filmcraft_codecs::hw::hevc_422_failed()
}

/// Whether (and why) a clip gets an automatic proxy under `prefs`.
pub fn auto_reason(info: &MediaInfo, prefs: &MediaPrefs) -> Option<Reason> {
    if prefs.proxies == ProxyMode::Off || info.kind != MediaKind::Movie {
        return None;
    }
    let v = info.video.as_ref()?;
    if !long_gop(v) {
        return None;
    }
    if is_422(v) {
        return Some(Reason::Chroma422);
    }
    (prefs.auto_proxies && large(v)).then_some(Reason::Large)
}

/// `<stem>_Proxy.<fingerprint as 16 hex digits>.<ext>`.
pub fn proxy_file_name(stem: &str, fingerprint: u64, ext: &str) -> String {
    format!("{stem}_Proxy.{fingerprint:016x}.{ext}")
}

/// The fingerprint in an automatic proxy's file name (None: not an automatic proxy).
pub fn proxy_fingerprint(path: &str) -> Option<u64> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    let (head, hex) = stem.rsplit_once('.')?;
    if !head.ends_with("_Proxy") || hex.len() != 16 || head.ends_with(".partial") {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

/// The file a job writes until it is complete.
fn partial_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = out.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mov".into());
    out.with_file_name(format!("{stem}.partial.{ext}"))
}

/// A clip's proxy state for status lists and badges (a clip with an attached proxy and no job has
/// none: its proxy is ready).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ProxyState {
    Queued,
    Building { fraction: f32 },
    Failed { error: String },
}

/// How a proxy job ended (written by the job on the worker thread, read by [`poll`]).
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Running,
    Done(String),
    Failed(String),
    Cancelled,
}

struct Tracked {
    job: JobId,
    /// The item the job was made for (its clip built the transcode).
    item: ItemId,
    /// Other items on the same file waiting for the same proxy (one job per output file).
    also: Vec<ItemId>,
    name: String,
    /// The original's path and fingerprint when the job was queued.
    original: String,
    fingerprint: u64,
    out: String,
    preset: &'static Preset,
    /// 0 = first try, 1 = the fallback preset after a failure.
    attempt: u8,
    reason: Option<Reason>,
    progress: Arc<Progress>,
    slot: Arc<Mutex<Slot>>,
}

/// The session's automatic proxies: jobs in flight and failures (`Session::auto_proxies`).
#[derive(Default)]
pub struct AutoProxies {
    /// Queue proxies on import, project open and preference changes. Off by default: only the
    /// desktop app turns it on (a CLI / headless session would leave partial files on exit).
    pub enabled: bool,
    tracked: Vec<Tracked>,
    failed: BTreeMap<ItemId, String>,
    /// The preferences [`poll`] last saw (it queues proxies when they start asking for more).
    seen: Option<(ProxyMode, bool)>,
    /// The preset tried first (None: [`crate::proxies::auto_proxy_preset`] of what is available).
    pub first_preset: Option<&'static Preset>,
    /// Tests: H.265 jobs fail as a VA-API encoder that cannot start would.
    #[cfg(test)]
    pub(crate) fail_hevc: bool,
}

impl std::fmt::Debug for AutoProxies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoProxies").field("jobs", &self.tracked.len()).field("failed", &self.failed.len()).finish()
    }
}

impl AutoProxies {
    /// The item's proxy job or failure (None: no job; ready when a proxy is attached).
    pub fn state(&self, item: ItemId) -> Option<ProxyState> {
        if let Some(t) = self.tracked.iter().find(|t| t.has(item)) {
            return Some(if t.progress.total.load(Ordering::Relaxed) > 0 {
                ProxyState::Building { fraction: t.progress.fraction().clamp(0.0, 1.0) }
            } else {
                ProxyState::Queued
            });
        }
        self.failed.get(&item).map(|e| ProxyState::Failed { error: e.clone() })
    }

    /// Jobs queued or running.
    pub fn pending(&self) -> usize {
        self.tracked.len()
    }
}

impl Tracked {
    fn has(&self, i: ItemId) -> bool {
        self.item == i || self.also.contains(&i)
    }

    fn items(&self) -> Vec<ItemId> {
        std::iter::once(self.item).chain(self.also.iter().copied()).collect()
    }
}

// ------------------------------------------------------------------ the job

/// One proxy transcode, stepped by the background worker.
pub(crate) struct ProxyJob {
    pool: Arc<MediaPool>,
    services: Arc<dyn Services>,
    /// The original file (opened on the worker at the first step: its own decoder, so playback's
    /// decoder state and caches are not disturbed).
    original: String,
    source: Option<SharedSource>,
    project: Arc<Project>,
    seq: ItemId,
    item: ItemId,
    settings: ExportSettings,
    partial: PathBuf,
    out: PathBuf,
    exporter: Option<Exporter>,
    progress: Arc<Progress>,
    slot: Arc<Mutex<Slot>>,
    in_step: bool,
    fail_with: Option<String>,
    /// Tests: the next step panics (as a decoder bug would).
    #[cfg(test)]
    pub(crate) panic_in_step: bool,
}

impl ProxyJob {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pool: Arc<MediaPool>,
        services: Arc<dyn Services>,
        original: String,
        project: Arc<Project>,
        seq: ItemId,
        item: ItemId,
        settings: ExportSettings,
        out: PathBuf,
        progress: Arc<Progress>,
        slot: Arc<Mutex<Slot>>,
    ) -> Self {
        Self {
            pool,
            services,
            original,
            source: None,
            project,
            seq,
            item,
            settings,
            partial: partial_path(&out),
            out,
            exporter: None,
            progress,
            slot,
            in_step: false,
            fail_with: None,
            #[cfg(test)]
            panic_in_step: false,
        }
    }

    fn set_slot(&self, v: Slot) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = v;
    }

    fn remove_partial(&mut self) {
        // close the file before deleting it
        self.exporter = None;
        if self.partial.exists() {
            log::info!("proxy: deleting the partial file {}", self.partial.display());
            if let Err(e) = std::fs::remove_file(&self.partial) {
                log::warn!("proxy: could not delete {}: {e}", self.partial.display());
            }
        }
    }

    /// One batch; Some(path) when the proxy is complete.
    fn advance(&mut self) -> std::result::Result<Option<String>, String> {
        if let Some(e) = &self.fail_with {
            return Err(e.clone());
        }
        #[cfg(test)]
        if self.panic_in_step {
            panic!("proxy job test panic");
        }
        if self.source.is_none() {
            self.source = Some(self.pool.open_file(&self.original, &*self.services).map_err(|e| format!("{}: {e}", self.original))?);
        }
        if self.exporter.is_none() {
            if let Some(d) = self.partial.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            if self.partial.exists() {
                log::info!("proxy: deleting the stale partial file {}", self.partial.display());
                std::fs::remove_file(&self.partial).map_err(|e| format!("{}: {e}", self.partial.display()))?;
            }
            self.settings.path = self.partial.to_string_lossy().into_owned();
            self.exporter = Some(Exporter::new(self.project.clone(), self.seq, &self.settings, &self.progress).map_err(|e| e.to_string())?);
        }
        let (Some(ex), Some(src)) = (self.exporter.as_mut(), self.source.clone()) else { return Ok(None) };
        let item = self.item;
        let provider = move |id: ItemId| (id == item).then(|| src.clone());
        match ex.step(&provider, &self.progress).map_err(|e| e.to_string())? {
            Step::Progress => Ok(None),
            Step::Pending => {
                // media bytes still loading (web): try again shortly
                std::thread::sleep(std::time::Duration::from_millis(5));
                Ok(None)
            }
            Step::Done(_) => {
                self.exporter = None;
                if self.out.exists() {
                    // something appeared there while the proxy was made (queueing removed the
                    // stale proxy it found): never replace a file this job did not write
                    return Err(format!("{} already exists; not replaced", self.out.display()));
                }
                std::fs::rename(&self.partial, &self.out).map_err(|e| format!("{}: {e}", self.out.display()))?;
                Ok(Some(self.out.to_string_lossy().into_owned()))
            }
        }
    }
}

impl BackgroundJob for ProxyJob {
    fn step(&mut self) -> std::result::Result<JobStep, String> {
        if filmcraft_media::cancel::cancelled() {
            self.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.in_step = true;
        let r = self.advance();
        self.in_step = false;
        match r {
            Ok(None) => Ok(JobStep::More),
            Ok(Some(path)) => {
                self.set_slot(Slot::Done(path));
                Ok(JobStep::Done)
            }
            Err(e) => {
                self.remove_partial();
                if !filmcraft_media::cancel::cancelled() {
                    self.set_slot(Slot::Failed(e.clone()));
                }
                Err(e)
            }
        }
    }

    fn fraction(&self) -> f32 {
        self.progress.fraction()
    }

    fn cancelled(&mut self) {
        self.remove_partial();
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if *slot == Slot::Running {
            // a step that never returned panicked (the worker caught it)
            *slot = if self.in_step { Slot::Failed("internal error (see the crash log)".into()) } else { Slot::Cancelled };
        }
    }
}

// ------------------------------------------------------------------ queueing

/// The original's fingerprint: recorded at import / relink, else read now.
fn fingerprint_of(s: &Session, identity: Option<filmcraft_project::MediaIdentity>, path: &str) -> Option<u64> {
    identity.map(|i| i.fingerprint).or_else(|| crate::relink::identity_of(&*s.services, path).ok().map(|i| i.fingerprint))
}

fn first_preset(s: &Session) -> &'static Preset {
    s.auto_proxies.first_preset.unwrap_or_else(|| crate::proxies::auto_proxy_preset(crate::proxies::hevc_encode_available()))
}

/// Where the proxy of `path` goes: Project Settings ▸ Scratch Disks ▸ Captured and Generated
/// `/Proxies`, else `Proxies` next to the media (as Clip ▸ Proxy ▸ Create Proxies).
fn proxy_out(s: &Session, path: &str, stem: &str, fingerprint: u64, preset: &Preset) -> PathBuf {
    // the project file says where; only an absolute folder is trusted (a relative one would land
    // wherever the app happens to run)
    let dest = s.project.settings.scratch.captured.clone().filter(|d| !d.is_empty() && Path::new(d).is_absolute()).map(|d| format!("{d}/Proxies"));
    crate::proxies::proxy_dir(path, dest.as_deref()).join(proxy_file_name(stem, fingerprint, preset.extension))
}

/// Whether a project item uses `path` as its original media (a proxy file imported as a clip):
/// such a file is never deleted or overwritten.
pub fn used_as_original(p: &Project, path: &Path) -> bool {
    p.items.values().any(|i| i.as_media().is_some_and(|m| matches!(&m.media, MediaRef::File { path: f } if Path::new(f) == path)))
}

/// Delete a file this module wrote, unless a project item uses it as original media. Logged.
fn delete_ours(p: &Project, path: &Path, why: &str) -> bool {
    if used_as_original(p, path) {
        log::warn!("proxy: not deleting {} ({why}): a clip in the project uses it as its media", path.display());
        return false;
    }
    log::info!("proxy: deleting {} ({why})", path.display());
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("proxy: could not delete {}: {e}", path.display());
            false
        }
    }
}

/// Set (or clear) items' proxy links **outside the undo history**: the current project and every
/// undo / redo snapshot holding the same original get the new link, so undoing or redoing an edit
/// neither loses nor brings back a proxy, and the redo stack survives. Not an edit: a clean
/// project stays clean (an automatic proxy is found again when the project is next opened).
pub fn set_links(s: &mut Session, links: &[(ItemId, Option<MediaRef>)]) {
    // (item, its original, the new link)
    let want: Vec<(ItemId, MediaRef, Option<MediaRef>)> =
        links.iter().filter_map(|(i, l)| Some((*i, s.project.item(*i)?.as_media()?.media.clone(), l.clone()))).collect();
    if want.is_empty() {
        return;
    }
    let patch = |p: &mut Arc<Project>| {
        let needs = want.iter().any(|(i, orig, l)| p.item(*i).and_then(|it| it.as_media()).is_some_and(|m| m.media == *orig && m.proxy != *l));
        if !needs {
            return;
        }
        let pm = Arc::make_mut(p);
        for (i, orig, l) in &want {
            if let Some(m) = pm.item_mut(*i).and_then(|it| it.as_media_mut())
                && m.media == *orig
            {
                m.proxy = l.clone();
            }
        }
    };
    patch(&mut s.project);
    for (_, p) in s.history.undo.iter_mut().chain(s.history.redo.iter_mut()) {
        patch(p);
    }
    s.bump_view();
}

/// What [`queue`] did with an item.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum Queued {
    /// A job was queued.
    Job { job: JobId, path: String, preset: &'static str },
    /// A matching proxy was already on disk and is now attached.
    Attached { path: String },
    /// Nothing to do.
    Skipped { reason: String },
}

fn skipped(r: impl Into<String>) -> Queued {
    Queued::Skipped { reason: r.into() }
}

/// Queue a proxy for `item` (or attach one already on disk). `forced`: the user asked (Create
/// Proxies), so a clip that already has a proxy of its own still gets the automatic one.
pub fn queue(s: &mut Session, item: ItemId, forced: bool) -> Queued {
    let Some((clip, path, name)) = crate::proxies::media_path(&s.project, item) else { return skipped("not file media") };
    if clip.info.kind != MediaKind::Movie || clip.info.video.is_none() {
        return skipped(format!("{name} has no video to proxy"));
    }
    if clip.offline || s.offline.missing.contains(&item) {
        return skipped(format!("{name} is offline"));
    }
    if s.auto_proxies.tracked.iter().any(|t| t.has(item)) {
        return skipped(format!("{name}: a proxy is already being made"));
    }
    if !forced && clip.proxy.is_some() {
        return skipped(format!("{name} already has a proxy"));
    }
    let Some(fp) = fingerprint_of(s, clip.identity, &path) else { return skipped(format!("{name}: the file can't be read")) };
    let preset = first_preset(s);
    let stem = Path::new(&path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
    let out = proxy_out(s, &path, &stem, fp, preset);
    let out_s = out.to_string_lossy().into_owned();
    if clip.proxy == Some(MediaRef::File { path: out_s.clone() }) {
        return skipped(format!("{name} already has this proxy"));
    }
    if used_as_original(&s.project, &out) || used_as_original(&s.project, &partial_path(&out)) {
        return skipped(format!("{name}: {out_s} is a clip in the project; not replaced"));
    }
    // another item on the same file: wait for the same proxy (one job per output file)
    if let Some(t) = s.auto_proxies.tracked.iter_mut().find(|t| t.out == out_s) {
        t.also.push(item);
        s.auto_proxies.failed.remove(&item);
        return Queued::Job { job: t.job, path: out_s, preset: t.preset.id };
    }
    s.auto_proxies.failed.remove(&item);
    if out.is_file() {
        // made before (a re-import, another project): use it when it fits the clip
        if crate::proxies::check_proxy(s, item, &out_s).is_ok() {
            set_links(s, &[(item, Some(MediaRef::File { path: out_s.clone() }))]);
            return Queued::Attached { path: out_s };
        }
        let project = s.project.clone();
        if !delete_ours(&project, &out, "it does not fit the clip") {
            return skipped(format!("{name}: {out_s} does not fit the clip and could not be replaced"));
        }
    }
    let reason = auto_reason(&clip.info, &s.prefs.media);
    match submit(s, item, &name, &path, fp, &out, preset, 0, reason) {
        Ok(job) => Queued::Job { job, path: out_s, preset: preset.id },
        Err(e) => {
            s.auto_proxies.failed.insert(item, e.to_string());
            skipped(format!("{name}: {e}"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn submit(
    s: &mut Session,
    item: ItemId,
    name: &str,
    path: &str,
    fp: u64,
    out: &Path,
    preset: &'static Preset,
    attempt: u8,
    reason: Option<Reason>,
) -> Result<JobId> {
    let (clip, _, _) = crate::proxies::media_path(&s.project, item).ok_or_else(|| EngineError::Other(format!("item {} is not file media", item.0)))?;
    let (project, seq, titem) = crate::proxies::transcode_project(&clip, name, None, preset)?;
    let settings = crate::proxies::transcode_settings(&clip, preset, &partial_path(out).to_string_lossy());
    let progress: Arc<Progress> = Arc::default();
    let slot = Arc::new(Mutex::new(Slot::Running));
    #[allow(unused_mut)]
    let mut job =
        ProxyJob::new(s.media.clone(), s.services.clone(), path.to_string(), project, seq, titem, settings, out.to_path_buf(), progress.clone(), slot.clone());
    #[cfg(test)]
    if s.auto_proxies.fail_hevc && preset.id == HEVC_HALF.id {
        job.fail_with = Some("VA-API H.265 encoder not available (test)".into());
    }
    let spec = JobSpec { kind: JobKind::Proxy, label: format!("Proxy: {name}"), chroma_422: reason == Some(Reason::Chroma422), rank: 0 };
    let id = s.background.submit(spec, Box::new(job)).map_err(EngineError::Other)?;
    s.auto_proxies.tracked.push(Tracked {
        job: id,
        item,
        also: Vec::new(),
        name: name.to_string(),
        original: path.to_string(),
        fingerprint: fp,
        out: out.to_string_lossy().into_owned(),
        preset,
        attempt,
        reason,
        progress,
        slot,
    });
    Ok(id)
}

/// Queue proxies for the items the preferences want proxied ([`auto_reason`]).
fn queue_wanted(s: &mut Session, items: &[ItemId]) -> Vec<(ItemId, Queued)> {
    if cfg!(target_arch = "wasm32") || !s.auto_proxies.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    for &i in items {
        let Some(m) = s.project.item(i).and_then(|it| it.as_media()) else { continue };
        if !matches!(m.media, MediaRef::File { .. }) || auto_reason(&m.info, &s.prefs.media).is_none() {
            continue;
        }
        let q = queue(s, i, false);
        out.push((i, q));
    }
    out
}

fn file_items(p: &Project) -> Vec<ItemId> {
    p.items.values().filter(|i| i.as_media().is_some_and(|m| matches!(m.media, MediaRef::File { .. }))).map(|i| i.id).collect()
}

/// After `file.import`: queue proxies for the new clips that want them.
pub fn on_import(s: &mut Session, items: &[ItemId]) {
    let _ = queue_wanted(s, items);
}

/// After a project opened (or was recovered): forget the previous project's jobs, then check the
/// automatic proxies and queue what is missing.
pub fn on_open(s: &mut Session) {
    cancel(s, None);
    s.auto_proxies.failed.clear();
    let items = file_items(&s.project);
    reconcile(s, &items);
}

/// After a relink: a proxy made from another file is detached and a new one queued.
pub fn after_relink(s: &mut Session, items: &[ItemId]) {
    reconcile(s, items);
}

/// Detach automatic proxies whose fingerprint no longer matches the original (and requeue them),
/// then queue what the preferences want.
fn reconcile(s: &mut Session, items: &[ItemId]) {
    let mut stale = Vec::new();
    for &i in items {
        let Some((clip, path, _)) = crate::proxies::media_path(&s.project, i) else { continue };
        let Some(MediaRef::File { path: proxy }) = &clip.proxy else { continue };
        let Some(made_from) = proxy_fingerprint(proxy) else { continue };
        if fingerprint_of(s, clip.identity, &path).is_some_and(|fp| fp != made_from) {
            stale.push(i);
        }
    }
    if !stale.is_empty() {
        // outside the undo history: opening a project or relinking adds no "Detach" step
        let links: Vec<(ItemId, Option<MediaRef>)> = stale.iter().map(|&i| (i, None)).collect();
        set_links(s, &links);
        // they had proxies: make new ones whatever the preferences say (unless proxies are off)
        if s.prefs.media.proxies != ProxyMode::Off && s.auto_proxies.enabled && !cfg!(target_arch = "wasm32") {
            for &i in &stale {
                let _ = queue(s, i, true);
            }
        }
    }
    let _ = queue_wanted(s, items);
}

/// `media.createProxies {preset: "auto"}`: queue automatic proxies for `items` whatever the
/// preferences say.
pub fn create(s: &mut Session, items: &[ItemId]) -> Result<Value> {
    if cfg!(target_arch = "wasm32") {
        return Err(EngineError::Other("automatic proxies need the desktop app".into()));
    }
    let rows: Vec<Value> = items.iter().map(|&i| json!({"item": i.0, "queued": queue(s, i, true)})).collect();
    if rows.iter().all(|r| r["queued"]["result"] == "skipped") {
        let why: Vec<&str> = rows.iter().filter_map(|r| r["queued"]["reason"].as_str()).collect();
        return Err(EngineError::Other(format!("nothing to proxy: {}", if why.is_empty() { "no items".to_string() } else { why.join("; ") })));
    }
    Ok(json!({"preset": first_preset(s).id, "items": rows}))
}

/// Cancel proxy jobs (of `items`, or all). Their partial files are deleted by the worker.
pub fn cancel(s: &mut Session, items: Option<&[ItemId]>) -> usize {
    let mut n = 0;
    let mut keep = Vec::new();
    for mut t in std::mem::take(&mut s.auto_proxies.tracked) {
        let Some(l) = items else {
            s.background.cancel(t.job);
            n += 1;
            continue;
        };
        let rest: Vec<ItemId> = t.items().into_iter().filter(|i| !l.contains(i)).collect();
        match rest.split_first() {
            None => {
                s.background.cancel(t.job);
                n += 1;
            }
            Some((first, more)) => {
                // other items on the same file still want this proxy
                t.item = *first;
                t.also = more.to_vec();
                keep.push(t);
            }
        }
    }
    s.auto_proxies.tracked = keep;
    n
}

/// Apply the outcomes of finished proxy jobs (call regularly; `Session::poll_persistence` does).
pub fn poll(s: &mut Session) {
    sync_prefs(s, None);
    follow_motion(s);
    if s.auto_proxies.tracked.is_empty() {
        return;
    }
    let mut ended = Vec::new();
    let mut i = 0;
    while i < s.auto_proxies.tracked.len() {
        let slot = s.auto_proxies.tracked.get(i).map(|t| t.slot.lock().unwrap_or_else(PoisonError::into_inner).clone());
        match slot {
            Some(Slot::Running) | None => i += 1,
            Some(slot) => ended.push((s.auto_proxies.tracked.remove(i), slot)),
        }
    }
    for (t, slot) in ended {
        match slot {
            Slot::Done(path) => finish(s, &t, path),
            Slot::Failed(e) => failed(s, t, e),
            Slot::Cancelled | Slot::Running => {}
        }
    }
}

fn finish(s: &mut Session, t: &Tracked, path: String) {
    // each item must still be the file the proxy was made from
    let same: Vec<ItemId> = t
        .items()
        .into_iter()
        .filter(|&i| {
            crate::proxies::media_path(&s.project, i).is_some_and(|(clip, p, _)| p == t.original && fingerprint_of(s, clip.identity, &p) == Some(t.fingerprint))
        })
        .collect();
    if same.is_empty() {
        log::info!("proxy {path}: its clip changed while it was made; not attached");
        let project = s.project.clone();
        delete_ours(&project, Path::new(&path), "its clip changed while it was made");
        return;
    }
    let checked = same.first().map(|&i| crate::proxies::check_proxy(s, i, &path)).unwrap_or(Ok(()));
    match checked {
        Ok(()) => {
            let links: Vec<(ItemId, Option<MediaRef>)> = same.iter().map(|&i| (i, Some(MediaRef::File { path: path.clone() }))).collect();
            set_links(s, &links);
            for i in &same {
                s.auto_proxies.failed.remove(i);
            }
            s.log.push(crate::panels::Level::Info, "proxies", format!("Proxy ready: {}", t.name));
        }
        Err(e) => {
            for i in &same {
                s.auto_proxies.failed.insert(*i, e.to_string());
            }
            s.log.push(crate::panels::Level::Warning, "proxies", format!("{}: the proxy could not be attached: {e}", t.name));
        }
    }
}

fn failed(s: &mut Session, t: Tracked, e: String) {
    if t.attempt == 0
        && t.preset.id == HEVC_HALF.id
        && let Some(fallback) = crate::proxies::preset(FALLBACK_PROXY_PRESET)
    {
        // VA-API H.265 encoding not available after all: ProRes 422 Proxy instead (same name)
        log::info!("proxy of {}: H.265 failed ({e}); trying {}", t.name, fallback.id);
        let out = PathBuf::from(&t.out).with_extension(fallback.extension);
        let ok = !used_as_original(&s.project, &out) && !used_as_original(&s.project, &partial_path(&out));
        if ok && let Ok(job) = submit(s, t.item, &t.name, &t.original, t.fingerprint, &out, fallback, 1, t.reason) {
            if let Some(n) = s.auto_proxies.tracked.iter_mut().find(|n| n.job == job) {
                n.also = t.also.clone();
            }
            return;
        }
    }
    s.log.push(crate::panels::Level::Warning, "proxies", format!("{}: the proxy could not be made: {e}", t.name));
    for i in t.items() {
        s.auto_proxies.failed.insert(i, e.clone());
    }
}

/// Proxies in Auto: while the user plays or scrubs (the background worker's activity, which the
/// UI reports every frame) every frame reads proxies, otherwise the originals. When the user
/// stops, the frame caches refresh (`Session::bump_view`) so the paused frame is redrawn from the
/// original. This is pool-wide: frames prefetched while playing also read proxies. A per-frame
/// hint ([`crate::media_pool::with_motion`]) exists, but nothing in the product sets it yet
/// (owed: the frame server sets it per frame job).
fn follow_motion(s: &mut Session) {
    let a = s.background.activity();
    let motion = a.playing || a.scrubbing;
    if s.media.set_motion(motion)
        && !motion
        && s.media.proxy_mode() == ProxyMode::Auto
        && s.project.items.values().any(|i| i.as_media().is_some_and(|m| m.proxy.is_some()))
    {
        s.bump_view();
    }
}

/// Queue proxies when the preferences start asking for more (Settings ▸ Media changed): proxies
/// no longer off, or automatic proxies switched on. `before`: the preferences to compare with
/// (None: those [`poll`] saw last).
fn sync_prefs(s: &mut Session, before: Option<(ProxyMode, bool)>) {
    let now = (s.prefs.media.proxies, s.prefs.media.auto_proxies);
    let seen = s.auto_proxies.seen.replace(now);
    let Some((mode, auto)) = before.or(seen) else { return };
    let widened = (mode == ProxyMode::Off && now.0 != ProxyMode::Off) || (!auto && now.1 && now.0 != ProxyMode::Off);
    if widened {
        let items = file_items(&s.project);
        let _ = queue_wanted(s, &items);
    }
}

/// Settings ▸ Media ▸ Proxies.
pub fn set_mode(s: &mut Session, mode: ProxyMode) -> Result<()> {
    let before = (s.prefs.media.proxies, s.prefs.media.auto_proxies);
    let mut next = s.prefs.clone();
    next.media.proxies = mode;
    s.set_prefs(next).map_err(|e| EngineError::Other(format!("saving preferences: {e}")))?;
    sync_prefs(s, Some(before));
    Ok(())
}

/// Settings ▸ Media ▸ Create proxies automatically….
pub fn set_auto(s: &mut Session, on: bool) -> Result<()> {
    let before = (s.prefs.media.proxies, s.prefs.media.auto_proxies);
    let mut next = s.prefs.clone();
    next.media.auto_proxies = on;
    s.set_prefs(next).map_err(|e| EngineError::Other(format!("saving preferences: {e}")))?;
    sync_prefs(s, Some(before));
    Ok(())
}

/// `media.proxyStatus`: counts, overall progress and each clip's state.
pub fn status(s: &Session) -> Value {
    let mut rows = Vec::new();
    let (mut queued, mut building, mut failed, mut ready) = (0, 0, 0, 0);
    let (mut done, mut total) = (0u64, 0u64);
    for t in &s.auto_proxies.tracked {
        let st = s.auto_proxies.state(t.item).unwrap_or(ProxyState::Queued);
        match st {
            ProxyState::Building { .. } => building += 1,
            _ => queued += 1,
        }
        done = done.saturating_add(t.progress.done.load(Ordering::Relaxed));
        total = total.saturating_add(t.progress.total.load(Ordering::Relaxed));
        let all: Vec<u64> = t.items().iter().map(|i| i.0).collect();
        rows.push(
            json!({"item": t.item.0, "items": all, "name": t.name, "status": st, "preset": t.preset.id, "path": t.out, "reason": t.reason, "job": t.job}),
        );
    }
    for (item, e) in &s.auto_proxies.failed {
        failed += 1;
        let name = s.project.item(*item).map(|i| i.name.clone()).unwrap_or_default();
        rows.push(json!({"item": item.0, "name": name, "status": ProxyState::Failed { error: e.clone() }}));
    }
    for it in s.project.items.values() {
        if it.as_media().is_some_and(|m| m.proxy.is_some()) {
            ready += 1;
        }
    }
    let progress = if total > 0 { (done as f64 / total as f64).clamp(0.0, 1.0) } else { 0.0 };
    json!({
        "mode": s.prefs.media.proxies,
        "autoProxies": s.prefs.media.auto_proxies,
        "hevcEncode": crate::proxies::hevc_encode_available(),
        "queued": queued,
        "building": building,
        "failed": failed,
        "ready": ready,
        "progress": progress,
        "worker": s.background.status().state,
        "items": rows,
    })
}

fn item_list(p: &Value) -> Option<Vec<ItemId>> {
    match (p.get("items").and_then(Value::as_array), crate::commands::u64_p(p, "item")) {
        (Some(a), _) => Some(a.iter().filter_map(Value::as_u64).map(ItemId).collect()),
        (None, Some(i)) => Some(vec![ItemId(i)]),
        _ => None,
    }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.proxyMode",
            label: "Proxy Mode",
            menu: &[],
            shortcut: None,
            params: r#"{"mode":"auto|on|off"?}"#,
            enabled: always,
            run: |s, p| {
                if let Some(m) = str_p(p, "mode") {
                    let m = ProxyMode::parse(m).ok_or_else(|| bad("media.proxyMode", "`mode` must be auto, on or off"))?;
                    set_mode(s, m)?;
                }
                Ok(json!({"mode": s.prefs.media.proxies}))
            },
            journal: false,
        },
        CommandSpec {
            id: "media.autoProxies",
            label: "Create Proxies Automatically",
            menu: &[],
            shortcut: None,
            params: r#"{"enabled":bool?}"#,
            enabled: always,
            run: |s, p| {
                let on = bool_p(p, "enabled").unwrap_or(!s.prefs.media.auto_proxies);
                set_auto(s, on)?;
                Ok(json!({"enabled": on, "queued": s.auto_proxies.pending()}))
            },
            journal: false,
        },
        CommandSpec {
            id: "media.cancelProxies",
            label: "Cancel Proxies",
            menu: &[],
            shortcut: None,
            params: r#"{"items":[id]?}"#,
            enabled: always,
            run: |s, p| {
                let items = item_list(p);
                let n = cancel(s, items.as_deref());
                Ok(json!({"cancelled": n}))
            },
            journal: false,
        },
        CommandSpec {
            id: "media.proxyStatus",
            label: "Proxy Status",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(status(s)),
            journal: false,
        },
    ]
}

#[cfg(test)]
impl ProxyJob {
    /// A job transcoding `item` of `s` to `out` with `preset` (tests drive it by hand).
    pub(crate) fn for_test(s: &Session, item: ItemId, out: &Path, preset: &'static Preset) -> Self {
        let (clip, path, name) = crate::proxies::media_path(&s.project, item).unwrap();
        let (project, seq, titem) = crate::proxies::transcode_project(&clip, &name, None, preset).unwrap();
        let settings = crate::proxies::transcode_settings(&clip, preset, &partial_path(out).to_string_lossy());
        Self::new(
            s.media.clone(),
            s.services.clone(),
            path,
            project,
            seq,
            titem,
            settings,
            out.to_path_buf(),
            Arc::default(),
            Arc::new(Mutex::new(Slot::Running)),
        )
    }

    pub(crate) fn partial(&self) -> &Path {
        &self.partial
    }

    pub(crate) fn outcome(&self) -> String {
        format!("{:?}", self.slot.lock().unwrap())
    }
}
