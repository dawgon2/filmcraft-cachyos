//! Writing HEVC parameter sets and slice segment headers (fork Phase 5.2b): the headers a
//! hardware encoder (VA-API) is given as packed headers. One layer, one sub-layer, 4:2:0 Main or
//! Main 10, or 4:2:2 Main 4:2:2 10 (fork 6E.4), I and P pictures with one reference (the previous picture), one slice per picture.
//! Every NAL unit comes out as its two-byte header plus the escaped RBSP, without a start code.

use filmcraft_bitstream::{BitWriter, escape_rbsp};

/// NAL unit types written here.
pub const NAL_TRAIL_R: u8 = 1;
pub const NAL_IDR_W_RADL: u8 = 19;
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;

/// Everything the parameter sets say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamParams {
    /// Picture size shown (the conformance window).
    pub width: u32,
    pub height: u32,
    /// Coded size: multiples of the minimum coding block, at least the shown size.
    pub coded_width: u32,
    pub coded_height: u32,
    /// 8 (Main) or 10 (Main 10).
    pub bit_depth: u32,
    /// 4:2:2 chroma (Main 4:2:2 10, a format range extensions profile; fork 6E.4) instead of 4:2:0.
    pub chroma_422: bool,
    /// general_level_idc (30 × the level) and the tier.
    pub level_idc: u8,
    pub high_tier: bool,
    pub log2_min_cb: u32,
    pub log2_ctb: u32,
    pub log2_min_tb: u32,
    pub log2_max_tb: u32,
    pub max_th_depth_inter: u32,
    pub max_th_depth_intra: u32,
    pub amp: bool,
    pub sao: bool,
    pub transform_skip: bool,
    /// cu_qp_delta_enabled_flag (rate control may change the QP inside a picture).
    pub cu_qp_delta: bool,
    pub log2_max_poc_lsb: u32,
    /// init_qp (26 + init_qp_minus26).
    pub init_qp: i32,
    /// dependent_slice_segments_enabled_flag (we write one slice per picture, so it only changes
    /// the PPS bits; radeonsi sets it, fork 6E.3 S8).
    pub dependent_slices: bool,
    /// The driver writes its own SPS / PPS whatever headers it is given (radeonsi): the
    /// transform depths, `dependent_slices` and `init_qp` here are what it writes and cannot
    /// change, so a shared stream must take them (fork 6E.3 S8, [`common_stream`]).
    pub rewritten: bool,
    /// H.273 colour primaries, transfer characteristics, matrix coefficients.
    pub colour: (u8, u8, u8),
    pub full_range: bool,
    /// (num_units_in_tick, time_scale): one picture lasts num_units_in_tick / time_scale s.
    pub timing: Option<(u32, u32)>,
}

impl StreamParams {
    /// Defaults for a `width`×`height` stream: 64×64 CTBs, 8×8 minimum coding blocks, transforms
    /// 4×4 to 32×32, coded size rounded up to 16, Rec. 709 limited range.
    pub fn new(width: u32, height: u32, bit_depth: u32) -> StreamParams {
        StreamParams {
            width,
            height,
            coded_width: width.div_ceil(16).saturating_mul(16),
            coded_height: height.div_ceil(16).saturating_mul(16),
            bit_depth,
            chroma_422: false,
            level_idc: level(width, height, 30, 1, 0).0,
            high_tier: false,
            log2_min_cb: 3,
            log2_ctb: 6,
            log2_min_tb: 2,
            log2_max_tb: 5,
            max_th_depth_inter: 3,
            max_th_depth_intra: 3,
            amp: true,
            sao: true,
            transform_skip: true,
            cu_qp_delta: false,
            log2_max_poc_lsb: 16,
            init_qp: 26,
            dependent_slices: false,
            rewritten: false,
            colour: (1, 1, 1),
            full_range: false,
            timing: None,
        }
    }

    /// The init_qp the PPS writes, in 1..=51: the encoder and the bitstream must use the same
    /// clamp, or the driver's QP and the decoder's QP drift apart (6E.3 review).
    pub fn init_qp_clamped(&self) -> i32 {
        self.init_qp.clamp(1, 51)
    }

    /// Coding tree blocks per picture.
    pub fn ctbs(&self) -> u32 {
        let ctb = 1u32 << self.log2_ctb.min(6);
        self.coded_width.div_ceil(ctb).saturating_mul(self.coded_height.div_ceil(ctb))
    }
}

