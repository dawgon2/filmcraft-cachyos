//! H.265 / HEVC export through the system ffmpeg (Linux): VA-API hardware encoding
//! (`hevc_vaapi`), else libx265.
//!
//! Rendered frames go to ffmpeg's stdin as raw `yuv420p` / `yuv420p10le` (converted by
//! [`filmcraft_export::frame_to_yuv420p`], the same matrix and range as the H.264 export) and the
//! encoded stream comes back on stdout as Annex B. FilmCraft splits it into access units, writes
//! them length-prefixed into its own MP4 (with AAC audio, metadata and colour boxes like every
//! other export), and derives each picture's presentation time from its picture order count, so
//! B-frames (libx265) get correct composition offsets.
//!
//! The `hvcC` record must be known when the muxer opens, before libx265's lookahead has output
//! anything, so [`HevcEncoder::new`] first encodes one black frame with exactly the same settings
//! and takes the parameter sets from it. That probe also checks the GPU takes the size and
//! profile: in Auto mode a VA-API failure there switches to libx265. Parameter sets in the real
//! stream that differ from the probe's stay in-band in their access unit.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, OnceLock};

use filmcraft_export::{ColorSignal, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, HevcEncoder as Choice, Result, VideoEncoder};
use filmcraft_hevc::params::{Pps, Sps};
use filmcraft_hevc::slice::{NalHeader, SliceHeader};
use filmcraft_isobmff::{HevcConfig, HevcNalArray, SampleEntry};
use filmcraft_time::FrameRate;

use crate::ffmpeg::{StderrTail, annexb_nals, ffmpeg, vaapi_device, vaapi_supports};

/// Raw frames queued for ffmpeg's stdin (a 4K 10-bit frame is 25 MB).
const WRITE_QUEUE: usize = 3;
/// Largest frame size taken.
const MAX_DIM: u32 = 8192;

/// Which ffmpeg HEVC encoders this ffmpeg has (`ffmpeg -encoders`). Probed once.
fn encoders() -> (bool, bool) {
    static E: OnceLock<(bool, bool)> = OnceLock::new();
    *E.get_or_init(|| {
        let Some(ff) = ffmpeg() else { return (false, false) };
        let out = Command::new(ff).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let has = |name: &str| text.lines().any(|l| l.split_whitespace().nth(1) == Some(name));
        (has("hevc_vaapi"), has("libx265"))
    })
}

/// Whether HEVC export can run here (ffmpeg with `hevc_vaapi` + a VA-API device, or `libx265`).
pub fn available() -> bool {
    let (vaapi, x265) = encoders();
    x265 || (vaapi && vaapi_device().is_some())
}

/// Register the encoder and mark [`Format::Hevc`] exportable (once; when [`available`]).
pub fn register() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        if !available() {
            log::info!("HEVC export unavailable: ffmpeg has neither hevc_vaapi (with a VA-API device) nor libx265");
            return false;
        }
        filmcraft_export::register_encoder(factory);
        filmcraft_export::register_format(Format::Hevc);
        true
    })
}

/// The encoder factory for [`Format::Hevc`].
pub fn factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    (format == Format::Hevc).then(|| HevcEncoder::new(w, h, rate, s).map(|e| Box::new(e) as Box<dyn VideoEncoder>))
}

/// The two ffmpeg encoders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vaapi,
    X265,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Vaapi => "hevc_vaapi",
            Backend::X265 => "libx265",
        }
    }
}

/// Everything that decides ffmpeg's arguments.
#[derive(Clone, Debug)]
struct Params {
    w: u32,
    h: u32,
    rate: FrameRate,
    ten_bit: bool,
    keyint: u32,
    crf: Option<u8>,
    cbr: bool,
    target_kbps: u32,
    max_kbps: u32,
    signal: ColorSignal,
}

impl Params {
    fn frame_bytes(&self) -> usize {
        let (w, h) = (self.w as usize, self.h as usize);
        (w * h + 2 * w.div_ceil(2) * h.div_ceil(2)) * if self.ten_bit { 2 } else { 1 }
    }

