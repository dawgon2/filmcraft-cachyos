//! Picture buffers and per-macroblock state.

use std::sync::{Arc, OnceLock};

use crate::sample::Sample;

/// Planar 4:2:0 or 4:2:2 sample buffers (MB-aligned dimensions, stride == width): `u8` for
/// 8-bit pictures, `u16` for higher bit depths (fork H2).
#[derive(Clone)]
pub struct Planes<S: Sample = u8> {
    pub y: Vec<S>,
    pub cb: Vec<S>,
    pub cr: Vec<S>,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    /// Chroma height: `height / 2` (4:2:0) or `height` (4:2:2).
    pub cheight: usize,
}

impl<S: Sample> Planes<S> {
    /// Black luma and neutral chroma at `bit_depth` (clamped to 8..=14, the range H.264 allows),
    /// 4:2:2 chroma when `c422`.
    pub fn with_depth(width: usize, height: usize, bit_depth: u8, c422: bool) -> Self {
        let (cw, ch) = (width / 2, if c422 { height } else { height / 2 });
        let mid = S::from_i32(1 << (bit_depth.clamp(8, 14) - 1));
        Planes { y: vec![S::default(); width * height], cb: vec![mid; cw * ch], cr: vec![mid; cw * ch], width, height, cwidth: cw, cheight: ch }
    }
    /// Mid-gray luma and neutral chroma at `bit_depth`.
    pub fn gray_with_depth(width: usize, height: usize, bit_depth: u8, c422: bool) -> Self {
        let mut p = Self::with_depth(width, height, bit_depth, c422);
        let mid = S::from_i32(1 << (bit_depth.clamp(8, 14) - 1));
        p.y.fill(mid);
        p
    }
}

/// Macroblock coding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MbKind {
    #[default]
    None,
    I4x4,
    I8x8,
    I16x16,
    IPcm,
    PSkip,
    BSkip,
    BDirect16x16,
    /// Any other P or B inter macroblock.
    Inter,
}

impl MbKind {
    #[inline]
    pub fn is_intra(self) -> bool {
        matches!(self, MbKind::I4x4 | MbKind::I8x8 | MbKind::I16x16 | MbKind::IPcm)
    }
}

/// Per-macroblock state kept for the whole picture (neighbour derivations, deblocking, co-located data).
#[derive(Clone, Copy)]
pub struct MbState {
    /// Slice number within the picture; `u32::MAX` = not (yet) decoded.
    pub slice_num: u32,
    pub kind: MbKind,
    pub transform_8x8: bool,
    /// coded_block_pattern: bits 0..3 luma, bits 4..5 chroma.
    pub cbp: u8,
    /// QP_Y (negative down to -QpBdOffsetY above 8 bits).
    pub qp: i8,
    /// QPc for Cb / Cr (used by deblocking).
    pub qpc: [i8; 2],
    pub intra_chroma_mode: u8,
    /// Intra4x4PredMode / Intra8x8PredMode per 4x4 block (raster order).
    pub intra_modes: [u8; 16],
    /// CAVLC: TotalCoeff per luma 4x4 block (raster). CABAC: coded_block_flag.
    pub nnz: [u8; 16],
    /// Same for chroma AC blocks [Cb, Cr][raster 2 wide: 2x2 (4:2:0) or 2x4 (4:2:2)].
    pub nnz_c: [[u8; 8]; 2],
    /// CABAC coded_block_flag of DC blocks: bit0 luma (I16x16), bit1 Cb, bit2 Cr.
    pub cbf_dc: u8,
    /// Luma 4x4 blocks (raster bit) with non-zero coefficients, for deblocking bS = 2.
    pub nz_mask: u16,
    pub ref_idx: [[i8; 4]; 2],
    /// Motion vectors per 4x4 block (raster order).
    pub mv: [[[i16; 2]; 16]; 2],
    /// |mvd| per 4x4 block (raster), saturated, for CABAC context selection.
    pub mvd: [[[u8; 2]; 16]; 2],
    /// Bit per 8x8 block predicted in direct mode.
    pub direct8x8: u8,
}

