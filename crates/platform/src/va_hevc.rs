//! HEVC decoding with libva in process (docs/adr/0002-linux-video-interop.md).
//!
//! [`VaHevc`] is the [`Accelerator`] behind `filmcraft_hevc::accel::AccelDecoder`: FilmCraft's own
//! parser and DPB decide what each picture references and when it is output, and this module
//! translates each picture into VA-API's HEVC buffers (picture parameters, scaling lists, one
//! slice-parameter and one slice-data buffer per slice segment) and decodes it into a pooled
//! surface. [`NativeVaDecoder`] is the `VideoDecoder` the media stack uses; for now it reads the
//! pictures back to memory (P010 → `PixelData::Yuv16Bi`, NV12 → planar `Yuv8`), bit-exact with
//! FilmCraft's software decoder. GPU-resident output follows (Phase 2.2b).

// The VA parameter structs are large C structs filled field by field from a zeroed default, in the
// order of va_dec_hevc.h.
#![allow(clippy::field_reassign_with_default)]

use std::sync::Arc;

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, VideoDecoder};
use filmcraft_frame::{Chroma, PixelData, VideoFrame, pool};
use filmcraft_hevc::accel::{AccelDecoder, AccelOutput, AccelPicture, Accelerator, RefUse};
use filmcraft_hevc::params::{Pps, ScalingList, Sps};
use filmcraft_hevc::slice::SliceType;

use crate::va::{Config, Context, Display, Surface, SurfacePool};
use crate::va_sys as sys;

/// The decoder's name (`perf.stats`, logs).
pub const NAME: &str = "VA-API HEVC (in process)";

/// Largest picture taken (VCN tops out at 8192×4352; libva at 16K).
const MAX_DIM: u32 = 16384;

fn herr(e: String) -> filmcraft_hevc::Error {
    filmcraft_hevc::Error::Invalid(format!("VA-API: {e}"))
}

/// The libva side of one HEVC stream.
pub struct VaHevc {
    display: Arc<Display>,
    config: Option<Arc<Config>>,
    context: Option<Context>,
    pool: Option<Arc<SurfacePool>>,
    /// (width, height, bit depth) the context and pool were made for.
    shape: (u32, u32, u32),
}

/// A decoded picture: the surface, shared by the DPB and the output.
pub type VaSurface = Arc<Surface>;

impl VaHevc {
    fn new(display: Arc<Display>) -> Self {
        Self { display, config: None, context: None, pool: None, shape: (0, 0, 0) }
    }

    /// (Re)make the config, context and pool when the stream's size or depth changes.
    fn ensure(&mut self, sps: &Sps) -> Result<(), String> {
        let shape = (sps.width, sps.height, sps.bit_depth_luma);
        if self.context.is_some() && self.shape == shape {
            return Ok(());
        }
        if sps.chroma_format_idc != 1 || sps.bit_depth_luma != sps.bit_depth_chroma || !matches!(sps.bit_depth_luma, 8 | 10) {
            return Err(format!("unsupported format: chroma_format_idc {}, {}-bit", sps.chroma_format_idc, sps.bit_depth_luma));
        }
        if sps.width == 0 || sps.height == 0 || sps.width > MAX_DIM || sps.height > MAX_DIM {
            return Err(format!("picture size {}x{}", sps.width, sps.height));
        }
        let ten = sps.bit_depth_luma == 10;
        let (profile, rt, fourcc) = if ten {
            (sys::VAProfile_VAProfileHEVCMain10, sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010)
        } else {
            (sys::VAProfile_VAProfileHEVCMain, sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12)
        };
        if !self.display.supports(profile, sys::VAEntrypoint_VAEntrypointVLD) {
            return Err(format!("the VA-API driver has no HEVC Main{} decoder", if ten { " 10" } else { "" }));
        }
        self.context = None;
        let config = Arc::new(Config::new(&self.display, profile, sys::VAEntrypoint_VAEntrypointVLD, rt)?);
        self.context = Some(Context::new(&config, sps.width, sps.height)?);
        self.config = Some(config);
        self.pool = Some(SurfacePool::shared(&self.display, rt, fourcc, sps.width, sps.height));
        self.shape = shape;
        Ok(())
    }
}