    /// ffmpeg's command line for `backend`, reading raw frames on stdin, writing Annex B on stdout.
    fn args(&self, backend: Backend, device: Option<&str>) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
        push(&["-hide_banner", "-nostats", "-loglevel", "error"]);
        if let (Backend::Vaapi, Some(dev)) = (backend, device) {
            push(&["-vaapi_device", dev]);
        }
        let (pix, hw) = if self.ten_bit { ("yuv420p10le", "p010") } else { ("yuv420p", "nv12") };
        let size = format!("{}x{}", self.w, self.h);
        let fps = format!("{}/{}", self.rate.num.max(1), self.rate.den.max(1));
        push(&["-f", "rawvideo", "-pix_fmt", pix, "-s", &size, "-framerate", &fps, "-i", "pipe:0", "-an", "-sn", "-dn"]);
        // tagged on the frames (setparams) so the encoders write the VUI colour description
        let (p, t, m) = color_names(self.signal);
        let tag = format!("setparams=color_primaries={p}:color_trc={t}:colorspace={m}:range=tv");
        let profile = if self.ten_bit { "main10" } else { "main" };
        let keyint = self.keyint.max(1).to_string();
        let (tk, mk) = (format!("{}k", self.target_kbps.max(100)), format!("{}k", self.max_kbps.max(self.target_kbps).max(100)));
        let bufsize = format!("{}k", self.max_kbps.max(self.target_kbps).max(100).saturating_mul(2));
        match backend {
            Backend::Vaapi => {
                let vf = format!("{tag},format={hw},hwupload");
                push(&["-vf", &vf, "-c:v", "hevc_vaapi", "-profile:v", profile, "-g", &keyint, "-idr_interval", "0"]);
                match self.crf {
                    Some(q) => {
                        let q = q.clamp(1, 51).to_string();
                        push(&["-rc_mode", "CQP", "-qp", &q]);
                    }
                    None if self.cbr => push(&["-rc_mode", "CBR", "-b:v", &tk, "-maxrate", &tk, "-bufsize", &bufsize]),
                    None => push(&["-rc_mode", "VBR", "-b:v", &tk, "-maxrate", &mk, "-bufsize", &bufsize]),
                }
            }
            Backend::X265 => {
                let x265 = format!("log-level=error:keyint={keyint}:min-keyint={keyint}:open-gop=0:repeat-headers=1");
                push(&["-vf", &tag, "-c:v", "libx265", "-preset", "medium", "-profile:v", profile, "-pix_fmt", pix, "-x265-params", &x265]);
                match self.crf {
                    Some(q) => {
                        let q = q.min(51).to_string();
                        push(&["-crf", &q]);
                    }
                    None if self.cbr => push(&["-b:v", &tk, "-maxrate", &tk, "-bufsize", &bufsize]),
                    None => push(&["-b:v", &tk, "-maxrate", &mk, "-bufsize", &bufsize]),
                }
            }
        }
        push(&["-f", "hevc", "pipe:1"]);
        a
    }
}

/// ffmpeg's names for the H.273 code points of `signal` (primaries, transfer, matrix).
fn color_names(signal: ColorSignal) -> (&'static str, &'static str, &'static str) {
    let primaries = if signal.primaries == 9 { "bt2020" } else { "bt709" };
    let transfer = match signal.transfer {
        16 => "smpte2084",
        18 => "arib-std-b67",
        _ => "bt709",
    };
    let matrix = if signal.matrix == 9 { "bt2020nc" } else { "bt709" };
    (primaries, transfer, matrix)
}

/// An ffmpeg encoding process.
struct Proc {
    child: Child,
    /// To the stdin writer (dropped to close stdin).
    tx: Option<SyncSender<Vec<u8>>>,
    /// Encoded bytes from the stdout reader; `Err` when stdout broke.
    rx: Receiver<std::result::Result<Vec<u8>, String>>,
    stderr: StderrTail,
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.tx = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn thread(name: &str, f: impl FnOnce() + Send + 'static) -> std::result::Result<(), String> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
                log::error!("HEVC encoder thread panicked");
            }
        })
        .map(|_| ())
        .map_err(|e| format!("cannot start an encoder thread: {e}"))
}

