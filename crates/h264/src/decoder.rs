//! Top-level decoder: NAL dispatch, picture boundaries, POC, DPB and output.
//!
//! The calling thread parses headers and does all DPB / reference list bookkeeping (which only needs
//! header information). Macroblock decoding of each picture runs as a job — on a thread pool with the
//! `threads` feature — that publishes finished macroblock rows into the picture's shared [`Frame`];
//! jobs of later pictures block per row on the reference data they need, so several pictures decode
//! concurrently (frame-level parallelism).

use crate::deblock::DeblockSample;
use crate::dpb::{Dpb, FieldPocs, Output, OutputMeta, Parity, RefMark};
use crate::error::{Error, Result, ensure, invalid, unsupported};
use crate::params::{Pps, Sps};
use crate::picture::{Frame, FrameRef, MbKind, MbState, Planes, RefPic};
use crate::sample::Sample;
use crate::slice::{NalHeader, Poc, PocState, SliceHeader, SliceType, is_new_picture, nal_type};
use crate::slicedec::{PicState, SliceDecoder};
use crate::transform::LevelScale;
use crate::{ColorInfo, Picture};
use filmcraft_bitstream::{BitReader, annexb_nals, length_prefixed_nals, unescape_rbsp};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// One slice of a picture job.
struct SliceJob<S: Sample> {
    sh: SliceHeader,
    pps: Arc<Pps>,
    sps: Arc<Sps>,
    rbsp: Vec<u8>,
    refs: [Vec<RefPic<S>>; 2],
    ls: Arc<LevelScale>,
}

/// A picture whose slices are being collected.
struct PendingPic<S: Sample> {
    /// The frame stored in the DPB (for a field: the field pair).
    frame: FrameRef<S>,
    /// What the slices decode into: `frame`, or one field of it.
    target: FrameRef<S>,
    /// A field picture's parity (0 top, 1 bottom).
    field: Option<usize>,
    /// The second field of `frame`.
    second: bool,
    /// PicOrderCnt of the picture (the field's for a field; 0 after MMCO 5).
    cur_poc: i32,
    sps: Arc<Sps>,
    first: SliceHeader,
    poc: Poc,
    pts: i64,
    key: bool,
    slices: Vec<SliceJob<S>>,
}

/// Counters describing what the decoder has seen (useful to check test coverage).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    pub pictures: u64,
    pub slices_cavlc: u64,
    pub slices_cabac: u64,
    pub slices_i: u64,
    pub slices_p: u64,
    pub slices_b: u64,
    pub mb_i4x4: u64,
    pub mb_i8x8: u64,
    pub mb_i16x16: u64,
    pub mb_pcm: u64,
    pub mb_p_skip: u64,
    pub mb_b_skip: u64,
    pub mb_b_direct16x16: u64,
    pub mb_inter: u64,
    /// Inter macroblocks using the 8x8 transform.
    pub mb_inter_8x8_transform: u64,
    pub mmco_ops: u64,
    pub long_term_marks: u64,
    pub frame_num_gaps: u64,
    pub weighted_slices: u64,
    pub temporal_direct_slices: u64,
    /// Non-reference pictures decoded in draft mode (deblocking skipped).
    pub draft_pictures: u64,
}

/// State shared with decoding jobs.
#[derive(Default)]
struct Shared<S: Sample> {
    /// Error slot and statistics, kept across a change of sample type (jobs of the old core may
    /// still finish after the switch).
    error: Arc<Mutex<Option<Error>>>,
    stats: Arc<Mutex<DecodeStats>>,
    /// Recycled picture buffers.
    pool: Mutex<Vec<PicState<S>>>,
}

/// A picture sample type the decoder runs on: `u8` for 8-bit streams, `u16` above (fork H2).
trait OutputSample: DeblockSample {
    /// The cropped planes of a finished frame as [`Picture`] samples: 8-bit planes, or
    /// (empty 8-bit planes and) 16-bit ones above 8 bits.
    fn crop(frame: &Frame<Self>, crop: (usize, usize, usize, usize)) -> CroppedPlanes;
}

type CroppedPlanes = ((Vec<u8>, Vec<u8>, Vec<u8>), Option<[Vec<u16>; 3]>);

impl OutputSample for u8 {
    fn crop(frame: &Frame<u8>, crop: (usize, usize, usize, usize)) -> CroppedPlanes {
        (frame.copy_cropped(crop), None)
    }
}

impl OutputSample for u16 {
    /// 8-bit pictures on this path ([`Decoder::set_wide_samples`]) come out as 8-bit planes.
    fn crop(frame: &Frame<u16>, crop: (usize, usize, usize, usize)) -> CroppedPlanes {
        let (y, u, v) = frame.copy_cropped_with(crop, Vec::with_capacity);
        if frame.bit_depth > 8 {
            return ((Vec::new(), Vec::new(), Vec::new()), Some([y, u, v]));
        }
        let narrow = |p: Vec<u16>| p.into_iter().map(|s| s.min(255) as u8).collect();
        ((narrow(y), narrow(u), narrow(v)), None)
    }
}