impl Accelerator for VaHevc {
    type Surface = VaSurface;

    fn new_surface(&mut self, sps: &Sps) -> filmcraft_hevc::Result<VaSurface> {
        self.ensure(sps).map_err(herr)?;
        let pool = self.pool.as_ref().ok_or_else(|| herr("no surface pool".into()))?;
        pool.get().map(Arc::new).map_err(herr)
    }

    fn decode(&mut self, pic: &AccelPicture<'_, VaSurface>) -> filmcraft_hevc::Result<()> {
        let ctx = self.context.as_ref().ok_or_else(|| herr("no decoding context".into()))?;
        let mut buffers = Vec::with_capacity(2 + 2 * pic.slices.len());
        let pp = picture_params(pic);
        buffers.push(ctx.buffer(sys::VABufferType_VAPictureParameterBufferType, as_bytes(&pp)).map_err(herr)?);
        if pic.sps.scaling_list_enabled {
            let list = pic.pps.scaling_list.as_ref().or(pic.sps.scaling_list.as_ref()).cloned().unwrap_or_else(ScalingList::default_lists);
            let iq = iq_matrix(&list);
            buffers.push(ctx.buffer(sys::VABufferType_VAIQMatrixBufferType, as_bytes(&iq)).map_err(herr)?);
        }
        let last = pic.slices.len().saturating_sub(1);
        for (i, s) in pic.slices.iter().enumerate() {
            let sp = slice_params(pic, i, i == last)?;
            buffers.push(ctx.buffer(sys::VABufferType_VASliceParameterBufferType, as_bytes(&sp)).map_err(herr)?);
            buffers.push(ctx.buffer(sys::VABufferType_VASliceDataBufferType, s.nal).map_err(herr)?);
        }
        ctx.decode(pic.target, &buffers).map_err(herr)
    }
}

/// A plain-data C struct as bytes (to copy into a libva buffer).
fn as_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is one of the bindgen-generated `#[repr(C)]` VA parameter structs (Copy, no
    // pointers, fully initialised from `Default` = zeroed); reading its bytes is defined.
    #[allow(unsafe_code)]
    unsafe {
        std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>())
    }
}

fn bit(v: bool, at: u32) -> u32 {
    (v as u32) << at
}

/// Tile column widths / row heights in CTBs (6-3, 6-4), explicit or uniform.
fn tile_sizes(n: u32, explicit: &[u32], total: u32, uniform: bool) -> Vec<u32> {
    let n = n.max(1);
    if uniform || explicit.is_empty() {
        (0..n).map(|i| ((i + 1) * total) / n - (i * total) / n).collect()
    } else {
        let mut v: Vec<u32> = explicit.iter().copied().take(n as usize - 1).collect();
        let used: u32 = v.iter().sum();
        v.push(total.saturating_sub(used));
        v
    }
}