impl Proc {
    fn start(args: &[String]) -> std::result::Result<Proc, String> {
        let ff = ffmpeg().ok_or("ffmpeg is not installed")?;
        let mut child = Command::new(ff)
            .args(args)
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
        let (otx, rx) = std::sync::mpsc::channel();
        let started = thread("hevc-write", move || {
            for frame in wrx {
                if stdin.write_all(&frame).is_err() {
                    break;
                }
            }
        })
        .and_then(|()| {
            thread("hevc-read", move || {
                let mut buf = vec![0u8; 1 << 16];
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) => return,
                        Ok(n) => {
                            if otx.send(Ok(buf.get(..n).unwrap_or_default().to_vec())).is_err() {
                                return;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            let _ = otx.send(Err(e.to_string()));
                            return;
                        }
                    }
                }
            })
        });
        let p = Proc { child, tx: Some(tx), rx, stderr };
        started?;
        Ok(p)
    }

    fn failure(&self, what: &str) -> String {
        match self.stderr.text() {
            t if t.is_empty() => what.to_string(),
            t => format!("{what}: {}", t.lines().last().unwrap_or_default()),
        }
    }

    fn send(&mut self, frame: Vec<u8>) -> std::result::Result<(), String> {
        let tx = self.tx.as_ref().ok_or("encoder input closed")?;
        tx.send(frame).map_err(|_| self.failure("ffmpeg stopped"))
    }

    /// Encoded bytes available now.
    fn take(&mut self) -> std::result::Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(Ok(b)) => out.extend(b),
                Ok(Err(e)) => return Err(self.failure(&e)),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(out),
            }
        }
    }

    /// Close stdin and read everything up to the end; checks the exit status.
    fn finish(&mut self) -> std::result::Result<Vec<u8>, String> {
        self.tx = None;
        let mut out = Vec::new();
        while let Ok(r) = self.rx.recv() {
            match r {
                Ok(b) => out.extend(b),
                Err(e) => return Err(self.failure(&e)),
            }
        }
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(self.failure(&format!("ffmpeg exited with {status}")));
        }
        Ok(out)
    }
}

/// One access unit out of the parser.
#[derive(Debug)]
struct Au {
    /// Length-prefixed (4-byte) NAL units.
    data: Vec<u8>,
    key: bool,
    /// Output order index (from the picture order count).
    display: i64,
}

fn rbsp(nal: &[u8]) -> Vec<u8> {
    filmcraft_bitstream::unescape_rbsp(nal.get(2..).unwrap_or_default())
}

fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(63, |h| (h >> 1) & 0x3f)
}

/// Splits an Annex B HEVC stream into access units and numbers them in output order.
#[derive(Default)]
struct Parser {
    /// Bytes not split yet (from the last start code on).
    pending: Vec<u8>,
    /// NAL units of the access unit being collected.
    au: Vec<Vec<u8>>,
    au_has_vcl: bool,
    /// Parameter sets by id (raw NAL units) and parsed.
    vps: HashMap<u32, Vec<u8>>,
    sps: HashMap<u32, (Vec<u8>, Arc<Sps>)>,
    pps: HashMap<u32, (Vec<u8>, Arc<Pps>)>,
    /// Parameter sets that the sample entry carries (left out of the samples).
    configured: Vec<Vec<u8>>,
    /// Access units output so far (decode order index of the next one).
    decoded: i64,
    /// Output index of POC 0 of the current coded video sequence.
    base: i64,
    prev_tid0: (i64, i64),
    first: bool,
}

impl Parser {
    fn new() -> Self {
        Parser { first: true, ..Default::default() }
    }

    /// Feed encoder output; complete access units come out.
    fn feed(&mut self, bytes: &[u8]) -> std::result::Result<Vec<Au>, String> {
        self.pending.extend_from_slice(bytes);
        // everything up to the last start code is complete
        let Some(last) = self.pending.windows(3).rposition(|w| w == [0, 0, 1]) else { return Ok(Vec::new()) };
        if last == 0 {
            return Ok(Vec::new());
        }
        let cut = if last > 0 && self.pending.get(last - 1) == Some(&0) { last - 1 } else { last };
        let rest = self.pending.split_off(cut);
        let done = std::mem::replace(&mut self.pending, rest);
        self.nals(&done)
    }

    /// End of stream: the rest is the last NAL unit; the last access unit comes out.
    fn finish(&mut self) -> std::result::Result<Vec<Au>, String> {
        let done = std::mem::take(&mut self.pending);
        let mut out = self.nals(&done)?;
        if self.au_has_vcl {
            out.push(self.close_au()?);
        }
        Ok(out)
    }

