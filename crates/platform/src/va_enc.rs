//! In-process VA-API HEVC encoding (fork Phase 5.2b; docs/adr/0002-linux-video-interop.md).
//!
//! FilmCraft writes the parameter sets and slice headers itself ([`filmcraft_hevc::write`]) and
//! hands them to the driver as packed headers; the driver encodes the slice data. I and P
//! pictures (each predicted from the one before), one slice per picture, a closed GOP (IDR) every
//! `keyint` pictures, Main or Main 10, constant QP / CBR / VBR. Pictures go to the GPU a few
//! ahead of their read-back ([`DEPTH`]), so uploading the next frame overlaps encoding this one.
//! Frames come as memory ([`VaHevcEncoder::encode`], uploaded with `vaPutImage`) or already in
//! one of the encoder's input surfaces ([`VaHevcEncoder::encode_surface`]: the export rendered
//! them there on the GPU, fork Phase 5.2c; [`VaHevcEncoder::target`] says where).
//!
//! Everything here is safe code over [`crate::va`].

use std::collections::VecDeque;
use std::sync::Arc;

use filmcraft_hevc::write::{self as hw, SlicePic, StreamParams};
use rayon::prelude::*;

use crate::va::{CodedBuffer, Config, Context, Display, MiscFrameRate, MiscHrd, MiscQualityLevel, MiscRateControl, Surface, SurfacePool};
use crate::va_sys as sys;

/// Pictures submitted before the oldest is read back.
const DEPTH: usize = 3;

/// Rate control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// Constant QP (1–51).
    Cqp(u8),
    Cbr {
        kbps: u32,
    },
    Vbr {
        kbps: u32,
        max_kbps: u32,
    },
}

/// What to encode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeParams {
    pub width: u32,
    pub height: u32,
    /// Pictures per second: `rate_num / rate_den`.
    pub rate_num: u32,
    pub rate_den: u32,
    pub ten_bit: bool,
    /// Pictures from one IDR to the next (1: all intra).
    pub keyint: u32,
    pub rc: RateControl,
    /// H.273 primaries, transfer, matrix.
    pub colour: (u8, u8, u8),
    /// Speed / quality level (fork 6E.2), mapped onto the driver's quality range.
    pub speed: filmcraft_export::HwEncodeSpeed,
    /// 4:2:2 (Main 4:2:2 10, fork 6E.4; needs `ten_bit`) instead of 4:2:0.
    pub chroma_422: bool,
    /// Prefer the driver's low-power entry point, `VAEntrypointEncSliceLP` (Intel's fixed-function
    /// encoder, fork 6E.3 S12): non-IDR pictures are then low-delay B (iHD's LP mode takes no P).
    /// Used only where the driver offers it for the profile and a small trial encode works
    /// ([`lp_usable`]); otherwise, or when it fails to open, `EncSlice` as before.
    pub low_power: bool,
}

/// Where the export may render this encoder's input (fork Phase 5.2c): surfaces of `pool`
/// (the coded size, P010 when `ten_bit`, else NV12), for a picture of `size`.
pub struct EncodeTarget {
    pub pool: Arc<SurfacePool>,
    pub ten_bit: bool,
    /// Y210 (4:2:2) surfaces: the compositor doesn't render into these yet (fork 6E.4).
    pub chroma_422: bool,
    pub size: (u32, u32),
}

/// An input surface: uploaded by the encoder, or shared with the frame that was rendered into it.
enum Source {
    Own(Surface),
    Shared(Arc<Surface>),
}

impl Source {
    fn surface(&self) -> &Surface {
        match self {
            Source::Own(s) => s,
            Source::Shared(s) => s,
        }
    }
}

struct InFlight {
    source: Source,
    /// The reference the picture was predicted from (kept until it is read back).
    _reference: Option<Surface>,
    coded: CodedBuffer,
}

/// A VA-API HEVC encoder: planar 4:2:0 frames in, one Annex B access unit per picture out (in
/// order, [`DEPTH`] pictures late until [`VaHevcEncoder::flush`]).
pub struct VaHevcEncoder {
    // dropped first: everything in flight, then the context
    inflight: VecDeque<InFlight>,
    spare: Vec<CodedBuffer>,
    reference: Option<(Surface, i32)>,
    sources: Arc<SurfacePool>,
    recons: Arc<SurfacePool>,
    ctx: Context,
    _config: Arc<Config>,
    params: EncodeParams,
    stream: StreamParams,
    /// Position in the GOP of the next picture (0: IDR).
    gop_pos: u32,
    coded_bytes: usize,
    /// The quality level sent with each sequence (`None`: the driver's default).
    quality_level: Option<u32>,
    /// This driver writes its own SPS / PPS ([`StreamParams::rewritten`]): it gets the QP as
    /// before (`pic_init_qp` = the QP), not `stream.init_qp` + a slice delta.
    rewrites: bool,
    /// On `VAEntrypointEncSliceLP`: non-IDR pictures are low-delay B (fork 6E.3 S12).
    lp: bool,
}

impl Drop for VaHevcEncoder {
    /// Pictures still in flight (an export cancelled or failed) are waited for before their
    /// surfaces go back to the pools and their coded buffers are destroyed.
    fn drop(&mut self) {
        for f in &self.inflight {
            let _ = f.source.surface().sync();
        }
    }
}

/// What [`VaHevcEncoder::probe`] found on a device.
struct Probe {
    stream: StreamParams,
    prof: sys::VAProfile,
    ep: sys::VAEntrypoint,
    rt: u32,
    rc_mode: u32,
    packed: u32,
    /// `VAConfigAttribEncQualityRange` (0: not reported).
    quality_range: u32,
}

/// Two-bit fields of the driver's HEVC feature / block-size attributes.
fn field(v: u32, shift: u32) -> u32 {
    (v >> shift) & 3
}

impl VaHevcEncoder {
    /// Whether the driver encodes HEVC (`ten_bit`: Main 10).
    pub fn supported(ten_bit: bool) -> bool {
        Display::for_encode().is_ok_and(|d| d.supports(profile(ten_bit), sys::VAEntrypoint_VAEntrypointEncSlice))
    }

    /// Whether a device encodes Main 4:2:2 10 (fork 6E.4: Intel's, not AMD's).
    pub fn supported_422() -> bool {
        Display::for_encode_422().is_ok()
    }

    /// The render node 4:2:2 is encoded on ([`Display::for_encode_422`]), for ffmpeg's fallback.
    pub fn node_422() -> Option<String> {
        Display::for_encode_422().ok().map(|d| d.node.clone())
    }

    /// How many quality levels the encode device's HEVC encoder offers
    /// (`VAConfigAttribEncQualityRange`; `None`: no encoder, or it does not say).
    pub fn quality_range(ten_bit: bool) -> Option<u32> {
        let d = Display::for_encode().ok()?;
        let a = d.config_attribs(profile(ten_bit), sys::VAEntrypoint_VAEntrypointEncSlice, &[sys::VAConfigAttribType_VAConfigAttribEncQualityRange]).ok()?;
        a.first().copied().flatten()
    }

    /// The quality level this encoder sends (`None`: the driver's default).
    pub fn quality_level(&self) -> Option<u32> {
        self.quality_level
    }

    /// An encoder on the process's encode display ([`Display::for_encode`]; 4:2:2 on
    /// [`Display::for_encode_422`]).
    pub fn new(params: EncodeParams) -> Result<Self, String> {
        let d = if params.chroma_422 { Display::for_encode_422()? } else { Display::for_encode()? };
        Self::new_on(params, d)
    }

    /// An encoder on display `d` (fork 6E.3: one per GPU, e.g. [`Display::for_node`]).
    pub fn new_on(params: EncodeParams, d: Arc<Display>) -> Result<Self, String> {
        Self::new_on_with(params, d, None)
    }

    /// The stream an encoder on display `d` would write for `params` (fork 6E.3: two GPUs agree on
    /// one with [`hw::common_stream`] and pass it to [`VaHevcEncoder::new_on_with`]).
    pub fn stream_params_for(params: &EncodeParams, d: &Display) -> Result<StreamParams, String> {
        Ok(Self::probe(params, d)?.stream)
    }

    /// An encoder on display `d` writing `stream` instead of the driver's own choice (fork 6E.3:
    /// identical parameter sets on two GPUs). `stream` must stay inside what this driver offers
    /// (block sizes, tools, the same picture), else an error. With [`EncodeParams::low_power`]
    /// an LP encoder that fails to open falls back to `EncSlice` (never an error because of LP).
    pub fn new_on_with(params: EncodeParams, d: Arc<Display>, stream: Option<StreamParams>) -> Result<Self, String> {
        let probe = Self::probe(&params, &d)?;
        if probe.ep != sys::VAEntrypoint_VAEntrypointEncSliceLP {
            return Self::open(params, d, stream, probe);
        }
        #[cfg(test)]
        let lp = if tests::FAIL_LP_OPEN.with(|f| f.get()) {
            Err("LP open failure injected by a test".to_string())
        } else {
            Self::open(params.clone(), d.clone(), stream.clone(), probe)
        };
        #[cfg(not(test))]
        let lp = Self::open(params.clone(), d.clone(), stream.clone(), probe);
        lp.or_else(|e| {
            eprintln!("VA-API HEVC: the low-power encoder on {} did not open ({e}); EncSlice", d.vendor);
            Self::open(params.clone(), d.clone(), stream, Self::probe_ep(&params, &d, false)?)
        })
    }