/// One stream description both of two encoders can write (fork 6E.3, two-GPU export: identical
/// parameter sets let the joined chunks share one `hvcC`). Coding tools are the intersection
/// (smallest CTB and largest transform, largest minimum blocks, shallowest depths, a flag only
/// when both have it), the coded size the larger, the level / tier the stricter. A driver that
/// rewrites the headers ([`StreamParams::rewritten`]) fixes the transform depths,
/// `dependent_slices` and `init_qp`: the other side takes them if its depths reach that far (two
/// rewriting drivers must agree).
/// `None` when the two describe different pictures (size, bit depth, chroma, colour, timing, POC
/// setup), two rewriting drivers disagree, or the result is not a valid block structure.
pub fn common_stream(a: &StreamParams, b: &StreamParams) -> Option<StreamParams> {
    let same = a.width == b.width
        && a.height == b.height
        && a.bit_depth == b.bit_depth
        && a.chroma_422 == b.chroma_422
        && a.colour == b.colour
        && a.full_range == b.full_range
        && a.timing == b.timing
        && a.log2_max_poc_lsb == b.log2_max_poc_lsb;
    let fixed = |p: &StreamParams| (p.max_th_depth_inter, p.max_th_depth_intra, p.dependent_slices, p.init_qp);
    // the other side must reach the pinned depths: iHD codes garbage above its reported maximum
    // while the SPS says more (measured, fork 6E.3 S8), so radeonsi's 4 and iHD's 2 can't share
    let reaches = |fix: &StreamParams, other: &StreamParams| {
        other.rewritten || (fix.max_th_depth_inter <= other.max_th_depth_inter && fix.max_th_depth_intra <= other.max_th_depth_intra)
    };
    let pinned = match (a.rewritten, b.rewritten) {
        (true, true) if fixed(a) != fixed(b) => return None,
        (true, _) if !reaches(a, b) => return None,
        (_, true) if !reaches(b, a) => return None,
        (true, _) => Some(fixed(a)),
        (_, true) => Some(fixed(b)),
        (false, false) if a.init_qp != b.init_qp || a.dependent_slices != b.dependent_slices => return None,
        (false, false) => None,
    };
    if !same {
        return None;
    }
    let log2_ctb = a.log2_ctb.min(b.log2_ctb);
    let log2_min_cb = a.log2_min_cb.max(b.log2_min_cb);
    let log2_min_tb = a.log2_min_tb.max(b.log2_min_tb);
    let log2_max_tb = a.log2_max_tb.min(b.log2_max_tb);
    // 7.4.3.2.1: 8 ≤ MinCb ≤ CTB ≤ 64, 4 ≤ MinTb < MinCb, MinTb ≤ MaxTb ≤ min(CTB, 32)
    let valid = (3..=6).contains(&log2_ctb)
        && (3..=log2_ctb).contains(&log2_min_cb)
        && log2_min_tb >= 2
        && log2_min_tb < log2_min_cb
        && log2_min_tb <= log2_max_tb
        && log2_max_tb <= log2_ctb.min(5);
    if !valid {
        return None;
    }
    let max_depth = log2_ctb - log2_min_tb;
    let mcb = 1u32 << log2_min_cb;
    let align = |v: u32| v.div_ceil(mcb).saturating_mul(mcb);
    let (level_idc, high_tier) = if (a.level_idc, a.high_tier) >= (b.level_idc, b.high_tier) { (a.level_idc, a.high_tier) } else { (b.level_idc, b.high_tier) };
    Some(StreamParams {
        coded_width: align(a.coded_width.max(b.coded_width)),
        coded_height: align(a.coded_height.max(b.coded_height)),
        level_idc,
        high_tier: a.high_tier || b.high_tier || high_tier,
        log2_min_cb,
        log2_ctb,
        log2_min_tb,
        log2_max_tb,
        max_th_depth_inter: a.max_th_depth_inter.min(b.max_th_depth_inter).min(max_depth),
        max_th_depth_intra: a.max_th_depth_intra.min(b.max_th_depth_intra).min(max_depth),
        rewritten: a.rewritten || b.rewritten,
        amp: a.amp && b.amp,
        sao: a.sao && b.sao,
        transform_skip: a.transform_skip && b.transform_skip,
        cu_qp_delta: a.cu_qp_delta && b.cu_qp_delta,
        ..a.clone()
    })
    .and_then(|mut c| {
        if let Some((inter, intra, dependent, qp)) = pinned {
            // the rewriting driver's values, which must still fit the shared block structure
            if inter > max_depth || intra > max_depth {
                return None;
            }
            (c.max_th_depth_inter, c.max_th_depth_intra, c.dependent_slices, c.init_qp) = (inter, intra, dependent, qp);
        }
        Some(c)
    })
}

/// Level limits (Table A.8): level_idc, MaxLumaPs, MaxLumaSr, MaxBR for the Main and High tiers
/// (kbit/s, Main / Main 10 profile).
const LEVELS: [(u8, u64, u64, u64, u64); 13] = [
    (30, 36_864, 552_960, 128, 128),
    (60, 122_880, 3_686_400, 1_500, 1_500),
    (63, 245_760, 7_372_800, 3_000, 3_000),
    (90, 552_960, 16_588_800, 6_000, 6_000),
    (93, 983_040, 33_177_600, 10_000, 10_000),
    (120, 2_228_224, 66_846_720, 12_000, 30_000),
    (123, 2_228_224, 133_693_440, 20_000, 50_000),
    (150, 8_912_896, 267_386_880, 25_000, 100_000),
    (153, 8_912_896, 534_773_760, 40_000, 160_000),
    (156, 8_912_896, 1_069_547_520, 60_000, 240_000),
    (180, 35_651_584, 1_069_547_520, 60_000, 240_000),
    (183, 35_651_584, 2_139_095_040, 120_000, 480_000),
    (186, 35_651_584, 4_278_190_080, 240_000, 800_000),
];

