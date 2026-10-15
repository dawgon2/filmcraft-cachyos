//! Slice data decoding: macroblock parsing (CAVLC), motion vector derivation, and reconstruction.

use crate::cavlc;
use crate::deblock::{self, DeblockSample};
use crate::error::{Result, ensure, invalid};
use crate::inter::{self, Weight};
use crate::intra::{self, Avail};
use crate::mbtypes::*;
use crate::params::{Pps, Sps};
use crate::picture::*;
use crate::sample::Sample;
use crate::slice::{SliceHeader, SliceType};
use crate::tables::*;
use crate::transform::{self, LevelScale};
use filmcraft_bitstream::BitReader;

/// Read the I_PCM samples of a macroblock starting at byte `pos` of `data`: 256 luma samples,
/// then `chroma` (64 for 4:2:0, 128 for 4:2:2) per chroma component (pcm_sample_luma /
/// pcm_sample_chroma: `bits` = [BitDepthY, BitDepthC] bits each). Returns them (unused tail
/// zero) with the number of bytes they take (always whole bytes).
pub(crate) fn read_pcm(data: &[u8], pos: usize, bits: [u32; 2], chroma: usize) -> Result<([u16; 512], usize)> {
    let n = 256 + 2 * chroma.min(128);
    let len = (256 * bits[0] as usize + (n - 256) * bits[1] as usize).div_ceil(8);
    let Some(src) = data.get(pos..pos.saturating_add(len)) else {
        return invalid("truncated I_PCM macroblock");
    };
    let mut r = BitReader::new(src);
    let mut out = [0u16; 512];
    for (i, o) in out.iter_mut().enumerate().take(n) {
        *o = r.read_bits(if i < 256 { bits[0] } else { bits[1] })? as u16;
    }
    Ok((out, len))
}

/// QP_Y after `mb_qp_delta` (7.4.5): wraps within -QpBdOffsetY ..= 51.
fn qp_after_delta(qp: i32, delta: i32, qp_bd_offset: i32) -> Result<i32> {
    let half = qp_bd_offset / 2;
    ensure!((-(26 + half)..=25 + half).contains(&delta), "mb_qp_delta {delta} out of range");
    Ok((qp + delta + 52 + 2 * qp_bd_offset).rem_euclid(52 + qp_bd_offset) - qp_bd_offset)
}

/// QP_C for QP_Y `qp` and a chroma QP offset (8.5.8, Table 8-15): -QpBdOffsetC ..= 39.
fn chroma_qp_for(qp: i32, offset: i32, qp_bd_offset_c: i32) -> i32 {
    let qpi = (qp + offset).clamp(-qp_bd_offset_c, 51);
    if qpi < 30 { qpi } else { QPC_TABLE.get(qpi as usize).map_or(qpi, |&v| v as i32) }
}

/// Per-slice data kept with the picture (deblocking, motion field export).
#[derive(Clone, Debug, Default)]
pub struct SliceInfo {
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    /// Unique ids of the pictures in RefPicList0/1.
    pub ref_ids: [Vec<u32>; 2],
    /// A field picture's slice (deblocking rules for field macroblocks).
    pub field: bool,
}

/// vertMvScale (8.4.1.2.1): how the colocated picture's vertical vectors relate to the current
/// picture's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MvScale {
    OneToOne,
    /// The current picture is a field, the colocated one a frame.
    FrmToFld,
    /// The current picture is a frame, the colocated one a field of a complementary pair.
    FldToFrm,
}

/// The picture under construction: private reconstruction buffers plus the shared [`Frame`] that
/// finished (deblocked) macroblock rows are published into.
pub struct PicState<S: Sample = u8> {
    pub planes: Planes<S>,
    pub mbs: Vec<MbState>,
    pub slices: Vec<SliceInfo>,
    pub mb_w: usize,
    pub mb_h: usize,
    pub poc: i32,
    /// Top and bottom field order counts of the picture (MBAFF field macroblocks).
    pub field_pocs: [i32; 2],
    pub decoded_mbs: usize,
    pub frame: FrameRef<S>,
    /// Decoded macroblocks per MB row.
    row_count: Vec<u16>,
    /// Leading MB rows that are fully reconstructed / deblocked / published.
    rows_recon: usize,
    rows_deblocked: usize,
    rows_published: usize,
    /// Draft mode for a non-reference picture: rows are published without deblocking.
    pub skip_deblock: bool,
    /// An MBAFF frame: deblocked by macroblock pairs, in decoding order.
    pub mbaff: bool,
}

impl<S: DeblockSample> PicState<S> {
    pub fn new(frame: FrameRef<S>) -> Self {
        let (mb_w, mb_h) = (frame.mb_w, frame.mb_h());
        PicState {
            planes: Planes::with_depth(mb_w * 16, mb_h * 16, frame.bit_depth, frame.c422),
            mbs: vec![MbState::default(); mb_w * mb_h],
            slices: Vec::new(),
            mb_w,
            mb_h,
            poc: frame.poc,
            field_pocs: [frame.poc; 2],
            decoded_mbs: 0,
            frame,
            row_count: vec![0; mb_h],
            rows_recon: 0,
            rows_deblocked: 0,
            rows_published: 0,
            skip_deblock: false,
            mbaff: false,
        }
    }

    /// Reuse the buffers of a finished picture of the same size for a new frame.
    pub fn reset(&mut self, frame: FrameRef<S>) {
        debug_assert_eq!((self.mb_w, self.mb_h), (frame.mb_w, frame.mb_h()));
        // start_mb() re-initialises each macroblock before use; only availability must be reset
        for m in &mut self.mbs {
            m.slice_num = u32::MAX;
        }
        self.slices.clear();
        self.poc = frame.poc;
        self.decoded_mbs = 0;
        self.frame = frame;
        self.row_count.fill(0);
        self.rows_recon = 0;
        self.rows_deblocked = 0;
        self.rows_published = 0;
        self.skip_deblock = false;
        self.mbaff = false;
    }

    /// Record a decoded macroblock and deblock / publish rows that became final.
    #[inline]
    pub fn mb_done(&mut self, addr: usize) {
        self.decoded_mbs += 1;
        let r = addr / self.mb_w;
        self.row_count[r] += 1;
        if self.row_count[r] as usize >= self.mb_w && r == self.rows_recon {
            while self.rows_recon < self.mb_h && self.row_count[self.rows_recon] as usize >= self.mb_w {
                self.rows_recon += 1;
            }
            self.flush_rows(false);
        }
    }