impl MbState {
    #[inline]
    pub fn kind_is_skip(&self) -> bool {
        matches!(self.kind, MbKind::PSkip | MbKind::BSkip)
    }
}

impl Default for MbState {
    fn default() -> Self {
        MbState {
            slice_num: u32::MAX,
            kind: MbKind::None,
            transform_8x8: false,
            cbp: 0,
            qp: 0,
            qpc: [0; 2],
            intra_chroma_mode: 0,
            intra_modes: [2; 16],
            nnz: [0; 16],
            nnz_c: [[0; 8]; 2],
            cbf_dc: 0,
            nz_mask: 0,
            ref_idx: [[-1; 4]; 2],
            mv: [[[0; 2]; 16]; 2],
            mvd: [[[0; 2]; 16]; 2],
            direct8x8: 0,
        }
    }
}

/// One published macroblock row of a decoded frame: final (deblocked) samples plus the motion data
/// needed as co-located information for direct prediction.
pub struct FrameRow<S: Sample = u8> {
    /// 16 luma lines.
    pub y: Box<[S]>,
    /// 8 (4:2:0) or 16 (4:2:2) lines per chroma component.
    pub cb: Box<[S]>,
    pub cr: Box<[S]>,
    /// Per 4x4 block: index mb_x * 16 + raster.
    pub mv: [Box<[[i16; 2]]>; 2],
    /// Per 8x8 block: index mb_x * 4 + b8.
    pub ref_idx: [Box<[i8]>; 2],
    /// Unique id of the referenced picture per 8x8 block (u32::MAX = none).
    pub ref_id: [Box<[u32]>; 2],
    pub intra: Box<[bool]>,
}

/// A decoded (or in-progress) frame as stored in the DPB and referenced by later pictures.
///
/// Rows are published in order by the thread decoding the picture; readers block (per row) until the
/// data they need is available, which lets several pictures decode concurrently.
pub struct Frame<S: Sample = u8> {
    pub id: u32,
    pub poc: i32,
    /// BitDepth of the samples (8 for `u8` frames).
    pub bit_depth: u8,
    /// 4:2:2 chroma (full height); 4:2:0 otherwise.
    pub c422: bool,
    /// log2 of the chroma lines per macroblock row: 3 (4:2:0) or 4 (4:2:2).
    c_shift: u32,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub cheight: usize,
    pub mb_w: usize,
    rows: Box<[OnceLock<FrameRow<S>>]>,
}

impl Frame {
    /// Crop and copy the frame into planar buffers from the host's plane allocator (waits for
    /// completion).
    pub fn copy_cropped(&self, crop: (usize, usize, usize, usize)) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        self.copy_cropped_with(crop, plane_allocator())
    }
}

impl<S: Sample> Frame<S> {
    pub fn with_depth(id: u32, poc: i32, mb_w: usize, mb_h: usize, bit_depth: u8, c422: bool) -> Self {
        let c_shift = if c422 { 4 } else { 3 };
        Frame {
            id,
            poc,
            bit_depth,
            c422,
            c_shift,
            width: mb_w * 16,
            height: mb_h * 16,
            cwidth: mb_w * 8,
            cheight: mb_h << c_shift,
            mb_w,
            rows: (0..mb_h).map(|_| OnceLock::new()).collect(),
        }
    }

    pub fn mb_h(&self) -> usize {
        self.rows.len()
    }

    /// Publish MB row `r` (ignored if already published).
    pub fn publish(&self, r: usize, row: FrameRow<S>) {
        let _ = self.rows[r].set(row);
    }

    pub fn is_published(&self, r: usize) -> bool {
        self.rows[r].get().is_some()
    }

