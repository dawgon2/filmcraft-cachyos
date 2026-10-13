//! HEVC decoding with libva in process (docs/adr/0002-linux-video-interop.md).
//!
//! [`VaHevc`] is the [`Accelerator`] behind `filmcraft_hevc::accel::AccelDecoder`: FilmCraft's own
//! parser and DPB decide what each picture references and when it is output, and this module
//! translates each picture into VA-API's HEVC buffers (picture parameters, scaling lists, one
//! slice-parameter and one slice-data buffer per slice segment) and decodes it into a pooled
//! surface. [`NativeVaDecoder`] is the `VideoDecoder` the media stack uses: 4:2:0 pictures stay
//! on the GPU (`PixelData::Gpu`, read back on demand: P010 → `Yuv16Bi`, NV12 → planar `Yuv8`),
//! bit-exact with FilmCraft's software decoder.
//!
//! HEVC Main 4:2:2 10 (fork Phase 6.1b) decodes on the display whose driver has that profile
//! ([`Display::for_hevc_422`]: Intel's media engine here; AMD's VCN has none), with the
//! range-extension parameter buffers that profile takes, into Y210 surfaces. Each picture is then
//! copied by that engine's video processor into a linear surface ([`LinearCopy`]) and handed out
//! as a GPU frame (`GpuFormat::Y210`), which the compositing GPU imports as a DMA-BUF (Intel →
//! AMD hand-off, FORK-PLAN §1a.30); without the hand-off (`FILMCRAFT_NO_GPU_IMPORT`, no video
//! processor, a failed copy or export: logged once) pictures are read back to `Yuv16Bi` 4:2:2
//! frames. Without such a display the stream is declined and decodes in software.

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

use crate::va::{Config, Context, Display, LinearCopy, Surface, SurfacePool};
use crate::va_sys as sys;

/// The decoder's name (`perf.stats`, logs).
pub const NAME: &str = "VA-API HEVC (in process)";
/// The decoder's name for HEVC Main 4:2:2 10 streams (`perf.stats`, logs).
pub const NAME_422: &str = "VA-API HEVC 4:2:2 (in process)";

/// Whether HEVC Main 4:2:2 10 decodes in hardware here, blocking until the display search is
/// done: the Hardware decoding setting is on, a display has the profile
/// ([`Display::for_hevc_422`]) and no 4:2:2 stream has failed in hardware this session.
pub fn hardware_422() -> bool {
    filmcraft_codecs::hw::hardware_decoding() && !filmcraft_codecs::hw::hevc_422_failed() && Display::for_hevc_422().is_ok()
}

/// [`hardware_422`]'s display part without waiting (the UI's probe, registered by
/// `filmcraft_platform::register`): `None` until the search has finished.
pub fn hardware_422_ready() -> Option<bool> {
    Display::hevc_422_ready()
}

/// Search for the 4:2:2 display on a background thread (vaInitialize on each render node), so
/// the UI's first question does not wait for it.
pub fn warm_422_probe() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("filmcraft-va-422-probe".into()).spawn(|| {
            let _ = std::panic::catch_unwind(Display::for_hevc_422);
        });
        if let Err(why) = spawned {
            log::info!("VA-API 4:2:2 probe not started ahead of time: {why}");
        }
    });
}

/// What a stream decodes as: (profile, render-target format, surface fourcc, the profile's name).
fn va_format(chroma_format_idc: u32, bits_luma: u32, bits_chroma: u32) -> Result<(sys::VAProfile, u32, u32, &'static str), String> {
    match (chroma_format_idc, bits_luma, bits_chroma) {
        (1, 8, 8) => Ok((sys::VAProfile_VAProfileHEVCMain, sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12, "Main")),
        (1, 10, 10) => Ok((sys::VAProfile_VAProfileHEVCMain10, sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010, "Main 10")),
        (2, 10, 10) => Ok((sys::VAProfile_VAProfileHEVCMain422_10, sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210, "Main 4:2:2 10")),
        _ => Err(format!("unsupported format: chroma_format_idc {chroma_format_idc}, {bits_luma}/{bits_chroma}-bit")),
    }
}

/// The display a stream of this chroma format decodes on: 4:2:0 on [`Display::get`], 4:2:2 on
/// [`Display::for_hevc_422`].
fn display_for(chroma_format_idc: u32) -> Result<Arc<Display>, String> {
    if chroma_format_idc == 2 { Display::for_hevc_422() } else { Display::get() }
}

/// Largest picture taken (VCN tops out at 8192×4352; libva at 16K).
const MAX_DIM: u32 = 16384;

fn herr(e: String) -> filmcraft_hevc::Error {
    filmcraft_hevc::Error::Invalid(format!("VA-API: {e}"))
}

/// The libva side of one HEVC stream.
pub struct VaHevc {
    config: Option<Arc<Config>>,
    context: Option<Context>,
    pool: Option<Arc<SurfacePool>>,
    /// (width, height, bit depth, chroma_format_idc) the context and pool were made for.
    shape: (u32, u32, u32, u32),
}

/// A decoded picture: the surface, shared by the DPB and the output.
pub type VaSurface = Arc<Surface>;

impl VaHevc {
    fn new() -> Self {
        Self { config: None, context: None, pool: None, shape: (0, 0, 0, 0) }
    }