/// What a decoder core hands to its successor when the sample type changes.
struct Parts {
    spss: Vec<Option<Arc<Sps>>>,
    ppss: Vec<Option<Arc<Pps>>>,
    nal_length_size: Option<usize>,
    next_id: u32,
    stats: Arc<Mutex<DecodeStats>>,
    error: Arc<Mutex<Option<Error>>>,
    #[cfg(feature = "threads")]
    pool: Option<rayon::ThreadPool>,
    max_in_flight: usize,
    threads: usize,
    draft: bool,
}

/// The decoder for one sample type.
struct Core<S: OutputSample> {
    spss: Vec<Option<Arc<Sps>>>,
    ppss: Vec<Option<Arc<Pps>>>,
    nal_length_size: Option<usize>,
    dpb: Dpb<FrameRef<S>>,
    poc_state: PocState,
    prev_ref_frame_num: u32,
    pending: Option<PendingPic<S>>,
    next_id: u32,
    active_sps: Option<Arc<Sps>>,
    ls_cache: Vec<(Arc<Pps>, Arc<LevelScale>)>,
    /// Pictures leaving the DPB, converted once their decoding job has finished.
    out_queue: VecDeque<Output<FrameRef<S>>>,
    shared: Arc<Shared<S>>,
    #[cfg(feature = "threads")]
    pool: Option<rayon::ThreadPool>,
    in_flight: VecDeque<FrameRef<S>>,
    max_in_flight: usize,
    threads: usize,
    /// Draft mode: skip deblocking of non-reference pictures ([`Decoder::set_draft`]).
    draft: bool,
    /// The last first field submitted while its second field may still follow (frame, parity).
    open_field: Option<(FrameRef<S>, usize)>,
}

enum Inner {
    /// 8-bit streams.
    Narrow(Box<Core<u8>>),
    /// Streams above 8 bits (and 8-bit ones with [`Decoder::set_wide_samples`]).
    Wide(Box<Core<u16>>),
}

/// H.264 decoder.
pub struct Decoder {
    inner: Inner,
    /// Pictures a replaced core output before the sample type changed.
    carry: Vec<Picture>,
    wide_samples: bool,
}

macro_rules! core {
    ($inner:expr, $c:ident => $e:expr) => {
        match $inner {
            Inner::Narrow($c) => $e,
            Inner::Wide($c) => $e,
        }
    };
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// The worker threads [`Decoder::new`] uses: the available cores (at most 16) with the `threads`
/// feature on native targets, 1 otherwise.
pub fn default_threads() -> usize {
    #[cfg(all(feature = "threads", not(target_arch = "wasm32")))]
    {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16)
    }
    #[cfg(not(all(feature = "threads", not(target_arch = "wasm32"))))]
    {
        1
    }
}

impl Decoder {
    /// A decoder using all available cores (with the `threads` feature).
    pub fn new() -> Self {
        Self::with_threads(default_threads())
    }

