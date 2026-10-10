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

use crate::va::{CodedBuffer, Config, Context, Display, MiscFrameRate, MiscHrd, MiscRateControl, Surface, SurfacePool};
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
}

/// Where the export may render this encoder's input (fork Phase 5.2c): surfaces of `pool`
/// (the coded size, P010 when `ten_bit`, else NV12), for a picture of `size`.
pub struct EncodeTarget {
    pub pool: Arc<SurfacePool>,
    pub ten_bit: bool,
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

/// Two-bit fields of the driver's HEVC feature / block-size attributes.
fn field(v: u32, shift: u32) -> u32 {
    (v >> shift) & 3
}

impl VaHevcEncoder {
    /// Whether the driver encodes HEVC (`ten_bit`: Main 10).
    pub fn supported(ten_bit: bool) -> bool {
        Display::for_encode().is_ok_and(|d| d.supports(profile(ten_bit), sys::VAEntrypoint_VAEntrypointEncSlice))
    }

    pub fn new(params: EncodeParams) -> Result<Self, String> {
        let EncodeParams { width: w, height: h, ten_bit, .. } = params;
        if w == 0 || h == 0 || w > 8192 || h > 8192 || w % 2 != 0 || h % 2 != 0 {
            return Err(format!("{w}x{h}"));
        }
        if params.rate_num == 0 || params.rate_den == 0 {
            return Err("frame rate".into());
        }
        let d = Display::for_encode()?;
        let (prof, ep) = (profile(ten_bit), sys::VAEntrypoint_VAEntrypointEncSlice);
        if !d.supports(prof, ep) {
            return Err(format!("the driver ({}) has no HEVC {} encoder", d.vendor, if ten_bit { "Main 10" } else { "Main" }));
        }
        use sys::{
            VAConfigAttribType_VAConfigAttribEncHEVCBlockSizes as BLOCKS, VAConfigAttribType_VAConfigAttribEncHEVCFeatures as FEATURES,
            VAConfigAttribType_VAConfigAttribEncPackedHeaders as PACKED, VAConfigAttribType_VAConfigAttribMaxPictureHeight as MAX_H,
            VAConfigAttribType_VAConfigAttribMaxPictureWidth as MAX_W, VAConfigAttribType_VAConfigAttribRTFormat as RT,
            VAConfigAttribType_VAConfigAttribRateControl as RC,
        };
        let a = d.config_attribs(prof, ep, &[RT, RC, PACKED, FEATURES, BLOCKS, MAX_W, MAX_H])?;
        let get = |i: usize| a.get(i).copied().flatten();
        let rt = if ten_bit { sys::VA_RT_FORMAT_YUV420_10 } else { sys::VA_RT_FORMAT_YUV420 };
        if get(0).is_some_and(|v| v & rt == 0) {
            return Err("the encoder does not take this picture format".into());
        }
        let rc_mode = match params.rc {
            RateControl::Cqp(_) => sys::VA_RC_CQP,
            RateControl::Cbr { .. } => sys::VA_RC_CBR,
            RateControl::Vbr { .. } => sys::VA_RC_VBR,
        };
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
        stream.colour = params.colour;
        stream.timing = Some((params.rate_den, params.rate_num));

        let config = Arc::new(Config::with_attribs(&d, prof, ep, &[(RT, rt), (RC, rc_mode), (PACKED, packed)])?);
        let ctx = Context::new(&config, stream.coded_width, stream.coded_height)?;
        let fourcc = if ten_bit { sys::VA_FOURCC_P010 } else { sys::VA_FOURCC_NV12 };
        let sources = SurfacePool::new(&d, rt, fourcc, stream.coded_width, stream.coded_height);
        let recons = SurfacePool::new(&d, rt, fourcc, stream.coded_width, stream.coded_height);
        // a picture rarely compresses worse than 1 byte per pixel; overflow is reported
        let coded_bytes = (stream.coded_width as usize * stream.coded_height as usize).saturating_add(1 << 20);
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
        })
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
    /// size. Returns the access units (Annex B) of pictures that finished.
    pub fn encode(&mut self, yuv: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let source = self.sources.get()?;
        let (w, h) = (self.params.width as usize, self.params.height as usize);
        let ten = self.params.ten_bit;
        source.write(|img, offsets, pitches| fill_semiplanar(img, offsets, pitches, yuv, w, h, ten))?;
        self.submit(Source::Own(source))?;
        self.finished()
    }

    /// Where frames may be rendered for [`encode_surface`](Self::encode_surface).
    pub fn target(&self) -> EncodeTarget {
        EncodeTarget { pool: self.sources.clone(), ten_bit: self.params.ten_bit, size: (self.params.width, self.params.height) }
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
        let (qp, qp_delta) = match self.params.rc {
            RateControl::Cqp(q) => (q.clamp(1, 51), 0),
            _ => (26, 0),
        };
        let slice_pic = SlicePic { idr, poc: self.gop_pos, qp_delta };

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
        f.set_coding_type(if idr { 1 } else { 2 });
        f.set_reference_pic_flag(1);
        f.set_transform_skip_enabled_flag(s.transform_skip as u32);
        f.set_cu_qp_delta_enabled_flag(s.cu_qp_delta as u32);
        f.set_pps_loop_filter_across_slices_enabled_flag(1);
        pic.pic_fields.bits = f;

        let mut slice = sys::VAEncSliceParameterBufferHEVC {
            num_ctu_in_slice: s.ctbs(),
            slice_type: if idr { 2 } else { 1 },
            max_num_merge_cand: 5,
            slice_qp_delta: qp_delta as i8,
            ..Default::default()
        };
        let mut sf = sys::_VAEncSliceParameterBufferHEVC__bindgen_ty_1__bindgen_ty_1::default();
        sf.set_last_slice_of_pic_flag(1);
        sf.set_slice_sao_luma_flag(s.sao as u32);
        sf.set_slice_sao_chroma_flag(s.sao as u32);
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
            general_profile_idc: if p.ten_bit { 2 } else { 1 },
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
        f.set_chroma_format_idc(1);
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
        let (kbps, max_kbps) = match p.rc {
            RateControl::Cqp(_) => return Ok(Vec::new()),
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
        Ok(vec![self.ctx.param_buffer(&MiscRateControl::new(rc))?, self.ctx.param_buffer(&MiscHrd::new(hrd))?, self.ctx.param_buffer(&MiscFrameRate::new(fr))?])
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let (w, h) = (320u32, 184u32);
        let params = EncodeParams { width: w, height: h, rate_num: 30, rate_den: 1, ten_bit: true, keyint: 4, rc: RateControl::Cqp(20), colour: (1, 1, 1) };
        let mut enc = VaHevcEncoder::new(params).expect("encoder");
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
        };
        let mut enc = VaHevcEncoder::new(params).expect("encoder");
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
}