    /// (Re)make the config, context and pool when the stream's size, depth or chroma format
    /// changes; the chroma format picks the display ([`display_for`]).
    fn ensure(&mut self, sps: &Sps) -> Result<(), String> {
        let shape = (sps.width, sps.height, sps.bit_depth_luma, sps.chroma_format_idc);
        if self.context.is_some() && self.shape == shape {
            return Ok(());
        }
        let (profile, rt, fourcc, name) = va_format(sps.chroma_format_idc, sps.bit_depth_luma, sps.bit_depth_chroma)?;
        if sps.separate_colour_plane {
            return Err("separate colour planes".into());
        }
        if sps.width == 0 || sps.height == 0 || sps.width > MAX_DIM || sps.height > MAX_DIM {
            return Err(format!("picture size {}x{}", sps.width, sps.height));
        }
        self.context = None;
        self.config = None;
        self.pool = None;
        let display = display_for(sps.chroma_format_idc)?;
        if !display.supports(profile, sys::VAEntrypoint_VAEntrypointVLD) {
            return Err(format!("the VA-API driver on {} has no HEVC {name} decoder", display.node));
        }
        let config = Arc::new(Config::new(&display, profile, sys::VAEntrypoint_VAEntrypointVLD, rt)?);
        self.context = Some(Context::new(&config, sps.width, sps.height)?);
        self.config = Some(config);
        self.pool = Some(SurfacePool::shared(&display, rt, fourcc, sps.width, sps.height));
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

    /// A stand-in for a missing reference (after a seek into an open GOP, or a damaged stream):
    /// mid-grey, as the software decoder generates it (8.3.3.2). A 4:2:2 (Intel) stream never
    /// predicts from a surface nothing wrote (the picture fails instead); 4:2:0 keeps the
    /// unwritten surface when the grey cannot be written, as before.
    fn missing_reference(&mut self, sps: &Sps, _poc: i32) -> filmcraft_hevc::Result<VaSurface> {
        let s = self.new_surface(sps)?;
        let fourcc = s.pool().fourcc;
        // nothing may still be decoding into it while the grey is copied in (the pool syncs
        // spares too; this covers a fresh surface's first use)
        let filled = s.sync().and_then(|()| {
            s.write(|bytes, _, _| {
                if fourcc == sys::VA_FOURCC_NV12 {
                    bytes.fill(0x80);
                } else {
                    // 16-bit little-endian, MSB-aligned: 0x8000 is code 512 of 10 bits
                    for p in bytes.as_chunks_mut::<2>().0 {
                        *p = [0x00, 0x80];
                    }
                }
                Ok(())
            })
        });
        match filled {
            Ok(()) => Ok(s),
            // 4:2:2 (Intel) never predicts from an unwritten surface
            Err(e) if fourcc == sys::VA_FOURCC_Y210 => Err(herr(format!("missing reference: {e}"))),
            // 4:2:0: as before, the unwritten surface
            Err(e) => {
                log::warn!("VA-API HEVC: missing reference left unwritten ({e})");
                Ok(s)
            }
        }
    }

    fn decode(&mut self, pic: &AccelPicture<'_, VaSurface>) -> filmcraft_hevc::Result<()> {
        let ctx = self.context.as_ref().ok_or_else(|| herr("no decoding context".into()))?;
        let mut buffers = Vec::with_capacity(2 + 2 * pic.slices.len());
        // the range-extension profiles (Main 4:2:2 10) take the extended structs: base + rext
        let rext = pic.sps.chroma_format_idc == 2;
        let pp = picture_params(pic)?;
        if rext {
            let mut x = sys::VAPictureParameterBufferHEVCExtension::default();
            x.base = pp;
            x.rext = picture_params_rext(pic.sps, pic.pps);
            buffers.push(ctx.param_buffer(&x).map_err(herr)?);
        } else {
            buffers.push(ctx.param_buffer(&pp).map_err(herr)?);
        }
        if pic.sps.scaling_list_enabled {
            let list = pic.pps.scaling_list.as_ref().or(pic.sps.scaling_list.as_ref()).cloned().unwrap_or_else(ScalingList::default_lists);
            let iq = iq_matrix(&list);
            buffers.push(ctx.param_buffer(&iq).map_err(herr)?);
        }
        let last = pic.slices.len().saturating_sub(1);
        for (i, s) in pic.slices.iter().enumerate() {
            let sp = slice_params(pic, i, i == last)?;
            if rext {
                let mut x = sys::VASliceParameterBufferHEVCExtension::default();
                x.base = sp;
                x.rext = slice_params_rext(pic, i)?;
                buffers.push(ctx.param_buffer(&x).map_err(herr)?);
            } else {
                buffers.push(ctx.param_buffer(&sp).map_err(herr)?);
            }
            buffers.push(ctx.slice_data(s.nal).map_err(herr)?);
        }
        ctx.decode(pic.target, &buffers).map_err(herr)
    }
}

fn bit(v: bool, at: u32) -> u32 {
    (v as u32) << at
}

/// Tile column widths / row heights in CTBs (6-3, 6-4), explicit or uniform; `max` tiles at most
/// (the VA arrays), every tile at least one CTB (the driver trusts these: hostile values are refused).
fn tile_sizes(n: u32, explicit: &[u32], total: u32, uniform: bool, max: usize) -> filmcraft_hevc::Result<Vec<u32>> {
    let n = n.max(1);
    if n as usize > max || n > total {
        return Err(herr(format!("{n} tiles in {total} CTBs (at most {max})")));
    }
    if uniform || explicit.is_empty() {
        let (n, total) = (u64::from(n), u64::from(total));
        Ok((0..n).map(|i| (((i + 1) * total) / n - (i * total) / n) as u32).collect())
    } else {
        let mut v: Vec<u32> = explicit.iter().copied().take(n as usize - 1).collect();
        let used: u64 = v.iter().map(|&x| u64::from(x)).sum();
        match u64::from(total).checked_sub(used) {
            Some(rest) if rest > 0 && v.len() + 1 == n as usize && v.iter().all(|&x| x > 0) => v.push(rest as u32),
            _ => return Err(herr(format!("explicit tile sizes {v:?} do not fit {total} CTBs"))),
        }
        Ok(v)
    }
}

fn picture_params(pic: &AccelPicture<'_, VaSurface>) -> filmcraft_hevc::Result<sys::VAPictureParameterBufferHEVC> {
    let (sps, pps): (&Sps, &Pps) = (pic.sps, pic.pps);
    let mut p = sys::VAPictureParameterBufferHEVC::default();
    // more references than VA's 15 slots would silently decode against the wrong pictures
    if pic.refs.len() > p.ReferenceFrames.len() {
        return Err(herr(format!("{} reference pictures (VA-API takes {})", pic.refs.len(), p.ReferenceFrames.len())));
    }
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
        let cols = tile_sizes(pps.num_tile_columns, &pps.column_widths, sps.pic_width_in_ctbs(), pps.uniform_spacing, p.column_width_minus1.len() + 1)?;
        let rows = tile_sizes(pps.num_tile_rows, &pps.row_heights, sps.pic_height_in_ctbs(), pps.uniform_spacing, p.row_height_minus1.len() + 1)?;
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
    Ok(p)
}

/// The range-extension picture parameters (7.3.2.2.2, 7.3.2.3.2). FilmCraft's parser refuses
/// streams that use the range-extension tools (`check_supported`), so these hold the defaults in
/// practice; they are filled from the parsed sets all the same.
fn picture_params_rext(sps: &Sps, pps: &Pps) -> sys::VAPictureParameterBufferHEVCRext {
    let (s, p) = (&sps.range_ext, &pps.range_ext);
    let mut r = sys::VAPictureParameterBufferHEVCRext::default();
    r.range_extension_pic_fields.value = bit(s.transform_skip_rotation, 0)
        | bit(s.transform_skip_context, 1)
        | bit(s.implicit_rdpcm, 2)
        | bit(s.explicit_rdpcm, 3)
        | bit(s.extended_precision_processing, 4)
        | bit(s.intra_smoothing_disabled, 5)
        | bit(s.high_precision_offsets, 6)
        | bit(s.persistent_rice_adaptation, 7)
        | bit(s.cabac_bypass_alignment, 8)
        | bit(p.cross_component_prediction, 9)
        | bit(p.chroma_qp_offset_list_enabled, 10);
    r.diff_cu_chroma_qp_offset_depth = p.diff_cu_chroma_qp_offset_depth.min(3) as u8;
    r.chroma_qp_offset_list_len_minus1 = p.cb_qp_offset_list.len().clamp(1, 6).saturating_sub(1) as u8;
    r.log2_sao_offset_scale_luma = p.log2_sao_offset_scale_luma.min(6) as u8;
    r.log2_sao_offset_scale_chroma = p.log2_sao_offset_scale_chroma.min(6) as u8;
    r.log2_max_transform_skip_block_size_minus2 = p.log2_max_transform_skip_size.clamp(2, 5).saturating_sub(2) as u8;
    for (d, &v) in r.cb_qp_offset_list.iter_mut().zip(&p.cb_qp_offset_list) {
        *d = v.clamp(-12, 12) as i8;
    }
    for (d, &v) in r.cr_qp_offset_list.iter_mut().zip(&p.cr_qp_offset_list) {
        *d = v.clamp(-12, 12) as i8;
    }
    r
}

/// The range-extension slice parameters: the weighted-prediction offsets at full width (the
/// driver reads them from here for range-extension profiles), no chroma QP offsets per CU, no
/// screen-content tools.
fn slice_params_rext(pic: &AccelPicture<'_, VaSurface>, i: usize) -> filmcraft_hevc::Result<sys::VASliceParameterBufferHEVCRext> {
    let s = pic.slices.get(i).ok_or_else(|| herr("slice index".into()))?;
    let mut r = sys::VASliceParameterBufferHEVCRext::default();
    if let Some(w) = &s.header.pwt {
        for (l, entries) in w.l.iter().enumerate() {
            let (lo, co) = if l == 0 { (&mut r.luma_offset_l0, &mut r.ChromaOffsetL0) } else { (&mut r.luma_offset_l1, &mut r.ChromaOffsetL1) };
            for (k, e) in entries.iter().enumerate().take(15) {
                lo[k] = e.luma.1.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
                for c in 0..2 {
                    co[k][c] = e.chroma[c].1.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
                }
            }
        }
    }
    Ok(r)
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
    // the driver reads the slice data within these bounds: they must lie in the NAL unit
    p.slice_data_size = u32::try_from(s.nal.len()).map_err(|_| herr("slice too large".into()))?;
    p.slice_data_offset = 0;
    p.slice_data_flag = sys::VA_SLICE_DATA_FLAG_ALL;
    p.slice_data_byte_offset =
        u32::try_from(s.data_byte_offset).ok().filter(|&o| o <= p.slice_data_size).ok_or_else(|| herr("slice data offset beyond the slice".into()))?;
    // the substreams (tiles / WPP rows) the entry points announce must lie in the slice data, each
    // at least one byte: a truncated slice is refused here instead of sending the engine past
    // its end (Intel's media engine resets on such damage, 6.1b)
    let substreams: u64 = h.entry_points.iter().map(|&o| u64::from(o)).sum();
    if u64::from(p.slice_data_byte_offset).saturating_add(substreams) >= u64::from(p.slice_data_size) {
        return Err(herr(format!("{} entry points ({substreams} bytes) past a {}-byte slice", h.entry_points.len(), p.slice_data_size)));
    }
    p.slice_segment_address = h.segment_address;
    for list in p.RefPicList.iter_mut() {
        list.fill(0xff);
    }
    for (l, refs) in s.ref_lists.iter().enumerate() {
        let list = p.RefPicList.get_mut(l).ok_or_else(|| herr("reference list index".into()))?;
        for (d, &r) in list.iter_mut().zip(refs) {
            // an entry must name a filled ReferenceFrames slot (refs ≤ 15, checked)
            if r >= pic.refs.len() {
                return Err(herr(format!("reference list entry {r} of {} references", pic.refs.len())));
            }
            *d = r as u8;
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

/// HEVC decoding through libva in process: a `VideoDecoder` for the media stack. 4:2:0 pictures
/// stay in their surfaces (`PixelData::Gpu`); 4:2:2 pictures are handed over in linear copies
/// (`PixelData::Gpu`, [`LinearCopy`]) or, without the hand-off, read back to memory.
pub struct NativeVaDecoder {
    dec: AccelDecoder<VaHevc>,
    info: NalStreamInfo,
    /// Test hook: fail every decode once this many samples were fed.
    fail_after: Option<u64>,
    fed: u64,
    /// A 4:2:2 picture failed in hardware: no more pictures go to the engine until `reset`
    /// (a seek restarts at a random-access point with an empty DPB). Intel's media engine hung
    /// (GPU reset, ~7 s each) when damaged streams were fed on after a failed picture (6.1b).
    stopped: bool,
    /// 4:2:2: the copier into linear surfaces for the hand-off (made on the first picture).
    handoff: Option<LinearCopy>,
    /// 4:2:2 pictures are read back instead of handed off (`FILMCRAFT_NO_GPU_IMPORT`, the
    /// hand-off failed for this stream, or [`NativeVaDecoder::set_handoff`]).
    handoff_off: bool,
    /// Test hook: the next hand-off fails as a refused copy / export would.
    fail_handoff: bool,
}

/// Linear memory for the 4:2:2 hand-off when the VA driver does not make linear surfaces itself
/// (iHD): exportable system memory from a Vulkan device of the decoder's own (made on first use
/// and kept), which Intel's video processor writes into and the compositor imports. Any linear
/// DMA-BUF imports, whichever device made it; the device is picked by the compositor's policy
/// (the high-performance adapter) but may still be another GPU than the compositor's
/// (`MESA_VK_DEVICE_SELECT`, a different adapter order), which costs nothing more. None: no such
/// device (then the driver's own surfaces are the only try).
fn handoff_alloc() -> Option<crate::va::DmaBufAlloc> {
    static DEVICE: std::sync::OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();
    let (device, _) = DEVICE
        .get_or_init(|| {
            let made = std::panic::catch_unwind(filmcraft_gpu::GpuCompositor::headless_device).ok().flatten();
            made.filter(|(d, _)| filmcraft_gpu::interop::can_import(d))
        })
        .as_ref()?;
    let device = device.clone();
    Some(Arc::new(move |size| filmcraft_gpu::interop::alloc_dmabuf(&device, size)))
}

/// Whether a handed-off picture must keep its decoded surface for CPU read-back: only when its
/// linear copy (over imported memory) cannot be read by mapping its DMA-BUF (probed once per
/// session on the first copy). Otherwise the decoded surface goes back to the pool right after
/// the copy, and read-back maps the copy ([`Surface::read`]).
fn keep_decoded(linear: &Surface) -> bool {
    if !matches!(linear.pool().linear, Some(crate::va::LinearAlloc::Imported(_))) {
        return false;
    }
    static MAPPED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let mapped = *MAPPED.get_or_init(|| match linear.read((0, 0, 2, 1), |_, _, _| {}) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("VA-API HEVC 4:2:2: handed-off pictures cannot be mapped ({e}); they keep their decoded surfaces for reading back");
            false
        }
    });
    !mapped
}

/// Log the first hand-off failure of the session (later ones are as the first).
fn handoff_failed(why: &str) {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::warn!("VA-API HEVC 4:2:2: handing pictures to the GPU failed ({why}); reading them back to memory instead");
    }
}

impl NativeVaDecoder {
    /// A decoder for the stream, or why the in-process VA-API path does not take it.
    pub fn new(info: NalStreamInfo) -> Result<Self, String> {
        if info.codec != NalCodec::Hevc {
            return Err("in-process VA-API decodes HEVC only".into());
        }
        let (profile, _, _, name) = va_format(info.chroma_format_idc, info.bit_depth_luma, info.bit_depth_chroma)
            .map_err(|_| format!("chroma format {} at {}-bit (VA-API decodes 4:2:0 8/10-bit and 4:2:2 10-bit)", info.chroma_format_idc, info.bit_depth_luma))?;
        let display = display_for(info.chroma_format_idc)?;
        if !display.supports(profile, sys::VAEntrypoint_VAEntrypointVLD) {
            return Err(format!("the VA-API driver on {} has no HEVC {name} decoder", display.node));
        }
        let mut dec = AccelDecoder::new(VaHevc::new());
        dec.set_nal_length_size(Some(info.length_size));
        for ps in &info.parameter_sets {
            dec.add_parameter_set(ps).map_err(|e| e.to_string())?;
        }
        if info.chroma_format_idc == 2 {
            log::info!("HEVC 4:2:2: decoding on {} ({})", display.node, display.vendor);
        }
        let handoff_off = std::env::var_os("FILMCRAFT_NO_GPU_IMPORT").is_some();
        Ok(Self { dec, info, fail_after: None, fed: 0, stopped: false, handoff: None, handoff_off, fail_handoff: false })
    }