    /// Whether this encoder runs on `VAEntrypointEncSliceLP` with low-delay B pictures.
    pub fn low_power(&self) -> bool {
        self.lp
    }

    fn open(params: EncodeParams, d: Arc<Display>, stream: Option<StreamParams>, probe: Probe) -> Result<Self, String> {
        let Probe { stream: own, prof, ep, rt, rc_mode, packed, quality_range } = probe;
        let lp = ep == sys::VAEntrypoint_VAEntrypointEncSliceLP;
        let rewrites = own.rewritten;
        let stream = match stream {
            None => own,
            Some(s) => {
                if hw::common_stream(&own, &s).as_ref() != Some(&s) {
                    return Err(format!("the driver ({}) cannot write the shared stream", d.vendor));
                }
                s
            }
        };
        let ten_bit = params.ten_bit;
        let c422 = params.chroma_422;
        use sys::{
            VAConfigAttribType_VAConfigAttribEncPackedHeaders as PACKED, VAConfigAttribType_VAConfigAttribRTFormat as RT,
            VAConfigAttribType_VAConfigAttribRateControl as RC,
        };
        let config = Arc::new(Config::with_attribs(&d, prof, ep, &[(RT, rt), (RC, rc_mode), (PACKED, packed)])?);
        let ctx = Context::new(&config, stream.coded_width, stream.coded_height)?;
        let fourcc = match (c422, ten_bit) {
            (true, _) => sys::VA_FOURCC_Y210,
            (false, true) => sys::VA_FOURCC_P010,
            (false, false) => sys::VA_FOURCC_NV12,
        };
        let sources = SurfacePool::new(&d, rt, fourcc, stream.coded_width, stream.coded_height);
        let recons = SurfacePool::new(&d, rt, fourcc, stream.coded_width, stream.coded_height);
        // a picture rarely compresses worse than 1 byte per pixel; overflow is reported
        let coded_bytes = (stream.coded_width as usize * stream.coded_height as usize).saturating_add(1 << 20);
        let quality_level = params.speed.level(quality_range);
        Ok(VaHevcEncoder {
            inflight: VecDeque::new(),
            spare: Vec::new(),
            reference: None,
            sources,
            recons,
            ctx,
            _config: config,
            params,
            stream,
            gop_pos: 0,
            coded_bytes,
            quality_level,
            rewrites,
            lp,
        })
    }

    /// The next picture starts a new GOP with an IDR (fork 6E.3: a chunk of a two-GPU export).
    pub fn restart_gop(&mut self) {
        self.gop_pos = 0;
    }

    /// Checks `params` against display `d`'s encoder and works out the stream it writes: on
    /// `EncSliceLP` when asked for and usable there (its own attributes), else `EncSlice`.
    fn probe(params: &EncodeParams, d: &Display) -> Result<Probe, String> {
        let prof = if params.chroma_422 { sys::VAProfile_VAProfileHEVCMain422_10 } else { profile(params.ten_bit) };
        if params.low_power
            && d.supports(prof, sys::VAEntrypoint_VAEntrypointEncSliceLP)
            && lp_usable(d, params)
            && let Ok(p) = Self::probe_ep(params, d, true)
        {
            return Ok(p);
        }
        Self::probe_ep(params, d, false)
    }

    /// [`probe`](Self::probe) on one entry point (`lp`: `EncSliceLP`).
    fn probe_ep(params: &EncodeParams, d: &Display, lp: bool) -> Result<Probe, String> {
        let &EncodeParams { width: w, height: h, ten_bit, .. } = params;
        if w == 0 || h == 0 || w > 8192 || h > 8192 || w % 2 != 0 || h % 2 != 0 {
            return Err(format!("{w}x{h}"));
        }
        if params.rate_num == 0 || params.rate_den == 0 {
            return Err("frame rate".into());
        }
        let c422 = params.chroma_422;
        if c422 && !ten_bit {
            return Err("4:2:2 is encoded as 10-bit only".into());
        }
        let prof = if c422 { sys::VAProfile_VAProfileHEVCMain422_10 } else { profile(ten_bit) };
        let ep = if lp { sys::VAEntrypoint_VAEntrypointEncSliceLP } else { sys::VAEntrypoint_VAEntrypointEncSlice };
        if !d.supports(prof, ep) {
            let name = if c422 {
                "Main 4:2:2 10"
            } else if ten_bit {
                "Main 10"
            } else {
                "Main"
            };
            return Err(format!("the driver ({}) has no HEVC {name} encoder", d.vendor));
        }
        use sys::{
            VAConfigAttribType_VAConfigAttribEncHEVCBlockSizes as BLOCKS, VAConfigAttribType_VAConfigAttribEncHEVCFeatures as FEATURES,
            VAConfigAttribType_VAConfigAttribEncPackedHeaders as PACKED, VAConfigAttribType_VAConfigAttribEncQualityRange as QR,
            VAConfigAttribType_VAConfigAttribMaxPictureHeight as MAX_H, VAConfigAttribType_VAConfigAttribMaxPictureWidth as MAX_W,
            VAConfigAttribType_VAConfigAttribRTFormat as RT, VAConfigAttribType_VAConfigAttribRateControl as RC,
        };
        let a = d.config_attribs(prof, ep, &[RT, RC, PACKED, FEATURES, BLOCKS, MAX_W, MAX_H, QR])?;
        let get = |i: usize| a.get(i).copied().flatten();
        let rt = match (c422, ten_bit) {
            (true, _) => sys::VA_RT_FORMAT_YUV422_10,
            (false, true) => sys::VA_RT_FORMAT_YUV420_10,
            (false, false) => sys::VA_RT_FORMAT_YUV420,
        };
        if get(0).is_some_and(|v| v & rt == 0) {
            return Err("the encoder does not take this picture format".into());
        }
        let rc_mode = rc_mode(params.rc);
        if get(1).is_some_and(|v| v & rc_mode == 0) {
            return Err(format!("the encoder has no rate control mode {rc_mode:#x}"));
        }
        let packed = sys::VA_ENC_PACKED_HEADER_SEQUENCE | sys::VA_ENC_PACKED_HEADER_SLICE;
        if get(2).is_none_or(|v| v & packed != packed) {
            return Err("the encoder takes no packed sequence / slice headers".into());
        }
        if get(5).is_some_and(|m| w > m) || get(6).is_some_and(|m| h > m) {
            return Err(format!("{w}x{h} is larger than the encoder's {}x{}", get(5).unwrap_or(0), get(6).unwrap_or(0)));
        }

        let mut stream = StreamParams::new(w, h, if ten_bit { 10 } else { 8 });
        stream.chroma_422 = c422;
        // the driver's coding tools and block sizes (defaults: what Mesa's encoders report)
        if let Some(b) = get(4) {
            stream.log2_ctb = field(b, 0) + 3;
            stream.log2_min_cb = field(b, 4) + 3;
            stream.log2_max_tb = field(b, 6) + 2;
            stream.log2_min_tb = field(b, 8) + 2;
            stream.max_th_depth_inter = field(b, 10);
            stream.max_th_depth_intra = field(b, 14);
        } else {
            stream.max_th_depth_inter = 4;
            stream.max_th_depth_intra = 4;
        }
        if stream.log2_min_cb > stream.log2_ctb || stream.log2_min_tb > stream.log2_max_tb || stream.log2_max_tb > stream.log2_min_cb.max(5) {
            return Err(format!("odd block sizes from the driver: {:#x}", get(4).unwrap_or(0)));
        }
        let cqp = matches!(params.rc, RateControl::Cqp(_));
        if let Some(f) = get(3) {
            stream.amp = field(f, 4) != 0;
            stream.sao = field(f, 6) != 0;
            stream.transform_skip = field(f, 20) != 0;
            stream.cu_qp_delta = field(f, 22) == 2 || (!cqp && field(f, 22) != 0);
        } else {
            stream.cu_qp_delta = !cqp;
        }
        // coded size: whole minimum coding blocks, and 16-aligned as the surfaces are
        let align = (1u32 << stream.log2_min_cb).max(16);
        stream.coded_width = w.div_ceil(align) * align;
        stream.coded_height = h.div_ceil(align) * align;
        let kbps = match params.rc {
            RateControl::Cqp(_) => 0,
            RateControl::Cbr { kbps } => kbps,
            RateControl::Vbr { kbps, max_kbps } => max_kbps.max(kbps),
        };
        (stream.level_idc, stream.high_tier) = hw::level(w, h, params.rate_num, params.rate_den, kbps);
        stream.init_qp = match params.rc {
            RateControl::Cqp(q) => q.clamp(1, 51) as i32,
            _ => 26,
        };
        if d.vendor.contains("radeonsi") {
            // radeonsi writes its own SPS / PPS whatever packed headers it gets (seen on Mesa
            // 26.2, fork 6E.3 S8): describe what it really writes, so a stream shared with
            // another GPU can match it byte for byte
            let max_depth = stream.log2_ctb.saturating_sub(stream.log2_min_tb);
            stream.max_th_depth_inter = max_depth.min(4);
            stream.max_th_depth_intra = max_depth.min(4);
            stream.dependent_slices = true;
            stream.init_qp = 26;
            stream.rewritten = true;
        }
        stream.colour = params.colour;
        stream.timing = Some((params.rate_den, params.rate_num));
        Ok(Probe { stream, prof, ep, rt, rc_mode, packed, quality_range: get(7).unwrap_or(0) })
    }

