//! A mid-gray picture the H.264 front end writes itself, for the hardware to decode as the
//! stand-in for reference pictures that were never decoded (a frame_num gap at the start of a
//! run: the seek into an open GOP). The software decoder conceals those with mid-gray frames,
//! and what is predicted from them comes out the same only if the hardware's stand-ins are
//! mid-gray too; but a surface merely *written* gray is not a picture the hardware decoded, and
//! Mesa's AMD decoder (VCN) refuses such a reference (`vaEndPicture` fails). So the front end has
//! the hardware decode one: a CAVLC I picture of the stream's size in which every macroblock is
//! Intra 16x16 DC-predicted with no residual, which decodes to 128 everywhere (8.3.3: DC
//! prediction without neighbours is `1 << (BitDepth - 1)`, so is chroma DC). It comes with its
//! own SPS / PPS (Baseline, CAVLC), whose sequence fields mirror the stream's ([`Like`]) so the
//! driver sees an ordinary picture of the sequence it is decoding.
//!
//! Safe code, compiled everywhere; checked against the software decoder in the tests.

use filmcraft_bitstream::{BitWriter, escape_rbsp};
use filmcraft_h264::params::Sps;

/// The stream's sequence fields the gray picture's SPS repeats (they are in the VA picture
/// parameters too, so the picture looks like one of the stream's to the driver).
#[derive(Clone, Copy, Debug)]
pub struct Like {
    pub level_idc: u8,
    pub log2_max_frame_num: u32,
    /// `Some(log2_max_poc_lsb)` for picture order count type 0; `None` for type 2 (type 1 is
    /// written as type 2: a stand-in's order count is 0 either way).
    pub poc_lsb_bits: Option<u32>,
    pub max_num_ref_frames: u32,
    pub direct_8x8_inference: bool,
}

impl Default for Like {
    fn default() -> Self {
        Self { level_idc: 51, log2_max_frame_num: 4, poc_lsb_bits: None, max_num_ref_frames: 1, direct_8x8_inference: true }
    }
}

impl Like {
    pub fn from_sps(sps: &Sps) -> Self {
        Self {
            level_idc: sps.level_idc,
            log2_max_frame_num: sps.log2_max_frame_num.clamp(4, 16),
            poc_lsb_bits: (sps.pic_order_cnt_type == 0).then_some(sps.log2_max_poc_lsb.clamp(4, 16)),
            max_num_ref_frames: sps.max_num_ref_frames.min(16),
            direct_8x8_inference: sps.direct_8x8_inference,
        }
    }
}

/// The NAL units (header byte first, emulation prevention applied, as the front end takes them)
/// of a one-picture stream `mbs` (width, height) macroblocks large, every sample mid-gray: the
/// SPS and PPS (both id 0) and the slice, an IDR (`idr`) or a non-IDR reference I picture of
/// `frame_num` (order count 0). `None` for an empty or oversized picture.
pub fn gray_picture(mbs: (u32, u32), like: Like, idr: bool, frame_num: u32) -> Option<[Vec<u8>; 3]> {
    let (w, h) = mbs;
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    let count = w.checked_mul(h)?;
    // (every writer ends with rbsp_trailing_bits, so it is byte-aligned)
    let nal = |header: u8, rbsp: &BitWriter| {
        let mut out = vec![header];
        out.extend(escape_rbsp(rbsp.bytes()));
        out
    };

    // sequence parameter set: Baseline, frame coding only, no cropping, no VUI
    let mut sps = BitWriter::new();
    sps.write_bits(66, 8); // profile_idc
    sps.write_bits(0xC0, 8); // constraint_set0 and 1 flags; reserved zero bits
    sps.write_bits(u32::from(like.level_idc), 8); // level_idc
    sps.write_ue(0); // seq_parameter_set_id
    sps.write_ue(like.log2_max_frame_num - 4); // log2_max_frame_num_minus4
    match like.poc_lsb_bits {
        Some(bits) => {
            sps.write_ue(0); // pic_order_cnt_type
            sps.write_ue(bits - 4); // log2_max_pic_order_cnt_lsb_minus4
        }
        None => sps.write_ue(2), // pic_order_cnt_type: order = 2 * frame_num
    }
    sps.write_ue(like.max_num_ref_frames); // max_num_ref_frames
    sps.write_bit(false); // gaps_in_frame_num_value_allowed_flag
    sps.write_ue(w - 1); // pic_width_in_mbs_minus1
    sps.write_ue(h - 1); // pic_height_in_map_units_minus1
    sps.write_bit(true); // frame_mbs_only_flag
    sps.write_bit(like.direct_8x8_inference); // direct_8x8_inference_flag
    sps.write_bit(false); // frame_cropping_flag
    sps.write_bit(false); // vui_parameters_present_flag
    sps.rbsp_trailing();

    // picture parameter set: CAVLC, one slice group, deblocking controllable from the slice
    let mut pps = BitWriter::new();
    pps.write_ue(0); // pic_parameter_set_id
    pps.write_ue(0); // seq_parameter_set_id
    pps.write_bit(false); // entropy_coding_mode_flag: CAVLC
    pps.write_bit(false); // bottom_field_pic_order_in_frame_present_flag
    pps.write_ue(0); // num_slice_groups_minus1
    pps.write_ue(0); // num_ref_idx_l0_default_active_minus1
    pps.write_ue(0); // num_ref_idx_l1_default_active_minus1
    pps.write_bit(false); // weighted_pred_flag
    pps.write_bits(0, 2); // weighted_bipred_idc
    pps.write_se(0); // pic_init_qp_minus26
    pps.write_se(0); // pic_init_qs_minus26
    pps.write_se(0); // chroma_qp_index_offset
    pps.write_bit(true); // deblocking_filter_control_present_flag
    pps.write_bit(false); // constrained_intra_pred_flag
    pps.write_bit(false); // redundant_pic_cnt_present_flag
    pps.rbsp_trailing();

    // the slice: header, then one macroblock after another
    let mut slice = BitWriter::new();
    slice.write_ue(0); // first_mb_in_slice
    slice.write_ue(7); // slice_type: I (all slices of the picture)
    slice.write_ue(0); // pic_parameter_set_id
    slice.write_bits(frame_num & ((1 << like.log2_max_frame_num) - 1), like.log2_max_frame_num); // frame_num
    if idr {
        slice.write_ue(0); // idr_pic_id
    }
    if let Some(bits) = like.poc_lsb_bits {
        slice.write_bits(0, bits); // pic_order_cnt_lsb
    }
    // dec_ref_pic_marking (a reference picture)
    if idr {
        slice.write_bit(false); // no_output_of_prior_pics_flag
        slice.write_bit(false); // long_term_reference_flag
    } else {
        slice.write_bit(false); // adaptive_ref_pic_marking_mode_flag: sliding window
    }
    slice.write_se(0); // slice_qp_delta
    slice.write_ue(1); // disable_deblocking_filter_idc: off
    for _ in 0..count {
        // mb_type I_16x16_2_0_0: Intra16x16PredMode 2 (DC), chroma cbp 0, luma cbp 0 (7.4.5, table 7-11)
        slice.write_ue(3);
        slice.write_ue(0); // intra_chroma_pred_mode: DC
        slice.write_se(0); // mb_qp_delta
        // Intra16x16DCLevel: coeff_token for TotalCoeff 0, TrailingOnes 0 at nC 0 (table 9-5)
        slice.write_bit(true);
    }
    slice.rbsp_trailing();

    // nal_ref_idc 3; IDR (5) or non-IDR (1) slice
    Some([nal(0x67, &sps), nal(0x68, &pps), nal(if idr { 0x65 } else { 0x61 }, &slice)])
}