    /// Deblock and publish rows. With `finished`, everything remaining is processed (also rows with
    /// missing macroblocks, so that waiting readers never block forever).
    pub fn flush_rows(&mut self, finished: bool) {
        // Deblocking row r must wait until row r + 1 is reconstructed (its intra prediction reads the
        // unfiltered bottom line of row r); row r is final once row r + 1 is deblocked.
        if self.mbaff {
            // MBAFF: pair rows, each MB pair top then bottom (decoding order), once the next pair
            // row is reconstructed (its intra prediction reads this one unfiltered)
            let limit = if finished { self.mb_h } else { self.rows_recon.saturating_sub(2) & !1 };
            while self.rows_deblocked < limit {
                let r = self.rows_deblocked;
                if !self.skip_deblock {
                    for x in 0..self.mb_w {
                        for b in 0..2 {
                            if r + b < self.mb_h {
                                deblock::deblock_mb_mbaff(self, (r + b) * self.mb_w + x, self.mb_w);
                            }
                        }
                    }
                }
                self.rows_deblocked = (r + 2).min(self.mb_h);
            }
        }
        let deblock_limit = if finished { self.mb_h } else { self.rows_recon.saturating_sub(1) };
        while !self.mbaff && self.rows_deblocked < deblock_limit {
            let r = self.rows_deblocked;
            if !self.skip_deblock {
                for addr in r * self.mb_w..(r + 1) * self.mb_w {
                    deblock::deblock_mb(self, addr, self.mb_w);
                }
            }
            self.rows_deblocked += 1;
        }
        let publish_limit = if finished { self.mb_h } else { self.rows_deblocked.saturating_sub(1) };
        while self.rows_published < publish_limit {
            let r = self.rows_published;
            let slices = &self.slices;
            // an MBAFF field macroblock's refIdx names a field: of frame refIdx >> 1, the MB's
            // parity (its row's) for even indices, the other one for odd ones (8.4.2.1)
            let ids = |st: &MbState, l: usize, ri: i8| {
                let list = slices.get(st.slice_num as usize).map(|s| &s.ref_ids[l]);
                let id = if st.field {
                    list.and_then(|ids| ids.get(ri as usize >> 1)).map(|&f| field_ref_id(f, (r & 1) ^ (ri as usize & 1)))
                } else {
                    list.and_then(|ids| ids.get(ri as usize).copied())
                };
                id.unwrap_or(u32::MAX)
            };
            let row = Frame::make_row(&self.planes, r, &self.mbs, &ids);
            self.frame.publish(r, row);
            self.rows_published += 1;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WpMode {
    Default,
    Explicit,
    Implicit,
}

/// Prediction weight tables of a slice.
struct Weights {
    mode: WpMode,
    luma_denom: i32,
    chroma_denom: i32,
    /// [list][ref] -> (weight, offset) for luma and two chroma components.
    luma: [Vec<(i32, i32)>; 2],
    chroma: [Vec<[(i32, i32); 2]>; 2],
    /// Implicit weights (w0, w1) indexed by ref0 * n1 + ref1.
    implicit: Vec<(i32, i32)>,
    n1: usize,
    /// MBAFF field macroblocks: implicit weights per current parity over the reference fields,
    /// indexed by ref0 * 2 * n1 + ref1.
    implicit_field: [Vec<(i32, i32)>; 2],
}

/// Current macroblock syntax (prediction part).
#[derive(Clone, Copy, Default)]
pub(crate) struct MbCur {
    pub info: MbTypeInfo,
    pub sub: [SubMbInfo; 4],
    /// rem_intra_pred_mode per block, or -1 when prev_intra_pred_mode_flag was set.
    pub rem_mode: [i8; 16],
    pub cbp: u8,
    pub ref_idx: [[i8; 4]; 2],
    /// mvd per (partition * 4 + sub-partition).
    pub mvd: [[[i16; 2]; 16]; 2],
}

/// Coefficient scratch for one macroblock.
pub(crate) struct Scratch<S: Sample = u8> {
    /// Luma: 16 4x4 blocks (raster block index * 16 + raster coefficient) or 4 8x8 blocks (b8 * 64 + raster).
    pub coef: [i32; 256],
    /// Chroma: [Cb, Cr][raster block (2 wide: 2x2, or 2x4 for 4:2:2) * 16 + raster coefficient].
    pub coef_c: [[i32; 128]; 2],
    /// Luma blocks with coefficients: raster 4x4 bits, or b8 bits in 8x8 mode.
    pub blk_nz: u16,
    /// Chroma blocks with coefficients (bit per raster 2x2 block).
    pub c_nz: [u8; 2],
    pub pred: [[S; 256]; 2],
    /// Chroma predictions [list][Cb, Cr], stride 8: 8x8 (4:2:0) or 8x16 (4:2:2).
    pub pred_c: [[[S; 128]; 2]; 2],
}

impl<S: Sample> Scratch<S> {
    pub fn new() -> Box<Self> {
        Box::new(Scratch {
            coef: [0; 256],
            coef_c: [[0; 128]; 2],
            blk_nz: 0,
            c_nz: [0; 2],
            pred: [[S::default(); 256]; 2],
            pred_c: [[[S::default(); 128]; 2]; 2],
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Nb {
    pub avail: bool,
    pub ref_idx: i8,
    pub mv: [i16; 2],
}

const NB_NA: Nb = Nb { avail: false, ref_idx: -1, mv: [0, 0] };

/// `C422`: the picture is 4:2:2 (a constant, so the 4:2:0 decoder carries no 4:2:2 branches).
/// `MBAFF`: the slice belongs to an MBAFF frame (fork H2.6; macroblocks in pairs, frame or
/// field per pair), a constant so other pictures carry no MBAFF code.
pub struct SliceDecoder<'a, S: Sample = u8, const C422: bool = false, const MBAFF: bool = false> {
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
    pub sps: &'a Sps,
    pub pic: &'a mut PicState<S>,
    pub refs: &'a [Vec<RefPic<S>>; 2],
    pub ls: &'a LevelScale,
    pub slice_num: u32,
    pub mb_w: usize,
    pub mb_h: usize,
    /// QP_Y of the current macroblock (-QpBdOffsetY ..= 51).
    pub qp: i32,
    pub chroma_qp_offset: [i32; 2],
    /// QpBdOffsetY / QpBdOffsetC: 6 * (BitDepth - 8).
    qp_bd: [i32; 2],
    // current macroblock
    pub mb_addr: usize,
    pub mb_x: usize,
    pub mb_y: usize,
    /// Neighbouring macroblocks A (left), B (above), C (above right), D (above left) when available.
    pub nb: [Option<usize>; 4],
    /// The current macroblock is a field macroblock of an MBAFF frame.
    pub mb_field: bool,
    /// MBAFF: top macroblocks (storage index) of the pairs A (left), B (above), C (above right)
    /// and D (above left) of the current pair, when available.
    pub(crate) pairs: [Option<usize>; 4],
    /// The current macroblock's first frame line and line step (2 for field macroblocks), luma
    /// and chroma.
    y_org: usize,
    c_org: usize,
    line_step: usize,
    pub mv_done: u16,
    pub cur: MbCur,
    pub s: Box<Scratch<S>>,
    /// (1 << BitDepth) - 1 of the picture; kernels take `S::code_max(bit_max)` (constant for `u8`).
    bit_max: i32,
    weights: Weights,
    /// Temporal direct: DistScaleFactor per refIdxL0, and whether pic0 is long-term.
    tdirect: Vec<(i32, bool)>,
    /// MBAFF field macroblocks: the same per current parity over the reference fields (refIdx
    /// in the field lists).
    tdirect_field: [Vec<(i32, bool)>; 2],
    /// CABAC: previous MB in decoding order had a non-zero mb_qp_delta.
    pub prev_qp_delta_nz: bool,
    /// Inverse scans: zig-zag, or the field scans in field pictures (8.5.6).
    scan4: &'static [u8; 16],
    scan8: &'static [u8; 64],
}

/// Interpolate a bw x bh luma block at (px, py) with motion vector `mv` from reference `rp` (a
/// frame, or a field: fork H2.5) into `y`, and the corresponding chroma blocks into `c` (stride
/// `cs`); `cur_field`: the current picture's parity when it is a field. `FIELD`: `rp` is a field
/// (a constant, so the frame path carries no field code).
#[allow(clippy::too_many_arguments)]
#[inline]
fn predict_block<S: Sample, const C422: bool, const FIELD: bool>(
    f: &Frame<S>,
    ref_parity: usize,
    cur_field: Option<usize>,
    mv: [i16; 2],
    px: usize,
    py: usize,
    bw: usize,
    bh: usize,
    y: (&mut [S], usize),
    c: [&mut [S]; 2],
    cs: usize,
    max: i32,
) {
    const LW: usize = 24;
    let mut win = [S::default(); LW * 21];
    let ix = px as i32 + (mv[0] as i32 >> 2);
    let iy = py as i32 + (mv[1] as i32 >> 2);
    let parity = if FIELD { ref_parity & 1 } else { 0 };
    if FIELD {
        f.field_luma_window(parity, ix - 2, iy - 2, bw + 5, bh + 5, &mut win, LW);
    } else {
        f.luma_window(ix - 2, iy - 2, bw + 5, bh + 5, &mut win, LW);
    }
    inter::mc_luma_win(&win, LW, (mv[0] & 3) as u32, (mv[1] & 3) as u32, bw, bh, y.0, y.1, max);
    let cx = (px / 2) as i32 + (mv[0] as i32 >> 3);
    let fx = (mv[0] & 7) as u32;
    if C422 {
        // 4:2:2 (8.4.2.2.2): chroma at full vertical resolution, the vertical vector in quarter
        // chroma samples: yIntC = yAL + (mv >> 2), yFracC = (mv & 3) << 1
        let cy = py as i32 + (mv[1] as i32 >> 2);
        let (cw, ch) = (bw / 2, bh);
        let fy = ((mv[1] & 3) << 1) as u32;
        for (comp, out) in c.into_iter().enumerate() {
            let mut w = [S::default(); 16 * 17];
            if FIELD {
                f.field_chroma_window(comp, parity, cx, cy, cw + 1, ch + 1, &mut w, 16);
            } else {
                f.chroma_window(comp, cx, cy, cw + 1, ch + 1, &mut w, 16);
            }
            inter::mc_chroma_win(&w, 16, fx, fy, cw, ch, out, cs);
        }
    } else {
        // 4:2:0 fields (Table 8-10): a reference field of the other parity shifts the vertical
        // chroma vector by a quarter chroma line (-2 from a top field to a bottom reference, +2
        // the other way)
        let off = match (FIELD, cur_field) {
            (true, Some(0)) if parity == 1 => -2,
            (true, Some(1)) if parity == 0 => 2,
            _ => 0,
        };
        let mvy = mv[1] as i32 + off;
        let cy = (py / 2) as i32 + (mvy >> 3);
        let (cw, ch) = (bw / 2, bh / 2);
        let fy = (mvy & 7) as u32;
        for (comp, out) in c.into_iter().enumerate() {
            let mut w = [S::default(); 16 * 9];
            if FIELD {
                f.field_chroma_window(comp, parity, cx, cy, cw + 1, ch + 1, &mut w, 16);
            } else {
                f.chroma_window(comp, cx, cy, cw + 1, ch + 1, &mut w, 16);
            }
            inter::mc_chroma_win(&w, 16, fx, fy, cw, ch, out, cs);
        }
    }
}

#[inline(always)]
fn median(a: i16, b: i16, c: i16) -> i16 {
    a.max(b).min(a.min(b).max(c))
}

impl<'a, S: DeblockSample, const C422: bool, const MBAFF: bool> SliceDecoder<'a, S, C422, MBAFF> {
    pub fn new(sh: &'a SliceHeader, pps: &'a Pps, sps: &'a Sps, pic: &'a mut PicState<S>, refs: &'a [Vec<RefPic<S>>; 2], ls: &'a LevelScale) -> Result<Self> {
        let slice_num = pic.slices.len() as u32;
        let chroma_qp_offset = [pps.chroma_qp_index_offset, pps.second_chroma_qp_index_offset];
        pic.slices.push(SliceInfo {
            disable_deblocking_filter_idc: sh.disable_deblocking_filter_idc,
            alpha_offset: sh.slice_alpha_c0_offset_div2 * 2,
            beta_offset: sh.slice_beta_offset_div2 * 2,
            ref_ids: [refs[0].iter().map(|r| r.id()).collect(), refs[1].iter().map(|r| r.id()).collect()],
            field: sh.field_pic,
        });
        pic.mbaff = MBAFF;
        let weights = Self::build_weights(sh, pps, refs, pic.poc, pic.field_pocs);
        let mut tdirect = Vec::new();
        let mut tdirect_field = [Vec::new(), Vec::new()];
        let scale = |cur: i32, poc0: i32, poc1: i32, long_term: bool| {
            let tb = cur.saturating_sub(poc0).clamp(-128, 127);
            let td = poc1.saturating_sub(poc0).clamp(-128, 127);
            if td == 0 || long_term {
                (256, true)
            } else {
                let tx = (16384 + (td / 2).abs()) / td;
                (((tb * tx + 32) >> 6).clamp(-1024, 1023), false)
            }
        };
        if sh.slice_type == SliceType::B && !sh.direct_spatial_mv_pred && !refs[1].is_empty() {
            let poc1 = refs[1][0].poc();
            for r0 in refs[0].iter() {
                tdirect.push(scale(pic.poc, r0.poc(), poc1, r0.long_term));
            }
            if MBAFF {
                // field macroblocks: the current MB's field, the field refIdxL0 names and the field
                // of RefPicList1[0] with the MB's parity (8.4.1.2.3)
                for (cp, table) in tdirect_field.iter_mut().enumerate() {
                    for i0 in 0..2 * refs[0].len() {
                        let Some(r0) = refs[0].get(i0 >> 1) else { continue };
                        table.push(scale(pic.field_pocs[cp], r0.field_pocs[cp ^ (i0 & 1)], refs[1][0].field_pocs[cp], r0.long_term));
                    }
                }
            }
        }
        let (mb_w, mb_h) = (pic.mb_w, pic.mb_h);
        let bit_max = (1i32 << pic.frame.bit_depth.clamp(8, 14)) - 1;
        Ok(SliceDecoder {
            sh,
            pps,
            sps,
            pic,
            refs,
            ls,
            slice_num,
            mb_w,
            mb_h,
            qp: sh.qp(pps),
            chroma_qp_offset,
            qp_bd: [6 * (sps.bit_depth_luma.clamp(8, 14) as i32 - 8), 6 * (sps.bit_depth_chroma.clamp(8, 14) as i32 - 8)],
            mb_addr: 0,
            mb_x: 0,
            mb_y: 0,
            nb: [None; 4],
            mb_field: false,
            pairs: [None; 4],
            y_org: 0,
            c_org: 0,
            line_step: 1,
            mv_done: 0,
            cur: MbCur::default(),
            s: Scratch::new(),
            bit_max,
            weights,
            tdirect,
            tdirect_field,
            prev_qp_delta_nz: false,
            scan4: if sh.field_pic { &FIELD_SCAN4 } else { &ZIGZAG4 },
            scan8: if sh.field_pic { &FIELD_SCAN8 } else { &ZIGZAG8 },
        })
    }

    fn build_weights(sh: &SliceHeader, pps: &Pps, refs: &[Vec<RefPic<S>>; 2], cur_poc: i32, field_pocs: [i32; 2]) -> Weights {
        let mut w = Weights {
            mode: WpMode::Default,
            luma_denom: 0,
            chroma_denom: 0,
            luma: [Vec::new(), Vec::new()],
            chroma: [Vec::new(), Vec::new()],
            implicit: Vec::new(),
            n1: refs[1].len(),
            implicit_field: [Vec::new(), Vec::new()],
        };
        let explicit = (pps.weighted_pred && sh.slice_type.is_p()) || (pps.weighted_bipred_idc == 1 && sh.slice_type.is_b());
        if explicit {
            if let Some(t) = &sh.pred_weight_table {
                w.mode = WpMode::Explicit;
                w.luma_denom = t.luma_log2_denom as i32;
                w.chroma_denom = t.chroma_log2_denom as i32;
                for (l, entries) in [&t.l0, &t.l1].into_iter().enumerate() {
                    w.luma[l] = entries.iter().map(|e| (e.luma_weight, e.luma_offset)).collect();
                    w.chroma[l] = entries.iter().map(|e| [(e.chroma_weight[0], e.chroma_offset[0]), (e.chroma_weight[1], e.chroma_offset[1])]).collect();
                }
            }
        } else if pps.weighted_bipred_idc == 2 && sh.slice_type.is_b() {
            w.mode = WpMode::Implicit;
            let implicit = |cur: i32, p0: i32, p1: i32, long_term: bool| {
                let tb = (cur.saturating_sub(p0)).clamp(-128, 127);
                let td = (p1.saturating_sub(p0)).clamp(-128, 127);
                if td == 0 || long_term {
                    (32, 32)
                } else {
                    let tx = (16384 + (td / 2).abs()) / td;
                    let dsf = ((tb * tx + 32) >> 6).clamp(-1024, 1023);
                    if (dsf >> 2) < -64 || (dsf >> 2) > 128 { (32, 32) } else { (64 - (dsf >> 2), dsf >> 2) }
                }
            };
            for r0 in refs[0].iter() {
                for r1 in refs[1].iter() {
                    w.implicit.push(implicit(cur_poc, r0.poc(), r1.poc(), r0.long_term || r1.long_term));
                }
            }
            if MBAFF {
                // field macroblocks (8.4.2.3.1): the current MB's field and the reference fields
                // refIdx >> 1 of the frame lists, same parity first
                for (cp, table) in w.implicit_field.iter_mut().enumerate() {
                    for i0 in 0..2 * refs[0].len() {
                        for i1 in 0..2 * refs[1].len() {
                            let field = |l: usize, i: usize| refs[l].get(i >> 1).map(|r| (r.field_pocs[if i & 1 == 0 { cp } else { 1 - cp }], r.long_term));
                            let (Some((p0, l0)), Some((p1, l1))) = (field(0, i0), field(1, i1)) else { continue };
                            table.push(implicit(field_pocs[cp], p0, p1, l0 || l1));
                        }
                    }
                }
            }
        }
        w
    }

    // ------------------------------------------------------------------------------------------
    // Neighbour helpers
    // ------------------------------------------------------------------------------------------

    /// The storage index (raster order) of the macroblock with address `addr` in decoding order:
    /// the same, or in an MBAFF frame 2 * pair + bottom (7.4.3).
    pub fn mb_index(&self, addr: usize) -> usize {
        if !MBAFF {
            return addr;
        }
        let (pair, w) = (addr / 2, self.mb_w.max(1));
        (pair / w * 2 + (addr & 1)) * w + pair % w
    }

    /// Set up the current macroblock (storage index `addr`) and neighbour availability; in an
    /// MBAFF frame as a field macroblock when `field`.
    pub fn start_mb_as(&mut self, addr: usize, field: bool) {
        self.mb_addr = addr;
        self.mb_x = addr % self.mb_w;
        self.mb_y = addr / self.mb_w;
        let sn = self.slice_num;
        self.pic.mbs[addr] = MbState { slice_num: sn, ..MbState::default() };
        if !MBAFF {
            // every picture but MBAFF frames: fixed neighbours, frame lines
            let mbs = &self.pic.mbs;
            let (w, x, y) = (self.mb_w, self.mb_x, self.mb_y);
            let ok = |a: usize| if mbs[a].slice_num == sn { Some(a) } else { None };
            self.nb = [
                if x > 0 { ok(addr - 1) } else { None },
                if y > 0 { ok(addr - w) } else { None },
                if y > 0 && x + 1 < w { ok(addr - w + 1) } else { None },
                if y > 0 && x > 0 { ok(addr - w - 1) } else { None },
            ];
            self.y_org = y * 16;
            self.c_org = y * self.chroma_blocks() * 2;
            self.mv_done = 0;
            return;
        }
        {
            let mbs = &self.pic.mbs;
            let w = self.mb_w;
            let (x, top) = (self.mb_x, (self.mb_y & !1) * w + self.mb_x);
            let ok = |a: usize| if mbs.get(a).is_some_and(|m| m.slice_num == sn) { Some(a) } else { None };
            let above = self.mb_y >= 2;
            self.pairs = [
                if x > 0 { ok(top - 1) } else { None },
                if above { ok(top - 2 * w) } else { None },
                if above && x + 1 < w { ok(top - 2 * w + 1) } else { None },
                if above && x > 0 { ok(top - 2 * w - 1) } else { None },
            ];
        }
        self.set_mb_field(field);
        self.mv_done = 0;
    }

    /// [`SliceDecoder::start_mb_as`] for a frame macroblock (every picture but MBAFF frames).
    pub fn start_mb(&mut self, addr: usize) {
        self.start_mb_as(addr, false);
    }

    /// Make the current macroblock a field (or frame) macroblock: neighbours, scans and sample
    /// addressing follow (MBAFF; mb_field_decoding_flag may only be known after the skip flags).
    pub fn set_mb_field(&mut self, field: bool) {
        self.mb_field = field;
        if let Some(m) = self.pic.mbs.get_mut(self.mb_addr) {
            m.field = field;
        }
        let w = self.mb_w;
        let cl = self.chroma_blocks() * 2; // chroma lines per MB: 8 or 16
        if field {
            let pair_y = self.mb_y & !1;
            self.y_org = pair_y * 16 + (self.mb_y & 1);
            self.c_org = pair_y * cl + (self.mb_y & 1);
            self.line_step = 2;
        } else {
            self.y_org = self.mb_y * 16;
            self.c_org = self.mb_y * cl;
            self.line_step = 1;
        }
        let coded_as_field = field || self.sh.field_pic;
        self.scan4 = if coded_as_field { &FIELD_SCAN4 } else { &ZIGZAG4 };
        self.scan8 = if coded_as_field { &FIELD_SCAN8 } else { &ZIGZAG8 };
        if MBAFF {
            let at = |s: &Self, x: i32, y: i32| s.locate(x, y, 16, 16).map(|(m, _, _)| m);
            self.nb = [at(self, -1, 0), at(self, 0, -1), at(self, 16, -1), at(self, -1, -1)];
        } else {
            let mbs = &self.pic.mbs;
            let sn = self.slice_num;
            let (x, y, addr) = (self.mb_x, self.mb_y, self.mb_addr);
            let ok = |a: usize| if mbs[a].slice_num == sn { Some(a) } else { None };
            self.nb = [
                if x > 0 { ok(addr - 1) } else { None },
                if y > 0 { ok(addr - w) } else { None },
                if y > 0 && x + 1 < w { ok(addr - w + 1) } else { None },
                if y > 0 && x > 0 { ok(addr - w - 1) } else { None },
            ];
        }
    }

    /// Whether the current macroblock is field-coded: a macroblock of a field picture, or a
    /// field macroblock of an MBAFF frame (field scans, field contexts).
    #[inline]
    pub fn field_coded(&self) -> bool {
        self.sh.field_pic || self.mb_field
    }

    /// MBAFF: mb_field_decoding_flag inferred for a pair without one (7.4.4): the left pair's,
    /// else the pair above's, else frame.
    pub fn inferred_mb_field(&self) -> bool {
        self.pairs[0].or(self.pairs[1]).is_some_and(|p| self.pic.mbs.get(p).is_some_and(|m| m.field))
    }

    /// The macroblock (storage index) holding the sample at (xn, yn) relative to the current
    /// macroblock's top-left sample, and the sample's position inside it (6.4.12): luma with
    /// `max_w` = `max_h` = 16, chroma with the chroma macroblock size. In MBAFF frames Table 6-4
    /// maps positions across frame / field pairs.
    pub fn locate(&self, xn: i32, yn: i32, max_w: i32, max_h: i32) -> Option<(usize, usize, usize)> {
        let in_x = (0..max_w).contains(&xn);
        if in_x && (0..max_h).contains(&yn) {
            return Some((self.mb_addr, xn as usize, yn as usize));
        }
        if yn >= max_h || (yn >= 0 && xn >= max_w) {
            return None;
        }
        let pos = |m: usize, ym: i32| Some((m, (xn + max_w).rem_euclid(max_w) as usize, (ym + max_h).rem_euclid(max_h) as usize));
        if !MBAFF {
            let n = if xn < 0 {
                if yn < 0 { self.nb[3] } else { self.nb[0] }
            } else if in_x {
                self.nb[1]
            } else {
                self.nb[2]
            };
            return pos(n?, yn);
        }
        let w = self.mb_w;
        let top = self.mb_y & 1 == 0;
        let frame = !self.mb_field;
        let field_pair = |m: usize| self.pic.mbs.get(m).is_some_and(|s| s.field);
        let (n, ym) = if xn < 0 && yn < 0 {
            // D
            match (frame, top) {
                (true, true) => (self.pairs[3]? + w, yn),
                // a bottom frame MB: the left pair's line above its first line (frame pair: the
                // top MB's last line; field pair: the bottom field's line 7)
                (true, false) => {
                    let a = self.pairs[0]?;
                    if field_pair(a) { (a + w, (yn + max_h) >> 1) } else { (a, yn) }
                }
                (false, true) => {
                    let d = self.pairs[3]?;
                    if field_pair(d) { (d, yn) } else { (d + w, 2 * yn) }
                }
                (false, false) => (self.pairs[3]? + w, yn),
            }
        } else if xn < 0 {
            // A, 0 <= yn < max_h
            let a = self.pairs[0]?;
            let parity = if yn % 2 == 0 { a } else { a + w };
            match (frame, top, field_pair(a)) {
                (true, true, false) => (a, yn),
                (true, true, true) => (parity, yn >> 1),
                (true, false, false) => (a + w, yn),
                (true, false, true) => (parity, (yn + max_h) >> 1),
                (false, true, false) if yn < max_h / 2 => (a, yn << 1),
                (false, true, false) => (a + w, (yn << 1) - max_h),
                (false, true, true) => (a, yn),
                (false, false, false) if yn < max_h / 2 => (a, (yn << 1) + 1),
                (false, false, false) => (a + w, (yn << 1) + 1 - max_h),
                (false, false, true) => (a + w, yn),
            }
        } else if in_x {
            // B
            match (frame, top) {
                (true, true) => (self.pairs[1]? + w, yn),
                (true, false) => (self.mb_addr - w, yn),
                (false, true) => {
                    let b = self.pairs[1]?;
                    if field_pair(b) { (b, yn) } else { (b + w, 2 * yn) }
                }
                (false, false) => (self.pairs[1]? + w, yn),
            }
        } else {
            // C
            match (frame, top) {
                (true, true) => (self.pairs[2]? + w, yn),
                (true, false) => return None,
                (false, true) => {
                    let c = self.pairs[2]?;
                    if field_pair(c) { (c, yn) } else { (c + w, 2 * yn) }
                }
                (false, false) => (self.pairs[2]? + w, yn),
            }
        };
        pos(n, ym)
    }

    /// The 4x4 luma block next to the current macroblock at 4x4 position (x, y) (x or y -1, or x
    /// 4 above): its macroblock and block position (6.4.11.4).
    #[inline]
    pub fn nb_blk4(&self, x: i32, y: i32) -> Option<(usize, usize, usize)> {
        if !MBAFF {
            return match (x < 0, y < 0) {
                (true, true) => self.nb[3].map(|m| (m, 3, 3)),
                (true, false) => self.nb[0].map(|m| (m, 3, y as usize)),
                (false, true) if x > 3 => self.nb[2].map(|m| (m, 0, 3)),
                (false, true) => self.nb[1].map(|m| (m, x as usize, 3)),
                (false, false) => Some((self.mb_addr, x as usize, y as usize)),
            };
        }
        let (m, xw, yw) = self.locate(if x < 0 { -1 } else { 4 * x }, if y < 0 { -1 } else { 4 * y }, 16, 16)?;
        Some((m, xw / 4, yw / 4))
    }

    /// [`SliceDecoder::nb_blk4`] for chroma 4x4 blocks (2 wide, 2 or 4 high).
    #[inline]
    pub fn nb_blk_c(&self, x: i32, y: i32) -> Option<(usize, usize, usize)> {
        let rows = self.chroma_blocks() / 2;
        if !MBAFF {
            return match (x < 0, y < 0) {
                (true, false) => self.nb[0].map(|m| (m, 1, y as usize)),
                (false, true) => self.nb[1].map(|m| (m, x as usize, rows - 1)),
                (false, false) => Some((self.mb_addr, x as usize, y as usize)),
                (true, true) => self.nb[3].map(|m| (m, 1, rows - 1)),
            };
        }
        let (m, xw, yw) = self.locate(if x < 0 { -1 } else { 4 * x }, if y < 0 { -1 } else { 4 * y }, 8, 4 * rows as i32)?;
        Some((m, xw / 4, yw / 4))
    }

    /// MBAFF: neighbour `m` is coded as field while the current macroblock is a frame one
    /// (`Some(true)`), the other way round (`Some(false)`), or alike (`None`).
    #[inline]
    pub fn mbaff_mismatch(&self, m: usize) -> Option<bool> {
        if !MBAFF {
            return None;
        }
        let nf = self.pic.mbs.get(m).is_some_and(|s| s.field);
        (nf != self.mb_field).then_some(nf)
    }

    /// Offset of the current macroblock's luma sample (x, y) in the picture, and the line stride.
    #[inline]
    fn luma_at(&self, x: usize, y: usize) -> (usize, usize) {
        let w = self.pic.planes.width;
        ((self.y_org + y * self.line_step) * w + self.mb_x * 16 + x, w * self.line_step)
    }

    /// The same for chroma.
    #[inline]
    fn chroma_at(&self, x: usize, y: usize) -> (usize, usize) {
        let w = self.pic.planes.cwidth;
        ((self.c_org + y * self.line_step) * w + self.mb_x * 8 + x, w * self.line_step)
    }

    /// A view of a plane for prediction functions taking (x, y, stride): (start offset, stride,
    /// the macroblock's first line in the view) so that line y of the view is line
    /// org + (y - first) * step of the picture.
    #[inline]
    fn view(&self, width: usize, org: usize) -> (usize, usize, usize) {
        ((org % self.line_step) * width, width * self.line_step, org / self.line_step)
    }

    #[inline]
    pub fn mb(&self) -> &MbState {
        &self.pic.mbs[self.mb_addr]
    }
    #[inline]
    pub fn mb_mut(&mut self) -> &mut MbState {
        &mut self.pic.mbs[self.mb_addr]
    }

    /// Neighbour MB usable for intra prediction (constrained_intra_pred aware).
    fn intra_nb(&self, n: usize) -> bool {
        match self.nb[n] {
            None => false,
            Some(a) => !self.pps.constrained_intra_pred || self.pic.mbs[a].kind.is_intra(),
        }
    }

    fn chroma_qp(&self, qp: i32, c: usize) -> i32 {
        chroma_qp_for(qp, self.chroma_qp_offset[c], self.qp_bd[1])
    }

    /// nC for a luma 4x4 block (raster index) - 9.2.1.
    fn nc_luma(&self, raster: usize) -> i32 {
        let (bx, by) = (raster & 3, raster >> 2);
        let cur = self.mb();
        let nnz = |n: Option<(usize, usize, usize)>| n.map(|(m, x, y)| self.pic.mbs[m].nnz[y * 4 + x]);
        let a = if bx > 0 { Some(cur.nnz[raster - 1]) } else { nnz(self.nb_blk4(-1, by as i32)) };
        let b = if by > 0 { Some(cur.nnz[raster - 4]) } else { nnz(self.nb_blk4(bx as i32, -1)) };
        match (a, b) {
            (Some(a), Some(b)) => (a as i32 + b as i32 + 1) >> 1,
            (Some(a), None) => a as i32,
            (None, Some(b)) => b as i32,
            _ => 0,
        }
    }

    fn nc_chroma(&self, c: usize, raster: usize) -> i32 {
        let (bx, by) = (raster & 1, raster >> 1);
        let cur = self.mb();
        // the block above in the MB above: its bottom row (1 for 4:2:0, 3 for 4:2:2)
        let nnz = |n: Option<(usize, usize, usize)>| n.map(|(m, x, y)| self.pic.mbs[m].nnz_c[c][y * 2 + x]);
        let a = if bx > 0 { Some(cur.nnz_c[c][raster - 1]) } else { nnz(self.nb_blk_c(-1, by as i32)) };
        let b = if by > 0 { Some(cur.nnz_c[c][raster - 2]) } else { nnz(self.nb_blk_c(bx as i32, -1)) };
        match (a, b) {
            (Some(a), Some(b)) => (a as i32 + b as i32 + 1) >> 1,
            (Some(a), None) => a as i32,
            (None, Some(b)) => b as i32,
            _ => 0,
        }
    }

    /// Motion data of the 4x4 block holding the luma sample (xs, ys) relative to the current MB
    /// (6.4.11.7: A at (x - 1, y), B at (x, y - 1), C at (x + w, y - 1), D at (x - 1, y - 1) of
    /// a partition at (x, y); in MBAFF frames the sample row decides the field across pairs).
    pub(crate) fn nb_motion(&self, list: usize, xs: i32, ys: i32) -> Nb {
        let (mb, bx, by) = if (0..16).contains(&xs) && (0..16).contains(&ys) {
            let (x, y) = (xs as usize / 4, ys as usize / 4);
            if self.mv_done & (1 << (y * 4 + x)) == 0 {
                return NB_NA;
            }
            (self.mb_addr, x, y)
        } else {
            let Some((m, xw, yw)) = self.locate(xs, ys, 16, 16) else { return NB_NA };
            (m, xw / 4, yw / 4)
        };
        let st = &self.pic.mbs[mb];
        let b8 = (by >> 1) * 2 + (bx >> 1);
        let r = st.ref_idx[list][b8];
        if r < 0 {
            return Nb { avail: true, ref_idx: -1, mv: [0, 0] };
        }
        let mv = st.mv[list][by * 4 + bx];
        // MBAFF (8.4.1.3.1): a frame neighbour of a field macroblock halves its vertical vector
        // and doubles its index, a field neighbour of a frame macroblock the other way round
        match self.mbaff_mismatch(mb) {
            None => Nb { avail: true, ref_idx: r, mv },
            Some(true) => Nb { avail: true, ref_idx: r >> 1, mv: [mv[0], mv[1].saturating_mul(2)] },
            Some(false) => Nb { avail: true, ref_idx: r.saturating_mul(2), mv: [mv[0], mv[1] / 2] },
        }
    }

    /// Returns (A, B, C) neighbour motion for a partition at (x, y) of width w (4x4 units), with C
    /// replaced by D when unavailable.
    fn nb_abc(&self, list: usize, x: usize, y: usize, w: usize) -> (Nb, Nb, Nb) {
        // luma sample positions of the partition's corner and width
        let (x, y, w) = (4 * x as i32, 4 * y as i32, 4 * w as i32);
        let a = self.nb_motion(list, x - 1, y);
        let b = self.nb_motion(list, x, y - 1);
        let mut c = self.nb_motion(list, x + w, y - 1);
        if !c.avail {
            c = self.nb_motion(list, x - 1, y - 1);
        }
        (a, b, c)
    }

    /// Luma motion vector prediction (8.4.1.3). `shape`: 1 = 16x8, 2 = 8x16, 0 = other.
    #[allow(clippy::too_many_arguments)]
    fn mvp(&self, list: usize, x: usize, y: usize, w: usize, ref_idx: i8, shape: u8, part: usize) -> [i16; 2] {
        let (a, b, c) = self.nb_abc(list, x, y, w);
        if shape == 1 {
            if part == 0 && b.ref_idx == ref_idx {
                return b.mv;
            }
            if part == 1 && a.ref_idx == ref_idx {
                return a.mv;
            }
        } else if shape == 2 {
            if part == 0 && a.ref_idx == ref_idx {
                return a.mv;
            }
            if part == 1 && c.ref_idx == ref_idx {
                return c.mv;
            }
        }
        Self::median_pred(a, b, c, ref_idx)
    }

    fn median_pred(a: Nb, mut b: Nb, mut c: Nb, ref_idx: i8) -> [i16; 2] {
        if !b.avail && !c.avail && a.avail {
            b = a;
            c = a;
        }
        let ma = a.ref_idx == ref_idx;
        let mb = b.ref_idx == ref_idx;
        let mc = c.ref_idx == ref_idx;
        match (ma, mb, mc) {
            (true, false, false) => a.mv,
            (false, true, false) => b.mv,
            (false, false, true) => c.mv,
            _ => [median(a.mv[0], b.mv[0], c.mv[0]), median(a.mv[1], b.mv[1], c.mv[1])],
        }
    }

    /// Store motion for a rectangle (4x4 units) of the current MB and mark it decoded.
    fn set_motion(&mut self, x: usize, y: usize, w: usize, h: usize, refs: [i8; 2], mvs: [[i16; 2]; 2]) {
        let st = &mut self.pic.mbs[self.mb_addr];
        for yy in y..y + h {
            for xx in x..x + w {
                let r = yy * 4 + xx;
                for l in 0..2 {
                    st.mv[l][r] = if refs[l] >= 0 { mvs[l] } else { [0, 0] };
                }
                self.mv_done |= 1 << r;
            }
        }
        for l in 0..2 {
            // ref_idx is per 8x8; rectangles never straddle 8x8 blocks with different refs
            for yy in (y..y + h).step_by(1) {
                for xx in x..x + w {
                    st.ref_idx[l][(yy >> 1) * 2 + (xx >> 1)] = refs[l];
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Direct prediction (8.4.1.2)
    // ------------------------------------------------------------------------------------------

    /// Co-located (mvCol, refIdxCol, referenced picture id, vertMvScale) for 4x4 block `raster` of
    /// the current MB (8.4.1.2.1).
    fn colocated(&self, raster: usize) -> ([i16; 2], i8, u32, MvScale) {
        const NONE: ([i16; 2], i8, u32, MvScale) = ([0, 0], -1, u32::MAX, MvScale::OneToOne);
        let Some(col) = self.refs[1].first() else { return NONE };
        let raster = if self.sps.direct_8x8_inference {
            let (bx, by) = (raster & 3, raster >> 2);
            (if by >= 2 { 12 } else { 0 }) + if bx >= 2 { 3 } else { 0 }
        } else {
            raster
        };
        // colPic (Table 8-6): a frame; for a field picture the field RefPicList1[0] when it was
        // decoded as a field, else the frame holding it, whose MB rows 2 * mb_y and 2 * mb_y + 1
        // cover the current field MB (mbAddrCol6, yM = 2 * yCol % 16); for a frame picture over a
        // complementary field pair the field nearer in POC (ties: bottom), whose MB row mb_y / 2
        // covers the current frame MB (mbAddrCol7, yM = 8 * (mb_y % 2) + 4 * (yCol / 8))
        let y_col = (raster >> 2) * 4;
        let (frame, mb_row, raster, scale) = if MBAFF {
            self.colocated_mbaff(col, raster, y_col)
        } else {
            match col.parity {
                None => match (col.frame.field(0), col.frame.field(1)) {
                    (Some(top), Some(bottom)) => {
                        let [t, b] = col.field_pocs;
                        let cur = self.pic.poc;
                        let f = if (i64::from(t) - i64::from(cur)).abs() < (i64::from(b) - i64::from(cur)).abs() { top } else { bottom };
                        (&**f, self.mb_y / 2, (2 * (self.mb_y % 2) + y_col / 8) * 4 + (raster & 3), MvScale::FldToFrm)
                    }
                    _ => (&*col.frame, self.mb_y, raster, MvScale::OneToOne),
                },
                Some(p) => match col.frame.field(p) {
                    Some(f) => (&**f, self.mb_y, raster, MvScale::OneToOne),
                    // a frame-coded picture; if MBAFF coded a field pair here, its MB of the current
                    // field's parity (bottom_field_flag)
                    None if col.frame.mb_h() > 2 * self.mb_y && col.frame.row(2 * self.mb_y).field.get(self.mb_x).copied().unwrap_or(false) => {
                        (&*col.frame, 2 * self.mb_y + usize::from(self.sh.bottom_field), raster, MvScale::OneToOne)
                    }
                    None => (&*col.frame, 2 * self.mb_y + y_col / 8, ((2 * y_col) % 16 / 4) * 4 + (raster & 3), MvScale::FrmToFld),
                },
            }
        };
        if frame.mb_w != self.mb_w || mb_row >= frame.mb_h() {
            return NONE;
        }
        let m = frame.row(mb_row);
        let mb = self.mb_x;
        if m.intra.get(mb).copied().unwrap_or(true) {
            return NONE;
        }
        let b8 = mb * 4 + (raster >> 3) * 2 + ((raster & 3) >> 1);
        let l = if m.ref_idx[0].get(b8).is_some_and(|&r| r >= 0) { 0 } else { 1 };
        let (Some(&mv), Some(&ri), Some(&id)) = (m.mv[l].get(mb * 16 + raster), m.ref_idx[l].get(b8), m.ref_id[l].get(b8)) else { return NONE };
        (mv, ri, id, scale)
    }

    /// [`SliceDecoder::colocated`] in an MBAFF frame (8.4.1.2.1): (colocated picture, MB row in
    /// it, 4x4 block, vertMvScale). Over a complementary field pair a field macroblock takes the
    /// field of its parity and a frame macroblock the field nearer in POC; over a frame the
    /// colocated pair's own frame / field coding decides.
    fn colocated_mbaff<'c>(&self, col: &'c RefPic<S>, raster: usize, y_col: usize) -> (&'c Frame<S>, usize, usize, MvScale) {
        let (top_row, bottom) = (self.mb_y & !1, self.mb_y & 1);
        let nearer = |[t, b]: [i32; 2]| {
            let cur = i64::from(self.pic.poc);
            usize::from((i64::from(t) - cur).abs() >= (i64::from(b) - cur).abs())
        };
        // a frame MB over field MBs: rows 8 * (mb_y % 2) + 4 * (yCol / 8) of the field MB
        let fld_to_frm = (2 * bottom + y_col / 8) * 4 + (raster & 3);
        if let (Some(top), Some(bot)) = (col.frame.field(0), col.frame.field(1)) {
            let pick = |p: usize| if p == 0 { &**top } else { &**bot };
            return if self.mb_field {
                (pick(bottom), self.mb_y >> 1, raster, MvScale::OneToOne)
            } else {
                (pick(nearer(col.field_pocs)), self.mb_y >> 1, fld_to_frm, MvScale::FldToFrm)
            };
        }
        let frame = &*col.frame;
        let col_field = top_row < frame.mb_h() && frame.row(top_row).field.get(self.mb_x).copied().unwrap_or(false);
        match (self.mb_field, col_field) {
            (false, false) | (true, true) => (frame, self.mb_y, raster, MvScale::OneToOne),
            // a field MB over a frame pair: the pair's top MB for yCol < 8, else the bottom one,
            // yM = 2 * yCol % 16
            (true, false) => (frame, top_row + y_col / 8, ((2 * y_col) % 16 / 4) * 4 + (raster & 3), MvScale::FrmToFld),
            (false, true) => (frame, top_row + nearer(col.field_pocs), fld_to_frm, MvScale::FldToFrm),
        }
    }

    /// Spatial direct: MB-level reference indices, predicted mvs and directZeroPrediction (8.4.1.2.2).
    fn spatial_direct_base(&self) -> ([i8; 2], [[i16; 2]; 2], bool) {
        let mut refs = [-1i8; 2];
        let mut nbs = [(NB_NA, NB_NA, NB_NA); 2];
        let min_pos = |x: i8, y: i8| if x >= 0 && y >= 0 { x.min(y) } else { x.max(y) };
        for (l, nb) in nbs.iter_mut().enumerate() {
            *nb = self.nb_abc(l, 0, 0, 4);
            refs[l] = min_pos(nb.0.ref_idx, min_pos(nb.1.ref_idx, nb.2.ref_idx));
        }
        let mut mvs = [[0i16; 2]; 2];
        if refs[0] < 0 && refs[1] < 0 {
            return ([0, 0], mvs, true);
        }
        for l in 0..2 {
            if refs[l] >= 0 {
                let (a, b, c) = nbs[l];
                mvs[l] = Self::median_pred(a, b, c, refs[l]);
            }
        }
        (refs, mvs, false)
    }

    /// Derive direct-mode motion for the 8x8 blocks in `b8_mask` of the current MB.
    fn derive_direct(&mut self, b8_mask: u8) -> Result<()> {
        let spatial = self.sh.direct_spatial_mv_pred;
        let (base_refs, base_mvs, direct_zero) = if spatial { self.spatial_direct_base() } else { ([0, 0], [[0, 0]; 2], false) };
        let col_short_term = self.refs[1].first().map(|r| !r.long_term).unwrap_or(false);
        for b8 in 0..4 {
            if b8_mask & (1 << b8) == 0 {
                continue;
            }
            let (x8, y8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            let unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
            for sub in (0..4).filter(|s| unit == 1 || *s == 0) {
                let (x, y) = (x8 + (sub & 1), y8 + (sub >> 1));
                let raster = y * 4 + x;
                let (mv_col, ref_col, id_col, scale) = self.colocated(raster);
                let (refs, mvs) = if spatial {
                    let col_zero = col_short_term && ref_col == 0 && (-1..=1).contains(&mv_col[0]) && (-1..=1).contains(&mv_col[1]);
                    let mut mvs = base_mvs;
                    for l in 0..2 {
                        if direct_zero || base_refs[l] < 0 || (base_refs[l] == 0 && col_zero) {
                            mvs[l] = [0, 0];
                        }
                    }
                    (base_refs, mvs)
                } else {
                    let field_mb = MBAFF && self.mb_field;
                    let cp = self.mb_y & 1;
                    let ref0 = if ref_col < 0 {
                        0
                    } else if field_mb {
                        // MBAFF field macroblocks index the fields of the frame lists (8.4.1.2.3):
                        // a frame refPicCol gives its field of the MB's parity (an even index)
                        let n = 2 * self.refs[0].len();
                        let found = if scale == MvScale::FrmToFld {
                            self.refs[0].iter().position(|r| r.frame.id == id_col).map(|k| 2 * k)
                        } else {
                            (0..n).find(|&i| self.refs[0].get(i >> 1).is_some_and(|r| field_ref_id(r.frame.id, cp ^ (i & 1)) == id_col))
                        };
                        found.map_or(0, |i| i as i8)
                    } else if scale == MvScale::FrmToFld {
                        // the field of refPicCol (a frame) with the current field's parity (8.4.1.2.3)
                        let parity = Some(usize::from(self.sh.bottom_field));
                        self.refs[0].iter().position(|r| r.frame.id == id_col && r.parity == parity).map_or(0, |i| i as i8)
                    } else if scale == MvScale::FldToFrm {
                        // the frame or complementary field pair holding refPicCol (a field)
                        self.refs[0].iter().position(|r| r.parity.is_none() && is_field_of(id_col, r.frame.id)).map_or(0, |i| i as i8)
                    } else {
                        match self.refs[0].iter().position(|r| r.id() == id_col) {
                            Some(i) => i as i8,
                            None => 0,
                        }
                    };
                    // the colocated vertical vector in the current picture's units
                    let mv_col = match scale {
                        MvScale::OneToOne => mv_col,
                        MvScale::FrmToFld => [mv_col[0], mv_col[1] / 2],
                        MvScale::FldToFrm => [mv_col[0], mv_col[1].saturating_mul(2)],
                    };
                    let table = if field_mb { &self.tdirect_field[cp] } else { &self.tdirect };
                    let (dsf, ltr) = table.get(ref0 as usize).copied().unwrap_or((256, true));
                    let (mv0, mv1) = if ltr {
                        (mv_col, [0, 0])
                    } else {
                        let s = |c: i16| ((dsf * c as i32 + 128) >> 8) as i16;
                        let m0 = [s(mv_col[0]), s(mv_col[1])];
                        (m0, [m0[0] - mv_col[0], m0[1] - mv_col[1]])
                    };
                    ([ref0, 0], [mv0, mv1])
                };
                self.set_motion(x, y, unit, unit, refs, mvs);
            }
        }
        let st = &mut self.pic.mbs[self.mb_addr];
        st.direct8x8 |= b8_mask;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------
    // Motion vector derivation for a parsed inter macroblock
    // ------------------------------------------------------------------------------------------

    /// P_Skip motion (8.4.1.1).
    pub fn derive_p_skip(&mut self) {
        let a = self.nb_motion(0, -1, 0);
        let b = self.nb_motion(0, 0, -1);
        let mv = if self.nb[0].is_none() || self.nb[1].is_none() || (a.ref_idx == 0 && a.mv == [0, 0]) || (b.ref_idx == 0 && b.mv == [0, 0]) {
            [0, 0]
        } else {
            self.mvp(0, 0, 0, 4, 0, 0, 0)
        };
        self.set_motion(0, 0, 4, 4, [0, -1], [mv, [0, 0]]);
    }

    /// Derive mvs of the current (non-skip) inter MB from `self.cur` (ref_idx, mvd).
    pub fn derive_inter_motion(&mut self) -> Result<()> {
        let info = self.cur.info;
        if info.kind == MbKind::BDirect16x16 || info.kind == MbKind::BSkip {
            return self.derive_direct(0xf);
        }
        // MBAFF field macroblocks index the fields of the frame lists: twice as many entries
        let k = 1 + (MBAFF && self.mb_field) as usize;
        let n = [self.refs[0].len() * k, self.refs[1].len() * k];
        if info.part != Part::P8x8 {
            let shape = match info.part {
                Part::P16x8 => 1,
                Part::P8x16 => 2,
                _ => 0,
            };
            for p in 0..info.part.num_parts() {
                let (x, y, w, h) = info.part.rect(p);
                let mut refs = [-1i8; 2];
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if info.pred[p] & (1 << l) != 0 {
                        let r = self.cur.ref_idx[l][p];
                        ensure!((r as usize) < n[l], "ref_idx {r} out of range (list {l}, size {})", n[l]);
                        let mvp = self.mvp(l, x, y, w, r, shape, p);
                        let d = self.cur.mvd[l][p * 4];
                        refs[l] = r;
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
            return Ok(());
        }
        for p in 0..4 {
            let sub = self.cur.sub[p];
            let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
            if sub.direct {
                self.derive_direct(1 << p)?;
                continue;
            }
            let mut refs = [-1i8; 2];
            for (l, r) in refs.iter_mut().enumerate() {
                if sub.pred & (1 << l) != 0 {
                    let v = self.cur.ref_idx[l][p];
                    ensure!((v as usize) < n[l], "ref_idx {v} out of range (list {l}, size {})", n[l]);
                    *r = v;
                }
            }
            for j in 0..sub.shape.num_parts() {
                let (sx, sy, w, h) = sub.shape.rect(j);
                let (x, y) = (x8 + sx, y8 + sy);
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if refs[l] >= 0 {
                        let mvp = self.mvp(l, x, y, w, refs[l], 0, 0);
                        let d = self.cur.mvd[l][p * 4 + j];
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------
    // Inter prediction
    // ------------------------------------------------------------------------------------------

    fn weight_for(&self, refs: [i8; 2], comp: usize) -> Weight {
        let w = &self.weights;
        match w.mode {
            WpMode::Default => Weight::Default,
            WpMode::Implicit => {
                if refs[0] >= 0 && refs[1] >= 0 {
                    let (r0, r1) = (refs[0] as usize, refs[1] as usize);
                    let pair = if MBAFF && self.mb_field { w.implicit_field[self.mb_y & 1].get(r0 * 2 * w.n1 + r1) } else { w.implicit.get(r0 * w.n1 + r1) };
                    let (w0, w1) = pair.copied().unwrap_or((32, 32));
                    Weight::Weighted { log_wd: 5, w0, w1, o0: 0, o1: 0 }
                } else {
                    Weight::Default
                }
            }
            WpMode::Explicit => {
                let get = |l: usize| -> (i32, i32) {
                    if refs[l] < 0 {
                        return (0, 0);
                    }
                    // MBAFF field macroblocks weight by the frame's index (refIdxL0WP, 8.4.2.3)
                    let r = if MBAFF && self.mb_field { refs[l] as usize >> 1 } else { refs[l] as usize };
                    if comp == 0 {
                        w.luma[l].get(r).copied().unwrap_or((1 << w.luma_denom, 0))
                    } else {
                        w.chroma[l].get(r).map(|c| c[comp - 1]).unwrap_or((1 << w.chroma_denom, 0))
                    }
                };
                let (w0, o0) = get(0);
                let (w1, o1) = get(1);
                let log_wd = if comp == 0 { w.luma_denom } else { w.chroma_denom };
                // offsets are in 8-bit units: o = offset * (1 << (BitDepth - 8)) (8.4.2.3)
                let depth = if comp == 0 { self.sps.bit_depth_luma } else { self.sps.bit_depth_chroma };
                let shift = depth.clamp(8, 14) - 8;
                Weight::Weighted { log_wd, w0, w1, o0: o0 << shift, o1: o1 << shift }
            }
        }
    }

    /// Motion-compensated prediction of a rectangle (4x4 units) written into the picture.
    fn mc_rect(&mut self, x4: usize, y4: usize, w4: usize, h4: usize) {
        let st = &self.pic.mbs[self.mb_addr];
        let r = y4 * 4 + x4;
        let b8 = (y4 >> 1) * 2 + (x4 >> 1);
        let refs = [st.ref_idx[0][b8], st.ref_idx[1][b8]];
        let mvs = [st.mv[0][r], st.mv[1][r]];
        let (bw, bh) = (w4 * 4, h4 * 4);
        let wts = [self.weight_for(refs, 0), self.weight_for(refs, 1), self.weight_for(refs, 2)];
        // the block's position in the picture it predicts from: the frame, or (field pictures and
        // MBAFF field macroblocks) the field
        let px = self.mb_x * 16 + x4 * 4;
        let mb_row = if MBAFF && self.mb_field { self.mb_y >> 1 } else { self.mb_y };
        let py = mb_row * 16 + y4 * 4;
        let field_mb = MBAFF && self.mb_field;
        let cur_field = if field_mb { Some(self.mb_y & 1) } else { self.sh.field_pic.then_some(usize::from(self.sh.bottom_field)) };
        // the reference of refIdx `ri` in list `l` (8.4.2.1): a frame, a field, or for an MBAFF
        // field macroblock the field of frame ri >> 1 with the same (ri even) or the other parity
        let (lists, cur) = (self.refs, self.mb_y & 1);
        let reference = |l: usize, ri: i8| -> Option<(&Frame<S>, Option<usize>)> {
            if ri < 0 {
                return None;
            }
            if field_mb {
                let rp = lists.get(l)?.get(ri as usize >> 1)?;
                return Some((&*rp.frame, Some(if ri & 1 == 0 { cur } else { 1 - cur })));
            }
            let rp = lists.get(l)?.get(ri as usize)?;
            Some((&*rp.frame, rp.parity))
        };
        let (r0, r1) = (reference(0, refs[0]), reference(1, refs[1]));
        let (use0, use1) = (r0.is_some(), r1.is_some());
        let (yat, ys) = self.luma_at(x4 * 4, y4 * 4);
        let c422 = C422;
        // chroma block origin and height: half the luma ones (4:2:0), full height (4:2:2)
        let (cat, cs) = self.chroma_at(x4 * 2, if c422 { y4 * 4 } else { y4 * 2 });
        let cbh = if c422 { bh } else { bh / 2 };
        let max = S::code_max(self.bit_max);
        let predict = |f: &Frame<S>, parity: Option<usize>, mv: [i16; 2], y: (&mut [S], usize), c: [&mut [S]; 2], cstride: usize| match parity {
            Some(p) => predict_block::<S, C422, true>(f, p, cur_field, mv, px, py, bw, bh, y, c, cstride, max),
            None => predict_block::<S, C422, false>(f, 0, cur_field, mv, px, py, bw, bh, y, c, cstride, max),
        };
        if use0 != use1 && wts.iter().all(|w| *w == Weight::Default) {
            // single-list unweighted prediction: interpolate straight into the picture
            let (l, Some((f, parity))) = (if use0 { 0 } else { 1 }, if use0 { r0 } else { r1 }) else { return };
            let planes = &mut self.pic.planes;
            let y = (&mut planes.y[yat..], ys);
            let c = [&mut planes.cb[cat..], &mut planes.cr[cat..]];
            predict(f, parity, mvs[l], y, c, cs);
            return;
        }
        let s = &mut *self.s;
        for (l, r) in [r0, r1].into_iter().enumerate() {
            let Some((f, parity)) = r else { continue };
            let [pc0, pc1] = &mut s.pred_c[l];
            let (y, c) = ((&mut s.pred[l][..], 16), [&mut pc0[..], &mut pc1[..]]);
            predict(f, parity, mvs[l], y, c, 8);
        }
        inter::weighted_store(
            &mut self.pic.planes.y[yat..],
            ys,
            if use0 { Some(&s.pred[0][..]) } else { None },
            if use1 { Some(&s.pred[1][..]) } else { None },
            16,
            bw,
            bh,
            wts[0],
            S::code_max(self.bit_max),
        );
        for c in 0..2 {
            let wc = wts[c + 1];
            let a = if use0 { Some(&s.pred_c[0][c][..]) } else { None };
            let b = if use1 { Some(&s.pred_c[1][c][..]) } else { None };
            let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
            inter::weighted_store(&mut plane[cat..], cs, a, b, 8, bw / 2, cbh, wc, S::code_max(self.bit_max));
        }
    }

    /// All 16 blocks of the current MB share references and motion vectors.
    fn uniform_motion(&self) -> bool {
        let st = self.mb();
        (0..2).all(|l| {
            let r = st.ref_idx[l];
            r[1] == r[0] && r[2] == r[0] && r[3] == r[0] && st.mv[l].iter().all(|m| *m == st.mv[l][0])
        })
    }

    /// Inter prediction for the whole current MB, using the stored motion.
    pub fn predict_inter(&mut self) {
        let info = self.cur.info;
        let direct_unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
        match info.kind {
            MbKind::PSkip => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 if self.uniform_motion() => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 => {
                for y in (0..4).step_by(direct_unit) {
                    for x in (0..4).step_by(direct_unit) {
                        self.mc_rect(x, y, direct_unit, direct_unit);
                    }
                }
            }
            _ => {
                if info.part != Part::P8x8 {
                    for p in 0..info.part.num_parts() {
                        let (x, y, w, h) = info.part.rect(p);
                        self.mc_rect(x, y, w, h);
                    }
                } else {
                    for p in 0..4 {
                        let sub = self.cur.sub[p];
                        let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
                        if sub.direct {
                            for y in (0..2).step_by(direct_unit) {
                                for x in (0..2).step_by(direct_unit) {
                                    self.mc_rect(x8 + x, y8 + y, direct_unit, direct_unit);
                                }
                            }
                        } else {
                            for j in 0..sub.shape.num_parts() {
                                let (sx, sy, w, h) = sub.shape.rect(j);
                                self.mc_rect(x8 + sx, y8 + sy, w, h);
                            }
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Intra prediction mode derivation (8.3.1.1 / 8.3.2.1)
    // ------------------------------------------------------------------------------------------

    fn intra_mode_nb(&self, n: Option<(usize, usize, usize)>) -> Option<u8> {
        // returns Some(mode) or None if dcPredModePredictedFlag must be set
        let (m, bx, by) = n?;
        let st = &self.pic.mbs[m];
        if self.pps.constrained_intra_pred && !st.kind.is_intra() {
            return None;
        }
        Some(match st.kind {
            MbKind::I4x4 | MbKind::I8x8 => st.intra_modes[by * 4 + bx],
            _ => 2,
        })
    }

    /// Resolve Intra4x4PredMode / Intra8x8PredMode for the current MB from `self.cur.rem_mode`.
    pub fn derive_intra_modes(&mut self, is8x8: bool) {
        let step = if is8x8 { 2 } else { 1 };
        let nblk = if is8x8 { 4 } else { 16 };
        for i in 0..nblk {
            let (bx, by) = if is8x8 {
                ((i & 1) * 2, (i >> 1) * 2)
            } else {
                let (x, y) = BLK4_XY[i];
                (x as usize, y as usize)
            };
            let a = if bx > 0 { Some(self.mb().intra_modes[by * 4 + bx - 1]) } else { self.intra_mode_nb(self.nb_blk4(-1, by as i32)) };
            let b = if by > 0 { Some(self.mb().intra_modes[(by - 1) * 4 + bx]) } else { self.intra_mode_nb(self.nb_blk4(bx as i32, -1)) };
            let pred = match (a, b) {
                (Some(a), Some(b)) => a.min(b),
                _ => 2,
            };
            let rem = self.cur.rem_mode[i];
            let mode = if rem < 0 {
                pred
            } else if (rem as u8) < pred {
                rem as u8
            } else {
                rem as u8 + 1
            };
            let st = self.mb_mut();
            for yy in by..by + step {
                for xx in bx..bx + step {
                    st.intra_modes[yy * 4 + xx] = mode;
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Reconstruction
    // ------------------------------------------------------------------------------------------

    fn add_luma_block4(&mut self, raster: usize) {
        if self.s.blk_nz & (1 << raster) == 0 {
            return;
        }
        let (at, stride) = self.luma_at((raster & 3) * 4, (raster >> 2) * 4);
        let Some(blk) = self.s.coef.get_mut(raster * 16..).and_then(|c| c.first_chunk_mut::<16>()) else {
            return;
        };
        transform::idct4_add(blk, &mut self.pic.planes.y[at..], stride, S::code_max(self.bit_max));
        blk.fill(0);
    }

    fn add_luma_block8(&mut self, b8: usize) {
        if self.s.blk_nz & (1 << b8) == 0 {
            return;
        }
        let (at, stride) = self.luma_at((b8 & 1) * 8, (b8 >> 1) * 8);
        let Some(blk) = self.s.coef.get_mut(b8 * 64..).and_then(|c| c.first_chunk_mut::<64>()) else {
            return;
        };
        transform::idct8_add(blk, &mut self.pic.planes.y[at..], stride, S::code_max(self.bit_max));
        blk.fill(0);
    }

    fn add_chroma(&mut self) {
        if C422 {
            self.add_chroma_n::<8>();
        } else {
            self.add_chroma_n::<4>();
        }
    }

    #[inline(always)]
    fn add_chroma_n<const NBLK: usize>(&mut self) {
        let nblk = NBLK;
        for c in 0..2 {
            if self.s.c_nz[c] == 0 {
                continue;
            }
            for b in 0..nblk {
                if self.s.c_nz[c] & (1 << b) == 0 {
                    continue;
                }
                let (at, stride) = self.chroma_at((b & 1) * 4, (b >> 1) * 4);
                let Some(blk) = self.s.coef_c[c].get_mut(b * 16..).and_then(|c| c.first_chunk_mut::<16>()) else {
                    continue;
                };
                let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
                transform::idct4_add(blk, &mut plane[at..], stride, S::code_max(self.bit_max));
                blk.fill(0);
            }
            self.s.c_nz[c] = 0;
        }
    }

    fn luma_avail4(&self, bx: usize, by: usize) -> Avail {
        self.luma_avail(bx, by, 4)
    }

    /// Neighbour MB `m` usable for intra prediction of the current one.
    #[inline]
    fn intra_usable(&self, m: usize) -> bool {
        m == self.mb_addr || !self.pps.constrained_intra_pred || self.pic.mbs.get(m).is_some_and(|s| s.kind.is_intra())
    }

    /// The left neighbours of luma rows y0 .. y0 + rows are usable (in MBAFF frames they may
    /// lie in both macroblocks of the left pair).
    fn intra_left(&self, y0: usize, rows: usize) -> bool {
        if !MBAFF {
            return self.intra_nb(0);
        }
        (y0..y0 + rows).all(|y| self.locate(-1, y as i32, 16, 16).is_some_and(|(m, _, _)| self.intra_usable(m)))
    }

    /// Availability for intra prediction of the block at 4x4 position (bx, by) whose left edge is
    /// `rows` luma rows high (4 or 8).
    fn luma_avail(&self, bx: usize, by: usize, rows: usize) -> Avail {
        if MBAFF {
            let ok = |x: i32, y: i32| self.locate(x, y, 16, 16).is_some_and(|(m, _, _)| self.intra_usable(m));
            let (x, y) = (4 * bx as i32, 4 * by as i32);
            let top_right = if by == 0 {
                ok(x + 4 * (rows as i32 / 4), -1)
            } else if bx + rows / 4 > 3 {
                false
            } else {
                RASTER_TO_BLK4[(by - 1) * 4 + bx + 1] < RASTER_TO_BLK4[by * 4 + bx]
            };
            return Avail {
                left: bx > 0 || self.intra_left(4 * by, rows),
                top: by > 0 || ok(x, -1),
                top_left: (bx > 0 && by > 0) || ok(x - 1, y - 1),
                top_right,
            };
        }
        let a = self.intra_nb(0);
        let b = self.intra_nb(1);
        let c = self.intra_nb(2);
        let d = self.intra_nb(3);
        let left = if bx > 0 { true } else { a };
        let top = if by > 0 { true } else { b };
        let top_left = match (bx > 0, by > 0) {
            (true, true) => true,
            (false, true) => a,
            (true, false) => b,
            (false, false) => d,
        };
        let top_right = if by == 0 {
            if bx < 3 { b } else { c }
        } else if bx == 3 {
            false
        } else {
            RASTER_TO_BLK4[(by - 1) * 4 + bx + 1] < RASTER_TO_BLK4[by * 4 + bx]
        };
        Avail { left, top, top_left, top_right }
    }

    /// Reconstruct an intra MB (prediction + residual) after its syntax has been parsed.
    pub fn reconstruct_intra(&mut self) {
        let kind = self.mb().kind;
        // in MBAFF frames a field macroblock's lines are every other picture line: predict in a
        // view with that line stride (the neighbouring samples are then where Table 6-4 puts
        // them)
        let (off, stride, y0) = self.view(self.pic.planes.width, self.y_org);
        let x0 = self.mb_x * 16;
        match kind {
            MbKind::I4x4 => {
                for (blk, &(bx, by)) in BLK4_XY.iter().enumerate() {
                    let (bx, by) = (bx as usize, by as usize);
                    let _ = blk;
                    let av = self.luma_avail4(bx, by);
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    intra::pred4x4(&mut self.pic.planes.y[off..], stride, x0 + bx * 4, y0 + by * 4, mode, av, S::code_max(self.bit_max));
                    self.add_luma_block4(by * 4 + bx);
                }
            }
            MbKind::I8x8 => {
                for b8 in 0..4 {
                    let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
                    let mut av = self.luma_avail(bx, by, 8);
                    av.top_right = match b8 {
                        0 => self.intra_nb(1),
                        1 => self.intra_nb(2),
                        2 => true,
                        _ => false,
                    };
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    intra::pred8x8(&mut self.pic.planes.y[off..], stride, x0 + bx * 4, y0 + by * 4, mode, av, S::code_max(self.bit_max));
                    self.add_luma_block8(b8);
                }
            }
            MbKind::I16x16 => {
                let av = Avail { left: self.intra_left(0, 16), top: self.intra_nb(1), top_left: self.intra_nb(3), top_right: false };
                intra::pred16x16(&mut self.pic.planes.y[off..], stride, x0, y0, self.cur.info.i16_mode, av, S::code_max(self.bit_max));
                for r in 0..16 {
                    self.add_luma_block4(r);
                }
            }
            _ => {}
        }
        let av = Avail { left: self.intra_left(0, 16), top: self.intra_nb(1), top_left: self.intra_nb(3), top_right: false };
        let (coff, cs, cy) = self.view(self.pic.planes.cwidth, self.c_org);
        let mode = self.mb().intra_chroma_mode;
        let max = S::code_max(self.bit_max);
        let cx = self.mb_x * 8;
        if MBAFF {
            // the left halves may lie in different macroblocks (constrained intra prediction)
            let halves = [self.intra_left(0, 8), self.intra_left(8, 8)];
            for plane in [&mut self.pic.planes.cb, &mut self.pic.planes.cr] {
                intra::pred_chroma_split(&mut plane[coff..], cs, cx, cy, mode, av, halves, max, C422);
            }
        } else if C422 {
            intra::pred_chroma_422(&mut self.pic.planes.cb[coff..], cs, cx, cy, mode, av, max);
            intra::pred_chroma_422(&mut self.pic.planes.cr[coff..], cs, cx, cy, mode, av, max);
        } else {
            intra::pred_chroma(&mut self.pic.planes.cb[coff..], cs, cx, cy, mode, av, max);
            intra::pred_chroma(&mut self.pic.planes.cr[coff..], cs, cx, cy, mode, av, max);
        }
        self.add_chroma();
        self.s.blk_nz = 0;
    }

    /// Add the residual of an inter MB (prediction already written).
    pub fn reconstruct_inter_residual(&mut self) {
        if self.mb().transform_8x8 {
            for b8 in 0..4 {
                self.add_luma_block8(b8);
            }
        } else if self.s.blk_nz != 0 {
            for r in 0..16 {
                self.add_luma_block4(r);
            }
        }
        self.add_chroma();
        self.s.blk_nz = 0;
    }

    /// Write I_PCM samples.
    pub fn write_pcm(&mut self, samples: &[u16; 512]) {
        for r in 0..16 {
            let (at, _) = self.luma_at(0, r);
            for (d, &v) in self.pic.planes.y[at..at + 16].iter_mut().zip(&samples[r * 16..r * 16 + 16]) {
                *d = S::from_i32(i32::from(v));
            }
        }
        let ch = self.chroma_blocks() * 2; // chroma lines per MB: 8 or 16
        for c in 0..2 {
            let src = &samples[256 + c * ch * 8..256 + (c + 1) * ch * 8];
            for r in 0..ch {
                let (at, _) = self.chroma_at(0, r);
                let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
                for (d, &v) in plane[at..at + 8].iter_mut().zip(&src[r * 8..r * 8 + 8]) {
                    *d = S::from_i32(i32::from(v));
                }
            }
        }
    }

    /// Chroma 4x4 blocks per component in a macroblock: 4 (4:2:0) or 8 (4:2:2).
    #[inline]
    pub const fn chroma_blocks(&self) -> usize {
        if C422 { 8 } else { 4 }
    }

    /// Bits per I_PCM sample: [BitDepthY, BitDepthC].
    pub fn pcm_bits(&self) -> [u32; 2] {
        [self.sps.bit_depth_luma.clamp(8, 14), self.sps.bit_depth_chroma.clamp(8, 14)]
    }

    /// Common state for an I_PCM MB.
    pub fn finish_pcm(&mut self) {
        let qpc = [self.chroma_qp(0, 0) as i8, self.chroma_qp(0, 1) as i8];
        let st = self.mb_mut();
        st.kind = MbKind::IPcm;
        st.qp = 0;
        st.qpc = qpc;
        st.nnz = [16; 16];
        st.nnz_c = [[16; 8]; 2];
        st.cbf_dc = 7;
        st.cbp = 0x2f;
        st.nz_mask = 0xffff;
        st.ref_idx = [[-1; 4]; 2];
    }

    /// Update QP after mb_qp_delta.
    pub fn apply_qp_delta(&mut self, delta: i32) -> Result<()> {
        self.qp = qp_after_delta(self.qp, delta, self.qp_bd[0])?;
        Ok(())
    }

    pub fn store_qp(&mut self) {
        let qp = self.qp;
        let qpc = [self.chroma_qp(qp, 0) as i8, self.chroma_qp(qp, 1) as i8];
        let st = self.mb_mut();
        st.qp = qp as i8;
        st.qpc = qpc;
    }

    // ------------------------------------------------------------------------------------------
    // Residual storage helpers (shared by CAVLC and CABAC)
    // ------------------------------------------------------------------------------------------

    fn luma_list4(&self) -> usize {
        if self.mb().kind.is_intra() { 0 } else { 3 }
    }

    /// Store a 4x4 luma block given in scan order (levels[0..16]); `ac_only` skips position 0.
    pub fn put_luma4(&mut self, raster: usize, levels: &[i32; 16], ac_only: bool) {
        let qp = self.qp + self.qp_bd[0];
        let ls = &self.ls.ls4[self.luma_list4()][(qp % 6) as usize];
        let blk = &mut self.s.coef[raster * 16..raster * 16 + 16];
        let scan = self.scan4;
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate().skip(ac_only as usize) {
            if lv != 0 {
                // masked: in range without bounds checks (the scans are permutations of 0..16)
                let p = (scan[k] & 15) as usize;
                blk[p] = transform::scale4(lv, ls[p], qp);
                any = true;
            }
        }
        if any {
            self.s.blk_nz |= 1 << raster;
            self.pic.mbs[self.mb_addr].nz_mask |= 1 << raster;
        }
    }

    /// Store an 8x8 luma block given in scan order.
    pub fn put_luma8(&mut self, b8: usize, levels: &[i32; 64]) {
        let qp = self.qp + self.qp_bd[0];
        let list = if self.mb().kind.is_intra() { 0 } else { 1 };
        let ls = &self.ls.ls8[list][(qp % 6) as usize];
        let blk = &mut self.s.coef[b8 * 64..b8 * 64 + 64];
        let scan = self.scan8;
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate() {
            if lv != 0 {
                let p = (scan[k] & 63) as usize;
                blk[p] = transform::scale8(lv, ls[p], qp);
                any = true;
            }
        }
        if any {
            self.s.blk_nz |= 1 << b8;
            let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            let m = (0b11u16 << (by * 4 + bx)) | (0b11u16 << ((by + 1) * 4 + bx));
            self.pic.mbs[self.mb_addr].nz_mask |= m;
        }
    }

    /// Intra16x16 DC levels in scan order -> scaled DC of each 4x4 block.
    pub fn put_luma_dc(&mut self, levels: &[i32; 16]) {
        let mut c = [0i32; 16];
        let scan = self.scan4;
        for (k, &lv) in levels.iter().enumerate() {
            c[(scan[k] & 15) as usize] = lv;
        }
        let qp = self.qp + self.qp_bd[0];
        let ls00 = self.ls.ls4[0][(qp % 6) as usize][0];
        transform::luma_dc_dequant(&mut c, qp, ls00);
        for (r, &dc) in c.iter().enumerate() {
            if dc != 0 {
                self.s.coef[r * 16] = dc;
                self.s.blk_nz |= 1 << r;
            }
        }
    }

    /// Chroma DC levels (4:2:0: 4 values in raster order) for component `c`.
    pub fn put_chroma_dc(&mut self, c: usize, levels: &[i32; 4]) {
        let mut v = *levels;
        let qpc = self.chroma_qp(self.qp, c) + self.qp_bd[1];
        let list = if self.mb().kind.is_intra() { 1 + c } else { 4 + c };
        let ls00 = self.ls.ls4[list][(qpc % 6) as usize][0];
        transform::chroma_dc_dequant_420(&mut v, qpc, ls00);
        for (b, &dc) in v.iter().enumerate() {
            if dc != 0 {
                self.s.coef_c[c][b * 16] = dc;
                self.s.c_nz[c] |= 1 << b;
            }
        }
    }

    /// 4:2:2 chroma DC levels (8 values in chroma DC scan order) for component `c`: scaled at
    /// QP'c + 3 (8.5.11.2).
    pub fn put_chroma_dc_422(&mut self, c: usize, levels: &[i32; 8]) {
        let mut v = *levels;
        let qp_dc = self.chroma_qp(self.qp, c) + self.qp_bd[1] + 3;
        let list = if self.mb().kind.is_intra() { 1 + c } else { 4 + c };
        let ls00 = self.ls.ls4[list][(qp_dc % 6) as usize][0];
        transform::chroma_dc_dequant_422(&mut v, qp_dc, ls00);
        for (b, &dc) in v.iter().enumerate() {
            if dc != 0 {
                self.s.coef_c[c][b * 16] = dc;
                self.s.c_nz[c] |= 1 << b;
            }
        }
    }

    /// Chroma AC levels (scan positions 1..16 in levels[1..]) for block `b` of component `c`.
    pub fn put_chroma_ac(&mut self, c: usize, b: usize, levels: &[i32; 16]) {
        let qpc = self.chroma_qp(self.qp, c) + self.qp_bd[1];
        let list = if self.mb().kind.is_intra() { 1 + c } else { 4 + c };
        let ls = &self.ls.ls4[list][(qpc % 6) as usize];
        let blk = &mut self.s.coef_c[c][b * 16..b * 16 + 16];
        let scan = self.scan4;
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate().skip(1) {
            if lv != 0 {
                let p = (scan[k] & 15) as usize;
                blk[p] = transform::scale4(lv, ls[p], qpc);
                any = true;
            }
        }
        if any {
            self.s.c_nz[c] |= 1 << b;
        }
    }

    // ------------------------------------------------------------------------------------------
    // CAVLC macroblock layer
    // ------------------------------------------------------------------------------------------

    /// Decode a P_Skip or B_Skip macroblock at the current address.
    pub fn decode_skip(&mut self) -> Result<()> {
        let b = self.sh.slice_type.is_b();
        self.cur = MbCur::default();
        self.cur.info.kind = if b { MbKind::BSkip } else { MbKind::PSkip };
        self.mb_mut().kind = self.cur.info.kind;
        self.store_qp();
        if b {
            self.derive_direct(0xf)?;
        } else {
            ensure!(!self.refs[0].is_empty(), "P_Skip with empty reference list");
            self.derive_p_skip();
        }
        self.predict_inter();
        self.prev_qp_delta_nz = false;
        Ok(())
    }

    fn read_te(r: &mut BitReader, range: u32) -> Result<u32> {
        Ok(if range > 1 { r.read_ue()? } else { !r.read_bit()? as u32 })
    }

    /// Parse and reconstruct one macroblock_layer() with CAVLC.
    pub fn decode_mb_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        let mb_type = r.read_ue()?;
        let st = self.sh.slice_type;
        let info = match st {
            SliceType::I | SliceType::Si => intra_mb_type(mb_type),
            SliceType::P | SliceType::Sp => {
                if mb_type < 5 {
                    p_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 5)
                }
            }
            SliceType::B => {
                if mb_type < 23 {
                    b_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 23)
                }
            }
        };
        let Some(mut info) = info else { return invalid(format!("mb_type {mb_type} out of range")) };
        self.cur = MbCur { info, ..Default::default() };
        if info.kind == MbKind::IPcm {
            r.byte_align();
            let (samples, len) = read_pcm(r.data(), r.byte_pos(), self.pcm_bits(), self.chroma_blocks() * 16)?;
            r.skip(len * 8)?;
            self.finish_pcm();
            self.write_pcm(&samples);
            return Ok(());
        }
        let mut transform_8x8 = false;
        if info.kind == MbKind::I4x4 && self.pps.transform_8x8_mode && r.read_flag()? {
            transform_8x8 = true;
            info.kind = MbKind::I8x8;
            self.cur.info.kind = MbKind::I8x8;
        }
        self.mb_mut().kind = info.kind;
        self.mb_mut().transform_8x8 = transform_8x8;
        // MBAFF field macroblocks: 2 * num_ref_idx_active entries (7.4.5.1)
        let k = 1 + (MBAFF && self.mb_field) as u32;
        let n_ref = [self.sh.num_ref_idx_active[0] * k, self.sh.num_ref_idx_active[1] * k];
        match info.kind {
            MbKind::I4x4 | MbKind::I8x8 => {
                let n = if info.kind == MbKind::I8x8 { 4 } else { 16 };
                for i in 0..n {
                    self.cur.rem_mode[i] = if r.read_flag()? { -1 } else { r.read_bits(3)? as i8 };
                }
                self.derive_intra_modes(info.kind == MbKind::I8x8);
                let m = r.read_ue()?;
                ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                self.mb_mut().intra_chroma_mode = m as u8;
            }
            MbKind::I16x16 => {
                let m = r.read_ue()?;
                ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                self.mb_mut().intra_chroma_mode = m as u8;
            }
            MbKind::BDirect16x16 => {}
            _ => {
                if info.part == Part::P8x8 {
                    for p in 0..4 {
                        let t = r.read_ue()?;
                        let sub = if st.is_b() { b_sub_mb_type(t) } else { p_sub_mb_type(t) };
                        let Some(sub) = sub else { return invalid(format!("sub_mb_type {t} out of range")) };
                        self.cur.sub[p] = sub;
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            self.cur.ref_idx[l][p] = if n_ref[l] > 1 && !info.ref0 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                        }
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            for j in 0..sub.shape.num_parts() {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4 + j] = [x as i16, y as i16];
                            }
                        }
                    }
                } else {
                    let np = info.part.num_parts();
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                self.cur.ref_idx[l][p] = if n_ref[l] > 1 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                            }
                        }
                    }
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4] = [x as i16, y as i16];
                            }
                        }
                    }
                }
            }
        }
        // coded_block_pattern
        let cbp = if info.kind == MbKind::I16x16 {
            info.i16_cbp
        } else {
            let code = r.read_ue()?;
            ensure!(code < 48, "coded_block_pattern out of range");
            let (i, p) = CBP_ME[code as usize];
            if info.kind.is_intra() { i } else { p }
        };
        self.cur.cbp = cbp;
        self.mb_mut().cbp = cbp;
        if !info.kind.is_intra() && cbp & 15 != 0 && self.pps.transform_8x8_mode && self.no_sub_8x8_lt() {
            let t = r.read_flag()?;
            self.mb_mut().transform_8x8 = t;
        }
        if !info.kind.is_intra() {
            self.derive_inter_motion()?;
        }
        if cbp != 0 || info.kind == MbKind::I16x16 {
            let d = r.read_se()?;
            self.apply_qp_delta(d)?;
        }
        self.store_qp();
        self.residual_cavlc(r, cbp)?;
        if info.kind.is_intra() {
            self.reconstruct_intra();
        } else {
            self.predict_inter();
            self.reconstruct_inter_residual();
        }
        Ok(())
    }

    /// Whether transform_size_8x8_flag may be present for an inter MB (NoSubMbPartSizeLessThan8x8 etc.).
    pub fn no_sub_8x8_lt(&self) -> bool {
        let info = self.cur.info;
        match info.kind {
            MbKind::BDirect16x16 => self.sps.direct_8x8_inference,
            MbKind::Inter if info.part == Part::P8x8 => {
                self.cur.sub.iter().all(|s| if s.direct { self.sps.direct_8x8_inference } else { s.shape == SubPart::S8x8 })
            }
            _ => true,
        }
    }

    fn residual_cavlc(&mut self, r: &mut BitReader, cbp: u8) -> Result<()> {
        let kind = self.mb().kind;
        let t8 = self.mb().transform_8x8;
        if kind == MbKind::I16x16 {
            let nc = self.nc_luma(0);
            let mut lv = [0i32; 16];
            cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?;
            self.put_luma_dc(&lv);
        }
        for b8 in 0..4 {
            let (bx8, by8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            if cbp & (1 << b8) == 0 {
                continue;
            }
            if t8 {
                let mut lv8 = [0i32; 64];
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(raster);
                    let mut lv = [0i32; 16];
                    let tc = cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?;
                    self.mb_mut().nnz[raster] = tc;
                    for (i, &v) in lv.iter().enumerate() {
                        lv8[4 * i + i4] = v;
                    }
                }
                self.put_luma8(b8, &lv8);
            } else {
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(raster);
                    let mut lv = [0i32; 16];
                    let tc = if kind == MbKind::I16x16 {
                        cavlc::residual_block(r, &mut lv[1..], 0, 14, 15, nc)?
                    } else {
                        cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?
                    };
                    self.mb_mut().nnz[raster] = tc;
                    self.put_luma4(raster, &lv, kind == MbKind::I16x16);
                }
            }
        }
        let cbp_c = cbp >> 4;
        let nblk = self.chroma_blocks();
        if cbp_c & 3 != 0 {
            for c in 0..2 {
                if nblk == 8 {
                    let mut lv = [0i32; 8];
                    cavlc::residual_block(r, &mut lv, 0, 7, 8, -2)?;
                    self.put_chroma_dc_422(c, &lv);
                } else {
                    let mut lv = [0i32; 4];
                    cavlc::residual_block(r, &mut lv, 0, 3, 4, -1)?;
                    self.put_chroma_dc(c, &lv);
                }
            }
        }
        if cbp_c & 2 != 0 {
            for c in 0..2 {
                for b in 0..nblk {
                    let nc = self.nc_chroma(c, b);
                    let mut lv = [0i32; 16];
                    let tc = cavlc::residual_block(r, &mut lv[1..], 0, 14, 15, nc)?;
                    self.mb_mut().nnz_c[c][b] = tc;
                    self.put_chroma_ac(c, b, &lv);
                }
            }
        }
        Ok(())
    }

    /// Decode slice_data() with CAVLC. `r` is positioned after the slice header.
    pub fn decode_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        let total = self.mb_w * self.mb_h;
        // MBAFF: first_mb_in_slice counts macroblock pairs
        let mut addr = (self.sh.first_mb_in_slice as usize).saturating_mul(1 + MBAFF as usize);
        let intra_slice = self.sh.slice_type.is_intra();
        let mut prev_skipped = false;
        // MBAFF: the bottom macroblock's mb_field_decoding_flag, read ahead for its skipped top
        let mut ahead: Option<bool> = None;
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            if !intra_slice {
                let run = r.read_ue()? as usize;
                ensure!(addr + run <= total, "mb_skip_run out of range");
                for i in 0..run {
                    let idx = self.mb_index(addr);
                    self.start_mb(idx);
                    if MBAFF {
                        let field = if addr % 2 == 1 {
                            self.pair_top_field(idx)
                        } else if i + 1 == run && r.more_rbsp_data() {
                            // the top is skipped and the bottom is not: the pair is what the
                            // bottom's flag (next in the stream) says (7.4.4)
                            let f = r.read_flag()?;
                            ahead = Some(f);
                            f
                        } else {
                            self.inferred_mb_field()
                        };
                        self.set_mb_field(field);
                    }
                    self.decode_skip()?;
                    self.pic.mb_done(idx);
                    addr += 1;
                }
                prev_skipped = run > 0;
                if run > 0 && !r.more_rbsp_data() {
                    break;
                }
                ensure!(addr < total, "macroblock address {addr} out of range");
            }
            let idx = self.mb_index(addr);
            self.start_mb(idx);
            if MBAFF {
                let field = if addr.is_multiple_of(2) {
                    r.read_flag()?
                } else if prev_skipped {
                    match ahead.take() {
                        Some(f) => f,
                        None => r.read_flag()?,
                    }
                } else {
                    self.pair_top_field(idx)
                };
                self.set_mb_field(field);
            }
            self.decode_mb_cavlc(r)?;
            self.pic.mb_done(idx);
            prev_skipped = false;
            addr += 1;
            if !r.more_rbsp_data() {
                break;
            }
        }
        Ok(())
    }

    /// MBAFF: the field flag of the top macroblock of the pair whose bottom macroblock is `idx`.
    pub fn pair_top_field(&self, idx: usize) -> bool {
        idx.checked_sub(self.mb_w).and_then(|t| self.pic.mbs.get(t)).is_some_and(|m| m.field)
    }
}

#[cfg(test)]
mod qp_tests {
    use super::*;

    #[test]
    fn pcm_samples_at_8_and_10_bits() {
        let data: Vec<u8> = (0..600u32).map(|i| (i * 37 % 251) as u8).collect();
        let (s8, n8) = read_pcm(&data, 3, [8, 8], 64).unwrap();
        assert_eq!(n8, 384);
        assert!(s8[..384].iter().zip(&data[3..]).all(|(&a, &b)| a == b as u16));
        assert!(s8[384..].iter().all(|&v| v == 0));
        // 10-bit: 3840 bits = 480 bytes, big-endian bit packing
        let (s10, n10) = read_pcm(&data, 0, [10, 10], 64).unwrap();
        assert_eq!(n10, 480);
        let mut r = BitReader::new(&data);
        assert!(s10[..384].iter().all(|&v| v as u32 == r.read_bits(10).unwrap() && v <= 1023));
        // luma 10, chroma 8
        assert_eq!(read_pcm(&data, 0, [10, 8], 64).unwrap().1, 448);
        // 4:2:2: 128 chroma samples per component
        let (s422, n422) = read_pcm(&data, 0, [8, 8], 128).unwrap();
        assert_eq!(n422, 512);
        assert!(s422.iter().zip(&data).all(|(&a, &b)| a == b as u16));
        assert!(read_pcm(&data, 0, [10, 10], 128).is_err()); // 640 bytes needed, 600 there
        assert_eq!(read_pcm(&data, 0, [8, 8], usize::MAX).unwrap().1, 512); // capped
        // truncated / hostile positions are errors, never panics
        assert!(read_pcm(&data, 121, [10, 10], 64).is_err());
        assert!(read_pcm(&data, usize::MAX, [8, 8], 64).is_err());
        assert!(read_pcm(&[], 0, [8, 8], 64).is_err());
    }

    #[test]
    fn qp_delta_wraps_by_bit_depth() {
        // 8-bit: 0 ..= 51
        assert_eq!(qp_after_delta(51, 1, 0).unwrap(), 0);
        assert_eq!(qp_after_delta(0, -1, 0).unwrap(), 51);
        assert_eq!(qp_after_delta(26, 25, 0).unwrap(), 51);
        assert!(qp_after_delta(26, 26, 0).is_err());
        assert!(qp_after_delta(26, -27, 0).is_err());
        // 10-bit (QpBdOffsetY 12): -12 ..= 51, deltas -32 ..= 31
        assert_eq!(qp_after_delta(0, -12, 12).unwrap(), -12);
        assert_eq!(qp_after_delta(-12, -1, 12).unwrap(), 51);
        assert_eq!(qp_after_delta(51, 1, 12).unwrap(), -12);
        assert_eq!(qp_after_delta(20, 31, 12).unwrap(), 51);
        assert_eq!(qp_after_delta(20, -32, 12).unwrap(), -12);
        assert!(qp_after_delta(0, 32, 12).is_err());
        assert!(qp_after_delta(0, -33, 12).is_err());
        // hostile values never panic
        assert!(qp_after_delta(0, i32::MIN, 12).is_err());
        assert!(qp_after_delta(0, i32::MAX, 36).is_err());
    }

    #[test]
    fn chroma_qp_by_bit_depth() {
        assert_eq!(chroma_qp_for(29, 0, 0), 29);
        assert_eq!(chroma_qp_for(51, 0, 0), 39);
        assert_eq!(chroma_qp_for(0, -12, 0), 0);
        assert_eq!(chroma_qp_for(51, 12, 0), 39);
        // 10-bit: negative QP_C down to -12
        assert_eq!(chroma_qp_for(-12, 0, 12), -12);
        assert_eq!(chroma_qp_for(-5, -12, 12), -12);
        assert_eq!(chroma_qp_for(30, 0, 12), 29);
        assert_eq!(chroma_qp_for(i32::MIN / 2, 0, 12), -12);
    }
}