/// The lowest level (and tier: high only when the main tier's bit rate is too low) for a
/// `width`×`height` stream at `rate_num / rate_den` pictures per second and `kbps` (0: unknown).
/// Beyond level 6.2: 6.2, high tier.
pub fn level(width: u32, height: u32, rate_num: u32, rate_den: u32, kbps: u32) -> (u8, bool) {
    let ps = width as u64 * height as u64;
    let side = width.max(height) as u64;
    let sr = ps.saturating_mul(rate_num as u64) / (rate_den.max(1) as u64);
    for &(idc, max_ps, max_sr, main_br, high_br) in &LEVELS {
        // each side at most sqrt(8 × MaxLumaPs)
        if ps > max_ps || side.saturating_mul(side) > max_ps.saturating_mul(8) || sr > max_sr {
            continue;
        }
        if (kbps as u64) <= main_br {
            return (idc, false);
        }
        if idc >= 120 && (kbps as u64) <= high_br {
            return (idc, true);
        }
    }
    (186, true)
}

fn nal(t: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![t << 1, 1];
    out.extend(escape_rbsp(rbsp));
    out
}

fn profile_tier_level(w: &mut BitWriter, p: &StreamParams) {
    let profile = if p.chroma_422 {
        4
    } else if p.bit_depth > 8 {
        2
    } else {
        1
    };
    w.write_bits(0, 2); // general_profile_space
    w.write_bit(p.high_tier);
    w.write_bits(profile, 5);
    // compatibility: a Main stream is also Main 10; a range extensions stream only itself
    w.write_bits(
        match profile {
            1 => 0x6000_0000,
            2 => 0x2000_0000,
            _ => 0x0800_0000,
        },
        32,
    );
    w.write_bit(true); // progressive_source
    w.write_bit(false); // interlaced_source
    w.write_bit(true); // non_packed_constraint
    w.write_bit(true); // frame_only_constraint
    if profile == 4 {
        // Main 4:2:2 10 (Table A.2): max_12bit, max_10bit, (not max_8bit), max_422chroma, (not
        // max_420chroma, monochrome, intra, one_picture_only), lower_bit_rate
        w.write_bits(0b1_1010_0001, 9);
        w.write_bits(0, 32); // 34 reserved bits + general_inbld_flag
        w.write_bits(0, 3);
    } else {
        w.write_bits(0, 32); // 43 reserved bits + general_inbld_flag
        w.write_bits(0, 12);
    }
    w.write_bits(p.level_idc as u32, 8);
}

/// Sub-layer ordering for one sub-layer: two pictures in the DPB (current + reference), no
/// reordering.
fn ordering(w: &mut BitWriter) {
    w.write_bit(true); // sub_layer_ordering_info_present_flag
    w.write_ue(1); // max_dec_pic_buffering_minus1
    w.write_ue(0); // max_num_reorder_pics
    w.write_ue(0); // max_latency_increase_plus1
}

/// The video parameter set (NAL unit).
pub fn vps(p: &StreamParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // vps_video_parameter_set_id
    w.write_bits(3, 2); // base layer internal, available
    w.write_bits(0, 6); // vps_max_layers_minus1
    w.write_bits(0, 3); // vps_max_sub_layers_minus1
    w.write_bit(true); // vps_temporal_id_nesting_flag
    w.write_bits(0xffff, 16);
    profile_tier_level(&mut w, p);
    ordering(&mut w);
    w.write_bits(0, 6); // vps_max_layer_id
    w.write_ue(0); // vps_num_layer_sets_minus1
    match p.timing {
        Some((tick, scale)) => {
            w.write_bit(true);
            w.write_bits(tick, 32);
            w.write_bits(scale, 32);
            w.write_bit(false); // vps_poc_proportional_to_timing_flag
            w.write_ue(0); // vps_num_hrd_parameters
        }
        None => w.write_bit(false),
    }
    w.write_bit(false); // vps_extension_flag
    w.rbsp_trailing();
    nal(NAL_VPS, &w.finish())
}