fn picture_params(pic: &AccelPicture<'_, VaSurface>) -> sys::VAPictureParameterBufferHEVC {
    let (sps, pps): (&Sps, &Pps) = (pic.sps, pic.pps);
    let mut p = sys::VAPictureParameterBufferHEVC::default();
    p.CurrPic.picture_id = pic.target.id;
    p.CurrPic.pic_order_cnt = pic.poc;
    for f in p.ReferenceFrames.iter_mut() {
        f.picture_id = sys::VA_INVALID_SURFACE;
        f.flags = sys::VA_PICTURE_HEVC_INVALID;
    }
    for (slot, r) in p.ReferenceFrames.iter_mut().zip(&pic.refs) {
        slot.picture_id = r.surface.id;
        slot.pic_order_cnt = r.poc;
        slot.flags = if r.long_term { sys::VA_PICTURE_HEVC_LONG_TERM_REFERENCE } else { 0 }
            | match r.usage {
                RefUse::StCurrBefore => sys::VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE,
                RefUse::StCurrAfter => sys::VA_PICTURE_HEVC_RPS_ST_CURR_AFTER,
                RefUse::LtCurr => sys::VA_PICTURE_HEVC_RPS_LT_CURR,
                RefUse::Foll => 0,
            };
    }
    p.pic_width_in_luma_samples = sps.width as u16;
    p.pic_height_in_luma_samples = sps.height as u16;
    let no_b = pic.slices.iter().all(|s| s.header.slice_type != SliceType::B);
    p.pic_fields.value = (sps.chroma_format_idc & 3)
        | bit(sps.separate_colour_plane, 2)
        | bit(sps.pcm, 3)
        | bit(sps.scaling_list_enabled, 4)
        | bit(pps.transform_skip, 5)
        | bit(sps.amp, 6)
        | bit(sps.strong_intra_smoothing, 7)
        | bit(pps.sign_data_hiding, 8)
        | bit(pps.constrained_intra_pred, 9)
        | bit(pps.cu_qp_delta_enabled, 10)
        | bit(pps.weighted_pred, 11)
        | bit(pps.weighted_bipred, 12)
        | bit(pps.transquant_bypass, 13)
        | bit(pps.tiles_enabled, 14)
        | bit(pps.entropy_coding_sync, 15)
        | bit(pps.loop_filter_across_slices, 16)
        | bit(pps.loop_filter_across_tiles, 17)
        | bit(sps.pcm_loop_filter_disabled, 18)
        | bit(sps.max_num_reorder == 0, 19)
        | bit(no_b, 20);
    p.sps_max_dec_pic_buffering_minus1 = sps.max_dec_pic_buffering.saturating_sub(1) as u8;
    p.bit_depth_luma_minus8 = sps.bit_depth_luma.saturating_sub(8) as u8;
    p.bit_depth_chroma_minus8 = sps.bit_depth_chroma.saturating_sub(8) as u8;
    if sps.pcm {
        p.pcm_sample_bit_depth_luma_minus1 = sps.pcm_bit_depth_luma.saturating_sub(1) as u8;
        p.pcm_sample_bit_depth_chroma_minus1 = sps.pcm_bit_depth_chroma.saturating_sub(1) as u8;
        p.log2_min_pcm_luma_coding_block_size_minus3 = sps.log2_min_pcm.saturating_sub(3) as u8;
        p.log2_diff_max_min_pcm_luma_coding_block_size = sps.log2_max_pcm.saturating_sub(sps.log2_min_pcm) as u8;
    }
    p.log2_min_luma_coding_block_size_minus3 = sps.log2_min_cb.saturating_sub(3) as u8;
    p.log2_diff_max_min_luma_coding_block_size = sps.log2_ctb.saturating_sub(sps.log2_min_cb) as u8;
    p.log2_min_transform_block_size_minus2 = sps.log2_min_tb.saturating_sub(2) as u8;
    p.log2_diff_max_min_transform_block_size = sps.log2_max_tb.saturating_sub(sps.log2_min_tb) as u8;
    p.max_transform_hierarchy_depth_intra = sps.max_th_depth_intra as u8;
    p.max_transform_hierarchy_depth_inter = sps.max_th_depth_inter as u8;
    p.init_qp_minus26 = (pps.init_qp - 26) as i8;
    p.diff_cu_qp_delta_depth = pps.diff_cu_qp_delta_depth as u8;
    p.pps_cb_qp_offset = pps.cb_qp_offset as i8;
    p.pps_cr_qp_offset = pps.cr_qp_offset as i8;
    p.log2_parallel_merge_level_minus2 = pps.log2_parallel_merge_level.saturating_sub(2) as u8;
    if pps.tiles_enabled {
        p.num_tile_columns_minus1 = pps.num_tile_columns.saturating_sub(1) as u8;
        p.num_tile_rows_minus1 = pps.num_tile_rows.saturating_sub(1) as u8;
        let cols = tile_sizes(pps.num_tile_columns, &pps.column_widths, sps.pic_width_in_ctbs(), pps.uniform_spacing);
        let rows = tile_sizes(pps.num_tile_rows, &pps.row_heights, sps.pic_height_in_ctbs(), pps.uniform_spacing);
        for (d, s) in p.column_width_minus1.iter_mut().zip(&cols) {
            *d = s.saturating_sub(1) as u16;
        }
        for (d, s) in p.row_height_minus1.iter_mut().zip(&rows) {
            *d = s.saturating_sub(1) as u16;
        }
    }
    let nal = pic.nal;
    let intra = pic.slices.iter().all(|s| s.header.is_intra());
    p.slice_parsing_fields.value = bit(pps.lists_modification_present, 0)
        | bit(sps.long_term_refs_present, 1)
        | bit(sps.temporal_mvp, 2)
        | bit(pps.cabac_init_present, 3)
        | bit(pps.output_flag_present, 4)
        | bit(pps.dependent_slice_segments_enabled, 5)
        | bit(pps.slice_chroma_qp_offsets_present, 6)
        | bit(sps.sao, 7)
        | bit(pps.deblocking_override_enabled, 8)
        | bit(pps.deblocking_disabled, 9)
        | bit(pps.slice_header_extension_present, 10)
        | bit(nal.is_irap(), 11)
        | bit(nal.is_idr(), 12)
        | bit(intra, 13);
    p.log2_max_pic_order_cnt_lsb_minus4 = sps.log2_max_poc_lsb.saturating_sub(4) as u8;
    p.num_short_term_ref_pic_sets = sps.st_rps.len() as u8;
    p.num_long_term_ref_pic_sps = sps.lt_ref_pics.len() as u8;
    p.num_ref_idx_l0_default_active_minus1 = pps.num_ref_idx_l0_default.saturating_sub(1) as u8;
    p.num_ref_idx_l1_default_active_minus1 = pps.num_ref_idx_l1_default.saturating_sub(1) as u8;
    p.pps_beta_offset_div2 = pps.beta_offset_div2 as i8;
    p.pps_tc_offset_div2 = pps.tc_offset_div2 as i8;
    p.num_extra_slice_header_bits = pps.num_extra_slice_header_bits as u8;
    p.st_rps_bits = pic.slices.first().map_or(0, |s| s.header.st_rps_bits as u32);
    p
}

