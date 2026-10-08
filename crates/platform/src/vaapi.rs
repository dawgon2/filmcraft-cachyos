//! VA-API hardware H.264 / HEVC decoding on Linux, through the system ffmpeg as a child process.
//!
//! Each run (from the first sample after creation or [`VideoDecoder::reset`] to a flush or the
//! next reset) is one `ffmpeg -hwaccel vaapi` process: samples go to its stdin as an Annex B
//! byte stream (the sample entry's parameter sets first), and decoded pictures come back on its
//! stdout as raw NV12 / P010 frames. P010 is read straight into the two planes of a
//! [`PixelData::Yuv16Bi`] (pooled buffers, no conversion pass: the GPU uploads them as they
//! are); NV12 is copied into planar [`PixelData::Yuv8`] as the software decoders lay it out.
//!
//! Decoding stays on the GPU or fails: pictures are downloaded from VA-API surfaces
//! (`-hwaccel_output_format vaapi` + `hwdownload`), so when VA-API cannot take the stream ffmpeg
//! errors out instead of quietly decoding in software, and [`crate::HybridDecoder`] switches to
//! FilmCraft's own decoder.
//!
//! ffmpeg outputs pictures in presentation order without timestamps (raw streams carry none), so
//! the `n`-th picture of a run gets the `n`-th smallest pts fed in that run. Pictures ffmpeg would
//! drop (the RASL pictures of a CRA a run starts at) are not fed, so the two sequences line up.
//!
//! Threads: a writer feeds stdin, a reader splits stdout into pictures, and a third drains
//! stderr; all run under `catch_unwind` and end with the process.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, Result, VideoDecoder};
use filmcraft_frame::{Chroma, PixelData, VideoFrame, pool};
use rayon::prelude::*;

use crate::ffmpeg::{StderrTail, ffmpeg, to_annexb, vaapi_device, vaapi_supports};

/// Pictures that may be decoding inside ffmpeg (beyond the stream's reorder depth) before a
/// `decode` call waits for one to come out: the demuxer's look-ahead plus the decoder's queue.
const MAX_IN_FLIGHT: usize = 24;
/// How long a picture may take to come out before the process counts as hung.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// Samples queued for the stdin writer (back-pressure on a stalled process).
const WRITE_QUEUE: usize = 64;
/// Threads converting decoded pictures (shared by every VA-API decoder).
const CONVERT_THREADS: usize = 4;
/// Largest picture taken (VA-API drivers top out at 8K / 16K).
const MAX_DIM: u32 = 16384;

/// What the reader thread sends.
enum Msg {
    Frame(VideoFrame),
    /// End of output (the process exited); `Err` when stdout broke mid-picture.
    End(std::result::Result<(), String>),
}

/// How decoded pictures come out of ffmpeg and what they carry.
#[derive(Clone, Debug)]
struct Layout {
    w: usize,
    h: usize,
    /// 8 (NV12) or 10 (P010, the value in the high bits of little-endian 16-bit words).
    bits: u32,
    color: filmcraft_color::ColorInfo,
    par: (u32, u32),
}

impl Layout {
    fn chroma_size(&self) -> (usize, usize) {
        (self.w.div_ceil(2), self.h.div_ceil(2))
    }
    /// Bytes of one raw picture on ffmpeg's stdout.
    fn frame_bytes(&self) -> usize {
        let (cw, ch) = self.chroma_size();
        let bps = if self.bits > 8 { 2 } else { 1 };
        (self.w * self.h + cw * ch * 2) * bps
    }
}

/// One ffmpeg process decoding one run.
struct Proc {
    child: Child,
    /// To the writer thread (dropped to close stdin).
    tx: Option<SyncSender<Vec<u8>>>,
    rx: Receiver<Msg>,
    stderr: StderrTail,
    /// The reader reported the end of output.
    ended: bool,
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.tx = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read one raw picture from `r` into `buf` (`false` at a clean EOF before its first byte).
fn read_raw(r: &mut impl Read, buf: &mut [u8]) -> std::result::Result<bool, String> {
    let mut got = 0usize;
    while got < buf.len() {
        let Some(rest) = buf.get_mut(got..) else { break };
        match r.read(rest) {
            Ok(0) if got == 0 => return Ok(false),
            Ok(0) => return Err(format!("decoder output ended mid-picture ({got} of {} bytes)", buf.len())),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(true)
}

/// The conversion threads. Not rayon's global pool: while an export renders, every global worker
/// can be a render task waiting on the GOP cache this decoder is filling, which would deadlock.
fn convert_pool() -> Option<&'static rayon::ThreadPool> {
    static P: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    P.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, CONVERT_THREADS);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("vaapi-convert-{i}"))
            .build()
            .map_err(|e| log::warn!("VA-API conversion pool: {e}; converting on one thread"))
            .ok()
    })
    .as_ref()
}