/// The sequence parameter set (NAL unit).
pub fn sps(p: &StreamParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // sps_video_parameter_set_id
    w.write_bits(0, 3); // sps_max_sub_layers_minus1
    w.write_bit(true); // sps_temporal_id_nesting_flag
    profile_tier_level(&mut w, p);
    w.write_ue(0); // sps_seq_parameter_set_id
    w.write_ue(if p.chroma_422 { 2 } else { 1 }); // chroma_format_idc 4:2:2 / 4:2:0
    w.write_ue(p.coded_width);
    w.write_ue(p.coded_height);
    // conformance window in chroma samples (SubWidthC = 2; SubHeightC = 1 for 4:2:2, else 2)
    let sub_h = if p.chroma_422 { 1 } else { 2 };
    let (right, bottom) = (p.coded_width.saturating_sub(p.width) / 2, p.coded_height.saturating_sub(p.height) / sub_h);
    w.write_bit(right > 0 || bottom > 0);
    if right > 0 || bottom > 0 {
        w.write_ue(0);
        w.write_ue(right);
        w.write_ue(0);
        w.write_ue(bottom);
    }
    w.write_ue(p.bit_depth.saturating_sub(8));
    w.write_ue(p.bit_depth.saturating_sub(8));
    w.write_ue(p.log2_max_poc_lsb.clamp(4, 16) - 4);
    ordering(&mut w);
    w.write_ue(p.log2_min_cb.saturating_sub(3));
    w.write_ue(p.log2_ctb.saturating_sub(p.log2_min_cb));
    w.write_ue(p.log2_min_tb.saturating_sub(2));
    w.write_ue(p.log2_max_tb.saturating_sub(p.log2_min_tb));
    w.write_ue(p.max_th_depth_inter);
    w.write_ue(p.max_th_depth_intra);
    w.write_bit(false); // scaling_list_enabled_flag
    w.write_bit(p.amp);
    w.write_bit(p.sao);
    w.write_bit(false); // pcm_enabled_flag
    w.write_ue(0); // num_short_term_ref_pic_sets (each slice carries its own)
    w.write_bit(false); // long_term_ref_pics_present_flag
    w.write_bit(false); // sps_temporal_mvp_enabled_flag
    w.write_bit(false); // strong_intra_smoothing_enabled_flag
    w.write_bit(true); // vui_parameters_present_flag
    // VUI
    w.write_bit(true); // aspect_ratio_info_present_flag
    w.write_bits(1, 8); // square pixels
    w.write_bit(false); // overscan_info_present_flag
    w.write_bit(true); // video_signal_type_present_flag
    w.write_bits(5, 3); // video_format: unspecified
    w.write_bit(p.full_range);
    w.write_bit(true); // colour_description_present_flag
    w.write_bits(p.colour.0 as u32, 8);
    w.write_bits(p.colour.1 as u32, 8);
    w.write_bits(p.colour.2 as u32, 8);
    w.write_bit(false); // chroma_loc_info_present_flag
    w.write_bit(false); // neutral_chroma_indication_flag
    w.write_bit(false); // field_seq_flag
    w.write_bit(false); // frame_field_info_present_flag
    w.write_bit(false); // default_display_window_flag
    match p.timing {
        Some((tick, scale)) => {
            w.write_bit(true);
            w.write_bits(tick, 32);
            w.write_bits(scale, 32);
            w.write_bit(false); // vui_poc_proportional_to_timing_flag
            w.write_bit(false); // vui_hrd_parameters_present_flag
        }
        None => w.write_bit(false),
    }
    w.write_bit(false); // bitstream_restriction_flag
    w.write_bit(false); // sps_extension_present_flag
    w.rbsp_trailing();
    nal(NAL_SPS, &w.finish())
}

/// The picture parameter set (NAL unit).
pub fn pps(p: &StreamParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // pps_pic_parameter_set_id
    w.write_ue(0); // pps_seq_parameter_set_id
    w.write_bit(p.dependent_slices); // dependent_slice_segments_enabled_flag
    w.write_bit(false); // output_flag_present_flag
    w.write_bits(0, 3); // num_extra_slice_header_bits
    w.write_bit(false); // sign_data_hiding_enabled_flag
    w.write_bit(false); // cabac_init_present_flag
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_se(p.init_qp_clamped() - 26);
    w.write_bit(false); // constrained_intra_pred_flag
    w.write_bit(p.transform_skip);
    w.write_bit(p.cu_qp_delta);
    if p.cu_qp_delta {
        w.write_ue(0); // diff_cu_qp_delta_depth
    }
    w.write_se(0); // pps_cb_qp_offset
    w.write_se(0); // pps_cr_qp_offset
    w.write_bit(false); // pps_slice_chroma_qp_offsets_present_flag
    w.write_bit(false); // weighted_pred_flag
    w.write_bit(false); // weighted_bipred_flag
    w.write_bit(false); // transquant_bypass_enabled_flag
    w.write_bit(false); // tiles_enabled_flag
    w.write_bit(false); // entropy_coding_sync_enabled_flag
    w.write_bit(true); // pps_loop_filter_across_slices_enabled_flag
    w.write_bit(false); // deblocking_filter_control_present_flag
    w.write_bit(false); // pps_scaling_list_data_present_flag
    w.write_bit(false); // lists_modification_present_flag
    w.write_ue(0); // log2_parallel_merge_level_minus2
    w.write_bit(false); // slice_segment_header_extension_present_flag
    w.write_bit(false); // pps_extension_present_flag
    w.rbsp_trailing();
    nal(NAL_PPS, &w.finish())
}