    fn nals(&mut self, data: &[u8]) -> std::result::Result<Vec<Au>, String> {
        let mut out = Vec::new();
        for nal in annexb_nals(data) {
            let t = nal_type(nal);
            let first_slice = t < 32 && nal.get(2).is_some_and(|b| b & 0x80 != 0);
            let starts_au = matches!(t, 32..=35 | 39 | 41..=44 | 48..=55) || first_slice;
            if self.au_has_vcl && starts_au {
                out.push(self.close_au()?);
            }
            match t {
                32 => {
                    let id = rbsp(nal).first().map_or(0, |b| (b >> 4) as u32);
                    self.vps.insert(id, nal.to_vec());
                }
                33 => {
                    let sps = Sps::parse(&rbsp(nal)).map_err(|e| format!("encoder SPS: {e}"))?;
                    self.sps.insert(sps.id, (nal.to_vec(), Arc::new(sps)));
                }
                34 => {
                    let pps = Pps::parse(&rbsp(nal)).map_err(|e| format!("encoder PPS: {e}"))?;
                    self.pps.insert(pps.id, (nal.to_vec(), Arc::new(pps)));
                }
                _ => {}
            }
            if t < 32 {
                self.au_has_vcl = true;
            }
            self.au.push(nal.to_vec());
        }
        Ok(out)
    }

    /// The access unit collected so far, with its output index.
    fn close_au(&mut self) -> std::result::Result<Au, String> {
        let nals = std::mem::take(&mut self.au);
        self.au_has_vcl = false;
        let slice = nals.iter().find(|n| nal_type(n) < 32).ok_or("access unit without a picture")?;
        let header = NalHeader::parse(slice).map_err(|e| format!("encoder slice: {e}"))?;
        let lookup = |id: u32| {
            let (_, pps) = self.pps.get(&id).ok_or_else(|| filmcraft_hevc::Error::Invalid(format!("PPS {id} missing")))?;
            let (_, sps) = self.sps.get(&pps.sps_id).ok_or_else(|| filmcraft_hevc::Error::Invalid(format!("SPS {} missing", pps.sps_id)))?;
            Ok((pps.clone(), sps.clone()))
        };
        let (sh, _, sps) = SliceHeader::parse(&rbsp(slice), header, lookup, None).map_err(|e| format!("encoder slice header: {e}"))?;
        // picture order count (8.3.1)
        let max_lsb = 1i64 << sps.log2_max_poc_lsb.min(16);
        let lsb = sh.poc_lsb as i64;
        let no_rasl = header.is_idr() || header.is_bla() || (header.is_cra() && self.first);
        let msb = if no_rasl {
            0
        } else {
            let (plsb, pmsb) = self.prev_tid0;
            if lsb < plsb && plsb - lsb >= max_lsb / 2 {
                pmsb + max_lsb
            } else if lsb > plsb && lsb - plsb > max_lsb / 2 {
                pmsb - max_lsb
            } else {
                pmsb
            }
        };
        let poc = msb + lsb;
        if header.temporal_id == 0 && !header.is_rasl() && !header.is_radl() && !header.is_sub_layer_non_ref() {
            self.prev_tid0 = (lsb, msb);
        }
        if header.is_idr() || header.is_bla() || self.first {
            // a closed GOP starts here: every earlier picture is output before it
            self.base = self.decoded - poc;
        }
        self.first = false;
        let display = self.base + poc;
        self.decoded += 1;
        let mut data = Vec::new();
        for n in &nals {
            let t = nal_type(n);
            // access unit delimiters, and parameter sets the sample entry has, stay out
            if t == 35 || (matches!(t, 32..=34) && self.configured.iter().any(|c| c == n)) {
                continue;
            }
            data.extend_from_slice(&(n.len() as u32).to_be_bytes());
            data.extend_from_slice(n);
        }
        Ok(Au { data, key: header.is_irap(), display })
    }