    /// MB row `r`, waiting until it has been published.
    #[inline]
    pub fn row(&self, r: usize) -> &FrameRow<S> {
        #[cfg(feature = "threads")]
        {
            self.rows[r].wait()
        }
        #[cfg(not(feature = "threads"))]
        {
            self.rows[r].get().expect("reference row decoded before use")
        }
    }

    /// Wait until the whole frame is available.
    pub fn wait_complete(&self) {
        for r in 0..self.rows.len() {
            self.row(r);
        }
    }

    /// Build a published row from a contiguous plane set (rows of MB row `r`) and motion data.
    pub fn make_row(planes: &Planes<S>, r: usize, mbs: &[MbState], ref_ids: &dyn Fn(&MbState, usize, i8) -> u32) -> FrameRow<S> {
        let w = planes.width;
        let cw = planes.cwidth;
        let mb_w = w / 16;
        let y = planes.y[r * 16 * w..(r + 1) * 16 * w].into();
        // chroma lines per MB row: 8 (4:2:0) or 16 (4:2:2)
        let cl = if planes.cheight == planes.height { 16 } else { 8 };
        let cb = planes.cb[r * cl * cw..(r + 1) * cl * cw].into();
        let cr = planes.cr[r * cl * cw..(r + 1) * cl * cw].into();
        let row_mbs = &mbs[r * mb_w..(r + 1) * mb_w];
        let mut mv = [vec![[0i16; 2]; mb_w * 16], vec![[0i16; 2]; mb_w * 16]];
        let mut ref_idx = [vec![-1i8; mb_w * 4], vec![-1i8; mb_w * 4]];
        let mut ref_id = [vec![u32::MAX; mb_w * 4], vec![u32::MAX; mb_w * 4]];
        let mut intra = vec![true; mb_w];
        for (x, st) in row_mbs.iter().enumerate() {
            if st.slice_num == u32::MAX {
                continue;
            }
            intra[x] = st.kind.is_intra();
            for l in 0..2 {
                mv[l][x * 16..x * 16 + 16].copy_from_slice(&st.mv[l]);
                for b in 0..4 {
                    let ri = st.ref_idx[l][b];
                    ref_idx[l][x * 4 + b] = ri;
                    if ri >= 0 {
                        ref_id[l][x * 4 + b] = ref_ids(st, l, ri);
                    }
                }
            }
        }
        let [mv0, mv1] = mv;
        let [ri0, ri1] = ref_idx;
        let [id0, id1] = ref_id;
        FrameRow { y, cb, cr, mv: [mv0.into(), mv1.into()], ref_idx: [ri0.into(), ri1.into()], ref_id: [id0.into(), id1.into()], intra: intra.into() }
    }

    /// A frame with every row published from `planes` and no motion (intra), e.g. "non-existing"
    /// frames inferred for frame_num gaps.
    pub fn from_planes(id: u32, poc: i32, planes: &Planes<S>, bit_depth: u8) -> Self {
        let (mb_w, mb_h) = (planes.width / 16, planes.height / 16);
        let f = Frame::with_depth(id, poc, mb_w, mb_h, bit_depth, planes.cheight == planes.height);
        let blank = vec![MbState::default(); mb_w * mb_h];
        for r in 0..mb_h {
            f.publish(r, Frame::make_row(planes, r, &blank, &|_, _, _| u32::MAX));
        }
        f
    }

    /// Copy the w x h luma window at (x0, y0) (clamped to the picture) into `out` (stride `os`).
    #[inline]
    pub fn luma_window(&self, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (width, height) = (self.width, self.height);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, height as i32 - 1) as usize;
            let row = self.row(yy >> 4);
            let line = &row.y[(yy & 15) * width..(yy & 15) * width + width];
            copy_line::<S, 24>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Copy the w x h window of chroma component `c` at (x0, y0) (clamped) into `out`.
    #[inline]
    pub fn chroma_window(&self, c: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        // the chroma lines per MB row as a constant (keeps the 4:2:0 path's speed)
        if self.c422 {
            self.chroma_window_n::<4>(c, x0, y0, w, h, out, os);
        } else {
            self.chroma_window_n::<3>(c, x0, y0, w, h, out, os);
        }
    }

