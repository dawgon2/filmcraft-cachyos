//! Stream-copy trim: write a new MP4/MOV holding only the samples needed to present a time range,
//! without re-encoding anything.
//!
//! # Units
//! The range is given as [`PresentationTime`]s: a count of `timescale` units per second on the
//! file's *presentation* timeline (edit lists applied: the time a player shows, starting at 0),
//! the same timeline as [`Track::sample_at_presentation_time`] but in a caller-chosen timescale,
//! so a sequence's `Tick` (254 016 000 000/s) can be passed without rounding.
//!
//! # Cut rules
//! - **Video:** let `F` be the frame on screen at `start` and `P` every sample whose presentation
//!   time lies in `[pts(F), end)`. The output keeps the contiguous decode-order run from the sync
//!   sample at or before the earliest member of `P` through the latest member of `P`. A sample
//!   only references samples earlier in decode order, so this run decodes every presented frame;
//!   when a presented sample is a *leading* picture of that sync sample (decoded after it but shown
//!   before it, as with an HEVC CRA's RASL pictures), the run starts one sync sample earlier.
//!   The first kept sample must be a clean entry point (H.264 IDR; HEVC IDR / CRA / BLA, read from
//!   its first VCL NAL unit), stepping back at most two sync samples, else the trim is refused;
//!   sync flags on other sync samples that are not clean entry points (open-GOP H.264 I pictures)
//!   are cleared in the output. Durations and composition offsets are copied, decode time is rebased to 0, and an edit list
//!   hides the pre-roll and limits playback to `end`. `start` is first moved back to the start of
//!   `F` (players drop a frame that begins before the edit), and every track uses that origin, so
//!   the output lasts `end − pts(F)`: at most one frame more than `end − start`.
//! - **AAC (and MP3) audio:** the frames overlapping the range plus two frames of decoder pre-roll,
//!   hidden by the edit list.
//! - **PCM:** cut at exact audio frames (no edit offset needed).
//! - **Timecode:** rewritten with [`Mp4Writer::add_timecode_track`], its start frame advanced by
//!   the frames cut from the front.
//! - Anything else (subtitles, other data, tracks with no media in the range) is dropped and named
//!   in [`TrimReport::dropped`].
//!
//! Inputs the cut can't be proven safe for are refused with [`TrimError::Unsupported`] (the caller
//! should then copy the whole file): fragmented files, several sample entries, edit lists other
//! than "leading empty edits + one normal-rate edit", video without sync samples, other audio
//! codecs, an empty result. A failure may leave `out` partly written; write to a temporary file.

use crate::codec::CodecConfig;
use crate::demux::{Edit, Mp4File, Sample, Track, TrackKind, open};
use crate::error::Error;
use crate::mux::{Brand, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
use crate::source::ByteSource;
use std::cmp::Ordering;
use std::io::{Read, Seek, Write};

/// A time on the file's presentation timeline: `value / timescale` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationTime {
    pub value: i64,
    pub timescale: u64,
}

impl PresentationTime {
    pub fn new(value: i64, timescale: u64) -> Self {
        PresentationTime { value, timescale }
    }
    /// Milliseconds (convenience for tests and tools).
    pub fn from_millis(ms: i64) -> Self {
        PresentationTime { value: ms, timescale: 1000 }
    }
}

/// Why a trim failed.
#[derive(Debug, thiserror::Error)]
pub enum TrimError {
    /// The file can't be trimmed by stream copy with confidence: copy the whole file instead.
    #[error("cannot trim by stream copy: {0}")]
    Unsupported(String),
    /// The requested range is empty, negative or lies past the end of the file.
    #[error("bad trim range: {0}")]
    BadRange(String),
    /// The input could not be read or parsed, or the output could not be written.
    #[error(transparent)]
    File(#[from] Error),
}

impl TrimError {
    /// True when the right fallback is copying the whole file (rather than reporting an error).
    pub fn copy_whole_file_instead(&self) -> bool {
        matches!(self, TrimError::Unsupported(_) | TrimError::File(Error::Unsupported(_) | Error::Mux(_)))
    }
}

/// One track kept in the output.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimmedTrack {
    /// `tkhd` id in the source file.
    pub source_track_id: u32,
    pub kind: TrackKind,
    /// Samples copied (PCM: chunks written).
    pub samples: usize,
    /// Source presentation range covered by the copied samples, pre-roll included (seconds,
    /// for reporting only).
    pub kept_start_seconds: f64,
    pub kept_end_seconds: f64,
}