/// [`convert_planes`] on the conversion pool (or this thread when it could not be made).
fn convert(buf: &[u8], l: &Layout) -> std::result::Result<VideoFrame, String> {
    match convert_pool() {
        Some(pool) => pool.install(|| convert_planes(buf, l)),
        None => convert_planes(buf, l),
    }
}

/// A raw NV12 / P010 picture as planar [`VideoFrame`] (rows converted in parallel).
fn convert_planes(buf: &[u8], l: &Layout) -> std::result::Result<VideoFrame, String> {
    if buf.len() < l.frame_bytes() {
        return Err("short picture".into());
    }
    let (w, h) = (l.w, l.h);
    let (cw, ch) = l.chroma_size();
    let data = if l.bits <= 8 {
        let (y, c) = buf.split_at(w * h);
        let mut yp = pool::take_u8(w * h);
        yp.extend_from_slice(y);
        let (mut u, mut v) = (pool::take_u8(cw * ch), pool::take_u8(cw * ch));
        u.resize(cw * ch, 0);
        v.resize(cw * ch, 0);
        u.par_chunks_mut(cw).zip(v.par_chunks_mut(cw)).zip(c.par_chunks(cw * 2)).for_each(|((ur, vr), src)| {
            for ((a, b), pair) in ur.iter_mut().zip(vr.iter_mut()).zip(src.as_chunks::<2>().0) {
                *a = pair[0];
                *b = pair[1];
            }
        });
        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    } else {
        let shift = 16 - l.bits;
        let (y, c) = buf.split_at(w * h * 2);
        let mut yp = pool::take_u16(w * h);
        yp.resize(w * h, 0);
        yp.par_chunks_mut(w).zip(y.par_chunks(w * 2)).for_each(|(dst, src)| {
            for (d, b) in dst.iter_mut().zip(src.as_chunks::<2>().0) {
                *d = u16::from_le_bytes(*b) >> shift;
            }
        });
        let (mut u, mut v) = (pool::take_u16(cw * ch), pool::take_u16(cw * ch));
        u.resize(cw * ch, 0);
        v.resize(cw * ch, 0);
        u.par_chunks_mut(cw).zip(v.par_chunks_mut(cw)).zip(c.par_chunks(cw * 4)).for_each(|((ur, vr), src)| {
            for ((a, b), q) in ur.iter_mut().zip(vr.iter_mut()).zip(src.as_chunks::<4>().0) {
                *a = u16::from_le_bytes([q[0], q[1]]) >> shift;
                *b = u16::from_le_bytes([q[2], q[3]]) >> shift;
            }
        });
        PixelData::Yuv16 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits: l.bits, alpha: None }
    };
    Ok(VideoFrame { width: w as u32, height: h as u32, data, color: l.color, par: l.par, pts: filmcraft_time::Tick::ZERO })
}

/// Read one raw P010 picture from `r` into pooled planes (`None` at a clean end of output).
fn read_p010(r: &mut impl Read, l: &Layout) -> std::result::Result<Option<VideoFrame>, String> {
    let (cw, ch) = l.chroma_size();
    let (yn, uvn) = (l.w * l.h, cw * ch * 2);
    let mut y = pool::take_u16_filled(yn);
    if !read_raw(r, bytemuck::cast_slice_mut(&mut y))? {
        return Ok(None);
    }
    let mut uv = pool::take_u16_filled(uvn);
    if !read_raw(r, bytemuck::cast_slice_mut(&mut uv))? {
        return Err("decoder output ended between the planes of a picture".into());
    }
    // P010 is little-endian on the pipe
    if cfg!(target_endian = "big") {
        y.iter_mut().chain(uv.iter_mut()).for_each(|v| *v = u16::from_le(*v));
    }
    let data = PixelData::Yuv16Bi { y: Arc::new(y), uv: Arc::new(uv), chroma: Chroma::C420, bits: l.bits };
    Ok(Some(VideoFrame { width: l.w as u32, height: l.h as u32, data, color: l.color, par: l.par, pts: filmcraft_time::Tick::ZERO }))
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> std::result::Result<(), String> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
                log::error!("VA-API decoder thread panicked");
            }
        })
        .map(|_| ())
        .map_err(|e| format!("cannot start a decoder thread: {e}"))
}