    /// A decoder using up to `threads` worker threads (1 = decode on the calling thread).
    pub fn with_threads(threads: usize) -> Self {
        crate::cavlc::init_tables();
        #[cfg(feature = "threads")]
        let pool = if threads > 1 && cfg!(not(target_arch = "wasm32")) {
            rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("h264-{i}")).build().ok()
        } else {
            None
        };
        #[cfg(feature = "threads")]
        let workers = if pool.is_some() { threads } else { 1 };
        #[cfg(not(feature = "threads"))]
        let workers = 1;
        let parts = Parts {
            spss: vec![None; 32],
            ppss: vec![None; 256],
            nal_length_size: None,
            next_id: 1,
            stats: Arc::default(),
            error: Arc::default(),
            #[cfg(feature = "threads")]
            pool,
            max_in_flight: threads.max(1) + 2,
            threads: workers,
            draft: false,
        };
        Decoder { inner: Inner::Narrow(Box::new(Core::from_parts(parts))), carry: Vec::new(), wide_samples: false }
    }

    /// Worker threads decoding pictures (1: on the calling thread).
    pub fn threads(&self) -> usize {
        core!(&self.inner, c => c.threads)
    }

    /// Draft mode (off by default) for reduced-resolution playback: pictures submitted from now
    /// on that are non-reference pictures (`nal_ref_idc` 0) skip the deblocking filter (8.7).
    /// Nothing predicts from a non-reference picture (and deblocking never changes the motion
    /// data used for direct prediction), so every other picture stays bit-exact; the draft
    /// pictures themselves are approximate and flagged [`Picture::draft`]. Never use it for
    /// export or for a frame shown while paused.
    pub fn set_draft(&mut self, on: bool) {
        core!(&mut self.inner, c => c.draft = on);
    }

    pub fn draft(&self) -> bool {
        core!(&self.inner, c => c.draft)
    }

    /// Test hook (fork H2): decode 8-bit streams through the 16-bit sample path too, from the
    /// next slice on, so every 8-bit fixture checks that path against the 8-bit one.
    #[doc(hidden)]
    pub fn set_wide_samples(&mut self, on: bool) {
        self.wide_samples = on;
    }

    /// Configure from an `avcC` (AVCDecoderConfigurationRecord) box payload.
    pub fn from_avcc(avcc: &[u8]) -> Result<Self> {
        let mut d = Decoder::new();
        d.configure_avcc(avcc)?;
        Ok(d)
    }

    /// Parse an `avcC` record: SPS/PPS and the NAL length size used by [`Decoder::decode`].
    pub fn configure_avcc(&mut self, avcc: &[u8]) -> Result<()> {
        ensure!(avcc.len() >= 7, "avcC too short");
        ensure!(avcc[0] == 1, "unsupported avcC version {}", avcc[0]);
        let length_size = Some((avcc[4] & 3) as usize + 1);
        core!(&mut self.inner, c => c.nal_length_size = length_size);
        let mut pos = 5;
        let read_sets = |pos: &mut usize, count: usize, d: &mut Decoder| -> Result<()> {
            for _ in 0..count {
                ensure!(*pos + 2 <= avcc.len(), "avcC truncated");
                let len = u16::from_be_bytes([avcc[*pos], avcc[*pos + 1]]) as usize;
                *pos += 2;
                ensure!(*pos + len <= avcc.len(), "avcC truncated");
                d.handle_nal(&avcc[*pos..*pos + len], 0)?;
                *pos += len;
            }
            Ok(())
        };
        let nsps = (avcc[pos] & 0x1f) as usize;
        pos += 1;
        read_sets(&mut pos, nsps, self)?;
        ensure!(pos < avcc.len(), "avcC truncated");
        let npps = avcc[pos] as usize;
        pos += 1;
        read_sets(&mut pos, npps, self)
    }

    /// Length of NAL length prefixes (from avcC), or None for Annex-B input.
    pub fn nal_length_size(&self) -> Option<usize> {
        core!(&self.inner, c => c.nal_length_size)
    }

    /// Decode one access unit (all NAL units of one picture, Annex-B or length-prefixed per
    /// configuration; a byte stream with several complete access units is accepted too). Returns
    /// pictures that became ready for output, in output order; `pts` travels with the picture of
    /// this access unit through reordering.
    pub fn decode(&mut self, data: &[u8], pts: i64) -> Result<Vec<Picture>> {
        let nals = match self.nal_length_size() {
            Some(n) => length_prefixed_nals(data, n)?,
            None => annexb_nals(data),
        };
        let mut result = Ok(());
        for nal in nals {
            if let Err(e) = self.handle_nal(nal, pts) {
                result = Err(e);
                break;
            }
        }
        let pics = core!(&mut self.inner, c => c.finish_access_unit(result))?;
        let mut out = std::mem::take(&mut self.carry);
        out.extend(pics);
        Ok(out)
    }

    /// Output all remaining pictures (end of stream).
    pub fn flush(&mut self) -> Vec<Picture> {
        let mut out = std::mem::take(&mut self.carry);
        out.extend(core!(&mut self.inner, c => c.flush()));
        out
    }

    /// Statistics accumulated so far (macroblock counts include finished pictures only).
    pub fn stats(&self) -> DecodeStats {
        core!(&self.inner, c => c.stats())
    }

    /// First error reported by a decoding job since the last call (also returned by
    /// [`Decoder::decode`]); useful after [`Decoder::flush`].
    pub fn take_error(&mut self) -> Option<Error> {
        core!(&mut self.inner, c => c.take_error())
    }

    fn handle_nal(&mut self, nal: &[u8], pts: i64) -> Result<()> {
        if let Some(wide) = core!(&self.inner, c => c.slice_is_wide(nal)) {
            let wide = wide || self.wide_samples;
            if wide != matches!(self.inner, Inner::Wide(_)) {
                self.switch_core(wide);
            }
        }
        core!(&mut self.inner, c => c.handle_nal(nal, pts))
    }

    /// Replace the core by one of the other sample type: the old one's remaining pictures are
    /// output first, parameter sets, settings, statistics and a pending error carry over.
    fn switch_core(&mut self, wide: bool) {
        let carry = &mut self.carry;
        let parts = core!(&mut self.inner, c => c.take_parts(carry));
        self.inner = if wide { Inner::Wide(Box::new(Core::from_parts(parts))) } else { Inner::Narrow(Box::new(Core::from_parts(parts))) };
    }
}

impl<S: OutputSample> Core<S> {
    fn from_parts(p: Parts) -> Self {
        let shared = Shared { error: p.error, stats: p.stats, pool: Mutex::new(Vec::new()) };
        Core {
            spss: p.spss,
            ppss: p.ppss,
            nal_length_size: p.nal_length_size,
            dpb: Dpb::new(),
            poc_state: PocState::default(),
            prev_ref_frame_num: 0,
            pending: None,
            next_id: p.next_id,
            active_sps: None,
            ls_cache: Vec::new(),
            out_queue: VecDeque::new(),
            shared: Arc::new(shared),
            #[cfg(feature = "threads")]
            pool: p.pool,
            in_flight: VecDeque::new(),
            max_in_flight: p.max_in_flight,
            threads: p.threads,
            draft: p.draft,
            open_field: None,
        }
    }