/// A source track left out of the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedTrack {
    pub source_track_id: u32,
    pub kind: TrackKind,
    pub reason: String,
}

/// What [`trim_mp4`] wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimReport {
    /// The source presentation time that the output's time 0 shows: the requested `start`
    /// snapped back to the start of the video frame on screen then (exact). A caller shifts its
    /// uses of the file by this.
    pub origin: PresentationTime,
    pub tracks: Vec<TrimmedTrack>,
    pub dropped: Vec<DroppedTrack>,
    pub bytes_written: u64,
    /// Presented duration of the output (seconds, reporting only).
    pub duration_seconds: f64,
}

const RATE_1: i32 = 0x10000;
/// AAC frames decoded before the first presented one so the decoder's overlap is primed.
const AAC_PREROLL: usize = 2;
/// Largest PCM read (bytes).
const PCM_BLOCK: u64 = 8 << 20;

/// Exact rational time `n / d` seconds (`d > 0`), with `n` and `d` kept in i64 / u64 range so
/// every cross product fits in i128.
#[derive(Clone, Copy, Debug)]
struct Rat {
    n: i64,
    d: u64,
}

impl Rat {
    fn new(n: i64, d: u64) -> Option<Rat> {
        (d > 0).then_some(Rat { n, d })
    }
    fn cmp(self, o: Rat) -> Ordering {
        let a = self.n as i128 * o.d as i128;
        let b = o.n as i128 * self.d as i128;
        a.cmp(&b)
    }
    fn min(self, o: Rat) -> Rat {
        if self.cmp(o) == Ordering::Greater { o } else { self }
    }
    fn max(self, o: Rat) -> Rat {
        if self.cmp(o) == Ordering::Less { o } else { self }
    }
    fn scaled(self, ts: u64) -> i128 {
        self.n as i128 * ts as i128
    }
    fn floor_in(self, ts: u64) -> i64 {
        clamp_i64(self.scaled(ts).div_euclid(self.d as i128))
    }
    fn ceil_in(self, ts: u64) -> i64 {
        let d = self.d as i128;
        clamp_i64(-(-self.scaled(ts)).div_euclid(d))
    }
    fn round_in(self, ts: u64) -> i64 {
        let d = self.d as i128;
        clamp_i64((self.scaled(ts).saturating_mul(2).saturating_add(d)).div_euclid(d.saturating_mul(2)))
    }
    fn seconds(self) -> f64 {
        self.n as f64 / self.d as f64
    }
}