    /// The parameter sets the stream starts with (Annex B: VPS, SPS, PPS).
    pub fn headers(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in [hw::vps(&self.stream), hw::sps(&self.stream), hw::pps(&self.stream)] {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&nal);
        }
        out
    }

    /// The parameters the stream is written with.
    pub fn stream(&self) -> &StreamParams {
        &self.stream
    }

    /// Encode one frame given as planar `yuv420p10le` (10-bit) or `yuv420p` of the encoder's
    /// size (`yuv422p10le` for a 4:2:2 encoder). Returns the access units (Annex B) of pictures that finished.
    pub fn encode(&mut self, yuv: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let source = self.sources.get()?;
        let (w, h) = (self.params.width as usize, self.params.height as usize);
        let ten = self.params.ten_bit;
        if self.params.chroma_422 {
            source.write(|img, offsets, pitches| fill_y210(img, offsets[0], pitches[0], yuv, w, h))?;
        } else {
            source.write(|img, offsets, pitches| fill_semiplanar(img, offsets, pitches, yuv, w, h, ten))?;
        }
        self.submit(Source::Own(source))?;
        self.finished()
    }

    /// Where frames may be rendered for [`encode_surface`](Self::encode_surface).
    pub fn target(&self) -> EncodeTarget {
        EncodeTarget {
            pool: self.sources.clone(),
            ten_bit: self.params.ten_bit,
            chroma_422: self.params.chroma_422,
            size: (self.params.width, self.params.height),
        }
    }

    /// Encode a frame already in one of the encoder's input surfaces ([`target`](Self::target)),
    /// fully written (the caller waited for the GPU). The surface returns to the pool once the
    /// picture is out and the last holder drops it.
    pub fn encode_surface(&mut self, surface: Arc<Surface>) -> Result<Vec<Vec<u8>>, String> {
        if !Arc::ptr_eq(surface.pool(), &self.sources) {
            return Err("the surface is not one of this encoder's inputs".into());
        }
        self.submit(Source::Shared(surface))?;
        self.finished()
    }

    /// Access units of the pictures beyond the in-flight depth.
    fn finished(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        while self.inflight.len() > DEPTH {
            out.push(self.retire()?);
        }
        Ok(out)
    }

    /// Every picture still in flight.
    pub fn flush(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        while !self.inflight.is_empty() {
            out.push(self.retire()?);
        }
        Ok(out)
    }

    fn retire(&mut self) -> Result<Vec<u8>, String> {
        let mut f = self.inflight.pop_front().ok_or("nothing in flight")?;
        f.source.surface().sync()?;
        let bytes = f.coded.read()?;
        self.spare.push(f.coded);
        if bytes.is_empty() {
            return Err("the encoder wrote nothing".into());
        }
        Ok(bytes)
    }

    fn submit(&mut self, source: Source) -> Result<(), String> {
        let idr = self.gop_pos == 0;
        let poc = self.gop_pos as i32;
        let reference = if idr { None } else { self.reference.take() };
        if !idr && reference.is_none() {
            return Err("P picture without a reference".into());
        }
        let recon = self.recons.get()?;
        let coded = match self.spare.pop() {
            Some(c) => c,
            None => self.ctx.coded_buffer(self.coded_bytes)?,
        };
        let s = &self.stream;
        let target = match self.params.rc {
            RateControl::Cqp(q) => q.clamp(1, 51),
            _ => 26,
        };
        // the PPS's init_qp plus the slice's delta gives the QP (fork 6E.3 S8: a shared stream
        // may carry another driver's init_qp); a rewriting driver writes both itself
        let init = if self.rewrites { target } else { u8::try_from(s.init_qp_clamped()).unwrap_or(26) };
        let (qp, qp_delta) = (init, i32::from(target) - i32::from(init));
        // iHD's low-power encoder takes low-delay B, not P (fork 6E.3 S12): the same one
        // reference in both lists
        let b = self.lp && !idr;
        let slice_pic = SlicePic { idr, poc: self.gop_pos, qp_delta, low_delay_b: b };

        let ctx = &self.ctx;
        let mut before = Vec::new();
        if idr {
            before.push(ctx.sequence_buffer(&self.sequence_params())?);
            for b in self.misc_buffers()? {
                before.push(b);
            }
            let [p, d] = ctx.packed_header(sys::VAEncPackedHeaderType_VAEncPackedHeaderSequence, &self.headers())?;
            before.push(p);
            before.push(d);
        }
        let mut slice_nal = vec![0, 0, 0, 1];
        slice_nal.extend(hw::slice_header(s, &slice_pic));
        let [p, d] = ctx.packed_header(sys::VAEncPackedHeaderType_VAEncPackedHeaderSlice, &slice_nal)?;
        before.push(p);
        before.push(d);

        let mut pic = sys::VAEncPictureParameterBufferHEVC { pic_init_qp: qp, collocated_ref_pic_index: 0xff, ..Default::default() };
        pic.decoded_curr_pic.pic_order_cnt = poc;
        pic.nal_unit_type = if idr { hw::NAL_IDR_W_RADL } else { hw::NAL_TRAIL_R };
        let mut f = sys::_VAEncPictureParameterBufferHEVC__bindgen_ty_1__bindgen_ty_1::default();
        f.set_idr_pic_flag(idr as u32);
        f.set_coding_type(match (idr, b) {
            (true, _) => 1,
            (false, true) => 3,
            (false, false) => 2,
        });
        f.set_reference_pic_flag(1);
        f.set_transform_skip_enabled_flag(s.transform_skip as u32);
        f.set_cu_qp_delta_enabled_flag(s.cu_qp_delta as u32);
        f.set_dependent_slice_segments_enabled_flag(s.dependent_slices as u32);
        f.set_pps_loop_filter_across_slices_enabled_flag(1);
        pic.pic_fields.bits = f;

        let mut slice = sys::VAEncSliceParameterBufferHEVC {
            num_ctu_in_slice: s.ctbs(),
            slice_type: match (idr, b) {
                (true, _) => 2,
                (false, true) => 0,
                (false, false) => 1,
            },
            max_num_merge_cand: 5,
            slice_qp_delta: qp_delta as i8,
            ..Default::default()
        };
        let mut sf = sys::_VAEncSliceParameterBufferHEVC__bindgen_ty_1__bindgen_ty_1::default();
        sf.set_last_slice_of_pic_flag(1);
        sf.set_slice_sao_luma_flag(s.sao as u32);
        sf.set_slice_sao_chroma_flag(s.sao as u32);
        if b {
            // as the packed slice header says (temporal MVP is off: collocated_from_l0 inferred 1;
            // mvd_l1_zero_flag 0, which iHD codes whatever it is told: SlicePic::low_delay_b)
            sf.set_mvd_l1_zero_flag(0);
            sf.set_collocated_from_l0_flag(1);
        }
        slice.slice_fields.bits = sf;

        let refs = reference.as_ref().map(|(r, p)| (r, *p));
        ctx.encode_hevc(source.surface(), &recon, refs, &coded, pic, slice, &before)?;
        drop(before);
        self.inflight.push_back(InFlight { source, _reference: reference.map(|r| r.0), coded });
        self.reference = Some((recon, poc));
        self.gop_pos = (self.gop_pos + 1) % self.params.keyint.max(1);
        Ok(())
    }

    fn sequence_params(&self) -> sys::VAEncSequenceParameterBufferHEVC {
        let s = &self.stream;
        let p = &self.params;
        let bps = match p.rc {
            RateControl::Cqp(_) => 0,
            RateControl::Cbr { kbps } | RateControl::Vbr { kbps, .. } => kbps.saturating_mul(1000),
        };
        let mut q = sys::VAEncSequenceParameterBufferHEVC {
            // Main 4:2:2 10 (RExt, 4) / Main 10 / Main: must match the packed VPS / SPS
            general_profile_idc: if s.chroma_422 {
                4
            } else if p.ten_bit {
                2
            } else {
                1
            },
            general_level_idc: s.level_idc,
            general_tier_flag: s.high_tier as u8,
            intra_period: p.keyint.max(1),
            intra_idr_period: p.keyint.max(1),
            ip_period: 1,
            bits_per_second: bps,
            pic_width_in_luma_samples: s.coded_width as u16,
            pic_height_in_luma_samples: s.coded_height as u16,
            log2_min_luma_coding_block_size_minus3: (s.log2_min_cb - 3) as u8,
            log2_diff_max_min_luma_coding_block_size: (s.log2_ctb - s.log2_min_cb) as u8,
            log2_min_transform_block_size_minus2: (s.log2_min_tb - 2) as u8,
            log2_diff_max_min_transform_block_size: (s.log2_max_tb - s.log2_min_tb) as u8,
            max_transform_hierarchy_depth_inter: s.max_th_depth_inter as u8,
            max_transform_hierarchy_depth_intra: s.max_th_depth_intra as u8,
            vui_parameters_present_flag: 1,
            aspect_ratio_idc: 1,
            vui_num_units_in_tick: p.rate_den,
            vui_time_scale: p.rate_num,
            ..Default::default()
        };
        let mut f = sys::_VAEncSequenceParameterBufferHEVC__bindgen_ty_1__bindgen_ty_1::default();
        f.set_chroma_format_idc(if s.chroma_422 { 2 } else { 1 });
        f.set_bit_depth_luma_minus8(s.bit_depth - 8);
        f.set_bit_depth_chroma_minus8(s.bit_depth - 8);
        f.set_amp_enabled_flag(s.amp as u32);
        f.set_sample_adaptive_offset_enabled_flag(s.sao as u32);
        f.set_low_delay_seq(1);
        q.seq_fields.bits = f;
        let mut v = sys::_VAEncSequenceParameterBufferHEVC__bindgen_ty_2__bindgen_ty_1::default();
        v.set_aspect_ratio_info_present_flag(1);
        v.set_vui_timing_info_present_flag(1);
        v.set_log2_max_mv_length_horizontal(15);
        v.set_log2_max_mv_length_vertical(15);
        q.vui_fields.bits = v;
        q
    }

    fn misc_buffers(&self) -> Result<Vec<crate::va::Buffer<'_>>, String> {
        let p = &self.params;
        let mut out = Vec::new();
        if let Some(level) = self.quality_level {
            out.push(self.ctx.param_buffer(&MiscQualityLevel::new(level))?);
        }
        let (kbps, max_kbps) = match p.rc {
            RateControl::Cqp(_) => return Ok(out),
            RateControl::Cbr { kbps } => (kbps, kbps),
            RateControl::Vbr { kbps, max_kbps } => (kbps, max_kbps.max(kbps)),
        };
        let max_bps = max_kbps.max(1).saturating_mul(1000);
        let rc = sys::VAEncMiscParameterRateControl {
            bits_per_second: max_bps,
            target_percentage: ((kbps.max(1) as u64 * 100) / max_kbps.max(1) as u64).clamp(1, 100) as u32,
            window_size: 1000,
            ..Default::default()
        };
        // a one-second buffer, three quarters full at the start
        let hrd = sys::VAEncMiscParameterHRD { buffer_size: max_bps, initial_buffer_fullness: max_bps / 4 * 3, ..Default::default() };
        let rate = if p.rate_num <= 0xffff && p.rate_den <= 0xffff { p.rate_num | (p.rate_den << 16) } else { p.rate_num.div_ceil(p.rate_den.max(1)) };
        let fr = sys::VAEncMiscParameterFrameRate { framerate: rate, ..Default::default() };
        out.push(self.ctx.param_buffer(&MiscRateControl::new(rc))?);
        out.push(self.ctx.param_buffer(&MiscHrd::new(hrd))?);
        out.push(self.ctx.param_buffer(&MiscFrameRate::new(fr))?);
        Ok(out)
    }
}