/// Up-right diagonal scan of an n×n block (6.5.3): position i → (x, y).
fn diag_scan(n: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(n * n);
    let (mut x, mut y) = (0i32, 0i32);
    let n = n as i32;
    while out.len() < (n * n) as usize {
        while y >= 0 {
            if x < n && y < n {
                out.push((x as usize, y as usize));
            }
            y -= 1;
            x += 1;
        }
        y = x;
        x = 0;
    }
    out
}

/// Scaling lists for VA-API, each in raster order of its 4×4 / 8×8 coefficient matrix (Mesa
/// reads them so; the parser keeps the coded up-right diagonal order). Verified bit-exact on the
/// `scaling_list` conformance fixture.
fn iq_matrix(l: &ScalingList) -> sys::VAIQMatrixBufferHEVC {
    let (s4, s8) = (diag_scan(4), diag_scan(8));
    let raster4 = |src: &[u8; 64]| {
        let mut o = [0u8; 16];
        for (i, &(x, y)) in s4.iter().enumerate() {
            o[y * 4 + x] = src[i];
        }
        o
    };
    let raster8 = |src: &[u8; 64]| {
        let mut o = [0u8; 64];
        for (i, &(x, y)) in s8.iter().enumerate() {
            o[y * 8 + x] = src[i];
        }
        o
    };
    let mut q = sys::VAIQMatrixBufferHEVC::default();
    for m in 0..6 {
        q.ScalingList4x4[m] = raster4(&l.lists[0][m]);
        q.ScalingList8x8[m] = raster8(&l.lists[1][m]);
        q.ScalingList16x16[m] = raster8(&l.lists[2][m]);
        q.ScalingListDC16x16[m] = l.dc[0][m];
    }
    // 32x32: matrixId 0 (intra) and 3 (inter) of the luma lists
    for (i, m) in [0usize, 3].into_iter().enumerate() {
        q.ScalingList32x32[i] = raster8(&l.lists[3][m]);
        q.ScalingListDC32x32[i] = l.dc[1][m];
    }
    q
}