impl Proc {
    fn start(info: &NalStreamInfo, l: &Layout) -> std::result::Result<Proc, String> {
        let ff = ffmpeg().ok_or("ffmpeg is not installed")?;
        let dev = vaapi_device().ok_or("no VA-API device")?;
        let (demux, fmt) = (if info.codec == NalCodec::Hevc { "hevc" } else { "h264" }, if l.bits > 8 { "p010le" } else { "nv12" });
        // ffmpeg applies the SPS conformance window itself; the crop filter (top-left aligned, the
        // only kind taken) pins the output size whatever the driver returns.
        let vf = format!("hwdownload,format={fmt},crop={}:{}:0:0:exact=1", l.w, l.h);
        let mut child = Command::new(ff)
            .args(["-hide_banner", "-nostats", "-loglevel", "error"])
            .args(["-hwaccel", "vaapi", "-hwaccel_device", dev, "-hwaccel_output_format", "vaapi"])
            // H.264 without a signalled reorder depth: assume the largest, never output early
            .args(["-strict", "strict", "-probesize", "32", "-analyzeduration", "0", "-f", demux, "-i", "pipe:0"])
            .args(["-an", "-sn", "-dn", "-vf", &vf, "-fps_mode", "passthrough", "-f", "rawvideo", "pipe:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
        let (Some(mut stdin), Some(mut stdout), Some(err)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("ffmpeg pipes missing".into());
        };
        let stderr = StderrTail::collect(err);
        let (tx, wrx) = std::sync::mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE);
        let (ftx, rx): (Sender<Msg>, Receiver<Msg>) = std::sync::mpsc::channel();
        let started = spawn_thread("vaapi-write", move || {
            for chunk in wrx {
                if stdin.write_all(&chunk).is_err() {
                    break;
                }
            }
            // dropping stdin closes it: ffmpeg drains and exits
        })
        .and_then(|()| {
            if l.bits > 8 {
                // P010: read the luma and the interleaved chroma plane straight into pooled
                // buffers and hand the frame over as it is
                let l = l.clone();
                return spawn_thread("vaapi-read", move || {
                    loop {
                        let msg = match read_p010(&mut stdout, &l) {
                            Ok(Some(f)) => Msg::Frame(f),
                            Ok(None) => Msg::End(Ok(())),
                            Err(e) => Msg::End(Err(e)),
                        };
                        let end = matches!(msg, Msg::End(_));
                        if ftx.send(msg).is_err() || end {
                            return;
                        }
                    }
                });
            }
            // reader → converter, raw buffers recycled back: I/O and conversion overlap
            let (rtx, rrx) = std::sync::mpsc::sync_channel::<std::result::Result<Vec<u8>, String>>(2);
            let (btx, brx) = std::sync::mpsc::channel::<Vec<u8>>();
            let size = l.frame_bytes();
            spawn_thread("vaapi-read", move || {
                loop {
                    let mut buf = brx.try_recv().unwrap_or_else(|_| vec![0u8; size]);
                    match read_raw(&mut stdout, &mut buf) {
                        Ok(true) => {
                            if rtx.send(Ok(buf)).is_err() {
                                return;
                            }
                        }
                        Ok(false) => return,
                        Err(e) => {
                            let _ = rtx.send(Err(e));
                            return;
                        }
                    }
                }
            })?;
            let l = l.clone();
            spawn_thread("vaapi-convert", move || {
                for raw in rrx {
                    let msg = match raw.and_then(|b| {
                        let f = convert(&b, &l);
                        let _ = btx.send(b);
                        f
                    }) {
                        Ok(f) => Msg::Frame(f),
                        Err(e) => {
                            let _ = ftx.send(Msg::End(Err(e)));
                            return;
                        }
                    };
                    if ftx.send(msg).is_err() {
                        return;
                    }
                }
                let _ = ftx.send(Msg::End(Ok(())));
            })
        });
        let proc = Proc { child, tx: Some(tx), rx, stderr, ended: false };
        started?;
        Ok(proc)
    }

    /// Send bytes to ffmpeg's stdin.
    fn write(&mut self, bytes: Vec<u8>) -> std::result::Result<(), String> {
        let tx = self.tx.as_ref().ok_or("decoder input closed")?;
        tx.send(bytes).map_err(|_| self.failure("decoder exited"))
    }

    /// An error message with what ffmpeg said.
    fn failure(&self, what: &str) -> String {
        match self.stderr.text() {
            t if t.is_empty() => what.to_string(),
            t => format!("{what}: {}", t.lines().last().unwrap_or_default()),
        }
    }

    /// Wait up to `timeout` for the next message (`None`: timed out).
    fn next(&mut self, timeout: Option<Duration>) -> std::result::Result<Option<Msg>, String> {
        if self.ended {
            return Ok(None);
        }
        let m = match timeout {
            Some(t) => match self.rx.recv_timeout(t) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => return Err(self.failure("VA-API decoder stopped responding")),
                Err(RecvTimeoutError::Disconnected) => Some(Msg::End(Err("decoder output closed".into()))),
            },
            None => match self.rx.try_recv() {
                Ok(m) => Some(m),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Msg::End(Err("decoder output closed".into()))),
            },
        };
        if matches!(m, Some(Msg::End(_))) {
            self.ended = true;
        }
        Ok(m)
    }
}