    /// Finish this core: its remaining pictures go to `out`, the rest to the returned parts.
    fn take_parts(&mut self, out: &mut Vec<Picture>) -> Parts {
        out.extend(self.flush());
        Parts {
            spss: std::mem::take(&mut self.spss),
            ppss: std::mem::take(&mut self.ppss),
            nal_length_size: self.nal_length_size,
            next_id: self.next_id,
            stats: self.shared.stats.clone(),
            error: self.shared.error.clone(),
            #[cfg(feature = "threads")]
            pool: self.pool.take(),
            max_in_flight: self.max_in_flight,
            threads: self.threads,
            draft: self.draft,
        }
    }

    /// For a slice NAL whose parameter sets are known: whether its SPS needs more than 8 bits.
    fn slice_is_wide(&self, nal: &[u8]) -> Option<bool> {
        let hdr = NalHeader::parse(*nal.first()?).ok()?;
        if !matches!(hdr.nal_unit_type, nal_type::SLICE | nal_type::IDR) {
            return None;
        }
        // first_mb_in_slice, slice_type and pic_parameter_set_id fit in the first few bytes
        let head = unescape_rbsp(nal.get(1..nal.len().min(33))?);
        let mut r = BitReader::new(&head);
        r.read_ue().ok()?;
        r.read_ue().ok()?;
        let pps_id = r.read_ue().ok()?;
        let pps = self.ppss.get(pps_id as usize)?.as_ref()?;
        let sps = self.spss.get(pps.sps_id as usize)?.as_ref()?;
        Some(sps.bit_depth_luma > 8 || sps.bit_depth_chroma > 8)
    }

    /// The access unit is complete: hand its picture to a decoding job and return the pictures
    /// ready for output (`result`: how parsing its NAL units went).
    fn finish_access_unit(&mut self, mut result: Result<()>) -> Result<Vec<Picture>> {
        self.submit_pending();
        if let Some(e) = self.take_error() {
            result = result.and(Err(e));
        }
        result?;
        // Return finished pictures without blocking on frames still being decoded (bounded latency).
        let max_queue = self.max_in_flight * 2;
        let mut pics = Vec::new();
        while let Some(o) = self.out_queue.front() {
            if !o.frame.is_published(o.frame.mb_h() - 1) && self.out_queue.len() <= max_queue {
                break;
            }
            let Some(o) = self.out_queue.pop_front() else { break };
            pics.push(make_picture(&o));
        }
        Ok(pics)
    }

    fn flush(&mut self) -> Vec<Picture> {
        self.submit_pending();
        self.close_open_field();
        let mut outs = Vec::new();
        self.dpb.flush(&mut outs);
        self.emit(outs);
        let pics = self.out_queue.drain(..).map(|o| make_picture(&o)).collect();
        while let Some(f) = self.in_flight.pop_front() {
            f.wait_complete();
        }
        pics
    }

    /// An unpaired first field gets its missing field ([`Frame::fill_missing_field`]).
    fn close_open_field(&mut self) {
        if let Some((frame, parity)) = self.open_field.take() {
            frame.fill_missing_field(parity);
        }
    }