fn slice_params(pic: &AccelPicture<'_, VaSurface>, i: usize, last: bool) -> filmcraft_hevc::Result<sys::VASliceParameterBufferHEVC> {
    let s = pic.slices.get(i).ok_or_else(|| herr("slice index".into()))?;
    let h = s.header;
    let mut p = sys::VASliceParameterBufferHEVC::default();
    p.slice_data_size = s.nal.len() as u32;
    p.slice_data_offset = 0;
    p.slice_data_flag = sys::VA_SLICE_DATA_FLAG_ALL;
    p.slice_data_byte_offset = s.data_byte_offset as u32;
    p.slice_segment_address = h.segment_address;
    for list in p.RefPicList.iter_mut() {
        list.fill(0xff);
    }
    for (l, refs) in s.ref_lists.iter().enumerate() {
        for (d, &r) in p.RefPicList[l].iter_mut().zip(refs) {
            *d = r.min(14) as u8;
        }
    }
    let slice_type = match h.slice_type {
        SliceType::B => 0,
        SliceType::P => 1,
        SliceType::I => 2,
    };
    p.LongSliceFlags.value = bit(last, 0)
        | bit(h.dependent, 1)
        | (slice_type << 2)
        | bit(h.sao_luma, 6)
        | bit(h.sao_chroma, 7)
        | bit(h.mvd_l1_zero, 8)
        | bit(h.cabac_init_flag, 9)
        | bit(h.temporal_mvp, 10)
        | bit(h.deblocking_disabled, 11)
        | bit(h.collocated_from_l0, 12)
        | bit(h.loop_filter_across_slices, 13);
    p.collocated_ref_idx = if h.temporal_mvp { h.collocated_ref_idx as u8 } else { 0xff };
    p.num_ref_idx_l0_active_minus1 = h.num_ref_idx[0].saturating_sub(1) as u8;
    p.num_ref_idx_l1_active_minus1 = h.num_ref_idx[1].saturating_sub(1) as u8;
    p.slice_qp_delta = h.qp_delta as i8;
    p.slice_cb_qp_offset = h.cb_qp_offset as i8;
    p.slice_cr_qp_offset = h.cr_qp_offset as i8;
    p.slice_beta_offset_div2 = h.beta_offset_div2 as i8;
    p.slice_tc_offset_div2 = h.tc_offset_div2 as i8;
    if let Some(w) = &h.pwt {
        p.luma_log2_weight_denom = w.luma_log2_denom as u8;
        p.delta_chroma_log2_weight_denom = w.chroma_log2_denom as i8 - w.luma_log2_denom as i8;
        let (ld, cd) = (1i32 << w.luma_log2_denom, 1i32 << w.chroma_log2_denom);
        for (l, entries) in w.l.iter().enumerate() {
            for (k, e) in entries.iter().enumerate().take(15) {
                let (dw, off) = (e.luma.0 - ld, e.luma.1);
                let (cdw, coff) = ([e.chroma[0].0 - cd, e.chroma[1].0 - cd], [e.chroma[0].1, e.chroma[1].1]);
                let (dlw, lo, dcw, co) = if l == 0 {
                    (&mut p.delta_luma_weight_l0, &mut p.luma_offset_l0, &mut p.delta_chroma_weight_l0, &mut p.ChromaOffsetL0)
                } else {
                    (&mut p.delta_luma_weight_l1, &mut p.luma_offset_l1, &mut p.delta_chroma_weight_l1, &mut p.ChromaOffsetL1)
                };
                dlw[k] = dw as i8;
                lo[k] = off as i8;
                for c in 0..2 {
                    dcw[k][c] = cdw[c] as i8;
                    co[k][c] = coff[c] as i8;
                }
            }
        }
    }
    p.five_minus_max_num_merge_cand = 5u32.saturating_sub(h.max_num_merge_cand) as u8;
    p.num_entry_point_offsets = h.entry_points.len().min(u16::MAX as usize) as u16;
    p.entry_offset_to_subset_array = 0;
    p.slice_data_num_emu_prevn_bytes = s.header_emulation_bytes.min(u16::MAX as usize) as u16;
    Ok(p)
}

// ------------------------------------------------------------------------------------ decoder

/// HEVC decoding through libva in process: a `VideoDecoder` for the media stack (pictures read
/// back to memory for now).
pub struct NativeVaDecoder {
    dec: AccelDecoder<VaHevc>,
    info: NalStreamInfo,
    /// Test hook: fail every decode once this many samples were fed.
    fail_after: Option<u64>,
    fed: u64,
}