fn clamp_i64(v: i128) -> i64 {
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T, TrimError> {
    Err(TrimError::Unsupported(msg.into()))
}

/// The presentation window of a track, from its edit list.
struct Window {
    /// Presentation time where the track's media starts (leading empty edits).
    start: Rat,
    /// Presentation time where it ends (edit end, or the end of the last sample).
    end: Rat,
}

fn track_window(t: &Track, movie_ts: u32) -> Result<Window, TrimError> {
    let ts = t.timescale as u64;
    let mut empty = 0u64;
    let mut seg = None;
    for e in &t.edits {
        if e.media_time == -1 {
            if seg.is_some() {
                return unsupported(format!("track {}: empty edit after media", t.id));
            }
            empty = empty.checked_add(e.segment_duration).ok_or_else(|| TrimError::Unsupported("edit list too long".into()))?;
        } else {
            if seg.is_some() {
                return unsupported(format!("track {}: edit list with several segments", t.id));
            }
            if e.media_rate != RATE_1 {
                return unsupported(format!("track {}: edit with media rate {:#x}", t.id, e.media_rate));
            }
            seg = Some(e.segment_duration);
        }
    }
    if !t.edits.is_empty() && seg.is_none() {
        return unsupported(format!("track {}: edit list without media", t.id));
    }
    let empty = i64::try_from(empty).map_err(|_| TrimError::Unsupported("edit list too long".into()))?;
    let mts = movie_ts as u64;
    let start = Rat::new(empty, mts).ok_or_else(|| TrimError::Unsupported("movie timescale is 0".into()))?;
    let mut media_end = i64::MIN;
    for s in &t.samples {
        let e = s.pts.saturating_add(t.edit_offset).saturating_add(s.duration as i64);
        media_end = media_end.max(e);
    }
    let mut end = Rat::new(media_end, ts).ok_or_else(|| TrimError::Unsupported(format!("track {}: timescale is 0", t.id)))?;
    if let Some(seg) = seg.filter(|&s| s > 0) {
        let seg = i64::try_from(seg).map_err(|_| TrimError::Unsupported("edit list too long".into()))?;
        if let Some(e) = Rat::new(empty.saturating_add(seg), mts) {
            end = end.min(e);
        }
    }
    Ok(Window { start, end })
}

/// Move `start` back to the start of the first video track's frame on screen at `start`, so the
/// output begins on a whole frame (players drop a frame that starts before the edit) and every
/// track keeps the same origin (A/V sync).
fn snap_to_video_frame(file: &Mp4File, windows: &[Option<Window>], start: Rat) -> Rat {
    for (t, w) in file.tracks.iter().zip(windows) {
        let Some(w) = w else { continue };
        if t.kind != TrackKind::Video {
            continue;
        }
        let s_eff = start.max(w.start);
        let ts = t.timescale as u64;
        let Some(f) = t.sample_at_presentation_time(s_eff.floor_in(ts)).and_then(|i| t.samples.get(i)) else { return start };
        let Some(p) = Rat::new(pres(t, f), ts) else { return start };
        if p.cmp(start) == Ordering::Less && p.cmp(w.start) != Ordering::Less {
            return p;
        }
        return start;
    }
    start
}

/// Bytes to copy for one output sample.
#[derive(Clone, Copy)]
enum Piece {
    /// A whole source sample.
    Sample(usize),
    /// A byte range of a PCM chunk, starting at raw media time `dts`.
    Pcm { offset: u64, len: u64, dts: i64 },
}

struct Plan {
    src: usize,
    cfg: TrackConfig,
    pieces: Vec<Piece>,
    /// Raw media time of the first kept sample (subtracted from every dts).
    base: i64,
    kept: (Rat, Rat),
}

/// Inputs shared by the per-track planners.
struct Range {
    /// Requested start on the presentation timeline.
    start: Rat,
    movie_ts: u32,
}

/// Edit list for the output: leading empty edit if the track starts after `start`, then one
/// segment from `s_eff` to `e_eff`, its media time rebased by `base` and never past `kept_end`.
fn output_edits(t: &Track, r: &Range, s_eff: Rat, e_eff: Rat, base: i64, kept_end_raw: i64) -> Result<Vec<Edit>, TrimError> {
    let ts = t.timescale as u64;
    let mts = r.movie_ts as u64;
    let media_time = s_eff.floor_in(ts).saturating_sub(t.edit_offset).saturating_sub(base);
    if media_time < 0 {
        return unsupported(format!("track {}: presentation starts before decode", t.id));
    }
    let available = kept_end_raw.saturating_sub(base).saturating_sub(media_time);
    let available_movie = Rat::new(available, ts).map(|a| a.floor_in(mts)).unwrap_or(0);
    let seg = e_eff.round_in(mts).saturating_sub(s_eff.round_in(mts)).min(available_movie);
    if seg <= 0 {
        return unsupported(format!("track {}: nothing to present", t.id));
    }
    let mut edits = Vec::new();
    let lead = s_eff.round_in(mts).saturating_sub(r.start.round_in(mts));
    if lead > 0 {
        edits.push(Edit { segment_duration: lead as u64, media_time: -1, media_rate: RATE_1 });
    }
    edits.push(Edit { segment_duration: seg as u64, media_time, media_rate: RATE_1 });
    Ok(edits)
}

fn base_config(t: &Track) -> Result<TrackConfig, TrimError> {
    let entry = t.entries.first().cloned().ok_or_else(|| TrimError::Unsupported(format!("track {}: no sample entry", t.id)))?;
    let mut cfg = TrackConfig::new(entry, t.timescale);
    cfg.language = t.language.clone();
    if !t.handler_name.is_empty() {
        cfg.handler_name = Some(t.handler_name.clone());
    }
    cfg.matrix = Some(t.matrix);
    Ok(cfg)
}

fn pres(t: &Track, s: &Sample) -> i64 {
    s.pts.saturating_add(t.edit_offset)
}

fn kept_range(t: &Track, samples: &[Sample]) -> Option<(Rat, Rat)> {
    let lo = samples.iter().map(|s| pres(t, s)).min()?;
    let hi = samples.iter().map(|s| pres(t, s).saturating_add(s.duration as i64)).max()?;
    Some((Rat::new(lo, t.timescale as u64)?, Rat::new(hi, t.timescale as u64)?))
}

/// True when the first VCL NAL unit of an AVCC/HVCC sample is a clean decoder entry point
/// (H.264: IDR; HEVC: IDR, CRA or BLA). Other codecs trust the sync flag.
fn clean_entry_point(codec: Option<&CodecConfig>, data: &[u8]) -> bool {
    let (len_size, hevc) = match codec {
        Some(CodecConfig::Avc(c)) => (c.length_size, false),
        Some(CodecConfig::Hevc(c)) => (c.length_size, true),
        _ => return true,
    };
    let ls = usize::from(len_size);
    if !(1..=4).contains(&ls) {
        return false;
    }
    let mut pos = 0usize;
    while let Some(head) = data.get(pos..pos.saturating_add(ls)) {
        let n = head.iter().fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        let start = pos.saturating_add(ls);
        let Some(&b0) = data.get(start) else { return false };
        if hevc {
            let ty = (b0 >> 1) & 0x3F;
            if ty <= 31 {
                return (16..=21).contains(&ty);
            }
        } else {
            let ty = b0 & 0x1F;
            if (1..=5).contains(&ty) {
                return ty == 5;
            }
        }
        if n == 0 {
            return false;
        }
        pos = start.saturating_add(n);
    }
    false
}

/// Sync samples a video cut may step back over (past shown leading pictures or a sync sample that
/// is not a clean entry point) before giving up.
const MAX_STEP_BACK: usize = 2;

/// Choose the first kept sample of a video track: the sync sample at or before `lo` that is a
/// clean entry point and has no presented leading pictures (`shown(i)` says whether sample `i` is
/// presented). Each step scans only the samples up to the next sync sample, and at most
/// [`MAX_STEP_BACK`] steps are taken, so the work is bounded by a few GOPs.
fn choose_video_start(
    t: &Track,
    lo: usize,
    clean: &mut dyn FnMut(usize) -> Result<bool, TrimError>,
    shown: &dyn Fn(&Sample) -> bool,
) -> Result<usize, TrimError> {
    if t.sync_samples().is_empty() {
        return unsupported(format!("track {}: no sync samples", t.id));
    }
    let syncs = t.sync_samples();
    let mut a = t.sync_sample_before(lo);
    for step in 0..=MAX_STEP_BACK {
        let Some(sa) = t.samples.get(a) else { return Err(Error::NoSuchSample(a).into()) };
        if !sa.is_sync {
            return unsupported(format!("track {}: no sync sample before the start", t.id));
        }
        if a == 0 {
            // The file's own start: it decodes here exactly as the source does.
            return Ok(0);
        }
        let k = syncs.partition_point(|&i| i as usize <= a);
        let next = syncs.get(k).map_or(t.samples.len(), |&i| i as usize);
        let gop = t.samples.get(a + 1..next).unwrap_or(&[]);
        let leading_shown = gop.iter().any(|s| s.pts < sa.pts && shown(s));
        if !leading_shown && clean(a)? {
            return Ok(a);
        }
        if step == MAX_STEP_BACK {
            break;
        }
        a = t.sync_sample_before(a - 1);
    }
    unsupported(format!("track {}: no clean entry point within {MAX_STEP_BACK} GOPs before the start", t.id))
}

fn plan_coded<S: ByteSource + ?Sized>(file: &Mp4File, src: &S, ti: usize, r: &Range, s_eff: Rat, e_eff: Rat, video: bool) -> Result<Option<Plan>, TrimError> {
    let t = file.tracks.get(ti).ok_or(Error::NoSuchTrack(ti))?;
    let ts = t.timescale as u64;
    let start_t = s_eff.floor_in(ts);
    let end_t = e_eff.ceil_in(ts);
    let Some(first) = t.sample_at_presentation_time(start_t) else { return Ok(None) };
    let first_pts = t.samples.get(first).map(|s| s.pts).ok_or(Error::NoSuchSample(first))?;
    // P: every sample presented in [pts(F), end).
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for (i, s) in t.samples.iter().enumerate() {
        if s.pts >= first_pts && pres(t, s) < end_t {
            lo = lo.min(i);
            hi = hi.max(i);
        }
    }
    if lo == usize::MAX {
        return Ok(None);
    }
    let a = if video {
        let codec = t.codec();
        let mut clean = |i: usize| -> Result<bool, TrimError> { Ok(clean_entry_point(codec, &file.read_sample(src, ti, i)?)) };
        let shown = |s: &Sample| s.pts >= first_pts && pres(t, s) < end_t;
        choose_video_start(t, lo, &mut clean, &shown)?
    } else {
        lo.saturating_sub(AAC_PREROLL)
    };
    let kept = t.samples.get(a..=hi).ok_or(Error::NoSuchSample(hi))?;
    if !video && kept.iter().any(|s| !s.is_sync) {
        return unsupported(format!("track {}: audio with non-sync samples", t.id));
    }
    let base = kept.first().map(|s| s.dts).ok_or(Error::NoSuchSample(a))?;
    for s in kept {
        if i32::try_from(s.pts.saturating_sub(s.dts)).is_err() {
            return unsupported(format!("track {}: composition offset out of range", t.id));
        }
    }
    let kept_end_raw = kept.iter().map(|s| s.pts.saturating_add(s.duration as i64)).max().unwrap_or(base);
    let mut cfg = base_config(t)?;
    cfg.edits = output_edits(t, r, s_eff, e_eff, base, kept_end_raw)?;
    let range = kept_range(t, kept).ok_or_else(|| TrimError::Unsupported("empty track".into()))?;
    Ok(Some(Plan { src: ti, cfg, pieces: (a..=hi).map(Piece::Sample).collect(), base, kept: range }))
}

fn plan_pcm(t: &Track, ti: usize, r: &Range, s_eff: Rat, e_eff: Rat, frame_bytes: u32) -> Result<Option<Plan>, TrimError> {
    let ts = t.timescale as u64;
    let fb = frame_bytes as u64;
    if fb == 0 {
        return unsupported("PCM frame size is 0");
    }
    let s_f = s_eff.floor_in(ts).saturating_sub(t.edit_offset);
    let e_f = e_eff.ceil_in(ts).saturating_sub(t.edit_offset);
    let mut pieces = Vec::new();
    let mut kept_end = s_f;
    for s in &t.samples {
        if (s.duration as u64).checked_mul(fb) != Some(s.size as u64) {
            return unsupported(format!("track {}: PCM chunk size does not match its frame count", t.id));
        }
        let c0 = s.dts;
        let c1 = s.dts.saturating_add(s.duration as i64);
        let lo = c0.max(s_f);
        let hi = c1.min(e_f);
        if lo >= hi {
            continue;
        }
        let skip = (lo - c0) as u64;
        let frames = (hi - lo) as u64;
        let offset = s.offset.checked_add(skip.saturating_mul(fb)).ok_or(Error::Truncated("PCM chunk"))?;
        // Copy in blocks of at most PCM_BLOCK bytes (whole frames) so no read is sized by the chunk.
        let block_frames = (PCM_BLOCK / fb).max(1);
        let mut done = 0u64;
        while done < frames {
            let n = block_frames.min(frames - done);
            let off = offset.checked_add(done.saturating_mul(fb)).ok_or(Error::Truncated("PCM chunk"))?;
            pieces.push(Piece::Pcm { offset: off, len: n.saturating_mul(fb), dts: lo.saturating_add(done as i64) });
            done += n;
        }
        kept_end = kept_end.max(hi);
    }
    let Some(Piece::Pcm { dts: base, .. }) = pieces.first().copied() else { return Ok(None) };
    let mut cfg = base_config(t)?;
    cfg.edits = output_edits(t, r, s_eff, e_eff, base, kept_end)?;
    let kept = (Rat::new(base.saturating_add(t.edit_offset), ts), Rat::new(kept_end.saturating_add(t.edit_offset), ts));
    let (Some(k0), Some(k1)) = kept else { return Ok(None) };
    Ok(Some(Plan { src: ti, cfg, pieces, base, kept: (k0, k1) }))
}

fn read_bytes<S: ByteSource + ?Sized>(src: &S, offset: u64, len: u64) -> Result<Vec<u8>, TrimError> {
    let end = offset.checked_add(len).ok_or(Error::Truncated("sample"))?;
    if end > src.len() {
        return Err(Error::Truncated("sample data").into());
    }
    let n = usize::try_from(len).map_err(|_| Error::TooLarge("sample size"))?;
    let mut v = vec![0u8; n];
    src.read_at(offset, &mut v).map_err(Error::Io)?;
    Ok(v)
}

/// Trim `src` to the presentation range `[start, end)` by copying samples (no re-encode) into
/// `out` (a faststart MP4, or MOV when the source is QuickTime).
///
/// `end` past the end of the file is clamped to it; `start` at or past the end is a
/// [`TrimError::BadRange`]. See the [module docs](self) for the cut rules and refusals.
pub fn trim_mp4<S: ByteSource, W: Read + Write + Seek>(src: S, start: PresentationTime, end: PresentationTime, mut out: W) -> Result<TrimReport, TrimError> {
    let (Some(start), Some(end)) = (Rat::new(start.value, start.timescale), Rat::new(end.value, end.timescale)) else {
        return Err(TrimError::BadRange("timescale is 0".into()));
    };
    if start.n < 0 {
        return Err(TrimError::BadRange("start is negative".into()));
    }
    if start.cmp(end) != Ordering::Less {
        return Err(TrimError::BadRange("start is not before end".into()));
    }
    let file: Mp4File = open(&src)?;
    if file.fragmented || file.fragment_count > 0 {
        return unsupported("fragmented file");
    }
    if file.timescale == 0 {
        return unsupported("movie timescale is 0");
    }
    let mut dropped = Vec::new();
    let mut windows = Vec::new();
    let mut file_end: Option<Rat> = None;
    for t in &file.tracks {
        if !matches!(t.kind, TrackKind::Video | TrackKind::Audio) {
            windows.push(None);
            continue;
        }
        if t.samples.is_empty() {
            dropped.push(DroppedTrack { source_track_id: t.id, kind: t.kind, reason: "no samples".into() });
            windows.push(None);
            continue;
        }
        if t.entries.len() != 1 || t.samples.iter().any(|s| s.description_index != 1) {
            return unsupported(format!("track {}: more than one sample description", t.id));
        }
        let w = track_window(t, file.timescale)?;
        file_end = Some(file_end.map_or(w.end, |e| e.max(w.end)));
        windows.push(Some(w));
    }
    let Some(file_end) = file_end else { return unsupported("no audio or video tracks") };
    if start.cmp(file_end) != Ordering::Less {
        return Err(TrimError::BadRange(format!("start {:.3}s is at or past the end ({:.3}s)", start.seconds(), file_end.seconds())));
    }
    let end = end.min(file_end);
    let start = snap_to_video_frame(&file, &windows, start);
    let range = Range { start, movie_ts: file.timescale };

    let mut plans: Vec<Plan> = Vec::new();
    let mut timecode: Option<usize> = None;
    for (ti, t) in file.tracks.iter().enumerate() {
        let drop = |reason: &str| DroppedTrack { source_track_id: t.id, kind: t.kind, reason: reason.into() };
        let w = match (t.kind, windows.get(ti).and_then(|w| w.as_ref())) {
            (TrackKind::Video | TrackKind::Audio, Some(w)) => w,
            (TrackKind::Video | TrackKind::Audio, None) => continue,
            (TrackKind::Timecode, _) if timecode.is_none() => {
                timecode = Some(ti);
                continue;
            }
            (TrackKind::Timecode, _) => {
                dropped.push(drop("second timecode track"));
                continue;
            }
            (TrackKind::Subtitle, _) => {
                dropped.push(drop("subtitle track"));
                continue;
            }
            (TrackKind::Other, _) => {
                dropped.push(drop("unsupported track type"));
                continue;
            }
        };
        let s_eff = start.max(w.start);
        let e_eff = end.min(w.end);
        if s_eff.cmp(e_eff) != Ordering::Less {
            dropped.push(drop("no media in the range"));
            continue;
        }
        let plan = match (t.kind, t.codec()) {
            (TrackKind::Video, _) => plan_coded(&file, &src, ti, &range, s_eff, e_eff, true)?,
            (TrackKind::Audio, Some(CodecConfig::Aac(_) | CodecConfig::Mp3)) => plan_coded(&file, &src, ti, &range, s_eff, e_eff, false)?,
            (TrackKind::Audio, Some(CodecConfig::Pcm(p))) => {
                let diff = (p.sample_rate - t.timescale as f64).abs();
                if !t.pcm_chunked || diff.is_nan() || diff > 0.5 {
                    return unsupported(format!("track {}: PCM timescale differs from the sample rate", t.id));
                }
                plan_pcm(t, ti, &range, s_eff, e_eff, p.bytes_per_frame())?
            }
            _ => {
                return unsupported(format!(
                    "track {}: audio codec {} can't be cut safely",
                    t.id,
                    t.entries.first().map(|e| e.format.to_string()).unwrap_or_default()
                ));
            }
        };
        match plan {
            Some(p) => plans.push(p),
            None => dropped.push(drop("no media in the range")),
        }
    }
    if plans.is_empty() {
        return unsupported("nothing to keep in the range");
    }

    let mut opts = WriterOptions::new(if file.is_quicktime { Brand::Mov } else { Brand::Mp4 });
    opts.movie_timescale = file.timescale;
    opts.metadata = file.metadata.entries.iter().filter(|(k, _)| k.starts_with('©')).cloned().collect();
    let begin = out.stream_position().map_err(Error::Io)?;
    let mut w = Mp4Writer::new(&mut out, opts)?;
    let mut out_idx = Vec::with_capacity(plans.len());
    for p in &plans {
        out_idx.push(w.add_track(p.cfg.clone())?);
    }
    if let Some(tci) = timecode {
        add_timecode(&file, tci, &plans, &out_idx, start, &mut w, &mut dropped)?;
    }

    // Interleave by decode time.
    let mut order: Vec<(usize, usize, i64)> = Vec::new();
    for (k, p) in plans.iter().enumerate() {
        let t = file.tracks.get(p.src).ok_or(Error::NoSuchTrack(p.src))?;
        for (j, piece) in p.pieces.iter().enumerate() {
            let dts = match *piece {
                Piece::Sample(i) => t.samples.get(i).map(|s| s.dts).ok_or(Error::NoSuchSample(i))?,
                Piece::Pcm { dts, .. } => dts,
            };
            order.push((k, j, dts.saturating_sub(p.base)));
        }
    }
    let ts_of = |k: usize| plans.get(k).and_then(|p| file.tracks.get(p.src)).map_or(1, |t| t.timescale as i128);
    order.sort_by(|a, b| (a.2 as i128 * ts_of(b.0)).cmp(&(b.2 as i128 * ts_of(a.0))).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    for (k, j, _) in order {
        let (Some(p), Some(&oi)) = (plans.get(k), out_idx.get(k)) else { continue };
        let piece = p.pieces.get(j).copied().ok_or(Error::NoSuchSample(j))?;
        match piece {
            Piece::Sample(i) => {
                let s = file.tracks.get(p.src).and_then(|t| t.samples.get(i)).copied().ok_or(Error::NoSuchSample(i))?;
                let data = file.read_sample(&src, p.src, i)?;
                let composition_offset = i32::try_from(s.pts.saturating_sub(s.dts)).map_err(|_| TrimError::Unsupported("composition offset".into()))?;
                // A sync flag on an open-GOP H.264 I picture (not IDR) is dropped in the output:
                // players start decoding at the last sync sample before the edit, which must be a
                // clean entry point (ISO/IEC 14496-15 sync samples are IDR pictures).
                let video = file.tracks.get(p.src).filter(|t| t.kind == TrackKind::Video);
                let is_sync = s.is_sync && video.is_none_or(|t| clean_entry_point(t.codec(), &data));
                w.write_sample(oi, WriteSample { data: &data, duration: s.duration, composition_offset, is_sync })?;
            }
            Piece::Pcm { offset, len, .. } => {
                let data = read_bytes(&src, offset, len)?;
                w.write_sample(oi, WriteSample { data: &data, duration: 0, composition_offset: 0, is_sync: true })?;
            }
        }
    }
    let out = w.finish_faststart()?;
    let end_pos = out.stream_position().map_err(Error::Io)?;

    let mts = file.timescale as u64;
    let duration = plans.iter().map(|p| p.cfg.edits.iter().map(|e| e.segment_duration).fold(0u64, u64::saturating_add)).max().unwrap_or(0);
    let tracks = plans
        .iter()
        .map(|p| {
            let t = file.tracks.get(p.src);
            TrimmedTrack {
                source_track_id: t.map_or(0, |t| t.id),
                kind: t.map_or(TrackKind::Other, |t| t.kind),
                samples: p.pieces.len(),
                kept_start_seconds: p.kept.0.seconds(),
                kept_end_seconds: p.kept.1.seconds(),
            }
        })
        .collect();
    let origin = PresentationTime { value: start.n, timescale: start.d };
    Ok(TrimReport { origin, tracks, dropped, bytes_written: end_pos.saturating_sub(begin), duration_seconds: duration as f64 / mts as f64 })
}

fn add_timecode<W: Write + Seek>(
    file: &Mp4File,
    tci: usize,
    plans: &[Plan],
    out_idx: &[usize],
    start: Rat,
    w: &mut Mp4Writer<W>,
    dropped: &mut Vec<DroppedTrack>,
) -> Result<(), TrimError> {
    let Some(t) = file.tracks.get(tci) else { return Ok(()) };
    let mut drop = |reason: &str| dropped.push(DroppedTrack { source_track_id: t.id, kind: t.kind, reason: reason.into() });
    let video = plans.iter().zip(out_idx).find(|(p, _)| file.tracks.get(p.src).is_some_and(|v| v.kind == TrackKind::Video));
    let Some((_, &vi)) = video else {
        drop("timecode without a kept video track");
        return Ok(());
    };
    let Some(CodecConfig::Timecode(tc)) = t.codec() else {
        drop("timecode track without a timecode entry");
        return Ok(());
    };
    if t.samples.len() != 1 || t.entries.len() != 1 || tc.timescale == 0 || tc.frame_duration == 0 {
        drop("timecode track layout not supported");
        return Ok(());
    }
    let Some(first) = tc.start_frame else {
        drop("timecode start frame unknown");
        return Ok(());
    };
    let tc_start = match track_window(t, file.timescale) {
        Ok(win) => win.start.floor_in(tc.timescale as u64),
        Err(_) => {
            drop("timecode edit list not supported");
            return Ok(());
        }
    };
    let cut = start.floor_in(tc.timescale as u64).saturating_sub(tc_start).max(0) / tc.frame_duration as i64;
    let Some(new_start) = u32::try_from(cut).ok().and_then(|c| first.checked_add(c)) else {
        drop("timecode start frame out of range");
        return Ok(());
    };
    w.add_timecode_track(vi, tc.clone(), new_start)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_rounding() {
        let r = Rat::new(1, 3).unwrap();
        assert_eq!(r.floor_in(10), 3);
        assert_eq!(r.ceil_in(10), 4);
        assert_eq!(r.round_in(10), 3);
        let n = Rat::new(-1, 3).unwrap();
        assert_eq!(n.floor_in(10), -4);
        assert_eq!(n.ceil_in(10), -3);
        let big = Rat::new(i64::MAX, 1).unwrap();
        assert_eq!(big.floor_in(u64::MAX), i64::MAX);
        assert_eq!(Rat::new(1, 2).unwrap().cmp(Rat::new(2, 4).unwrap()), Ordering::Equal);
    }

    #[test]
    fn entry_point_nal_walk() {
        let avc = CodecConfig::Avc(crate::codec::AvcConfig::new(Vec::new(), Vec::new(), 4));
        let idr = [0, 0, 0, 2, 6, 0, 0, 0, 0, 1, 0x65];
        assert!(clean_entry_point(Some(&avc), &idr));
        let non_idr = [0, 0, 0, 1, 0x41];
        assert!(!clean_entry_point(Some(&avc), &non_idr));
        for bad in [&[][..], &[0xFF, 0xFF, 0xFF, 0xFF, 0x65][..], &[0, 0, 0, 0][..], &[0, 0][..], &[0, 0, 0, 0, 6][..]] {
            let _ = clean_entry_point(Some(&avc), bad);
        }
        assert!(clean_entry_point(None, &[]));
    }

    #[test]
    fn bad_ranges() {
        let data = vec![0u8; 64];
        let mut out = std::io::Cursor::new(Vec::new());
        let t = PresentationTime::from_millis;
        assert!(matches!(trim_mp4(data.as_slice(), t(10), t(10), &mut out), Err(TrimError::BadRange(_))));
        assert!(matches!(trim_mp4(data.as_slice(), t(-1), t(10), &mut out), Err(TrimError::BadRange(_))));
        assert!(matches!(trim_mp4(data.as_slice(), PresentationTime::new(0, 0), t(10), &mut out), Err(TrimError::BadRange(_))));
        assert!(trim_mp4(data.as_slice(), t(0), t(10), &mut out).is_err());
    }
}