    /// Test hook: from the `n`-th sample on, every decode fails as a lost device would.
    pub fn fail_after(&mut self, n: u64) {
        self.fail_after = Some(n);
    }

    /// 4:2:2: hand pictures to the compositing GPU (`on`, the default unless
    /// `FILMCRAFT_NO_GPU_IMPORT` is set) or read them back to memory.
    pub fn set_handoff(&mut self, on: bool) {
        self.handoff_off = !on;
        if !on {
            self.handoff = None;
        }
    }

    /// Test hook: the next 4:2:2 hand-off fails (as a refused copy or export); the stream reads
    /// pictures back from then on.
    pub fn fail_next_handoff(&mut self) {
        self.fail_handoff = true;
    }

    fn is_422(&self) -> bool {
        self.info.chroma_format_idc == 2
    }

    /// A decoded 4:2:2 picture copied into a linear surface and exported (fork 6.1b hand-off), or
    /// None (read it back instead): the hand-off is off, or failed now (then off for the stream).
    fn hand_off(&mut self, o: &AccelOutput<VaSurface>) -> Option<VideoFrame> {
        // the compositor refused such a picture this session (logged there): no copy at all
        if self.handoff_off || filmcraft_frame::packed_422_refused() {
            return None;
        }
        let (cx, cy, cw, ch) = o.meta.crop;
        let made = if std::mem::take(&mut self.fail_handoff) {
            Err("test hook".to_string())
        } else {
            if self.handoff.as_ref().is_none_or(|h| h.size != (cw, ch)) {
                self.handoff = None;
                match LinearCopy::new(o.surface.pool().display(), sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210, cw, ch, handoff_alloc()) {
                    Ok(c) => self.handoff = Some(c),
                    Err(e) => {
                        self.handoff_off = true;
                        handoff_failed(&e);
                        return None;
                    }
                }
            }
            self.handoff.as_ref().ok_or_else(|| "no copier".to_string()).and_then(|c| c.copy(&o.surface, (cx, cy, cw, ch)))
        };
        match made {
            Ok(linear) => {
                let read_from = keep_decoded(&linear).then(|| (o.surface.clone(), (cx, cy)));
                let pic = VaPicture { surface: Arc::new(linear), origin: (0, 0), ten_bit: true, read_from };
                Some(VideoFrame {
                    width: cw,
                    height: ch,
                    data: PixelData::Gpu(Arc::new(pic)),
                    color: self.info.color,
                    par: self.info.par,
                    pts: filmcraft_time::Tick::ZERO,
                })
            }
            Err(e) => {
                self.handoff = None;
                self.handoff_off = true;
                handoff_failed(&e);
                None
            }
        }
    }