    fn stats(&self) -> DecodeStats {
        self.shared.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn stat(&self, f: impl FnOnce(&mut DecodeStats)) {
        if let Ok(mut s) = self.shared.stats.lock() {
            f(&mut s);
        }
    }

    fn take_error(&mut self) -> Option<Error> {
        self.shared.error.lock().ok().and_then(|mut e| e.take())
    }

    fn handle_nal(&mut self, nal: &[u8], pts: i64) -> Result<()> {
        if nal.is_empty() {
            return Ok(());
        }
        let hdr = NalHeader::parse(nal[0])?;
        match hdr.nal_unit_type {
            nal_type::SPS => {
                let sps = Sps::parse(&unescape_rbsp(&nal[1..]))?;
                let id = sps.id as usize;
                self.spss[id] = Some(Arc::new(sps));
            }
            nal_type::PPS => {
                let rbsp = unescape_rbsp(&nal[1..]);
                let spss: Vec<Option<Sps>> = self.spss.iter().map(|s| s.as_ref().map(|s| (**s).clone())).collect();
                let pps = Pps::parse(&rbsp, &spss)?;
                let id = pps.id as usize;
                self.ppss[id] = Some(Arc::new(pps));
            }
            nal_type::SLICE | nal_type::IDR => self.handle_slice(nal, hdr, pts)?,
            nal_type::SLICE_DPA | nal_type::SLICE_DPB | nal_type::SLICE_DPC => {
                return unsupported("data partitioning (Extended profile)");
            }
            nal_type::END_SEQ | nal_type::END_STREAM => self.submit_pending(),
            // SEI, AUD, filler, SPS extension, prefix NAL, subset SPS, auxiliary and extension slices: skipped.
            _ => {}
        }
        Ok(())
    }

    fn level_scale(&mut self, pps: &Arc<Pps>) -> Arc<LevelScale> {
        if let Some((_, ls)) = self.ls_cache.iter().find(|(p, _)| Arc::ptr_eq(p, pps)) {
            return ls.clone();
        }
        let ls: Arc<LevelScale> = Arc::from(LevelScale::new(&pps.scaling));
        self.ls_cache.retain(|(p, _)| self.ppss.iter().flatten().any(|q| Arc::ptr_eq(p, q)));
        self.ls_cache.push((pps.clone(), ls.clone()));
        ls
    }

    fn handle_slice(&mut self, nal: &[u8], hdr: NalHeader, pts: i64) -> Result<()> {
        let rbsp = unescape_rbsp(&nal[1..]);
        let ppss = &self.ppss;
        let spss = &self.spss;
        let mut found: Option<(Arc<Pps>, Arc<Sps>)> = None;
        let (sh, _, _) = SliceHeader::parse(&rbsp, hdr, |id| {
            let pps = ppss[id as usize].as_ref().ok_or_else(|| Error::MissingParameterSet(format!("PPS {id}")))?;
            let sps = spss[pps.sps_id as usize].as_ref().ok_or_else(|| Error::MissingParameterSet(format!("SPS {}", pps.sps_id)))?;
            found = Some((pps.clone(), sps.clone()));
            Ok((&**pps, &**sps))
        })?;
        let Some((pps, sps)) = found else {
            return Err(Error::MissingParameterSet("PPS".into()));
        };
        sps.check_software_supported()?;
        if pps.num_slice_groups > 1 {
            return unsupported("slice groups (FMO)");
        }
        if matches!(sh.slice_type, SliceType::Sp | SliceType::Si) {
            return unsupported("SP/SI slices");
        }
        if sh.redundant_pic_cnt > 0 {
            return Ok(()); // redundant slices are ignored
        }
        let new_pic = match &self.pending {
            None => true,
            Some(p) => is_new_picture(&p.first, &sh, &sps) || sh.first_mb_in_slice == 0 || !Arc::ptr_eq(&p.sps, &sps),
        };
        if new_pic {
            self.submit_pending();
            if sh.first_mb_in_slice != 0 && self.active_sps.is_none() {
                return invalid("stream does not start with the first slice of a picture");
            }
            self.start_picture(&sh, &sps, pts)?;
        }
        let ls = self.level_scale(&pps);
        let Some(pending) = self.pending.as_mut() else {
            return invalid("slice without a started picture");
        };
        let refs = if sh.field_pic {
            // reference fields (8.2.4.2.5): a field of an entry, with that field's POC and marking
            self.dpb.build_field_ref_lists(&sh, pending.poc.picture(&sh), sps.max_frame_num(), |e, p| RefPic {
                frame: e.frame.clone(),
                long_term: e.field(p).mark == RefMark::Long,
                parity: Some(usize::from(p == Parity::Bottom)),
                poc: e.field(p).poc,
                field_pocs: [e.top.poc, e.bottom.poc],
            })?
        } else {
            // a frame, or a complementary field pair (its POC is the smaller field POC, 8.2.1)
            self.dpb.build_ref_lists(&sh, pending.poc.frame(), sps.max_frame_num(), |e| RefPic {
                poc: e.poc,
                field_pocs: [e.top.poc, e.bottom.poc],
                ..RefPic::frame(e.frame.clone(), e.mark == RefMark::Long)
            })?
        };
        if !sh.slice_type.is_intra() {
            ensure!(!refs[0].is_empty(), "no reference pictures available for inter slice");
            if sh.slice_type.is_b() {
                ensure!(!refs[1].is_empty(), "empty RefPicList1 in B slice");
            }
        }
        let weighted = (pps.weighted_pred && sh.slice_type.is_p()) || (pps.weighted_bipred_idc != 0 && sh.slice_type.is_b());
        let temporal = sh.slice_type.is_b() && !sh.direct_spatial_mv_pred;
        let (cabac, st, mmcos, lt) = (
            pps.entropy_coding_mode,
            sh.slice_type,
            sh.mmcos.len() as u64,
            sh.long_term_reference as u64 + sh.mmcos.iter().filter(|m| m.op == 3 || m.op == 6).count() as u64,
        );
        pending.slices.push(SliceJob { sh, pps, sps, rbsp, refs, ls });
        self.stat(|s| {
            if cabac {
                s.slices_cabac += 1;
            } else {
                s.slices_cavlc += 1;
            }
            match st {
                SliceType::I | SliceType::Si => s.slices_i += 1,
                SliceType::P | SliceType::Sp => s.slices_p += 1,
                SliceType::B => s.slices_b += 1,
            }
            s.weighted_slices += weighted as u64;
            s.temporal_direct_slices += temporal as u64;
            s.mmco_ops += mmcos;
            s.long_term_marks += lt;
        });
        Ok(())
    }

    fn start_picture(&mut self, sh: &SliceHeader, sps: &Arc<Sps>, pts: i64) -> Result<()> {
        let mb_w = sps.pic_width_in_mbs as usize;
        let mb_h = sps.frame_height_in_mbs() as usize;
        // One depth per frame for now (luma's); H2.2 decides on differing luma / chroma depths.
        let depth = sps.bit_depth_luma.clamp(8, 14) as u8;
        let c422 = sps.chroma_format_idc == 2;
        // A second field (asked before IDR / gap handling) decodes into its first field's frame;
        // any other picture closes an unpaired first field.
        let second = sh.field_pic && self.dpb.is_second_field(sh);
        let parity = usize::from(sh.bottom_field);
        let pair = match self.open_field.take() {
            Some((f, p)) if second && p != parity && f.mb_w == mb_w && f.mb_h() == mb_h && f.c422 == c422 && f.bit_depth == depth => Some(f),
            Some((f, p)) => {
                f.fill_missing_field(p);
                None
            }
            None => None,
        };
        if second && pair.is_none() {
            return invalid("second field does not match its first field");
        }
        // Activate SPS; a resolution change flushes the DPB.
        let changed = match &self.active_sps {
            None => true,
            Some(a) => a.width() != sps.width() || a.height() != sps.height() || a.max_dpb_frames() != sps.max_dpb_frames(),
        };
        if changed && self.active_sps.is_some() {
            let mut outs = Vec::new();
            self.dpb.flush(&mut outs);
            self.emit(outs);
            self.dpb.entries.clear();
        }
        self.active_sps = Some(sps.clone());
        self.dpb.capacity = sps.max_dpb_frames();
        self.dpb.max_reorder = sps.max_num_reorder_frames().min(self.dpb.capacity);
        if second {
            // neither IDR handling nor a frame_num gap: the field joins its first field's entry
        } else if sh.idr {
            let mut outs = Vec::new();
            self.dpb.idr(sh.no_output_of_prior_pics, &mut outs);
            self.emit(outs);
            self.prev_ref_frame_num = 0;
        } else {
            let max = sps.max_frame_num();
            if sh.frame_num != self.prev_ref_frame_num && sh.frame_num != (self.prev_ref_frame_num + 1) % max {
                // frame_num gap (8.2.5.2): insert "non-existing" frames. Their samples are never used by
                // conforming streams; copy the latest decoded frame for concealment.
                let meta = Arc::new(output_meta(sps, pts, false));
                let next_id = &mut self.next_id;
                let poc_state = &mut self.poc_state;
                let last = self.dpb.entries.iter().filter(|e| !e.non_existing).max_by_key(|e| e.frame.id).map(|e| e.frame.clone());
                let mut make = |_fnum: u32| {
                    let id = *next_id;
                    *next_id += 1;
                    let planes = match &last {
                        Some(f) if f.mb_w == mb_w && f.mb_h() == mb_h => {
                            let mut p = Planes::with_depth(mb_w * 16, mb_h * 16, depth, c422);
                            let (y, u, v) = f.copy_cropped_with((0, 0, mb_w * 16, mb_h * 16), Vec::with_capacity);
                            p.y = y;
                            p.cb = u;
                            p.cr = v;
                            p
                        }
                        _ => Planes::gray_with_depth(mb_w * 16, mb_h * 16, depth, c422),
                    };
                    (Arc::new(Frame::from_planes(id, 0, &planes, depth)), 0, meta.clone())
                };
                let mut upd = |fnum: u32| poc_state.update_gap_frame(fnum, sps);
                self.shared.stats.lock().map(|mut s| s.frame_num_gaps += 1).ok();
                self.dpb.fill_frame_num_gap(self.prev_ref_frame_num, sh.frame_num, max, sps.max_num_ref_frames as usize, &mut make, &mut upd);
                self.prev_ref_frame_num = (sh.frame_num + max - 1) % max;
            }
        }
        let poc = self.poc_state.compute(sh, sps);
        // MMCO5: the picture's POC becomes relative to itself (tempPicOrderCnt, 8.2.1).
        let frame_poc = if sh.has_mmco5() { 0 } else { poc.picture(sh) };
        let id = self.next_id;
        self.next_id += 1;
        let (frame, target, field) = if sh.field_pic {
            let frame = match pair {
                Some(f) => f,
                None => Arc::new(Frame::field_pair(id, frame_poc, mb_w, mb_h, depth, c422)),
            };
            let Some(target) = frame.field(parity).cloned() else {
                return invalid("field picture without field storage");
            };
            (frame, target, Some(parity))
        } else {
            let frame = Arc::new(Frame::with_depth(id, frame_poc, mb_w, mb_h, depth, c422));
            (frame.clone(), frame, None)
        };
        self.pending = Some(PendingPic {
            frame,
            target,
            field,
            second,
            cur_poc: frame_poc,
            sps: sps.clone(),
            first: sh.clone(),
            poc,
            pts,
            key: sh.idr,
            slices: Vec::new(),
        });
        Ok(())
    }

    /// Start decoding the pending picture and do its reference marking / DPB insertion.
    fn submit_pending(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let PendingPic { frame, target, field, second, cur_poc, sps, first, poc, pts, key, slices } = p;
        let draft = self.draft && first.nal_ref_idc == 0;
        // a frame's two field order counts (field macroblocks of MBAFF frames; 0-based after MMCO 5)
        let temp = if first.has_mmco5() { poc.frame() } else { 0 };
        let field_pocs = if field.is_some() { [cur_poc; 2] } else { [poc.top.wrapping_sub(temp), poc.bottom.wrapping_sub(temp)] };
        self.dispatch(target, slices, draft, (cur_poc, field_pocs), second.then(|| frame.clone()));
        self.poc_state.update(&first, &poc);
        if first.nal_ref_idc != 0 {
            self.prev_ref_frame_num = if first.has_mmco5() { 0 } else { first.frame_num };
        }
        let meta = Arc::new(OutputMeta { draft, ..output_meta(&sps, pts, key) });
        let mut outs = Vec::new();
        let (max_frame_num, max_refs) = (sps.max_frame_num(), sps.max_num_ref_frames as usize);
        if let Some(parity) = field {
            let temp = if first.has_mmco5() { poc.picture(&first) } else { 0 };
            let pocs = FieldPocs { top: poc.top.wrapping_sub(temp), bottom: poc.bottom.wrapping_sub(temp) };
            self.dpb.store(&first, frame.clone(), pocs, max_frame_num, max_refs, meta, &mut outs);
            if !second {
                self.open_field = Some((frame, parity));
            }
        } else {
            // both field order counts: fields of later field pictures reference them (the entry's
            // frame POC stays the smaller one, 0 after MMCO 5)
            let temp = if first.has_mmco5() { poc.frame() } else { 0 };
            let pocs = FieldPocs { top: poc.top.wrapping_sub(temp), bottom: poc.bottom.wrapping_sub(temp) };
            self.dpb.store(&first, frame, pocs, max_frame_num, max_refs, meta, &mut outs);
        }
        self.emit(outs);
    }

    /// Decode `target`'s slices as a job; `pair`: the field pair whose frame rows to publish
    /// afterwards (when `target` is its second field).
    fn dispatch(&mut self, target: FrameRef<S>, slices: Vec<SliceJob<S>>, draft: bool, poc: (i32, [i32; 2]), pair: Option<FrameRef<S>>) {
        let shared = self.shared.clone();
        #[cfg(feature = "threads")]
        if let Some(pool) = &self.pool {
            self.in_flight.retain(|f| !f.is_published(f.mb_h() - 1));
            while self.in_flight.len() >= self.max_in_flight {
                if let Some(f) = self.in_flight.pop_front() {
                    f.wait_complete();
                }
            }
            // the job is done when the last row it publishes is
            self.in_flight.push_back(pair.clone().unwrap_or_else(|| target.clone()));
            pool.spawn_fifo(move || run_job(target, slices, &shared, draft, poc, pair));
            return;
        }
        run_job(target, slices, &shared, draft, poc, pair);
    }

    fn emit(&mut self, outs: Vec<Output<FrameRef<S>>>) {
        self.out_queue.extend(outs);
    }
}

/// Publish every row of `frame` not published yet as mid-gray without motion, so readers never
/// block on a picture whose decoding failed.
fn publish_gray<S: OutputSample>(frame: &Frame<S>) {
    let fallback = Planes::gray_with_depth(frame.width, frame.height, frame.bit_depth, frame.c422);
    let blank = vec![MbState::default(); frame.mb_w * frame.mb_h()];
    for r in 0..frame.mb_h() {
        if !frame.is_published(r) {
            frame.publish(r, Frame::make_row(&fallback, r, &blank, &|_, _, _| u32::MAX));
        }
    }
}

/// Decode all slices of one picture (a frame or a field) and publish its rows; for the second
/// field of `pair`, then the pair's frame rows.
fn run_job<S: OutputSample>(frame: FrameRef<S>, slices: Vec<SliceJob<S>>, shared: &Shared<S>, draft: bool, poc: (i32, [i32; 2]), pair: Option<FrameRef<S>>) {
    let reuse = shared.pool.lock().ok().and_then(|mut p| p.pop());
    let mut pic = match reuse {
        Some(mut p) if p.mb_w == frame.mb_w && p.mb_h == frame.mb_h() && (p.planes.cheight == p.planes.height) == frame.c422 => {
            p.reset(frame.clone());
            p
        }
        _ => PicState::new(frame.clone()),
    };
    pic.skip_deblock = draft;
    // the two fields of a frame share its buffers but have their own order counts
    (pic.poc, pic.field_pocs) = poc;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut first_err = None;
        for s in &slices {
            if let Err(e) = decode_slice(s, &mut pic) {
                first_err.get_or_insert(e);
            }
        }
        pic.flush_rows(true);
        first_err
    }));
    let err = match result {
        Ok(e) => e,
        Err(_) => {
            // Make sure readers never block on this frame.
            publish_gray(&frame);
            pic = PicState::new(frame.clone());
            Some(Error::Invalid("internal error while decoding a picture".into()))
        }
    };
    let err = match pair {
        Some(pair) if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pair.interleave_all())).is_err() => {
            publish_gray(&pair);
            err.or_else(|| Some(Error::Invalid("internal error while interleaving fields".into())))
        }
        _ => err,
    };
    if let Ok(mut s) = shared.stats.lock() {
        s.pictures += 1;
        s.draft_pictures += draft as u64;
        for st in &pic.mbs {
            match st.kind {
                MbKind::I4x4 => s.mb_i4x4 += 1,
                MbKind::I8x8 => s.mb_i8x8 += 1,
                MbKind::I16x16 => s.mb_i16x16 += 1,
                MbKind::IPcm => s.mb_pcm += 1,
                MbKind::PSkip => s.mb_p_skip += 1,
                MbKind::BSkip => s.mb_b_skip += 1,
                MbKind::BDirect16x16 => s.mb_b_direct16x16 += 1,
                MbKind::Inter => s.mb_inter += 1,
                MbKind::None => {}
            }
            if st.transform_8x8 && !st.kind.is_intra() {
                s.mb_inter_8x8_transform += 1;
            }
        }
    }
    if let (Some(e), Ok(mut slot)) = (err, shared.error.lock()) {
        slot.get_or_insert(e);
    }
    if let Ok(mut p) = shared.pool.lock()
        && p.len() < 4
    {
        p.push(pic);
    }
}