/// One picture's slice (the whole picture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlicePic {
    /// An IDR picture (intra, POC 0); otherwise a P picture predicted from the previous one.
    pub idr: bool,
    /// Picture order count (since the IDR).
    pub poc: u32,
    /// slice_qp_delta (QP − init_qp).
    pub qp_delta: i32,
    /// A non-IDR picture as low-delay B (fork 6E.3 S12): slice_type B, the same RPS (one picture
    /// before), so both lists hold that picture; `mvd_l1_zero_flag` 0 (iHD's LP encoder codes list
    /// 1 motion vector differences: 1 desyncs its slice data, measured). No reordering: it is
    /// still predicted only from the past. Ignored for an IDR.
    pub low_delay_b: bool,
}

/// The slice segment header NAL unit up to and including `byte_alignment()`: the encoder appends
/// the slice data. Its last byte is never 0 (it holds the alignment's one bit), so nothing needs
/// escaping where the data starts.
pub fn slice_header(p: &StreamParams, s: &SlicePic) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bit(true); // first_slice_segment_in_pic_flag
    if s.idr {
        w.write_bit(false); // no_output_of_prior_pics_flag
    }
    w.write_ue(0); // slice_pic_parameter_set_id
    let b = !s.idr && s.low_delay_b;
    w.write_ue(match (s.idr, b) {
        (true, _) => 2,      // I
        (false, true) => 0,  // B
        (false, false) => 1, // P
    });
    if !s.idr {
        let bits = p.log2_max_poc_lsb.clamp(4, 16);
        w.write_bits(s.poc & ((1u32 << bits) - 1), bits);
        w.write_bit(false); // short_term_ref_pic_set_sps_flag
        // st_ref_pic_set(0): one picture before, used
        w.write_ue(1); // num_negative_pics
        w.write_ue(0); // num_positive_pics
        w.write_ue(0); // delta_poc_s0_minus1
        w.write_bit(true); // used_by_curr_pic_s0_flag
    }
    if p.sao {
        w.write_bit(true); // slice_sao_luma_flag
        w.write_bit(true); // slice_sao_chroma_flag
    }
    if !s.idr {
        // the PPS defaults: one entry in list 0 (and list 1 for B, which the RPS fills with the
        // same picture: RefPicList1 takes StCurrAfter, empty, then StCurrBefore)
        w.write_bit(false); // num_ref_idx_active_override_flag
        if b {
            w.write_bit(false); // mvd_l1_zero_flag (see SlicePic::low_delay_b)
        }
        // no cabac_init_flag (not present in the PPS), no collocated_* (temporal MVP off:
        // collocated_from_l0 is inferred 1), no weighted prediction
        w.write_ue(0); // five_minus_max_num_merge_cand
    }
    let init = p.init_qp_clamped();
    w.write_se(s.qp_delta.clamp(-(init + 6 * (p.bit_depth as i32 - 8)), 51 - init));
    // slice_loop_filter_across_slices_enabled_flag (pps flag set; SAO on or deblocking on)
    w.write_bit(false);
    // byte_alignment()
    w.write_bit(true);
    w.align_zero();
    nal(if s.idr { NAL_IDR_W_RADL } else { NAL_TRAIL_R }, &w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Pps, Sps, Vps};
    use crate::slice::{NalHeader, SliceHeader};
    use std::sync::Arc;

    #[test]
    fn init_qp_clamp_matches_the_encoders() {
        // the VA encoder clamps init_qp to 1..=51; the PPS / slice header must use the same range
        for (qp, want) in [(-5, 1), (0, 1), (1, 1), (26, 26), (51, 51), (99, 51)] {
            let p = StreamParams { init_qp: qp, ..StreamParams::new(64, 64, 8) };
            assert_eq!(p.init_qp_clamped(), want, "init_qp {qp}");
        }
    }

    fn rbsp(nal: &[u8]) -> Vec<u8> {
        filmcraft_bitstream::unescape_rbsp(&nal[2..])
    }

    fn params(bit_depth: u32) -> StreamParams {
        let mut p = StreamParams::new(6240, 4160, bit_depth);
        p.colour = (9, 16, 9);
        p.timing = Some((1001, 30000));
        p.cu_qp_delta = true;
        p.init_qp = 30;
        p
    }

    #[test]
    fn parameter_sets_parse_back() {
        for (bd, (w, h)) in [(10, (6240, 4160)), (8, (1918, 1080)), (10, (2160, 3840))] {
            let mut p = params(bd);
            (p.width, p.height) = (w, h);
            (p.coded_width, p.coded_height) = (w.div_ceil(16) * 16, h.div_ceil(16) * 16);
            Vps::parse(&rbsp(&vps(&p))).expect("vps");
            let sps = Sps::parse(&rbsp(&sps(&p))).expect("sps");
            assert_eq!((sps.width, sps.height), (p.coded_width, p.coded_height));
            let (l, r, t, b) = sps.conf_win;
            assert_eq!((sps.width - (l + r) * 2, sps.height - (t + b) * 2), (w, h));
            assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (bd, bd));
            assert_eq!((sps.log2_min_cb, sps.log2_ctb, sps.log2_min_tb, sps.log2_max_tb), (3, 6, 2, 5));
            assert_eq!(sps.log2_max_poc_lsb, 16);
            assert!(sps.amp && sps.sao && !sps.temporal_mvp);
            let vui = sps.vui.as_ref().expect("vui");
            assert_eq!((vui.colour_primaries, vui.transfer_characteristics, vui.matrix_coefficients), (9, 16, 9));
            assert!(!vui.full_range);
            assert_eq!(vui.timing, Some((1001, 30000)));
            assert_eq!(sps.max_dec_pic_buffering, 2);
            assert_eq!(sps.ptl.profile_idc as u32, if bd == 10 { 2 } else { 1 });
            let pps = Pps::parse(&rbsp(&pps(&p))).expect("pps");
            assert_eq!(pps.init_qp, 30);
            assert!(pps.cu_qp_delta_enabled);
        }
    }

    /// Main 4:2:2 10 (fork 6E.4): profile 4 with its constraint flags, chroma_format_idc 2, and a
    /// conformance window in 4:2:2 chroma units (half width, full height).
    #[test]
    fn parameter_sets_422_parse_back() {
        for (w, h) in [(6240, 4160), (1918, 1078), (2160, 3838)] {
            let mut p = params(10);
            p.chroma_422 = true;
            (p.width, p.height) = (w, h);
            (p.coded_width, p.coded_height) = (w.div_ceil(16) * 16, h.div_ceil(16) * 16);
            let v = vps(&p);
            Vps::parse(&rbsp(&v)).expect("vps");
            let s = sps(&p);
            let sps = Sps::parse(&rbsp(&s)).expect("sps");
            assert_eq!(sps.chroma_format_idc, 2);
            assert_eq!((sps.ptl.profile_idc, sps.ptl.compatibility), (4, 0x0800_0000));
            assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (10, 10));
            let (l, r, t, b) = sps.conf_win;
            assert_eq!((sps.width - (l + r) * 2, sps.height - (t + b)), (w, h));
            // the nine format range extensions flags right after the four source flags: bit
            // 44 + 4 of the profile_tier_level (2 + 1 + 5 + 32 + 4 bits in), RBSP after the
            // 2-byte NAL header and the SPS's first byte
            let rb = rbsp(&s);
            let bit = |n: usize| (rb.get(n / 8).copied().unwrap_or(0) >> (7 - n % 8)) & 1;
            let flags: Vec<u8> = (0..9).map(|i| bit(8 + 44 + i)).collect();
            assert_eq!(flags, vec![1, 1, 0, 1, 0, 0, 0, 0, 1]);
            // the stream after it still parses: same level byte position as 4:2:0
            assert_eq!(sps.ptl.level_idc, p.level_idc);
        }
    }

    #[test]
    fn slice_headers_parse_back() {
        let p = params(10);
        let (sps, pps) = (Arc::new(Sps::parse(&rbsp(&sps(&p))).expect("sps")), Arc::new(Pps::parse(&rbsp(&pps(&p))).expect("pps")));
        let pic = |idr, poc, qp_delta| SlicePic { idr, poc, qp_delta, low_delay_b: false };
        for s in [pic(true, 0, -4), pic(false, 7, 3), pic(false, 70_000, 0), SlicePic { low_delay_b: true, ..pic(true, 0, 2) }] {
            let nal = slice_header(&p, &s);
            assert_ne!(nal.last(), Some(&0));
            let header = NalHeader::parse(&nal).expect("nal header");
            assert_eq!(header.is_idr(), s.idr);
            let r = rbsp(&nal);
            let (sh, _, _) = SliceHeader::parse(&r, header, |_| Ok((pps.clone(), sps.clone())), None).expect("slice header");
            assert!(sh.first_slice_segment_in_pic);
            assert_eq!(sh.qp_delta, s.qp_delta);
            assert_eq!(sh.data_offset, r.len(), "the header ends at the byte alignment");
            if !s.idr {
                assert_eq!(sh.poc_lsb, s.poc & 0xffff);
                assert_eq!(sh.st_rps.s0, vec![(-1, true)]);
                assert!(sh.st_rps.s1.is_empty());
                assert_eq!(sh.num_ref_idx[0], 1);
                assert_eq!(sh.max_num_merge_cand, 5);
            }
            assert!(sh.sao_luma && sh.sao_chroma);
            assert!(!sh.is_b(), "an IDR stays I even when low_delay_b is asked for");
        }
    }

    /// Fork 6E.3 S12: with `low_delay_b` false the IDR and P slice headers are byte for byte what
    /// the writer before S12 produced (bytes captured from 94c32319's writer), SAO on and off.
    #[test]
    fn i_and_p_slice_headers_unchanged_by_low_delay_b() {
        #[rustfmt::skip]
        let golden: [(bool, bool, u32, i32, &[u8]); 8] = [
            (true, true, 0, -4, &[38, 1, 175, 18, 128]),
            (true, false, 7, 3, &[2, 1, 208, 0, 57, 125, 50]),
            (true, false, 70000, 0, &[2, 1, 208, 139, 129, 125, 160]),
            (true, false, 1, -12, &[2, 1, 208, 0, 9, 125, 12, 160]),
            (false, true, 0, -4, &[38, 1, 172, 74]),
            (false, false, 7, 3, &[2, 1, 208, 0, 57, 116, 200]),
            (false, false, 70000, 0, &[2, 1, 208, 139, 129, 118, 128]),
            (false, false, 1, -12, &[2, 1, 208, 0, 9, 116, 50, 128]),
        ];
        for (sao, idr, poc, qp_delta, want) in golden {
            let p = StreamParams { sao, ..params(10) };
            let got = slice_header(&p, &SlicePic { idr, poc, qp_delta, low_delay_b: false });
            assert_eq!(got, want, "sao {sao} idr {idr} poc {poc} qp_delta {qp_delta}");
            if idr {
                // an IDR ignores low_delay_b
                assert_eq!(slice_header(&p, &SlicePic { idr, poc, qp_delta, low_delay_b: true }), want);
            }
        }
    }

    /// Fork 6E.3 S12: a low-delay B slice header (Intel's `EncSliceLP`) parses back as B with one
    /// entry in each list, the same picture before (POC − 1), `mvd_l1_zero_flag` 0 and
    /// `collocated_from_l0` 1; for every SAO / init_qp setup and through the decoder's list build.
    #[test]
    fn low_delay_b_slice_headers_parse_back() {
        for sao in [true, false] {
            let p = StreamParams { sao, ..params(10) };
            let (sps, pps) = (Arc::new(Sps::parse(&rbsp(&sps(&p))).expect("sps")), Arc::new(Pps::parse(&rbsp(&pps(&p))).expect("pps")));
            for (poc, qp_delta) in [(1u32, 0i32), (29, -5), (70_000, 7)] {
                let s = SlicePic { idr: false, poc, qp_delta, low_delay_b: true };
                let nal = slice_header(&p, &s);
                assert_ne!(nal.last(), Some(&0));
                let header = NalHeader::parse(&nal).expect("nal header");
                assert!(!header.is_idr());
                let r = rbsp(&nal);
                let (sh, _, _) = SliceHeader::parse(&r, header, |_| Ok((pps.clone(), sps.clone())), None).expect("slice header");
                assert!(sh.is_b(), "slice type B");
                assert_eq!(sh.poc_lsb, poc & 0xffff);
                assert_eq!(sh.st_rps.s0, vec![(-1, true)], "one picture before");
                assert!(sh.st_rps.s1.is_empty(), "nothing after: no reordering");
                assert_eq!((sh.num_ref_idx[0], sh.num_ref_idx[1]), (1, 1));
                assert!(!sh.mvd_l1_zero && sh.collocated_from_l0);
                assert_eq!(sh.max_num_merge_cand, 5);
                assert_eq!(sh.qp_delta, qp_delta);
                assert_eq!((sh.sao_luma, sh.sao_chroma), (sao, sao));
                assert_eq!(sh.data_offset, r.len(), "the header ends at the byte alignment");
                // one bit longer than the P header (mvd_l1_zero_flag), so never the same bytes
                assert_ne!(nal, slice_header(&p, &SlicePic { low_delay_b: false, ..s }));
            }
        }
    }

    #[test]
    fn levels_follow_table_a8() {
        assert_eq!(level(3840, 2160, 30, 1, 0), (150, false));
        assert_eq!(level(3840, 2160, 60, 1, 0), (153, false));
        assert_eq!(level(3840, 2160, 30, 1, 80_000), (150, true));
        assert_eq!(level(6240, 4160, 30000, 1001, 200_000), (180, true));
        assert_eq!(level(1920, 1080, 30, 1, 0), (120, false));
        assert_eq!(level(2160, 3840, 30, 1, 0), (150, false));
        // beyond everything
        assert_eq!(level(8192, 8192, 300, 1, 10_000_000), (186, true));
        assert_eq!(level(0, 0, 0, 0, 0), (30, false));
    }

    #[test]
    fn common_stream_intersects_two_drivers() {
        let a = StreamParams::new(6240, 4160, 10);
        // identical: unchanged
        assert_eq!(common_stream(&a, &a), Some(a.clone()));
        // a driver with 32×32 CTBs, 16×16 minimum CBs, no AMP / SAO and a higher level
        let b = StreamParams {
            log2_ctb: 5,
            log2_min_cb: 4,
            log2_max_tb: 5,
            amp: false,
            sao: false,
            max_th_depth_inter: 4,
            level_idc: 183,
            high_tier: true,
            ..a.clone()
        };
        let c = common_stream(&a, &b).expect("compatible");
        assert_eq!(common_stream(&b, &a).as_ref(), Some(&c), "order does not matter");
        assert_eq!((c.log2_ctb, c.log2_min_cb, c.log2_min_tb, c.log2_max_tb), (5, 4, 2, 5));
        assert!(!c.amp && !c.sao && c.transform_skip);
        assert_eq!((c.max_th_depth_inter, c.max_th_depth_intra), (3, 3));
        assert_eq!((c.level_idc, c.high_tier), (183, true));
        assert_eq!((c.coded_width % 16, c.coded_height % 16), (0, 0));
        assert!(c.coded_width >= 6240 && c.coded_height >= 4160);
        // and it is a stream our parser reads back with those tools
        let sps = Sps::parse(&rbsp(&sps(&c))).expect("sps");
        assert_eq!((sps.log2_min_cb, sps.log2_ctb, sps.log2_min_tb, sps.log2_max_tb), (4, 5, 2, 5));
        assert_eq!((sps.width, sps.height), (c.coded_width, c.coded_height));
        Pps::parse(&rbsp(&pps(&c))).expect("pps");
    }

    #[test]
    fn common_stream_refuses_different_pictures_and_bad_blocks() {
        let a = StreamParams::new(3840, 2160, 10);
        for b in [
            StreamParams::new(3840, 2176, 10),
            StreamParams::new(3840, 2160, 8),
            StreamParams { chroma_422: true, ..a.clone() },
            StreamParams { timing: Some((1001, 30000)), ..a.clone() },
            StreamParams { colour: (9, 16, 9), ..a.clone() },
            StreamParams { init_qp: 30, ..a.clone() },
            // min CB above the other's CTB: no common block structure
            StreamParams { log2_ctb: 6, log2_min_cb: 6, ..a.clone() },
            // hostile values from a driver
            StreamParams { log2_ctb: u32::MAX, log2_min_cb: u32::MAX, log2_min_tb: u32::MAX, ..a.clone() },
        ] {
            let small = StreamParams { log2_ctb: 4, ..a.clone() };
            let pair = if b.log2_min_cb == 6 { (&small, &b) } else { (&a, &b) };
            assert_eq!(common_stream(pair.0, pair.1), None, "{b:?}");
        }
    }

    /// Fork 6E.3 S8: a driver that rewrites the SPS / PPS (radeonsi) fixes the transform depths,
    /// the dependent-slices flag and init_qp; the shared stream takes them, both orders.
    #[test]
    fn common_stream_takes_a_rewriting_drivers_values() {
        // a driver that can code depth 4 (iHD reports 2 and can't: see the last asserts)
        let intel = StreamParams { max_th_depth_inter: 4, max_th_depth_intra: 4, init_qp: 22, ..StreamParams::new(6240, 4160, 10) };
        let amd = StreamParams { max_th_depth_inter: 4, max_th_depth_intra: 4, dependent_slices: true, init_qp: 26, rewritten: true, ..intel.clone() };
        for (x, y) in [(&intel, &amd), (&amd, &intel)] {
            let c = common_stream(x, y).expect("shared");
            assert_eq!((c.max_th_depth_inter, c.max_th_depth_intra, c.dependent_slices, c.init_qp, c.rewritten), (4, 4, true, 26, true));
            // the same stream again: each side accepts it unchanged (VaHevcEncoder::new_on_with)
            assert_eq!(common_stream(x, &c).as_ref(), Some(&c));
            let pps = Pps::parse(&rbsp(&pps(&c))).expect("pps");
            assert!(pps.dependent_slice_segments_enabled);
            assert_eq!(pps.init_qp, 26);
        }
        // two rewriting drivers that disagree, and depths the block structure can't hold
        assert_eq!(common_stream(&amd, &StreamParams { init_qp: 30, ..amd.clone() }), None);
        assert_eq!(common_stream(&intel, &StreamParams { max_th_depth_inter: 9, ..amd.clone() }), None);
        // iHD on Mesa 26 / iHD 26.3: depth 2 at most, radeonsi writes 4: no shared stream (hev1)
        let ihd = StreamParams { max_th_depth_inter: 2, max_th_depth_intra: 2, ..intel.clone() };
        assert_eq!(common_stream(&ihd, &amd), None);
        assert_eq!(common_stream(&amd, &ihd), None);
        // without a rewriting driver, init_qp and the flag still have to match
        assert_eq!(common_stream(&intel, &StreamParams { dependent_slices: true, ..intel.clone() }), None);
    }

    #[test]
    fn ctb_count() {
        let p = StreamParams::new(3840, 2160, 10);
        assert_eq!(p.ctbs(), 60 * 34);
    }
}