    /// Decoded pictures as frames (synced: the decode has finished when they are handed out):
    /// GPU frames for 4:2:0, linear GPU copies for 4:2:2 (read back without the hand-off).
    fn convert(&mut self, outs: Vec<AccelOutput<VaSurface>>) -> filmcraft_codecs::Result<Vec<DecodedFrame>> {
        outs.into_iter()
            .map(|o| {
                o.surface.sync().map_err(|e| CodecError::Decode(format!("VA-API: {e}")))?;
                let frame = if o.surface.pool().fourcc == sys::VA_FOURCC_Y210 {
                    match self.hand_off(&o) {
                        Some(f) => f,
                        None => download(&o, &self.info)?,
                    }
                } else {
                    let (cx, cy, cw, ch) = o.meta.crop;
                    let pic = VaPicture { surface: o.surface.clone(), origin: (cx, cy), ten_bit: o.meta.bit_depth > 8, read_from: None };
                    VideoFrame {
                        width: cw,
                        height: ch,
                        data: PixelData::Gpu(Arc::new(pic)),
                        color: self.info.color,
                        par: self.info.par,
                        pts: filmcraft_time::Tick::ZERO,
                    }
                };
                Ok(DecodedFrame { pts: o.meta.pts, frame, draft: false })
            })
            .collect()
    }