/// Whether display `d`'s `EncSliceLP` encodes low-delay B pictures for `params`' profile (fork
/// 6E.3 S12). A rate-control mode LP does not offer is `false` before any trial. Otherwise a
/// small trial encode (IDR + two B pictures of moving texture, constant QP 26 for a CQP export,
/// the default speed: the caller's QP and speed don't decide) that FilmCraft's own decoder must
/// read back close to the source, once per render node, bit depth, chroma and rate-control mode.
/// Only definite results are kept (the driver refused the profile / mode, or the trial ran and
/// its stream did or did not decode well); a transient error (display, context, surface or
/// picture submission) is `false` this time and tried again at the next open.
fn lp_usable(d: &Display, params: &EncodeParams) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static SEEN: OnceLock<Mutex<HashMap<(String, bool, bool, u32), bool>>> = OnceLock::new();
    let rc = rc_mode(params.rc);
    let prof = if params.chroma_422 { sys::VAProfile_VAProfileHEVCMain422_10 } else { profile(params.ten_bit) };
    let offered = d.config_attribs(prof, sys::VAEntrypoint_VAEntrypointEncSliceLP, &[sys::VAConfigAttribType_VAConfigAttribRateControl]);
    if !offered.is_ok_and(|a| a.first().copied().flatten().is_some_and(|v| v & rc != 0)) {
        return false;
    }
    let key = (d.node.clone(), params.ten_bit, params.chroma_422, rc);
    let seen = SEEN.get_or_init(Default::default);
    // not held during the trial: two lanes racing may both try it, which is harmless
    if let Some(&ok) = seen.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return ok;
    }
    let ok = match lp_trial(d, params) {
        Ok(()) => Some(true),
        Err(Trial::Definite(e)) => {
            eprintln!("VA-API HEVC: low-power encoding on {} ({}) is not usable: {e}", d.node, d.vendor);
            Some(false)
        }
        Err(Trial::Transient(e)) => {
            eprintln!("VA-API HEVC: low-power trial on {} ({}) failed, EncSlice this time: {e}", d.node, d.vendor);
            None
        }
    };
    if let Some(ok) = ok {
        seen.lock().unwrap_or_else(PoisonError::into_inner).insert(key, ok);
    }
    ok.unwrap_or(false)
}

/// Why a low-power trial failed: kept for the process, or worth another try.
enum Trial {
    Definite(String),
    Transient(String),
}

fn rc_mode(rc: RateControl) -> u32 {
    match rc {
        RateControl::Cqp(_) => sys::VA_RC_CQP,
        RateControl::Cbr { .. } => sys::VA_RC_CBR,
        RateControl::Vbr { .. } => sys::VA_RC_VBR,
    }
}

fn lp_trial(d: &Display, params: &EncodeParams) -> Result<(), Trial> {
    use Trial::{Definite, Transient};
    let (w, h) = (256usize, 144usize);
    let rc = match params.rc {
        RateControl::Cqp(_) => RateControl::Cqp(26),
        other => other,
    };
    let small = EncodeParams { width: w as u32, height: h as u32, keyint: 30, rc, speed: Default::default(), ..params.clone() };
    let display = Display::for_node(&d.node).map_err(Transient)?;
    // the profile, entry point, picture format, rate control or packed headers refused
    let probe = VaHevcEncoder::probe_ep(&small, &display, true).map_err(Definite)?;
    let mut enc = VaHevcEncoder::open(small.clone(), display, None, probe).map_err(Transient)?;
    let ten = small.ten_bit;
    // a moving ramp with fine detail: inter prediction with real motion vectors
    let luma = |x: usize, y: usize, i: usize| {
        let v = ((x + y + i * 3) % 128) * 3 + (x.wrapping_mul(31) ^ y.wrapping_mul(17)) % 16;
        if ten { 64 + 2 * v as u16 } else { 16 + (v / 2) as u16 }
    };
    let rows = if small.chroma_422 { h } else { h / 2 };
    let chroma = w / 2 * rows * 2;
    let frames = 3;
    let mut stream = Vec::new();
    for i in 0..frames {
        let mut yuv = Vec::with_capacity((w * h + chroma) * 2);
        for y in 0..h {
            for x in 0..w {
                if ten {
                    yuv.extend_from_slice(&luma(x, y, i).to_le_bytes());
                } else {
                    yuv.push(u8::try_from(luma(x, y, i)).unwrap_or(u8::MAX));
                }
            }
        }
        for _ in 0..chroma {
            if ten {
                yuv.extend_from_slice(&512u16.to_le_bytes());
            } else {
                yuv.push(128);
            }
        }
        stream.extend(enc.encode(&yuv).map_err(Transient)?.concat());
    }
    stream.extend(enc.flush().map_err(Transient)?.concat());
    let mut dec = filmcraft_hevc::Decoder::new();
    let mut pics = dec.decode(&stream, 0).map_err(|e| Definite(format!("our decoder: {e}")))?;
    pics.extend(dec.flush());
    if pics.len() != frames {
        return Err(Definite(format!("{} of {frames} pictures decode", pics.len())));
    }
    for (i, p) in pics.iter().enumerate() {
        let mut se = 0f64;
        for y in 0..h {
            for x in 0..w {
                let at = y.saturating_mul(p.y_stride).saturating_add(x);
                let got = match &p.y {
                    filmcraft_hevc::Plane::U8(v) => v.get(at).map(|&s| f64::from(s)),
                    filmcraft_hevc::Plane::U16(v) => v.get(at).map(|&s| f64::from(s)),
                }
                .ok_or_else(|| Definite("a decoded picture smaller than the source".into()))?;
                se += (f64::from(luma(x, y, i)) - got).powi(2);
            }
        }
        let peak = if ten { 1023.0f64 } else { 255.0 };
        let psnr = 10.0 * (peak * peak / (se / (w * h) as f64).max(1e-9)).log10();
        // slice data coded with other syntax than its header says (mvd_l1_zero_flag 1 on iHD,
        // measured) decodes far below this; a working stream is well above at any export QP
        if psnr < 25.0 {
            return Err(Definite(format!("picture {i} decodes at {psnr:.1} dB luma PSNR")));
        }
    }
    Ok(())
}

fn profile(ten_bit: bool) -> sys::VAProfile {
    if ten_bit { sys::VAProfile_VAProfileHEVCMain10 } else { sys::VAProfile_VAProfileHEVCMain }
}