#[cfg(test)]
mod tests {
    use super::{Like, gray_picture};

    fn decode(nals: &[Vec<u8>; 3], d: &mut filmcraft_h264::Decoder) -> Vec<filmcraft_h264::Picture> {
        let mut au = Vec::new();
        for n in nals {
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend_from_slice(n);
        }
        d.decode(&au, 0).expect("decodes")
    }

    fn assert_gray(p: &filmcraft_h264::Picture, w: u32, h: u32, what: &str) {
        assert_eq!((p.width, p.height), (w * 16, h * 16), "{what}: size");
        let gray =
            |plane: &[u8], stride: usize, pw: u32, ph: u32| (0..ph as usize).all(|r| plane[r * stride..r * stride + pw as usize].iter().all(|&v| v == 128));
        assert!(gray(&p.y, p.y_stride, p.width, p.height), "{what}: luma");
        assert!(gray(&p.u, p.uv_stride, p.chroma_width, p.chroma_height), "{what}: Cb");
        assert!(gray(&p.v, p.uv_stride, p.chroma_width, p.chroma_height), "{what}: Cr");
    }

    /// The software decoder makes mid-gray pictures of the right size from it: as an IDR, and as
    /// a non-IDR picture after one, with order count type 0 and 2 and the sequence fields of a
    /// real stream's SPS.
    #[test]
    fn decodes_to_mid_gray_in_software() {
        for (w, h) in [(1, 1), (3, 2), (40, 23), (120, 68)] {
            for like in
                [Like::default(), Like { poc_lsb_bits: Some(6), log2_max_frame_num: 5, max_num_ref_frames: 4, level_idc: 40, direct_8x8_inference: false }]
            {
                let what = format!("{w}x{h} {like:?}");
                let mut d = filmcraft_h264::Decoder::new();
                let idr = gray_picture((w, h), like, true, 0).expect("a picture");
                let mut pics = decode(&idr, &mut d);
                let next = gray_picture((w, h), like, false, 1).expect("a picture");
                pics.extend(decode(&next, &mut d));
                pics.extend(d.flush());
                assert_eq!(pics.len(), 2, "{what}: two pictures");
                for p in &pics {
                    assert_gray(p, w, h, &what);
                }
                // the SPS round-trips through the parser with the fields asked for
                let sps = filmcraft_h264::params::Sps::parse(&filmcraft_bitstream::unescape_rbsp(&idr[0][1..])).expect("SPS");
                let back = Like::from_sps(&sps);
                assert_eq!(
                    (back.level_idc, back.log2_max_frame_num, back.poc_lsb_bits, back.max_num_ref_frames, back.direct_8x8_inference),
                    (like.level_idc, like.log2_max_frame_num, like.poc_lsb_bits, like.max_num_ref_frames, like.direct_8x8_inference),
                    "{what}"
                );
            }
        }
        assert!(gray_picture((0, 4), Like::default(), true, 0).is_none() && gray_picture((4, 2000), Like::default(), true, 0).is_none());
    }
}