/// The layout of a stream's pictures, or why VA-API does not take it.
fn layout(info: &NalStreamInfo) -> std::result::Result<Layout, String> {
    if info.chroma_format_idc != 1 {
        return Err(format!("chroma format {} (VA-API decodes 4:2:0)", info.chroma_format_idc));
    }
    if info.bit_depth_luma != info.bit_depth_chroma {
        return Err("luma and chroma bit depths differ".into());
    }
    let (profile, bits) = match (info.codec, info.bit_depth_luma) {
        (NalCodec::Hevc, 8) => ("VAProfileHEVCMain", 8),
        (NalCodec::Hevc, 10) => ("VAProfileHEVCMain10", 10),
        (NalCodec::H264, 8) if !info.interlaced && matches!(info.profile_idc, 66 | 77 | 100) => {
            (if info.profile_idc == 100 { "VAProfileH264High" } else { "VAProfileH264Main" }, 8)
        }
        (NalCodec::H264, _) => return Err("H.264 profile VA-API does not decode (interlaced, 10-bit, 4:2:2 or 4:4:4)".into()),
        (NalCodec::Hevc, b) => return Err(format!("{b}-bit HEVC")),
    };
    if !vaapi_supports(profile, "VAEntrypointVLD") {
        return Err(format!("the VA-API driver has no {profile} decoder"));
    }
    let (cx, cy, w, h) = info.crop;
    if cx != 0 || cy != 0 {
        return Err("cropping from the top or left".into());
    }
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        return Err(format!("picture size {w}x{h}"));
    }
    Ok(Layout { w: w as usize, h: h as usize, bits, color: info.color, par: info.par })
}

/// A VA-API decoder for one H.264 / HEVC stream.
pub struct VaapiDecoder {
    info: NalStreamInfo,
    layout: Layout,
    proc: Option<Proc>,
    /// Presentation times fed in this run and not yet output.
    waiting: BTreeSet<i64>,
    /// Pictures output in this run.
    out_count: u64,
    /// HEVC: skip the RASL pictures of the CRA / BLA this run started at (ffmpeg drops them).
    skip_rasl: bool,
    first: bool,
    name: &'static str,
    /// Test hook: fail every decode once this many samples were fed.
    fail_after: Option<u64>,
    fed: u64,
}

impl VaapiDecoder {
    /// A decoder for the stream, or why VA-API does not take it (no ffmpeg, no VA-API device,
    /// unsupported format).
    pub fn new(info: NalStreamInfo) -> std::result::Result<Self, String> {
        let layout = layout(&info)?;
        if ffmpeg().is_none() {
            return Err("ffmpeg is not installed".into());
        }
        if vaapi_device().is_none() {
            return Err("no VA-API device".into());
        }
        let name = match info.codec {
            NalCodec::H264 => "VA-API H.264",
            NalCodec::Hevc => "VA-API HEVC",
        };
        Ok(Self { info, layout, proc: None, waiting: BTreeSet::new(), out_count: 0, skip_rasl: false, first: true, name, fail_after: None, fed: 0 })
    }