    /// A 4:2:2 stream failed in hardware: the clip badges warn again, saying so (session-wide).
    fn note_failure(&self) {
        if self.is_422() {
            filmcraft_codecs::hw::note_hevc_422_failure();
        }
    }
}

/// A decoded picture in a VA surface, as a frame's GPU pixels.
pub struct VaPicture {
    /// The memory handed to the GPU (exported): the decoded surface (4:2:0) or its linear copy
    /// (4:2:2 hand-off).
    surface: VaSurface,
    origin: (u32, u32),
    ten_bit: bool,
    /// A linear copy's decoded surface and crop origin, read back for the CPU paths only when the
    /// copy cannot be mapped ([`keep_decoded`]; iHD's vaGetImage of a surface over imported
    /// memory returns zeros). None otherwise: the decoded surface is back in its pool after the
    /// copy, and read-back maps the copy.
    read_from: Option<(VaSurface, (u32, u32))>,
}

impl VaPicture {
    /// A decoded 4:2:0 picture in `surface` (NV12, or P010 when `ten_bit`), its visible picture
    /// starting at `origin`; read back from the surface itself. The fork's zero-copy accelerator
    /// for upstream's front ends ([`crate::va_accel`], Y1) makes its frames this way.
    pub(crate) fn new(surface: VaSurface, origin: (u32, u32), ten_bit: bool) -> Self {
        Self { surface, origin, ten_bit, read_from: None }
    }
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
        if self.surface.pool().fourcc == sys::VA_FOURCC_Y210 {
            filmcraft_frame::GpuFormat::Y210
        } else if self.ten_bit {
            filmcraft_frame::GpuFormat::P010
        } else {
            filmcraft_frame::GpuFormat::Nv12
        }
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
        match &self.read_from {
            Some((decoded, (x, y))) => read_back(decoded, (*x, *y, width, height)),
            None => read_back(&self.surface, (self.origin.0, self.origin.1, width, height)),
        }
    }
    fn gpu_bytes(&self) -> usize {
        let (w, h) = self.surface.pool().size;
        let px = w as usize * h as usize;
        if self.surface.pool().fourcc == sys::VA_FOURCC_Y210 {
            // packed 4:2:2, 16-bit samples: 4 bytes per pixel, plus the decoded surface it holds
            let decoded = self.read_from.as_ref().map_or(0, |(s, _)| (s.pool().size.0 as usize).saturating_mul(s.pool().size.1 as usize).saturating_mul(4));
            return px.saturating_mul(4).saturating_add(decoded);
        }
        (px * 3 / 2) * if self.ten_bit { 2 } else { 1 }
    }
}