/// Planar 4:2:0 (`yuv420p10le` when `ten`, else `yuv420p`) of `w`×`h` into a P010 / NV12 image:
/// luma at `offsets[0]` with `pitches[0]`, interleaved chroma at `offsets[1]`. Rows and columns
/// beyond the frame (the coded size's padding, cropped by the conformance window) are left as
/// they are (the zero-copy path repeats the last ones: same picture, different padding).
/// Offsets and pitches come from the driver: anything outside `img` is an error.
fn fill_semiplanar(img: &mut [u8], offsets: [usize; 2], pitches: [usize; 2], yuv: &[u8], w: usize, h: usize, ten: bool) -> Result<(), String> {
    let bps = if ten { 2 } else { 1 };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let need = (w * h + 2 * cw * ch) * bps;
    if yuv.len() < need {
        return Err(format!("frame has {} bytes, {need} needed", yuv.len()));
    }
    let (ysrc, rest) = yuv.split_at(w * h * bps);
    let (usrc, vsrc) = rest.split_at(cw * ch * bps);
    let row_bytes = [w * bps, cw * 2 * bps];
    if pitches[0] < row_bytes[0] || pitches[1] < row_bytes[1] || offsets[0] > offsets[1] {
        return Err("image pitches smaller than the frame".into());
    }
    if offsets[1] > img.len() {
        return Err("image offset".into());
    }
    let (lo, hi) = img.split_at_mut(offsets[1]);
    let luma = lo.get_mut(offsets[0]..).ok_or("image offset")?;
    if luma.len() < pitches[0] * (h - 1) + row_bytes[0] || hi.len() < pitches[1] * (ch - 1) + row_bytes[1] {
        return Err("image smaller than the frame".into());
    }
    let ten_bit = |d: &mut [u8], s: &[u8]| {
        let v = u16::from_le_bytes([s[0], s[1]]) << 6;
        d.copy_from_slice(&v.to_le_bytes());
    };
    luma.par_chunks_mut(pitches[0]).zip(ysrc.par_chunks(row_bytes[0])).for_each(|(row, src)| {
        let row = &mut row[..row_bytes[0]];
        if ten {
            row.as_chunks_mut::<2>().0.iter_mut().zip(src.as_chunks::<2>().0).for_each(|(d, s)| ten_bit(d, s));
        } else {
            row.copy_from_slice(src);
        }
    });
    let csrc = usrc.par_chunks(cw * bps).zip(vsrc.par_chunks(cw * bps));
    hi.par_chunks_mut(pitches[1]).zip(csrc).for_each(|(row, (u, v))| {
        let pairs = row[..row_bytes[1]].chunks_exact_mut(2 * bps);
        for (d, (u, v)) in pairs.zip(u.chunks_exact(bps).zip(v.chunks_exact(bps))) {
            let (du, dv) = d.split_at_mut(bps);
            if ten {
                ten_bit(du, u);
                ten_bit(dv, v);
            } else {
                du.copy_from_slice(u);
                dv.copy_from_slice(v);
            }
        }
    });
    Ok(())
}