    #[inline(always)]
    fn chroma_window_n<const SHIFT: u32>(&self, c: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (cw, ch) = (self.cwidth, self.cheight);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, ch as i32 - 1) as usize;
            let row = self.row(yy >> SHIFT);
            let l = yy & ((1 << SHIFT) - 1);
            let plane = if c == 0 { &row.cb } else { &row.cr };
            let line = &plane[l * cw..l * cw + cw];
            copy_line::<S, 16>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Crop and copy the frame into planar buffers from `alloc` (an empty buffer with room for the
    /// given number of samples; waits for completion).
    pub fn copy_cropped_with(&self, crop: (usize, usize, usize, usize), alloc: fn(usize) -> Vec<S>) -> (Vec<S>, Vec<S>, Vec<S>) {
        let (cx, cy, cw, ch) = crop;
        let mut y = alloc(cw * ch);
        for r in cy..cy + ch {
            let row = self.row(r >> 4);
            y.extend_from_slice(&row.y[(r & 15) * self.width + cx..(r & 15) * self.width + cx + cw]);
        }
        let (ccy, cch) = if self.c422 { (cy, ch) } else { (cy / 2, ch.div_ceil(2)) };
        let (ccx, ccw) = (cx / 2, cw.div_ceil(2));
        let mut u = alloc(ccw * cch);
        let mut v = alloc(ccw * cch);
        for r in ccy..ccy + cch {
            let row = self.row(r >> self.c_shift);
            let o = (r & ((1 << self.c_shift) - 1)) * self.cwidth + ccx;
            u.extend_from_slice(&row.cb[o..o + ccw]);
            v.extend_from_slice(&row.cr[o..o + ccw]);
        }
        (y, u, v)
    }
}

static PLANE_ALLOCATOR: std::sync::OnceLock<fn(usize) -> Vec<u8>> = std::sync::OnceLock::new();

/// Where output planes come from: an empty buffer with room for the given number of samples.
/// The host sets it once to recycle the planes of pictures it is done with; the first call wins.
pub fn set_plane_allocator(alloc: fn(usize) -> Vec<u8>) {
    let _ = PLANE_ALLOCATOR.set(alloc);
}

fn plane_allocator() -> fn(usize) -> Vec<u8> {
    PLANE_ALLOCATOR.get().copied().unwrap_or(Vec::with_capacity)
}

/// Copy `w` samples of `line` starting at x0 into `out`; when possible a fixed-size block of `N`
/// samples is copied instead (cheaper than a variable-length copy; `out` must then have room for `N`).
#[inline(always)]
fn copy_line<S: Sample, const N: usize>(line: &[S], x0: i32, out: &mut [S], w: usize) {
    if w <= N
        && x0 >= 0
        && let Some(src) = line.get(x0 as usize..).and_then(|l| l.first_chunk::<N>())
        && let Some(dst) = out.first_chunk_mut::<N>()
    {
        *dst = *src;
    } else {
        copy_clamped(line, x0, &mut out[..w]);
    }
}

/// Copy `out.len()` samples of `line` starting at x0, replicating edge samples outside the line.
#[inline(always)]
fn copy_clamped<S: Sample>(line: &[S], x0: i32, out: &mut [S]) {
    let w = out.len();
    if x0 >= 0 && x0 as usize + w <= line.len() {
        out.copy_from_slice(&line[x0 as usize..x0 as usize + w]);
    } else {
        let max = line.len() as i32 - 1;
        for (c, o) in out.iter_mut().enumerate() {
            *o = line[(x0 + c as i32).clamp(0, max) as usize];
        }
    }
}

pub type FrameRef<S = u8> = Arc<Frame<S>>;

/// An entry of RefPicList0/1 for the current slice.
#[derive(Clone)]
pub struct RefPic<S: Sample = u8> {
    pub frame: FrameRef<S>,
    pub long_term: bool,
}

impl<S: Sample> RefPic<S> {
    pub fn poc(&self) -> i32 {
        self.frame.poc
    }
    pub fn id(&self) -> u32 {
        self.frame.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_bit_frame_stores_windows_and_crops() {
        let mut p = Planes::<u16>::with_depth(32, 32, 10, false);
        assert_eq!(p.cb[0], 512);
        assert_eq!(Planes::<u16>::gray_with_depth(16, 16, 10, false).y[0], 512);
        for (i, v) in p.y.iter_mut().enumerate() {
            *v = (i % 1024) as u16;
        }
        p.cr.fill(1023);
        let f = Frame::from_planes(7, 0, &p, 10);
        assert_eq!((f.bit_depth, f.mb_h()), (10, 2));
        // Window straddling the left / top edge replicates edge samples.
        let mut win = vec![0u16; 4 * 24];
        f.luma_window(-2, -1, 4, 3, &mut win, 24);
        assert_eq!(&win[..4], &[0, 0, 0, 1]);
        assert_eq!(&win[24..28], &[0, 0, 0, 1]);
        assert_eq!(&win[48..52], &[32, 32, 32, 33]);
        // Fast fixed-size path inside the picture.
        let mut win = vec![0u16; 24];
        f.luma_window(3, 20, 20, 1, &mut win, 24);
        assert_eq!(win[0], 20 * 32 + 3);
        let mut cw = vec![0u16; 16];
        f.chroma_window(1, 0, 0, 8, 1, &mut cw, 16);
        assert_eq!(cw[0], 1023);
        let (y, u, v) = f.copy_cropped_with((1, 1, 30, 30), Vec::with_capacity);
        assert_eq!((y.len(), u.len(), v.len()), (900, 225, 225));
        assert_eq!((y[0], u[0], v[0]), (33, 512, 1023));
    }

    #[test]
    fn four_two_two_frame_stores_full_height_chroma() {
        let mut p = Planes::<u16>::with_depth(32, 32, 10, true);
        assert_eq!((p.cwidth, p.cheight, p.cb.len()), (16, 32, 512));
        for (i, v) in p.cb.iter_mut().enumerate() {
            *v = i as u16; // value = y * 16 + x
        }
        let f = Frame::from_planes(1, 0, &p, 10);
        assert!(f.c422);
        assert_eq!((f.cheight, f.mb_h()), (32, 2));
        // chroma line 20 lives in MB row 1 (16 chroma lines per MB row)
        let mut w = vec![0u16; 16];
        f.chroma_window(0, 2, 20, 4, 1, &mut w, 16);
        assert_eq!(&w[..4], &[20 * 16 + 2, 20 * 16 + 3, 20 * 16 + 4, 20 * 16 + 5]);
        // cropping keeps full-height chroma
        let (_, u, _) = f.copy_cropped_with((2, 3, 28, 27), Vec::with_capacity);
        assert_eq!(u.len(), 14 * 27);
        assert_eq!(u[0], 3 * 16 + 1);
        // 4:2:0 is unchanged
        let f = Frame::from_planes(2, 0, &Planes::<u8>::with_depth(32, 32, 8, false), 8);
        assert!(!f.c422);
        assert_eq!(f.cheight, 16);
    }

    #[test]
    fn hostile_bit_depth_is_clamped() {
        let p = Planes::<u16>::with_depth(16, 16, 200, false);
        assert_eq!(p.cb[0], 1 << 13);
        let p = Planes::<u8>::with_depth(16, 16, 0, false);
        assert_eq!(p.cb[0], 128);
    }
}