/// Read the rectangle (x, y, w, h) of a surface back to memory, by the surface's format:
/// P010 → `Yuv16Bi` 4:2:0, Y210 → `Yuv16Bi` 4:2:2 (both MSB-aligned 10-bit), NV12 → planar `Yuv8`.
fn read_back(surface: &Surface, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> Result<PixelData, String> {
    // before allocating: the rectangle must lie in the surface
    let (sw, sh) = surface.pool().size;
    if cw == 0 || ch == 0 || cx.checked_add(cw).is_none_or(|r| r > sw) || cy.checked_add(ch).is_none_or(|b| b > sh) {
        return Err(format!("rectangle {cw}x{ch} at ({cx}, {cy}) outside the {sw}x{sh} surface"));
    }
    let (w, h) = (cw as usize, ch as usize);
    let (ccw, cch) = (w.div_ceil(2), h.div_ceil(2));
    let fourcc = surface.pool().fourcc;
    Ok(if fourcc == sys::VA_FOURCC_Y210 {
        // one packed plane, per two pixels Y0 Cb Y1 Cr (16-bit little-endian, MSB-aligned), split
        // into a luma plane and a Cb/Cr-interleaved plane at half width, full height (P210)
        let mut y = pool::take_u16_filled(w * h);
        let mut uv = pool::take_u16_filled(ccw * 2 * h);
        surface.read((cx, cy, cw, ch), |_, r, row| {
            let (Some(dy), Some(duv)) = (y.get_mut(r * w..(r + 1) * w), uv.get_mut(r * ccw * 2..(r + 1) * ccw * 2)) else { return };
            for (i, q) in row.as_chunks::<8>().0.iter().enumerate() {
                let s = |k: usize| u16::from_le_bytes([q[2 * k], q[2 * k + 1]]);
                if let Some(d) = dy.get_mut(2 * i) {
                    *d = s(0);
                }
                if let Some(d) = dy.get_mut(2 * i + 1) {
                    *d = s(2);
                }
                if let Some(d) = duv.get_mut(2 * i..2 * i + 2) {
                    d[0] = s(1);
                    d[1] = s(3);
                }
            }
        })?;
        PixelData::Yuv16Bi { y: Arc::new(y), uv: Arc::new(uv), chroma: Chroma::C422, bits: 10 }
    } else if fourcc == sys::VA_FOURCC_P010 {
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
    let data = read_back(&o.surface, (cx, cy, cw, ch)).map_err(|e| CodecError::Decode(format!("VA-API: {e}")))?;
    Ok(VideoFrame { width: cw, height: ch, data, color: info.color, par: info.par, pts: filmcraft_time::Tick::ZERO })
}

impl VideoDecoder for NativeVaDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> filmcraft_codecs::Result<Vec<DecodedFrame>> {
        self.fed += 1;
        if self.fail_after.is_some_and(|n| self.fed > n) {
            self.note_failure();
            return Err(CodecError::Decode("VA-API decoder failed (test hook)".into()));
        }
        if self.stopped {
            return Err(CodecError::Decode("VA-API HEVC 4:2:2: stopped after a failed picture (until the next seek)".into()));
        }
        let r = self.dec.decode(sample, pts).map_err(|e| CodecError::Decode(e.to_string())).and_then(|outs| self.convert(outs));
        if r.is_err() {
            self.stopped = self.is_422();
            self.note_failure();
        }
        r
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        if self.stopped {
            // nothing more goes to the engine: drop the pending picture and the DPB
            self.dec.reset();
            return Vec::new();
        }
        match self.dec.flush().map_err(|e| CodecError::Decode(e.to_string())).and_then(|o| self.convert(o)) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("VA-API HEVC: {e} while flushing");
                Vec::new()
            }
        }
    }

    fn reset(&mut self) {
        self.stopped = false;
        self.dec.reset();
    }

    fn name(&self) -> &str {
        if self.is_422() { NAME_422 } else { NAME }
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.info.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.info.is_disposable(sample)
    }
}

/// Decode a whole Annex B HEVC stream in process and read every picture back, in output order
/// (diagnostics and tests). The first SPS's chroma format picks the display (4:2:2 on
/// [`Display::for_hevc_422`]).
pub fn decode_annexb(data: &[u8]) -> Result<Vec<VideoFrame>, String> {
    decode_annexb_with(data, false)
}