    /// Test hook: from the `n`-th sample on, every decode fails as a lost device would.
    pub fn fail_after(&mut self, n: u64) {
        self.fail_after = Some(n);
    }

    /// Give `f` the smallest presentation time waiting.
    fn stamp(&mut self, f: VideoFrame) -> std::result::Result<DecodedFrame, String> {
        let pts = self.waiting.pop_first().ok_or("the VA-API decoder output more pictures than it was fed")?;
        self.out_count += 1;
        Ok(DecodedFrame { pts, frame: f, draft: false })
    }

    /// Collect the pictures that came out; waits while more than the in-flight limit are inside
    /// ffmpeg, or until the end of output when `drain`.
    fn collect(&mut self, drain: bool) -> std::result::Result<Vec<DecodedFrame>, String> {
        let mut out = Vec::new();
        let limit = self.info.reorder.saturating_add(MAX_IN_FLIGHT);
        while let Some(p) = self.proc.as_mut() {
            let wait = drain || self.waiting.len() > limit;
            match p.next(wait.then_some(PICTURE_TIMEOUT))? {
                None => break,
                Some(Msg::Frame(f)) => out.push(self.stamp(f)?),
                Some(Msg::End(Ok(()))) => {
                    let status = p.child.wait().map_err(|e| e.to_string())?;
                    if !status.success() {
                        return Err(p.failure(&format!("ffmpeg exited with {status}")));
                    }
                    if !drain {
                        return Err(p.failure("the VA-API decoder stopped early"));
                    }
                    break;
                }
                Some(Msg::End(Err(e))) => return Err(p.failure(&e)),
            }
        }
        Ok(out)
    }
}

impl VideoDecoder for VaapiDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        self.fed += 1;
        if self.fail_after.is_some_and(|n| self.fed > n) {
            return Err(CodecError::Decode("VA-API decoder failed (test hook)".into()));
        }
        if self.info.codec == NalCodec::Hevc {
            let types = self.info.nal_types(sample);
            if types.iter().any(|t| (16..=23).contains(t)) {
                // a CRA / BLA starting a run: its RASL pictures reference pictures never decoded
                self.skip_rasl = self.first && types.iter().any(|t| (16..=21).contains(t) && !(19..=20).contains(t));
            } else if self.skip_rasl && types.iter().any(|t| matches!(t, 8 | 9)) {
                self.first = false;
                return Ok(Vec::new());
            }
        }
        let mut bytes = Vec::with_capacity(sample.len() + 64);
        if self.proc.is_none() {
            self.proc = Some(Proc::start(&self.info, &self.layout).map_err(CodecError::Decode)?);
            for ps in &self.info.parameter_sets {
                bytes.extend_from_slice(&[0, 0, 0, 1]);
                bytes.extend_from_slice(ps);
            }
        }
        self.first = false;
        to_annexb(sample, self.info.length_size, &mut bytes);
        let Some(p) = self.proc.as_mut() else { return Err(CodecError::Decode("no VA-API process".into())) };
        p.write(bytes).map_err(CodecError::Decode)?;
        self.waiting.insert(pts);
        self.collect(false).map_err(CodecError::Decode)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let Some(p) = self.proc.as_mut() else { return Vec::new() };
        p.tx = None;
        let out = match self.collect(true) {
            Ok(out) => out,
            Err(e) => {
                log::warn!("{}: {e} while flushing", self.name);
                Vec::new()
            }
        };
        if !self.waiting.is_empty() {
            log::warn!("{}: {} pictures never came out", self.name, self.waiting.len());
        }
        // the run is over: the next sample starts a new process
        self.proc = None;
        self.waiting.clear();
        out
    }

    fn reset(&mut self) {
        self.proc = None;
        self.waiting.clear();
        self.out_count = 0;
        self.skip_rasl = false;
        self.first = true;
    }

    fn name(&self) -> &str {
        self.name
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.info.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.info.is_disposable(sample)
    }
}

/// Whether VA-API can decode H.264 (`NalCodec::H264`) or HEVC here at all.
pub fn hardware_available(codec: NalCodec) -> bool {
    ffmpeg().is_some()
        && vaapi_device().is_some()
        && match codec {
            NalCodec::Hevc => vaapi_supports("VAProfileHEVCMain", "VAEntrypointVLD"),
            NalCodec::H264 => vaapi_supports("VAProfileH264High", "VAEntrypointVLD"),
        }
}