/// Planar `yuv422p10le` of `w`×`h` (`w` even) into a Y210 image (fork 6E.4): one packed plane at
/// `offset` with `pitch`, each pixel pair as Y0 U Y1 V, 16-bit words with the 10 bits at the top
/// (Intel's 4:2:2 encode surface). Anything outside `img` is an error.
fn fill_y210(img: &mut [u8], offset: usize, pitch: usize, yuv: &[u8], w: usize, h: usize) -> Result<(), String> {
    if !w.is_multiple_of(2) || w == 0 || h == 0 {
        return Err(format!("{w}x{h}"));
    }
    let cw = w / 2;
    let need = (w * h + 2 * cw * h) * 2;
    if yuv.len() < need {
        return Err(format!("frame has {} bytes, {need} needed", yuv.len()));
    }
    let row_bytes = w * 4;
    if pitch < row_bytes {
        return Err("image pitch smaller than the frame".into());
    }
    let plane = img.get_mut(offset..).ok_or("image offset")?;
    if plane.len() < pitch * (h - 1) + row_bytes {
        return Err("image smaller than the frame".into());
    }
    let (ysrc, rest) = yuv.split_at(w * h * 2);
    let (usrc, vsrc) = rest.split_at(cw * h * 2);
    let word = |s: &[u8]| (u16::from_le_bytes([s[0], s[1]]) << 6).to_le_bytes();
    let rows = ysrc.par_chunks(w * 2).zip(usrc.par_chunks(cw * 2).zip(vsrc.par_chunks(cw * 2)));
    plane.par_chunks_mut(pitch).zip(rows).for_each(|(row, (y, (u, v)))| {
        let quads = row[..row_bytes].as_chunks_mut::<8>().0;
        for (i, d) in quads.iter_mut().enumerate() {
            let (y0, y1) = (&y[i * 4..i * 4 + 2], &y[i * 4 + 2..i * 4 + 4]);
            d[0..2].copy_from_slice(&word(y0));
            d[2..4].copy_from_slice(&word(&u[i * 2..i * 2 + 2]));
            d[4..6].copy_from_slice(&word(y1));
            d[6..8].copy_from_slice(&word(&v[i * 2..i * 2 + 2]));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Makes [`VaHevcEncoder::new_on_with`]'s LP open fail on this thread (fork 6E.3 S12).
        pub(super) static FAIL_LP_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// The render nodes whose driver offers `EncSliceLP` for Main 10.
    fn lp_displays() -> Vec<Arc<Display>> {
        crate::ffmpeg::render_nodes()
            .iter()
            .filter_map(|n| Display::for_node(n).ok())
            .filter(|d| d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSliceLP))
            .collect()
    }

    fn lp_params(rc: RateControl) -> EncodeParams {
        EncodeParams {
            width: 256,
            height: 144,
            rate_num: 24,
            rate_den: 1,
            ten_bit: true,
            keyint: 30,
            rc,
            colour: (1, 1, 1),
            speed: Default::default(),
            chroma_422: false,
            low_power: true,
        }
    }

    /// Encodes 3 grey 10-bit frames and checks our decoder reads 3 pictures back.
    fn three_frames_decode(enc: &mut VaHevcEncoder) {
        let yuv: Vec<u8> = std::iter::repeat_n(400u16.to_le_bytes(), 256 * 144 * 3 / 2).flatten().collect();
        let mut stream = Vec::new();
        for _ in 0..3 {
            stream.extend(enc.encode(&yuv).expect("encode").concat());
        }
        stream.extend(enc.flush().expect("flush").concat());
        let mut dec = filmcraft_hevc::Decoder::new();
        let mut pics = dec.decode(&stream, 0).expect("decode");
        pics.extend(dec.flush());
        assert_eq!(pics.len(), 3);
    }

    /// Fork 6E.3 S12: VBR / CBR asked for with `low_power` on a device whose LP encoder lacks
    /// those modes (iHD 26.3: CQP only) opens `EncSlice`; where LP has them, LP. Skips without LP.
    #[test]
    fn low_power_without_the_rate_control_mode_opens_encslice() {
        let displays = lp_displays();
        if displays.is_empty() {
            eprintln!("SKIPPED: no VA-API HEVC Main 10 EncSliceLP");
        }
        for d in displays {
            for rc in [RateControl::Vbr { kbps: 4_000, max_kbps: 6_000 }, RateControl::Cbr { kbps: 4_000 }] {
                let lp_has = d
                    .config_attribs(profile(true), sys::VAEntrypoint_VAEntrypointEncSliceLP, &[sys::VAConfigAttribType_VAConfigAttribRateControl])
                    .ok()
                    .and_then(|a| a.first().copied().flatten())
                    .is_some_and(|v| v & rc_mode(rc) != 0);
                let mut enc = VaHevcEncoder::new_on(lp_params(rc), d.clone()).expect("encoder");
                if !lp_has {
                    assert!(!enc.low_power(), "{} {rc:?}", d.vendor);
                }
                eprintln!("{} {rc:?}: low power {}", d.vendor, enc.low_power());
                three_frames_decode(&mut enc);
            }
        }
    }

    /// Fork 6E.3 S12: an LP encoder that fails to open (injected) gives an `EncSlice` encoder,
    /// never an error; the shared stream of the EncSlice side is still accepted. Skips without LP.
    #[test]
    fn low_power_open_failure_falls_back_to_encslice() {
        let displays = lp_displays();
        if displays.is_empty() {
            eprintln!("SKIPPED: no VA-API HEVC Main 10 EncSliceLP");
        }
        for d in displays {
            let params = lp_params(RateControl::Cqp(24));
            assert!(VaHevcEncoder::new_on(params.clone(), d.clone()).expect("encoder").low_power(), "{}: LP usable", d.vendor);
            let stream = VaHevcEncoder::stream_params_for(&EncodeParams { low_power: false, ..params.clone() }, &d).expect("stream");
            FAIL_LP_OPEN.with(|f| f.set(true));
            let enc = VaHevcEncoder::new_on_with(params.clone(), d.clone(), Some(stream.clone()));
            FAIL_LP_OPEN.with(|f| f.set(false));
            let mut enc = enc.expect("EncSlice fallback");
            assert!(!enc.low_power(), "{}", d.vendor);
            assert_eq!(enc.stream(), &stream);
            three_frames_decode(&mut enc);
        }
    }

    #[test]
    fn y210_fill_packs_and_shifts() {
        // 4x2: Y 0..8, U 100,101 / 102,103, V 200,201 / 202,203 (per row: 4:2:2)
        let mut yuv = Vec::new();
        for v in (0..8u16).chain(100..104).chain(200..204) {
            yuv.extend_from_slice(&v.to_le_bytes());
        }
        let mut img = vec![0u8; 4 + 2 * 20];
        fill_y210(&mut img, 4, 20, &yuv, 4, 2).expect("fill");
        let at = |o: usize| u16::from_le_bytes([img[o], img[o + 1]]) >> 6;
        let row = |r: usize| (0..8).map(|i| at(4 + r * 20 + i * 2)).collect::<Vec<_>>();
        assert_eq!(row(0), vec![0, 100, 1, 200, 2, 101, 3, 201]);
        assert_eq!(row(1), vec![4, 102, 5, 202, 6, 103, 7, 203]);
        // too small an image, frame or pitch, an odd width: errors, not panics
        assert!(fill_y210(&mut img[..39], 4, 20, &yuv, 4, 2).is_err());
        assert!(fill_y210(&mut img, 4, 20, &yuv[..10], 4, 2).is_err());
        assert!(fill_y210(&mut img, 4, 8, &yuv, 4, 2).is_err());
        assert!(fill_y210(&mut img, 4, 20, &yuv, 3, 2).is_err());
        assert!(fill_y210(&mut img, 999, 20, &yuv, 4, 2).is_err());
    }

    #[test]
    fn semiplanar_fill_interleaves_and_shifts() {
        // 4x2 10-bit: Y 0..8, U 100,101, V 200,201
        let mut yuv = Vec::new();
        for v in (0u16..8).chain([100, 101, 200, 201]) {
            yuv.extend_from_slice(&v.to_le_bytes());
        }
        let mut img = vec![0u8; 64];
        fill_semiplanar(&mut img, [0, 32], [16, 16], &yuv, 4, 2, true).expect("fill");
        let at = |o: usize| u16::from_le_bytes([img[o], img[o + 1]]);
        assert_eq!((at(0), at(6), at(16), at(22)), (0, 3 << 6, 4 << 6, 7 << 6));
        assert_eq!((at(32), at(34), at(36), at(38)), (100 << 6, 200 << 6, 101 << 6, 201 << 6));
        // too small an image or frame is an error, not a panic
        assert!(fill_semiplanar(&mut img[..38], [0, 32], [16, 16], &yuv, 4, 2, true).is_err());
        assert!(fill_semiplanar(&mut img, [0, 32], [16, 16], &yuv[..10], 4, 2, true).is_err());
        assert!(fill_semiplanar(&mut img, [0, 32], [4, 16], &yuv, 4, 2, true).is_err());
    }

    /// Encode a few frames on the GPU and decode them with FilmCraft's own decoder. Skipped
    /// without a VA-API HEVC encoder.
    #[test]
    fn encodes_a_stream_our_decoder_reads() {
        if !VaHevcEncoder::supported(true) {
            eprintln!("no VA-API HEVC encoder; skipped");
            return;
        }
        // every speed / quality level the driver offers gives a stream our decoder reads (6E.2)
        let range = VaHevcEncoder::quality_range(true);
        eprintln!("quality range {range:?}");
        for speed in filmcraft_export::HwEncodeSpeed::ALL {
            encode_and_decode(speed, range);
        }
    }

    /// Main 4:2:2 10 (fork 6E.4): a vertical chroma gradient (only 4:2:2 keeps it per row) comes
    /// back through FilmCraft's own 4:2:2 decoder, on whichever device encodes 4:2:2 (Intel's here).
    /// Skipped where none does.
    #[test]
    fn encodes_422_our_decoder_reads() {
        let (w, h) = (320u32, 184u32);
        let params = EncodeParams {
            width: w,
            height: h,
            rate_num: 30,
            rate_den: 1,
            ten_bit: true,
            keyint: 4,
            rc: RateControl::Cqp(18),
            colour: (1, 1, 1),
            speed: Default::default(),
            chroma_422: true,
            low_power: false,
        };
        if !VaHevcEncoder::supported_422() {
            // refused cleanly, not a crash
            assert!(VaHevcEncoder::new(params).is_err());
            eprintln!("no VA-API HEVC 4:2:2 encoder; skipped");
            return;
        }
        // 8-bit 4:2:2 is refused
        assert!(VaHevcEncoder::new(EncodeParams { ten_bit: false, ..params.clone() }).is_err());
        let mut enc = VaHevcEncoder::new(params).expect("encoder");
        let cw = w / 2;
        let chroma = |y: u32| 200 + y * 3; // per row: lost if the chroma were 4:2:0
        let mut stream = Vec::new();
        let frames = 3;
        for _ in 0..frames {
            let mut yuv = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    yuv.extend_from_slice(&(300 + ((x + 2 * y) % 200) as u16).to_le_bytes());
                }
            }
            for plane in 0..2u32 {
                for y in 0..h {
                    for _ in 0..cw {
                        yuv.extend_from_slice(&((chroma(y) + plane * 50) as u16).to_le_bytes());
                    }
                }
            }
            for au in enc.encode(&yuv).expect("encode") {
                stream.extend(au);
            }
        }
        for au in enc.flush().expect("flush") {
            stream.extend(au);
        }
        let mut dec = filmcraft_hevc::Decoder::new();
        let mut pics = dec.decode(&stream, 0).expect("decode");
        pics.extend(dec.flush());
        assert_eq!(pics.len(), frames);
        for p in &pics {
            assert_eq!(p.chroma_format, filmcraft_hevc::ChromaFormat::Yuv422);
            assert_eq!((p.width, p.height, p.chroma_width, p.chroma_height, p.bit_depth), (w, h, cw, h, 10));
            // chroma rows follow the gradient within a few codes (QP 18)
            let mut worst = 0i64;
            for y in 0..h {
                for x in [0, cw / 2, cw - 1] {
                    let at = y as usize * p.uv_stride + x as usize;
                    worst = worst.max((p.u.get(at) as i64 - chroma(y) as i64).abs());
                    worst = worst.max((p.v.get(at) as i64 - (chroma(y) + 50) as i64).abs());
                }
            }
            assert!(worst <= 8, "chroma off by {worst}");
        }
    }

    /// One encoder per GPU in one process (fork 6E.3): every render node with an HEVC Main 10
    /// encoder encodes a few pictures our decoder reads; a missing node is an error.
    #[test]
    fn an_encoder_on_each_gpu() {
        assert!(Display::for_node("/dev/dri/no-such-node").is_err());
        let mut encoded = Vec::new();
        for node in crate::ffmpeg::render_nodes() {
            let Ok(d) = Display::for_node(&node) else { continue };
            if !d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSlice) {
                continue;
            }
            let (w, h) = (256u32, 144u32);
            let params = EncodeParams {
                width: w,
                height: h,
                rate_num: 24,
                rate_den: 1,
                ten_bit: true,
                keyint: 2,
                rc: RateControl::Cqp(22),
                colour: (1, 1, 1),
                speed: Default::default(),
                chroma_422: false,
                low_power: false,
            };
            let mut enc = VaHevcEncoder::new_on(params, d.clone()).expect("encoder");
            let yuv: Vec<u8> = std::iter::repeat_n(400u16.to_le_bytes(), (w * h * 3 / 2) as usize).flatten().collect();
            let mut stream = Vec::new();
            for _ in 0..3 {
                stream.extend(enc.encode(&yuv).expect("encode").concat());
            }
            stream.extend(enc.flush().expect("flush").concat());
            let mut dec = filmcraft_hevc::Decoder::new();
            let mut pics = dec.decode(&stream, 0).expect("decode");
            pics.extend(dec.flush());
            assert_eq!(pics.len(), 3, "{node}");
            // the same node twice gives the same display
            assert!(Arc::ptr_eq(&d, &Display::for_node(&node).expect("again")));
            encoded.push(format!("{} ({})", node, d.vendor));
        }
        eprintln!("encoded on {encoded:?}");
    }

    /// Fork 6E.3 S8: AMD's Mesa driver is recognised as radeonsi (it rewrites the SPS / PPS), so
    /// its stream is marked `rewritten`; a vendor string that stops saying "radeonsi" fails here
    /// instead of silently sharing one `hvc1` entry with another GPU. Skips without an AMD encoder.
    #[test]
    fn amd_mesa_is_recognised_as_radeonsi() {
        let params = EncodeParams {
            width: 256,
            height: 144,
            rate_num: 24,
            rate_den: 1,
            ten_bit: true,
            keyint: 30,
            rc: RateControl::Cqp(22),
            colour: (1, 1, 1),
            speed: Default::default(),
            chroma_422: false,
            low_power: false,
        };
        let mut seen = false;
        for d in crate::ffmpeg::render_nodes().iter().filter_map(|n| Display::for_node(n).ok()) {
            let amd = d.vendor.contains("Mesa") && d.vendor.contains("AMD");
            if !amd || !d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSlice) {
                continue;
            }
            seen = true;
            assert!(d.vendor.contains("radeonsi"), "AMD Mesa vendor string changed: {}", d.vendor);
            let s = VaHevcEncoder::stream_params_for(&params, &d).expect("stream");
            assert!(s.rewritten && s.dependent_slices && s.init_qp == 26, "{}", d.vendor);
        }
        if !seen {
            eprintln!("SKIPPED: no AMD VA-API HEVC encoder");
        }
    }

    /// Two GPUs on one stream description (fork 6E.3): every encoder accepts the common stream of
    /// all devices and hands its driver byte-equal packed headers, and chunks from each device
    /// (each starting a new GOP, with its own in-band parameter sets) joined in order decode as
    /// one stream. A stream a driver cannot write is refused.
    #[test]
    fn one_shared_stream_on_every_gpu() {
        let (w, h) = (256u32, 144u32);
        let params = EncodeParams {
            width: w,
            height: h,
            rate_num: 24,
            rate_den: 1,
            ten_bit: true,
            keyint: 30,
            rc: RateControl::Cqp(22),
            colour: (1, 1, 1),
            speed: Default::default(),
            chroma_422: false,
            low_power: false,
        };
        let displays: Vec<Arc<Display>> = crate::ffmpeg::render_nodes()
            .iter()
            .filter_map(|n| Display::for_node(n).ok())
            .filter(|d| d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSlice))
            .collect();
        let Some(first) = displays.first() else { return };
        let mut common = VaHevcEncoder::stream_params_for(&params, first).expect("stream");
        // the GPUs whose drivers can write one stream with the first (radeonsi + iHD can't:
        // radeonsi writes transform depth 4, iHD codes 2 at most; such lanes use hev1, S8)
        let mut sharing = vec![first.clone()];
        for d in displays.get(1..).unwrap_or(&[]) {
            let s = VaHevcEncoder::stream_params_for(&params, d).expect("stream");
            match hw::common_stream(&common, &s) {
                Some(c) => {
                    common = c;
                    sharing.push(d.clone());
                }
                None => eprintln!("{} cannot share a stream with {}: hev1", d.vendor, first.vendor),
            }
        }
        let displays = sharing;
        // a different picture is refused
        let other = StreamParams { width: w + 16, ..common.clone() };
        assert!(VaHevcEncoder::new_on_with(params.clone(), first.clone(), Some(other)).is_err());
        // textured luma (a ramp plus fine detail) so the transform trees split: a driver that
        // ignored the shared transform depths would decode as garbage (fork 6E.3 S8)
        let luma = |x: u32, y: u32| 64 + ((x + y) % 256) as u16 * 3 + ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) % 64) as u16;
        let mut yuv = Vec::new();
        for y in 0..h {
            for x in 0..w {
                yuv.extend_from_slice(&luma(x, y).to_le_bytes());
            }
        }
        for c in [400u16, 600] {
            for _ in 0..(w / 2) * (h / 2) {
                yuv.extend_from_slice(&c.to_le_bytes());
            }
        }
        let mut joined = Vec::new();
        let mut headers = None;
        for d in &displays {
            let mut enc = VaHevcEncoder::new_on_with(params.clone(), d.clone(), Some(common.clone())).expect("encoder");
            let hd = enc.headers();
            assert_eq!(headers.get_or_insert_with(|| hd.clone()), &hd, "{}", d.vendor);
            enc.restart_gop();
            let start = joined.len();
            for _ in 0..3 {
                joined.extend(enc.encode(&yuv).expect("encode").concat());
            }
            joined.extend(enc.flush().expect("flush").concat());
            // what the driver actually wrote: iHD writes our packed headers verbatim; radeonsi
            // rewrites the SPS / PPS from its own parameters, which the shared stream now carries
            // (fork 6E.3 S8), so every chunk starts with the same bytes: one `hvcC`
            let verbatim = joined.get(start..).is_some_and(|c| c.starts_with(&hd));
            assert!(verbatim, "{}: the driver's headers differ from the shared stream", d.vendor);
        }
        let mut dec = filmcraft_hevc::Decoder::new();
        let mut pics = dec.decode(&joined, 0).expect("decode");
        pics.extend(dec.flush());
        assert_eq!(pics.len(), 3 * displays.len());
        for (i, p) in pics.iter().enumerate() {
            let mut se = 0f64;
            for y in 0..h {
                for x in 0..w {
                    let got = p.y.get(y as usize * p.y_stride + x as usize) as f64;
                    se += (f64::from(luma(x, y)) - got).powi(2);
                }
            }
            let psnr = 10.0 * (1023.0f64.powi(2) / (se / f64::from(w * h)).max(1e-9)).log10();
            assert!(psnr > 34.0, "picture {i}: luma PSNR {psnr:.1} dB");
            eprintln!("picture {i}: luma PSNR {psnr:.1} dB");
        }
        eprintln!("shared stream on {:?}", displays.iter().map(|d| d.vendor.clone()).collect::<Vec<_>>());
    }

    /// Fork 6E.3 S12: on every device whose driver offers `EncSliceLP` for Main 10 (Intel's iHD),
    /// 60 pictures of a moving 10-bit 4:2:0 picture go through the low-power encoder as IDR +
    /// low-delay B, and FilmCraft's own decoder reads every one back close to the source. The
    /// slices are B with both lists one picture back. Skipped where no driver offers LP.
    #[test]
    fn low_power_low_delay_b_our_decoder_reads() {
        use filmcraft_hevc::params::{Pps, Sps};
        use filmcraft_hevc::slice::{NalHeader, SliceHeader};
        let (w, h) = (320u32, 184u32);
        let params = EncodeParams {
            width: w,
            height: h,
            rate_num: 30,
            rate_den: 1,
            ten_bit: true,
            keyint: 30,
            rc: RateControl::Cqp(20),
            colour: (1, 1, 1),
            speed: Default::default(),
            chroma_422: false,
            low_power: true,
        };
        let luma = |x: u32, y: u32, i: u32| 64 + ((x + y + i * 5) % 256) as u16 * 3 + ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) % 32) as u16;
        let mut seen = false;
        for node in crate::ffmpeg::render_nodes() {
            let Ok(d) = Display::for_node(&node) else { continue };
            if !d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSliceLP) {
                continue;
            }
            seen = true;
            let mut enc = VaHevcEncoder::new_on(params.clone(), d.clone()).expect("encoder (LP or its EncSlice fallback)");
            assert!(enc.low_power(), "{} ({}) offers EncSliceLP but the LP encoder was not used (see the log above)", node, d.vendor);
            let frames = 60u32;
            let mut stream = Vec::new();
            for i in 0..frames {
                let mut yuv = Vec::new();
                for y in 0..h {
                    for x in 0..w {
                        yuv.extend_from_slice(&luma(x, y, i).to_le_bytes());
                    }
                }
                for c in [400u16, 600] {
                    for _ in 0..(w / 2) * (h / 2) {
                        yuv.extend_from_slice(&c.to_le_bytes());
                    }
                }
                stream.extend(enc.encode(&yuv).expect("encode").concat());
            }
            stream.extend(enc.flush().expect("flush").concat());
            // the slice headers: I at each IDR, else B with one picture back in both lists
            let nals = crate::ffmpeg::annexb_nals(&stream);
            let unit = |t: u8| {
                nals.iter().find(|n| n.first().is_some_and(|b| (b >> 1) & 0x3f == t)).map(|n| filmcraft_bitstream::unescape_rbsp(n.get(2..).unwrap_or(&[])))
            };
            let sps = Arc::new(Sps::parse(&unit(33).expect("sps")).expect("sps"));
            let pps = Arc::new(Pps::parse(&unit(34).expect("pps")).expect("pps"));
            let mut kinds = String::new();
            for n in nals.iter().filter(|n| n.first().is_some_and(|b| (b >> 1) & 0x3f < 32)) {
                let header = NalHeader::parse(n).expect("nal header");
                let r = filmcraft_bitstream::unescape_rbsp(n.get(2..).unwrap_or(&[]));
                let (sh, _, _) = SliceHeader::parse(&r, header, |_| Ok((pps.clone(), sps.clone())), None).expect("slice header");
                if header.is_idr() {
                    kinds.push('I');
                } else {
                    assert!(sh.is_b(), "a non-IDR LP picture is B");
                    assert_eq!((sh.st_rps.s0.clone(), sh.st_rps.s1.len(), sh.num_ref_idx), (vec![(-1, true)], 0, [1, 1]));
                    kinds.push('B');
                }
            }
            assert_eq!(kinds, format!("I{}I{}", "B".repeat(29), "B".repeat(29)));
            let mut dec = filmcraft_hevc::Decoder::new();
            let mut pics = dec.decode(&stream, 0).expect("decode");
            pics.extend(dec.flush());
            assert_eq!(pics.len(), frames as usize, "{node}: every picture decodes");
            let mut worst = f64::MAX;
            for (i, p) in pics.iter().enumerate() {
                assert_eq!((p.width, p.height, p.bit_depth), (w, h, 10));
                let mut se = 0f64;
                for y in 0..h {
                    for x in 0..w {
                        let got = p.y.get(y as usize * p.y_stride + x as usize) as f64;
                        se += (f64::from(luma(x, y, i as u32)) - got).powi(2);
                    }
                }
                let psnr = 10.0 * (1023.0f64.powi(2) / (se / f64::from(w * h)).max(1e-9)).log10();
                assert!(psnr > 38.0, "{node} picture {i}: luma PSNR {psnr:.1} dB");
                worst = worst.min(psnr);
            }
            eprintln!("{node} ({}): LP low-delay B, {} bytes, worst luma PSNR {worst:.1} dB", d.vendor, stream.len());
            // asked for LP where there is none: EncSlice as before
            if let Some(amd) = crate::ffmpeg::render_nodes().iter().filter_map(|n| Display::for_node(n).ok()).find(|d| {
                d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSlice) && !d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSliceLP)
            }) {
                let enc = VaHevcEncoder::new_on(params.clone(), amd.clone()).expect("encoder");
                assert!(!enc.low_power(), "{}", amd.vendor);
            }
        }
        if !seen {
            eprintln!("SKIPPED: no VA-API HEVC Main 10 EncSliceLP");
        }
    }

    fn encode_and_decode(speed: filmcraft_export::HwEncodeSpeed, range: Option<u32>) {
        let (w, h) = (320u32, 184u32);
        let params = EncodeParams {
            width: w,
            height: h,
            rate_num: 30,
            rate_den: 1,
            ten_bit: true,
            keyint: 4,
            rc: RateControl::Cqp(20),
            colour: (1, 1, 1),
            speed,
            chroma_422: false,
            low_power: false,
        };
        let mut enc = VaHevcEncoder::new(params).expect("encoder");
        assert_eq!(enc.quality_level(), speed.level(range.unwrap_or(0)), "{speed:?}");
        let mut stream = Vec::new();
        let frames = 6;
        for i in 0..frames {
            let mut yuv = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let v = 64 + ((x + y + i * 7) % 256) as u16 * 3;
                    yuv.extend_from_slice(&v.to_le_bytes());
                }
            }
            for c in [400u16, 600] {
                for _ in 0..(w / 2) * (h / 2) {
                    yuv.extend_from_slice(&c.to_le_bytes());
                }
            }
            for au in enc.encode(&yuv).expect("encode") {
                stream.extend(au);
            }
        }
        for au in enc.flush().expect("flush") {
            stream.extend(au);
        }
        // FilmCraft's software decoder reads it back close to the source (QP 20)
        let mut dec = filmcraft_hevc::Decoder::new();
        let mut pics = dec.decode(&stream, 0).expect("decode");
        pics.extend(dec.flush());
        assert_eq!(pics.len(), frames as usize);
        for (i, p) in pics.iter().enumerate() {
            assert_eq!((p.width, p.height, p.bit_depth), (w, h, 10));
            let mut se = 0f64;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let want = 64.0 + ((x + y + i * 7) % 256) as f64 * 3.0;
                    let got = p.y.get(y * p.y_stride + x) as f64;
                    se += (want - got).powi(2);
                }
            }
            let mse = se / (w * h) as f64;
            let psnr = 10.0 * (1023.0f64.powi(2) / mse.max(1e-9)).log10();
            assert!(psnr > 40.0, "picture {i}: luma PSNR {psnr:.1} dB");
        }
        let nals = crate::ffmpeg::annexb_nals(&stream);
        let vcl: Vec<u8> = nals.iter().map(|n| (n[0] >> 1) & 0x3f).filter(|t| *t < 32).collect();
        assert_eq!(vcl, vec![19, 1, 1, 1, 19, 1], "IDR every 4 pictures, P between");
        assert!(nals.iter().any(|n| (n[0] >> 1) & 0x3f == 33), "parameter sets in the stream");
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Timing of the encoder at 4K 10-bit: upload (conversion + vaPutImage) and the encoder's
    /// throughput. `cargo test --release -p filmcraft-platform --lib va_enc_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn va_enc_speed() {
        if !VaHevcEncoder::supported(true) {
            return;
        }
        // FILMCRAFT_ENC_SPEED_6K=1: 6240x4160
        let (w, h) = if std::env::var_os("FILMCRAFT_ENC_SPEED_6K").is_some() { (6240u32, 4160u32) } else { (3840u32, 2160u32) };
        let params = EncodeParams {
            width: w,
            height: h,
            rate_num: 30,
            rate_den: 1,
            ten_bit: true,
            keyint: 30,
            rc: RateControl::Vbr { kbps: 80_000, max_kbps: 100_000 },
            colour: (1, 1, 1),
            // FILMCRAFT_ENC_SPEED=speed|balanced|quality (fork 6E.1 / 6E.2)
            speed: std::env::var("FILMCRAFT_ENC_SPEED").ok().and_then(|v| filmcraft_export::HwEncodeSpeed::from_name(&v)).unwrap_or_default(),
            chroma_422: false,
            low_power: false,
        };
        let mut enc = VaHevcEncoder::new(params).expect("encoder");
        eprintln!("quality range {:?}, level {:?}", VaHevcEncoder::quality_range(true), enc.quality_level());
        let n = (w * h * 3 / 2) as usize;
        let yuv: Vec<u8> = (0..n)
            .flat_map(|i| {
                let x = (i as u64).wrapping_mul(0x9E3779B97F4A7C15) >> 54;
                (64 + (i * 7 % 700) as u16 + x as u16 / 8).to_le_bytes()
            })
            .collect();
        let src = enc.sources.get().expect("surface");
        let t = std::time::Instant::now();
        for _ in 0..30 {
            src.write(|img, o, p| fill_semiplanar(img, o, p, &yuv, w as usize, h as usize, true)).expect("write");
        }
        eprintln!("upload: {:.1} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / 30.0);
        let t = std::time::Instant::now();
        let mut fill = 0.0;
        for _ in 0..30 {
            let mut img = vec![0u8; w as usize * h as usize * 3];
            let t2 = std::time::Instant::now();
            fill_semiplanar(&mut img, [0, (w * h * 2) as usize], [(w * 2) as usize, (w * 2) as usize], &yuv, w as usize, h as usize, true).expect("fill");
            fill += t2.elapsed().as_secs_f64();
        }
        let _ = t;
        eprintln!("conversion only: {:.1} ms/frame", fill * 1000.0 / 30.0);
        drop(src);
        let t = std::time::Instant::now();
        let frames: Vec<Vec<u8>> = (0..3u64)
            .map(|k| {
                (0..n)
                    .flat_map(|i| {
                        let x = (i as u64 ^ (k << 40)).wrapping_mul(0x9E3779B97F4A7C15) >> 54;
                        (64 + (i * 7 % 700) as u16 + x as u16 / 8).to_le_bytes()
                    })
                    .collect()
            })
            .collect();
        for i in 0..60 {
            enc.encode(&frames[i % 3]).expect("encode");
        }
        enc.flush().expect("flush");
        eprintln!("encode incl. upload: {:.1} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / 60.0);
    }

    /// Fork 6E.3 S12: `EncSliceLP` (low-delay B) against `EncSlice` (P) on each device that offers
    /// LP for Main 10, same frames, same settings: encode incl. upload, frames per second.
    /// `FILMCRAFT_ENC_SPEED_SIZE=WxH` (default 3840x2160).
    /// `cargo test --release -p filmcraft-platform --lib va_enc_lp_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn va_enc_lp_speed() {
        let (w, h) = std::env::var("FILMCRAFT_ENC_SPEED_SIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or((3840u32, 2160u32));
        let n = (w * h * 3 / 2) as usize;
        let frames: Vec<Vec<u8>> = (0..3u64)
            .map(|k| {
                (0..n)
                    .flat_map(|i| {
                        let x = (i as u64 ^ (k << 40)).wrapping_mul(0x9E3779B97F4A7C15) >> 54;
                        (64 + (i * 7 % 700) as u16 + x as u16 / 8).to_le_bytes()
                    })
                    .collect()
            })
            .collect();
        for node in crate::ffmpeg::render_nodes() {
            let Ok(d) = Display::for_node(&node) else { continue };
            if !d.supports(profile(true), sys::VAEntrypoint_VAEntrypointEncSliceLP) {
                continue;
            }
            for low_power in [false, true] {
                let params = EncodeParams {
                    width: w,
                    height: h,
                    rate_num: 30,
                    rate_den: 1,
                    ten_bit: true,
                    keyint: 30,
                    // constant QP: iHD 26.3's LP encoder offers no other rate control here (no
                    // HuC bit-rate control), so VBR / CBR stay on EncSlice
                    rc: RateControl::Cqp(24),
                    colour: (1, 1, 1),
                    speed: Default::default(),
                    chroma_422: false,
                    low_power,
                };
                let mut enc = VaHevcEncoder::new_on(params, d.clone()).expect("encoder");
                // warm-up (the first IDR, surface allocation)
                enc.encode(&frames[0]).expect("encode");
                let t = std::time::Instant::now();
                let count = 90;
                let mut bytes = 0;
                for i in 0..count {
                    bytes += enc.encode(&frames[i % 3]).expect("encode").iter().map(Vec::len).sum::<usize>();
                }
                bytes += enc.flush().expect("flush").iter().map(Vec::len).sum::<usize>();
                let s = t.elapsed().as_secs_f64();
                eprintln!(
                    "{node} ({}) {w}x{h} {}: {:.1} fps ({:.1} ms/frame incl. upload), {} kB",
                    d.vendor,
                    if enc.low_power() { "EncSliceLP (low-delay B)" } else { "EncSlice (P)" },
                    count as f64 / s,
                    s * 1000.0 / count as f64,
                    bytes / 1000
                );
            }
        }
    }
}