fn decode_slice<S: OutputSample>(s: &SliceJob<S>, pic: &mut PicState<S>) -> Result<()> {
    // the chroma format and MBAFF as type-level constants (no 4:2:2 or MBAFF branches in the
    // 4:2:0 / progressive decoder)
    let mbaff = s.sps.mb_adaptive_frame_field && !s.sh.field_pic;
    match (pic.frame.c422, mbaff) {
        (false, false) => decode_slice_as::<S, false, false>(s, pic),
        (true, false) => decode_slice_as::<S, true, false>(s, pic),
        (false, true) => decode_slice_as::<S, false, true>(s, pic),
        (true, true) => decode_slice_as::<S, true, true>(s, pic),
    }
}

fn decode_slice_as<S: OutputSample, const C422: bool, const MBAFF: bool>(s: &SliceJob<S>, pic: &mut PicState<S>) -> Result<()> {
    let mut sd = SliceDecoder::<S, C422, MBAFF>::new(&s.sh, &s.pps, &s.sps, pic, &s.refs, &s.ls)?;
    if s.pps.entropy_coding_mode {
        sd.decode_cabac(&s.rbsp)
    } else {
        let mut r = BitReader::new(&s.rbsp);
        r.seek_bits(s.sh.header_bits);
        sd.decode_cavlc(&mut r)
    }
}