    /// The `hvcC` record of the parameter sets seen (VPS, SPS, PPS).
    fn config(&self) -> std::result::Result<HevcConfig, String> {
        let (sps_nal, sps) = self.sps.values().next().ok_or("the encoder wrote no SPS")?;
        let vps: Vec<Vec<u8>> = self.vps.values().cloned().collect();
        let pps: Vec<Vec<u8>> = self.pps.values().map(|p| p.0.clone()).collect();
        if vps.is_empty() || pps.is_empty() {
            return Err("the encoder wrote no VPS or PPS".into());
        }
        // sps_video_parameter_set_id(4) max_sub_layers_minus1(3) temporal_id_nesting(1), then the
        // general profile_tier_level: 1 + 4 + 6 + 1 bytes
        let r = rbsp(sps_nal);
        let ptl = r.get(..13).ok_or("SPS too short")?;
        let constraint = ptl.get(6..12).map_or(0u64, |b| b.iter().fold(0u64, |a, &x| (a << 8) | x as u64));
        Ok(HevcConfig {
            general_profile_space: ptl[1] >> 6,
            general_tier_flag: ptl[1] & 0x20 != 0,
            general_profile_idc: ptl[1] & 0x1f,
            general_profile_compatibility_flags: u32::from_be_bytes([ptl[2], ptl[3], ptl[4], ptl[5]]),
            general_constraint_indicator_flags: constraint,
            general_level_idc: ptl[12],
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: sps.chroma_format_idc as u8,
            bit_depth_luma: sps.bit_depth_luma as u8,
            bit_depth_chroma: sps.bit_depth_chroma as u8,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: ((ptl[0] >> 1) & 7) + 1,
            temporal_id_nested: ptl[0] & 1 != 0,
            length_size: 4,
            arrays: vec![
                HevcNalArray { completeness: true, nal_type: 32, nalus: vps },
                HevcNalArray { completeness: true, nal_type: 33, nalus: vec![sps_nal.clone()] },
                HevcNalArray { completeness: true, nal_type: 34, nalus: pps },
            ],
        })
    }

    /// Reorder depth of the stream (highest sub-layer).
    fn reorder(&self) -> i64 {
        self.sps.values().next().map_or(0, |s| s.1.max_num_reorder as i64)
    }
}

/// Encode one black frame with `args` and return the parser that saw its parameter sets.
fn probe(args: &[String], p: &Params) -> std::result::Result<Parser, String> {
    let mut proc = Proc::start(args)?;
    let black = if p.ten_bit {
        let (y, c) = (p.w as usize * p.h as usize, 2 * (p.w as usize).div_ceil(2) * (p.h as usize).div_ceil(2));
        let mut v = Vec::with_capacity(p.frame_bytes());
        v.extend(std::iter::repeat_n(64u16.to_le_bytes(), y).flatten());
        v.extend(std::iter::repeat_n(512u16.to_le_bytes(), c).flatten());
        v
    } else {
        let y = p.w as usize * p.h as usize;
        let mut v = vec![16u8; y];
        v.resize(p.frame_bytes(), 128);
        v
    };
    proc.send(black)?;
    let out = proc.finish()?;
    let mut parser = Parser::new();
    let aus = parser.feed(&out)?.into_iter().chain(parser.finish()?).count();
    if aus != 1 {
        return Err(format!("probe encode gave {aus} pictures"));
    }
    Ok(parser)
}

/// The HEVC export encoder.
pub struct HevcEncoder {
    params: Params,
    backend: Backend,
    proc: Proc,
    parser: Parser,
    config: HevcConfig,
    reorder: i64,
    fed: i64,
    yuv: Vec<u8>,
}

impl HevcEncoder {
    pub fn new(w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Result<Self> {
        if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
            return Err(ExportError::Unsupported(format!("HEVC export of {w}x{h} (even sizes up to {MAX_DIM})")));
        }
        if rate.num <= 0 || rate.den <= 0 || rate.num > u32::MAX as i64 || rate.den > u32::MAX as i64 {
            return Err(ExportError::Unsupported(format!("frame rate {}/{}", rate.num, rate.den)));
        }
        let params = Params {
            w,
            h,
            rate,
            ten_bit: s.hevc_bit_depth == 10,
            keyint: s.keyframe_distance.unwrap_or(0).max(1),
            crf: s.crf,
            cbr: s.bitrate_mode == filmcraft_export::BitrateMode::Cbr,
            target_kbps: s.bitrate_kbps,
            max_kbps: s.max_bitrate_kbps.unwrap_or(s.bitrate_kbps),
            signal: s.signal,
        };
        let (has_vaapi, has_x265) = encoders();
        let profile = if params.ten_bit { "VAProfileHEVCMain10" } else { "VAProfileHEVCMain" };
        let vaapi_ok = has_vaapi && vaapi_device().is_some() && vaapi_supports(profile, "VAEntrypointEncSlice");
        let order: Vec<Backend> = match s.hevc_encoder {
            Choice::Auto => [vaapi_ok.then_some(Backend::Vaapi), has_x265.then_some(Backend::X265)].into_iter().flatten().collect(),
            Choice::Vaapi if vaapi_ok => vec![Backend::Vaapi],
            Choice::Vaapi => {
                return Err(ExportError::Unsupported(
                    "VA-API HEVC encoding is not available (ffmpeg hevc_vaapi, a VA-API device and the driver's HEVC encoder are needed)".into(),
                ));
            }
            Choice::X265 if has_x265 => vec![Backend::X265],
            Choice::X265 => return Err(ExportError::Unsupported("this ffmpeg has no libx265".into())),
        };
        let mut why = Vec::new();
        for backend in order {
            let args = params.args(backend, vaapi_device());
            let started = probe(&args, &params).and_then(|probe| {
                let config = probe.config()?;
                let proc = Proc::start(&args)?;
                Ok((probe, config, proc))
            });
            match started {
                Ok((probe, config, proc)) => {
                    log::info!("HEVC export: {} {}x{} {}-bit", backend.name(), w, h, if params.ten_bit { 10 } else { 8 });
                    let mut parser = Parser::new();
                    parser.configured = config.arrays.iter().flat_map(|a| a.nalus.iter().cloned()).collect();
                    let reorder = probe.reorder();
                    return Ok(HevcEncoder { params, backend, proc, parser, config, reorder, fed: 0, yuv: Vec::new() });
                }
                Err(e) => {
                    log::warn!("HEVC export: {} unusable: {e}", backend.name());
                    why.push(format!("{}: {e}", backend.name()));
                }
            }
        }
        if why.is_empty() {
            why.push("ffmpeg has no HEVC encoder".into());
        }
        Err(ExportError::Unsupported(format!("HEVC encoder not available ({})", why.join("; "))))
    }