/// [`decode_annexb`], with `handoff`: 4:2:2 pictures come out as the decoder hands them to the
/// compositing GPU (linear copies, `GpuFormat::Y210` GPU frames, fork 6.1b) instead of read back;
/// an error when the copy cannot be made. 4:2:0 pictures are read back either way.
pub fn decode_annexb_with(data: &[u8], handoff: bool) -> Result<Vec<VideoFrame>, String> {
    let mut dec = AccelDecoder::new(VaHevc::new());
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
    let mut copier: Option<LinearCopy> = None;
    outs.iter()
        .map(|o| {
            if !(handoff && o.surface.pool().fourcc == sys::VA_FOURCC_Y210) {
                return download(o, &info).map_err(|e| e.to_string());
            }
            let (cx, cy, cw, ch) = o.meta.crop;
            o.surface.sync()?;
            if copier.as_ref().is_none_or(|c| c.size != (cw, ch)) {
                copier = Some(LinearCopy::new(o.surface.pool().display(), sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210, cw, ch, handoff_alloc())?);
            }
            let linear = copier.as_ref().ok_or("no copier")?.copy(&o.surface, (cx, cy, cw, ch))?;
            let read_from = keep_decoded(&linear).then(|| (o.surface.clone(), (cx, cy)));
            let pic = VaPicture { surface: Arc::new(linear), origin: (0, 0), ten_bit: true, read_from };
            Ok(VideoFrame { width: cw, height: ch, data: PixelData::Gpu(Arc::new(pic)), color: info.color, par: info.par, pts: filmcraft_time::Tick::ZERO })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::tile_sizes;

    #[test]
    fn tile_sizes_follow_the_spec() {
        assert_eq!(tile_sizes(3, &[], 10, true, 20).ok(), Some(vec![3, 3, 4]));
        assert_eq!(tile_sizes(3, &[2, 5], 10, false, 20).ok(), Some(vec![2, 5, 3]));
        assert_eq!(tile_sizes(0, &[], 7, true, 20).ok(), Some(vec![7]));
    }

    /// Unsafe-code review round 2: the driver trusts these arrays, so hostile PPS values are
    /// refused instead of being truncated to VA's slots or wrapped (u32 overflow panicked in
    /// debug builds before for a huge picture).
    #[test]
    fn hostile_tile_sizes_are_refused() {
        assert!(tile_sizes(21, &[], 100, true, 20).is_err(), "more tiles than VA slots");
        assert!(tile_sizes(5, &[], 4, true, 20).is_err(), "more tiles than CTBs");
        assert!(tile_sizes(3, &[6, 5], 10, false, 20).is_err(), "explicit sizes past the picture");
        assert!(tile_sizes(3, &[5, 5], 10, false, 20).is_err(), "an empty last tile");
        assert!(tile_sizes(3, &[0, 5], 10, false, 20).is_err(), "an empty explicit tile");
        assert!(tile_sizes(3, &[4], 10, false, 20).is_err(), "too few explicit sizes");
        let huge = tile_sizes(20, &[], u32::MAX, true, 20).expect("no overflow");
        assert_eq!(huge.iter().map(|&x| u64::from(x)).sum::<u64>(), u64::from(u32::MAX));
    }

    /// Formats: 4:2:0 8/10-bit and 4:2:2 10-bit only; anything else (other depths, mixed
    /// depths, 4:0:0 / 4:4:4, hostile values) is refused before any driver call.
    #[test]
    fn va_formats() {
        use super::{sys, va_format};
        assert_eq!(
            va_format(2, 10, 10).map(|f| (f.0, f.1, f.2)),
            Ok((sys::VAProfile_VAProfileHEVCMain422_10, sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210))
        );
        assert_eq!(va_format(1, 10, 10).map(|f| f.2), Ok(sys::VA_FOURCC_P010));
        assert_eq!(va_format(1, 8, 8).map(|f| f.2), Ok(sys::VA_FOURCC_NV12));
        for (c, l, ch) in [(2, 8, 8), (2, 12, 12), (2, 10, 8), (1, 12, 12), (0, 8, 8), (3, 10, 10), (u32::MAX, u32::MAX, 0)] {
            assert!(va_format(c, l, ch).is_err(), "{c} {l}/{ch}");
        }
    }

    /// The range-extension structs (bindgen, `gen-va-sys.sh`) against the C compiler's view of
    /// libva 2.24's `va_dec_hevc.h` (GCC, x86-64 Linux): sizes, alignments, field offsets and the
    /// bit positions of the flag words, printed by a C program with `sizeof` / `offsetof` and
    /// each bit-field set alone. The driver reads base + rext from one buffer, so a wrong offset
    /// would hand it garbage.
    #[test]
    fn range_extension_layouts_match_the_header() {
        use super::sys::*;
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!((size_of::<VAPictureParameterBufferHEVC>(), align_of::<VAPictureParameterBufferHEVC>()), (604, 4));
        assert_eq!((size_of::<VAPictureParameterBufferHEVCRext>(), align_of::<VAPictureParameterBufferHEVCRext>()), (24, 4));
        assert_eq!((size_of::<VAPictureParameterBufferHEVCScc>(), align_of::<VAPictureParameterBufferHEVCScc>()), (780, 4));
        assert_eq!((size_of::<VAPictureParameterBufferHEVCExtension>(), align_of::<VAPictureParameterBufferHEVCExtension>()), (1408, 4));
        assert_eq!((size_of::<VASliceParameterBufferHEVC>(), align_of::<VASliceParameterBufferHEVC>()), (264, 4));
        assert_eq!((size_of::<VASliceParameterBufferHEVCRext>(), align_of::<VASliceParameterBufferHEVCRext>()), (188, 4));
        assert_eq!((size_of::<VASliceParameterBufferHEVCExtension>(), align_of::<VASliceParameterBufferHEVCExtension>()), (452, 4));
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, range_extension_pic_fields), 0);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, diff_cu_chroma_qp_offset_depth), 4);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, chroma_qp_offset_list_len_minus1), 5);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, log2_sao_offset_scale_luma), 6);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, log2_sao_offset_scale_chroma), 7);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, log2_max_transform_skip_block_size_minus2), 8);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, cb_qp_offset_list), 9);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCRext, cr_qp_offset_list), 15);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCExtension, base), 0);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCExtension, rext), 604);
        assert_eq!(offset_of!(VAPictureParameterBufferHEVCExtension, scc), 628);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, luma_offset_l0), 0);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, ChromaOffsetL0), 30);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, luma_offset_l1), 90);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, ChromaOffsetL1), 120);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, slice_ext_flags), 180);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCRext, slice_act_y_qp_offset), 184);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCExtension, base), 0);
        assert_eq!(offset_of!(VASliceParameterBufferHEVCExtension, rext), 264);
        // flag words (filled through `value` with bit i = the i-th field): each bindgen setter
        // alone sets that bit and no other, as the header's bit-fields do
        type Bits = _VAPictureParameterBufferHEVCRext__bindgen_ty_1__bindgen_ty_1;
        let setters: [fn(&mut Bits); 11] = [
            |b| b.set_transform_skip_rotation_enabled_flag(1),
            |b| b.set_transform_skip_context_enabled_flag(1),
            |b| b.set_implicit_rdpcm_enabled_flag(1),
            |b| b.set_explicit_rdpcm_enabled_flag(1),
            |b| b.set_extended_precision_processing_flag(1),
            |b| b.set_intra_smoothing_disabled_flag(1),
            |b| b.set_high_precision_offsets_enabled_flag(1),
            |b| b.set_persistent_rice_adaptation_enabled_flag(1),
            |b| b.set_cabac_bypass_alignment_enabled_flag(1),
            |b| b.set_cross_component_prediction_enabled_flag(1),
            |b| b.set_chroma_qp_offset_list_enabled_flag(1),
        ];
        for (i, set) in setters.iter().enumerate() {
            let mut b = Bits::default();
            set(&mut b);
            assert!((0..32).all(|k| b._bitfield_1.get_bit(k) == (k == i)), "rext pic flag {i}");
        }
        let mut sl = _VASliceParameterBufferHEVCRext__bindgen_ty_1__bindgen_ty_1::default();
        sl.set_cu_chroma_qp_offset_enabled_flag(1);
        assert!((0..32).all(|k| sl._bitfield_1.get_bit(k) == (k == 0)), "cu_chroma_qp_offset_enabled_flag");
        assert_eq!((VA_RT_FORMAT_YUV422_10, VA_FOURCC_Y210, VAProfile_VAProfileHEVCMain422_10), (512, 808530521, 24));
    }

    /// The video-processing structs of the 4:2:2 hand-off (fork 6.1b) against libva 2.24's
    /// `va_vpp.h` / `va_drmcommon.h` as GCC lays them out (x86-64 Linux; `sizeof` / `offsetof`
    /// printed by a C program built against the MIT-licensed headers), and the constants used.
    #[test]
    fn video_processing_layouts_match_the_header() {
        use crate::va_sys::*;
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!((size_of::<VARectangle>(), align_of::<VARectangle>()), (8, 2));
        assert_eq!((offset_of!(VARectangle, x), offset_of!(VARectangle, y), offset_of!(VARectangle, width), offset_of!(VARectangle, height)), (0, 2, 4, 6));
        assert_eq!((size_of::<VADRMFormatModifierList>(), align_of::<VADRMFormatModifierList>()), (16, 8));
        assert_eq!((offset_of!(VADRMFormatModifierList, num_modifiers), offset_of!(VADRMFormatModifierList, modifiers)), (0, 8));
        assert_eq!(size_of::<VAProcColorProperties>(), 8);
        type P = VAProcPipelineParameterBuffer;
        assert_eq!((size_of::<P>(), align_of::<P>()), (224, 8));
        let offsets = [
            offset_of!(P, surface),
            offset_of!(P, surface_region),
            offset_of!(P, surface_color_standard),
            offset_of!(P, output_region),
            offset_of!(P, output_background_color),
            offset_of!(P, output_color_standard),
            offset_of!(P, pipeline_flags),
            offset_of!(P, filter_flags),
            offset_of!(P, filters),
            offset_of!(P, num_filters),
            offset_of!(P, rotation_state),
            offset_of!(P, input_color_properties),
            offset_of!(P, output_color_properties),
            offset_of!(P, processing_mode),
            offset_of!(P, output_hdr_metadata),
            offset_of!(P, va_reserved),
        ];
        assert_eq!(offsets, [0, 8, 16, 24, 32, 36, 40, 44, 48, 56, 92, 132, 140, 148, 152, 160]);
        assert_eq!(
            (
                VABufferType_VAProcPipelineParameterBufferType,
                VASurfaceAttribType_VASurfaceAttribDRMFormatModifiers,
                VASurfaceAttribType_VASurfaceAttribUsageHint,
                VA_SURFACE_ATTRIB_USAGE_HINT_EXPORT,
                VAEntrypoint_VAEntrypointVideoProc,
                VAProfile_VAProfileNone,
            ),
            (41, 9, 8, 32, 10, -1)
        );
    }
}