impl NativeVaDecoder {
    /// A decoder for the stream, or why the in-process VA-API path does not take it.
    pub fn new(info: NalStreamInfo) -> Result<Self, String> {
        if info.codec != NalCodec::Hevc {
            return Err("in-process VA-API decodes HEVC only".into());
        }
        if info.chroma_format_idc != 1 {
            return Err(format!("chroma format {} (VA-API decodes 4:2:0)", info.chroma_format_idc));
        }
        if info.bit_depth_luma != info.bit_depth_chroma || !matches!(info.bit_depth_luma, 8 | 10) {
            return Err(format!("{}-bit HEVC", info.bit_depth_luma));
        }
        let display = Display::get()?;
        let profile = if info.bit_depth_luma == 10 { sys::VAProfile_VAProfileHEVCMain10 } else { sys::VAProfile_VAProfileHEVCMain };
        if !display.supports(profile, sys::VAEntrypoint_VAEntrypointVLD) {
            return Err("the VA-API driver has no decoder for this HEVC profile".into());
        }
        let mut dec = AccelDecoder::new(VaHevc::new(display));
        dec.set_nal_length_size(Some(info.length_size));
        for ps in &info.parameter_sets {
            dec.add_parameter_set(ps).map_err(|e| e.to_string())?;
        }
        Ok(Self { dec, info, fail_after: None, fed: 0 })
    }

    /// Test hook: from the `n`-th sample on, every decode fails as a lost device would.
    pub fn fail_after(&mut self, n: u64) {
        self.fail_after = Some(n);
    }

    /// Decoded pictures as GPU frames (synced: the decode has finished when they are handed out).
    fn convert(&self, outs: Vec<AccelOutput<VaSurface>>) -> filmcraft_codecs::Result<Vec<DecodedFrame>> {
        outs.into_iter()
            .map(|o| {
                o.surface.sync().map_err(|e| CodecError::Decode(format!("VA-API: {e}")))?;
                let (cx, cy, cw, ch) = o.meta.crop;
                let pic = VaPicture { surface: o.surface.clone(), origin: (cx, cy), ten_bit: o.meta.bit_depth > 8 };
                let frame = VideoFrame {
                    width: cw,
                    height: ch,
                    data: PixelData::Gpu(Arc::new(pic)),
                    color: self.info.color,
                    par: self.info.par,
                    pts: filmcraft_time::Tick::ZERO,
                };
                Ok(DecodedFrame { pts: o.meta.pts, frame, draft: false })
            })
            .collect()
    }
}

/// A decoded picture in a VA surface, as a frame's GPU pixels.
pub struct VaPicture {
    surface: VaSurface,
    origin: (u32, u32),
    ten_bit: bool,
}

impl std::fmt::Debug for VaPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VaPicture(surface {}, {}-bit)", self.surface.id, if self.ten_bit { 10 } else { 8 })
    }
}

impl filmcraft_frame::GpuSurface for VaPicture {
    fn memory_id(&self) -> u64 {
        self.surface.mem().memory_id
    }
    fn liveness(&self) -> std::sync::Weak<()> {
        self.surface.mem().liveness()
    }
    fn format(&self) -> filmcraft_frame::GpuFormat {
        if self.ten_bit { filmcraft_frame::GpuFormat::P010 } else { filmcraft_frame::GpuFormat::Nv12 }
    }
    fn coded_size(&self) -> (u32, u32) {
        self.surface.pool().size
    }
    fn crop_origin(&self) -> (u32, u32) {
        self.origin
    }
    fn dmabuf(&self) -> Option<filmcraft_frame::DmaBuf> {
        match self.surface.export() {
            Ok(e) => Some(e.dmabuf.clone()),
            Err(e) => {
                log::warn!("VA-API surface export failed: {e}");
                None
            }
        }
    }
    fn download(&self, width: u32, height: u32) -> Result<PixelData, String> {
        read_back(&self.surface, (self.origin.0, self.origin.1, width, height), self.ten_bit)
    }
    fn gpu_bytes(&self) -> usize {
        let (w, h) = self.surface.pool().size;
        (w as usize * h as usize * 3 / 2) * if self.ten_bit { 2 } else { 1 }
    }
}