    /// The ffmpeg encoder in use.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Packets with composition offsets: decode index `d` gets dts `d - reorder` (frames), its
    /// pts is its output index.
    fn stamp(&mut self, aus: Vec<Au>, first_decode: i64) -> Result<Vec<EncodedPacket>> {
        let den = self.params.rate.den;
        let mut out = Vec::with_capacity(aus.len());
        for (k, au) in aus.into_iter().enumerate() {
            let d = first_decode + k as i64;
            let offset = au.display - (d - self.reorder);
            if offset < 0 || au.display >= self.fed || au.display < 0 {
                return Err(ExportError::Encode(format!("HEVC encoder output order is inconsistent (picture {d} shown at {})", au.display)));
            }
            let co = offset.checked_mul(den).and_then(|v| i32::try_from(v).ok()).ok_or_else(|| ExportError::Encode("composition offset overflow".into()))?;
            out.push(EncodedPacket { data: au.data, key: au.key, duration: den as u32, composition_offset: co });
        }
        Ok(out)
    }

    fn drain(&mut self, bytes: &[u8], end: bool) -> Result<Vec<EncodedPacket>> {
        let first = self.parser.decoded;
        let mut aus = self.parser.feed(bytes).map_err(ExportError::Encode)?;
        if end {
            aus.extend(self.parser.finish().map_err(ExportError::Encode)?);
        }
        self.stamp(aus, first)
    }
}

impl VideoEncoder for HevcEncoder {
    fn sample_entry(&self) -> SampleEntry {
        let mut e = SampleEntry::hevc(self.config.clone(), self.params.w as u16, self.params.h as u16);
        self.params.signal.apply_to(&mut e, false);
        e
    }

    fn timescale(&self) -> u32 {
        self.params.rate.num as u32
    }

    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
        if (f.width, f.height) != (self.params.w, self.params.h) {
            return Err(ExportError::Encode(format!("frame is {}x{}, the encoder {}x{}", f.width, f.height, self.params.w, self.params.h)));
        }
        let mut yuv = std::mem::take(&mut self.yuv);
        filmcraft_export::frame_to_yuv420p(f, self.params.signal, self.params.ten_bit, &mut yuv);
        let frame = yuv.clone();
        self.yuv = yuv;
        self.proc.send(frame).map_err(ExportError::Encode)?;
        self.fed += 1;
        let bytes = self.proc.take().map_err(ExportError::Encode)?;
        self.drain(&bytes, false)
    }

    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let bytes = self.proc.finish().map_err(ExportError::Encode)?;
        let out = self.drain(&bytes, true)?;
        if self.parser.decoded != self.fed {
            return Err(ExportError::Encode(format!("HEVC encoder returned {} of {} pictures", self.parser.decoded, self.fed)));
        }
        Ok(out)
    }

    fn media_start(&self) -> Option<i64> {
        (self.reorder > 0).then(|| self.reorder.saturating_mul(self.params.rate.den))
    }
}