fn output_meta(sps: &Sps, pts: i64, key: bool) -> OutputMeta {
    let vui = sps.vui.clone().unwrap_or_default();
    OutputMeta {
        pts,
        key,
        crop: sps.crop_rect(),
        full_range: vui.full_range,
        colour_primaries: vui.colour_primaries,
        transfer_characteristics: vui.transfer_characteristics,
        matrix_coefficients: vui.matrix_coefficients,
        sar: vui.sar,
        draft: false,
    }
}

fn make_picture<S: OutputSample>(o: &Output<FrameRef<S>>) -> Picture {
    let (cx, cy, cw, ch) = o.meta.crop;
    let (cx, cy, cw, ch) = (cx as usize, cy as usize, cw as usize, ch as usize);
    let ((y, u, v), high) = S::crop(&o.frame, (cx, cy, cw, ch));
    let (ccw, cch) = (cw.div_ceil(2), if o.frame.c422 { ch } else { ch.div_ceil(2) });
    Picture {
        width: cw as u32,
        height: ch as u32,
        chroma_width: ccw as u32,
        chroma_height: cch as u32,
        y,
        u,
        v,
        bit_depth: u32::from(o.frame.bit_depth),
        chroma_format_idc: if o.frame.c422 { 2 } else { 1 },
        high,
        y_stride: cw,
        uv_stride: ccw,
        pts: o.meta.pts,
        poc: o.poc,
        key: o.meta.key,
        color: ColorInfo {
            full_range: o.meta.full_range,
            primaries: o.meta.colour_primaries,
            transfer: o.meta.transfer_characteristics,
            matrix: o.meta.matrix_coefficients,
        },
        sar: o.meta.sar,
        draft: o.meta.draft,
    }
}