/// Read the rectangle (x, y, w, h) of a surface back to memory: P010 → `Yuv16Bi`, NV12 → `Yuv8`.
fn read_back(surface: &Surface, (cx, cy, cw, ch): (u32, u32, u32, u32), ten_bit: bool) -> Result<PixelData, String> {
    let (w, h) = (cw as usize, ch as usize);
    let (ccw, cch) = (w.div_ceil(2), h.div_ceil(2));
    Ok(if ten_bit {
        let mut y = pool::take_u16_filled(w * h);
        let mut uv = pool::take_u16_filled(ccw * cch * 2);
        surface.read((cx, cy, cw, ch), |plane, r, row| {
            let dst = if plane == 0 { y.get_mut(r * w..(r + 1) * w) } else { uv.get_mut(r * ccw * 2..(r + 1) * ccw * 2) };
            if let Some(dst) = dst {
                for (d, b) in dst.iter_mut().zip(row.as_chunks::<2>().0) {
                    *d = u16::from_le_bytes(*b);
                }
            }
        })?;
        PixelData::Yuv16Bi { y: Arc::new(y), uv: Arc::new(uv), chroma: Chroma::C420, bits: 10 }
    } else {
        let mut y = pool::take_u8(w * h);
        y.resize(w * h, 0);
        let (mut u, mut v) = (pool::take_u8(ccw * cch), pool::take_u8(ccw * cch));
        u.resize(ccw * cch, 128);
        v.resize(ccw * cch, 128);
        surface.read((cx, cy, cw, ch), |plane, r, row| {
            if plane == 0 {
                if let (Some(dst), Some(src)) = (y.get_mut(r * w..(r + 1) * w), row.get(..w)) {
                    dst.copy_from_slice(src);
                }
            } else if let (Some(du), Some(dv)) = (u.get_mut(r * ccw..(r + 1) * ccw), v.get_mut(r * ccw..(r + 1) * ccw)) {
                for ((a, b), p) in du.iter_mut().zip(dv.iter_mut()).zip(row.as_chunks::<2>().0) {
                    *a = p[0];
                    *b = p[1];
                }
            }
        })?;
        PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    })
}

/// Read a decoded picture back to memory, cropped.
fn download(o: &AccelOutput<VaSurface>, info: &NalStreamInfo) -> filmcraft_codecs::Result<VideoFrame> {
    let (cx, cy, cw, ch) = o.meta.crop;
    let data = read_back(&o.surface, (cx, cy, cw, ch), o.meta.bit_depth > 8).map_err(|e| CodecError::Decode(format!("VA-API: {e}")))?;
    Ok(VideoFrame { width: cw, height: ch, data, color: info.color, par: info.par, pts: filmcraft_time::Tick::ZERO })
}

impl VideoDecoder for NativeVaDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> filmcraft_codecs::Result<Vec<DecodedFrame>> {
        self.fed += 1;
        if self.fail_after.is_some_and(|n| self.fed > n) {
            return Err(CodecError::Decode("VA-API decoder failed (test hook)".into()));
        }
        let outs = self.dec.decode(sample, pts).map_err(|e| CodecError::Decode(e.to_string()))?;
        self.convert(outs)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        match self.dec.flush().map_err(|e| CodecError::Decode(e.to_string())).and_then(|o| self.convert(o)) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("VA-API HEVC: {e} while flushing");
                Vec::new()
            }
        }
    }

    fn reset(&mut self) {
        self.dec.reset();
    }

    fn name(&self) -> &str {
        NAME
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.info.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.info.is_disposable(sample)
    }
}

/// Decode a whole Annex B HEVC stream in process and read every picture back, in output order
/// (diagnostics and tests).
pub fn decode_annexb(data: &[u8]) -> Result<Vec<VideoFrame>, String> {
    let display = Display::get()?;
    let mut dec = AccelDecoder::new(VaHevc::new(display));
    let mut outs = dec.decode(data, 0).map_err(|e| e.to_string())?;
    outs.extend(dec.flush().map_err(|e| e.to_string())?);
    let info = NalStreamInfo {
        codec: NalCodec::Hevc,
        length_size: 4,
        highest_tid: None,
        parameter_sets: Vec::new(),
        coded: (0, 0),
        crop: (0, 0, 0, 0),
        chroma_format_idc: 1,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        interlaced: false,
        profile_idc: 1,
        color: filmcraft_color::ColorInfo::REC709,
        par: (1, 1),
        reorder: 0,
    };
    outs.iter().map(|o| download(o, &info).map_err(|e| e.to_string())).collect()
}
