//! Writing HEVC parameter sets and slice segment headers (fork Phase 5.2b): the headers a
//! hardware encoder (VA-API) is given as packed headers. One layer, one sub-layer, 4:2:0, Main or
//! Main 10, I and P pictures with one reference (the previous picture), one slice per picture.
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
            colour: (1, 1, 1),
            full_range: false,
            timing: None,
        }
    }

    /// Coding tree blocks per picture.
    pub fn ctbs(&self) -> u32 {
        let ctb = 1u32 << self.log2_ctb.min(6);
        self.coded_width.div_ceil(ctb).saturating_mul(self.coded_height.div_ceil(ctb))
    }
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
    let profile = if p.bit_depth > 8 { 2 } else { 1 };
    w.write_bits(0, 2); // general_profile_space
    w.write_bit(p.high_tier);
    w.write_bits(profile, 5);
    // compatibility: a Main stream is also Main 10
    w.write_bits(if profile == 1 { 0x6000_0000 } else { 0x2000_0000 }, 32);
    w.write_bit(true); // progressive_source
    w.write_bit(false); // interlaced_source
    w.write_bit(true); // non_packed_constraint
    w.write_bit(true); // frame_only_constraint
    w.write_bits(0, 32); // 43 reserved bits + general_inbld_flag
    w.write_bits(0, 12);
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
    w.write_ue(1); // chroma_format_idc 4:2:0
    w.write_ue(p.coded_width);
    w.write_ue(p.coded_height);
    // conformance window in chroma samples (SubWidthC = SubHeightC = 2)
    let (right, bottom) = (p.coded_width.saturating_sub(p.width) / 2, p.coded_height.saturating_sub(p.height) / 2);
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
    w.write_bit(false); // dependent_slice_segments_enabled_flag
    w.write_bit(false); // output_flag_present_flag
    w.write_bits(0, 3); // num_extra_slice_header_bits
    w.write_bit(false); // sign_data_hiding_enabled_flag
    w.write_bit(false); // cabac_init_present_flag
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_se(p.init_qp.clamp(0, 51) - 26);
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
    w.write_ue(if s.idr { 2 } else { 1 }); // slice_type I / P
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
        w.write_bit(false); // num_ref_idx_active_override_flag
        w.write_ue(0); // five_minus_max_num_merge_cand
    }
    let init = p.init_qp.clamp(0, 51);
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

    #[test]
    fn slice_headers_parse_back() {
        let p = params(10);
        let (sps, pps) = (Arc::new(Sps::parse(&rbsp(&sps(&p))).expect("sps")), Arc::new(Pps::parse(&rbsp(&pps(&p))).expect("pps")));
        for s in [SlicePic { idr: true, poc: 0, qp_delta: -4 }, SlicePic { idr: false, poc: 7, qp_delta: 3 }, SlicePic { idr: false, poc: 70_000, qp_delta: 0 }]
        {
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
    fn ctb_count() {
        let p = StreamParams::new(3840, 2160, 10);
        assert_eq!(p.ctbs(), 60 * 34);
    }
}
