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
    /// A field macroblock of an MBAFF frame (mb_field_decoding_flag, fork H2.6).
    pub field: bool,
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
            field: false,
        }
    }
}

/// One published macroblock row of a decoded frame: final (deblocked) samples plus the motion data
/// needed as co-located information for direct prediction.
#[derive(Clone)]
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
    /// Per MB: a field macroblock of an MBAFF frame (its vectors are in field units).
    pub field: Box<[bool]>,
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
    /// Frame MB rows. For a field pair they are interleaved from the two fields
    /// ([`Frame::interleave_row`]) so frame readers never see the difference.
    rows: Box<[OnceLock<FrameRow<S>>]>,
    /// A field-coded frame (PAFF, fork H2.5): its top and bottom fields as half-height frames,
    /// each published by its own decoding job; field readers use the `field_*` windows.
    fields: Option<Box<[FrameRef<S>; 2]>>,
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
            fields: None,
        }
    }

    /// A frame coded as two field pictures: `mb_h` frame MB rows (even), each field `mb_h / 2`
    /// rows of 16 field lines.
    pub fn field_pair(id: u32, poc: i32, mb_w: usize, mb_h: usize, bit_depth: u8, c422: bool) -> Self {
        let half = mb_h / 2;
        let field = || Arc::new(Frame::with_depth(id, poc, mb_w, half, bit_depth, c422));
        Frame { fields: Some(Box::new([field(), field()])), ..Frame::with_depth(id, poc, mb_w, half * 2, bit_depth, c422) }
    }

    /// Field `parity` (0 top, 1 bottom) of a field-coded frame.
    pub fn field(&self, parity: usize) -> Option<&FrameRef<S>> {
        self.fields.as_ref().and_then(|f| f.get(parity))
    }

    /// For a field pair: publish every frame MB row from the two fields (waits for both).
    pub fn interleave_all(&self) {
        for r in 0..self.rows.len().div_ceil(2) {
            self.interleave_row(r);
        }
    }

    /// For a field pair whose field `present` has no second field (an unpaired field): the
    /// missing field becomes a copy of the present one (each line doubled in the frame) and the
    /// frame rows are published, so readers and output never wait on it. Waits for the present
    /// field; rows the missing field already has are kept.
    pub fn fill_missing_field(&self, present: usize) {
        let (Some(have), Some(missing)) = (self.field(present), self.field(1 - (present & 1))) else { return };
        for r in 0..have.mb_h() {
            if !missing.is_published(r) {
                missing.publish(r, have.row(r).clone());
            }
        }
        self.interleave_all();
    }

    pub fn mb_h(&self) -> usize {
        self.rows.len()
    }

    /// Publish MB row `r` (ignored if already published or out of range).
    pub fn publish(&self, r: usize, row: FrameRow<S>) {
        if let Some(slot) = self.rows.get(r) {
            let _ = slot.set(row);
        }
    }

    pub fn is_published(&self, r: usize) -> bool {
        self.rows.get(r).is_some_and(|s| s.get().is_some())
    }

    /// For a field pair: build and publish frame MB rows 2r and 2r + 1 from field row r of both
    /// fields (waits for both). Motion data of the frame rows is left empty (intra, no
    /// references) until H2.5d defines colocated data for frames over field pairs.
    pub fn interleave_row(&self, r: usize) {
        let Some(f) = &self.fields else { return };
        let (top, bot) = (f[0].row(r), f[1].row(r));
        let (w, cw) = (self.width, self.cwidth);
        let cl = 1usize << self.c_shift; // chroma lines per MB row
        for half in 0..2 {
            let fr = 2 * r + half;
            if fr >= self.rows.len() || self.is_published(fr) {
                continue;
            }
            // frame line k of this MB row = field line (16 * half + k) / 2 of field k & 1
            let y: Box<[S]> = (0..16)
                .flat_map(|k| {
                    let src = if k & 1 == 0 { &top.y } else { &bot.y };
                    let l = (16 * half + k) / 2;
                    src[l * w..(l + 1) * w].iter().copied()
                })
                .collect();
            let chroma = |pick: fn(&FrameRow<S>) -> &[S]| -> Box<[S]> {
                (0..cl)
                    .flat_map(|k| {
                        let src = pick(if k & 1 == 0 { top } else { bot });
                        let l = (cl * half + k) / 2;
                        src[l * cw..(l + 1) * cw].iter().copied()
                    })
                    .collect()
            };
            let mbw = self.mb_w;
            self.publish(
                fr,
                FrameRow {
                    y,
                    cb: chroma(|r| &r.cb),
                    cr: chroma(|r| &r.cr),
                    mv: [vec![[0i16; 2]; mbw * 16].into(), vec![[0i16; 2]; mbw * 16].into()],
                    ref_idx: [vec![-1i8; mbw * 4].into(), vec![-1i8; mbw * 4].into()],
                    ref_id: [vec![u32::MAX; mbw * 4].into(), vec![u32::MAX; mbw * 4].into()],
                    intra: vec![true; mbw].into(),
                    field: vec![false; mbw].into(),
                },
            );
        }
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

    /// Frame luma line `yy` (< height), waiting for it.
    #[inline]
    fn luma_line(&self, yy: usize) -> &[S] {
        let row = self.row(yy >> 4);
        &row.y[(yy & 15) * self.width..(yy & 15) * self.width + self.width]
    }

    /// Frame chroma line `yy` (< cheight) of component `c`, waiting for it.
    #[inline]
    fn chroma_line(&self, c: usize, yy: usize) -> &[S] {
        let row = self.row(yy >> self.c_shift);
        let l = yy & ((1 << self.c_shift) - 1);
        let plane = if c == 0 { &row.cb } else { &row.cr };
        &plane[l * self.cwidth..l * self.cwidth + self.cwidth]
    }

    /// Copy the w x h luma window at (x0, y0) of field `parity` (0 top, 1 bottom; clamped to
    /// the field) into `out`: a field picture's own rows, or every other line of a frame.
    pub fn field_luma_window(&self, parity: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        if let Some(f) = self.field(parity) {
            return f.luma_window(x0, y0, w, h, out, os);
        }
        let fh = (self.height / 2) as i32;
        for r in 0..h {
            let l = (y0 + r as i32).clamp(0, fh - 1) as usize;
            copy_line::<S, 24>(self.luma_line(2 * l + (parity & 1)), x0, &mut out[r * os..], w);
        }
    }

    /// [`Frame::field_luma_window`] for chroma component `c`.
    pub fn field_chroma_window(&self, c: usize, parity: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        if let Some(f) = self.field(parity) {
            return f.chroma_window(c, x0, y0, w, h, out, os);
        }
        let fh = (self.cheight / 2) as i32;
        for r in 0..h {
            let l = (y0 + r as i32).clamp(0, fh - 1) as usize;
            copy_line::<S, 16>(self.chroma_line(c, 2 * l + (parity & 1)), x0, &mut out[r * os..], w);
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
        let mut field = vec![false; mb_w];
        for (x, st) in row_mbs.iter().enumerate() {
            if st.slice_num == u32::MAX {
                continue;
            }
            intra[x] = st.kind.is_intra();
            field[x] = st.field;
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
        FrameRow {
            y,
            cb,
            cr,
            mv: [mv0.into(), mv1.into()],
            ref_idx: [ri0.into(), ri1.into()],
            ref_id: [id0.into(), id1.into()],
            intra: intra.into(),
            field: field.into(),
        }
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
    /// A reference field (fork H2.5): its parity (0 top, 1 bottom) in `frame`; `None`: the frame.
    pub parity: Option<usize>,
    /// PicOrderCnt of the reference: the frame's, or the field's.
    pub poc: i32,
    /// Top and bottom field order counts of the entry (a field pair's colocated field choice).
    pub field_pocs: [i32; 2],
}

/// The id of field `parity` of the frame with id `frame_id` ([`RefPic::id`]).
pub fn field_ref_id(frame_id: u32, parity: usize) -> u32 {
    0x8000_0000 | ((frame_id & 0x1fff_ffff) << 1) | (parity as u32 & 1)
}

/// Whether reference id `id` names a field of the frame with id `frame_id`.
pub fn is_field_of(id: u32, frame_id: u32) -> bool {
    id & 0x8000_0000 != 0 && id != u32::MAX && id != u32::MAX - 1 && (id >> 1) & 0x1fff_ffff == frame_id & 0x1fff_ffff
}

impl<S: Sample> RefPic<S> {
    /// The whole frame as a reference.
    pub fn frame(frame: FrameRef<S>, long_term: bool) -> Self {
        RefPic { poc: frame.poc, field_pocs: [frame.poc; 2], frame, long_term, parity: None }
    }
    pub fn poc(&self) -> i32 {
        self.poc
    }
    /// Unique id of the reference picture: the frame's id, or for a field one of its own (bit 31
    /// set, never `u32::MAX` / `u32::MAX - 1`), so deblocking compares reference fields.
    pub fn id(&self) -> u32 {
        match self.parity {
            None => self.frame.id,
            Some(p) => field_ref_id(self.frame.id, p),
        }
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

    /// A field pair (PAFF) reads like the frame its lines interleave into, and a frame-coded
    /// frame reads like its two fields through the field views (fork H2.5a).
    #[test]
    fn field_pair_and_frame_views_agree() {
        let (w, h) = (32usize, 64usize);
        // frame line y holds y * 3 + x (luma), y * 2 + x (chroma) for a recognisable pattern
        let mut frame = Planes::<u16>::with_depth(w, h, 10, false);
        for y in 0..h {
            for x in 0..w {
                frame.y[y * w + x] = (y * 3 + x) as u16;
            }
        }
        for y in 0..h / 2 {
            for x in 0..w / 2 {
                frame.cb[y * w / 2 + x] = (y * 2 + x) as u16;
                frame.cr[y * w / 2 + x] = (1000 + y * 2 + x) as u16;
            }
        }
        let progressive = Frame::from_planes(1, 0, &frame, 10);
        // the same picture as two fields
        let pair = Frame::<u16>::field_pair(2, 0, w / 16, h / 16, 10, false);
        let blank = vec![MbState::default(); (w / 16) * (h / 32)];
        for parity in 0..2 {
            let mut f = Planes::<u16>::with_depth(w, h / 2, 10, false);
            for l in 0..h / 2 {
                f.y[l * w..(l + 1) * w].copy_from_slice(&frame.y[(2 * l + parity) * w..(2 * l + parity + 1) * w]);
            }
            for l in 0..h / 4 {
                let src = (2 * l + parity) * w / 2;
                f.cb[l * w / 2..(l + 1) * w / 2].copy_from_slice(&frame.cb[src..src + w / 2]);
                f.cr[l * w / 2..(l + 1) * w / 2].copy_from_slice(&frame.cr[src..src + w / 2]);
            }
            let field = pair.field(parity).unwrap();
            for r in 0..field.mb_h() {
                field.publish(r, Frame::make_row(&f, r, &blank, &|_, _, _| u32::MAX));
            }
        }
        assert_eq!((pair.mb_h(), pair.height), (4, 64));
        assert!(!pair.is_published(0));
        for r in 0..2 {
            pair.interleave_row(r);
        }
        assert!((0..4).all(|r| pair.is_published(r)));
        pair.wait_complete();
        // frame views agree (including clamping above / below)
        let window = |f: &Frame<u16>, field: Option<usize>, c: Option<usize>, x0: i32, y0: i32| {
            let mut out = vec![0u16; 24 * 24];
            match (field, c) {
                (None, None) => f.luma_window(x0, y0, 9, 21, &mut out, 24),
                (Some(p), None) => f.field_luma_window(p, x0, y0, 9, 21, &mut out, 24),
                (None, Some(c)) => f.chroma_window(c, x0, y0, 5, 9, &mut out, 24),
                (Some(p), Some(c)) => f.field_chroma_window(c, p, x0, y0, 5, 9, &mut out, 24),
            }
            out
        };
        for (x0, y0) in [(-3, -4), (5, 10), (20, 50), (27, 60)] {
            assert_eq!(window(&pair, None, None, x0, y0), window(&progressive, None, None, x0, y0), "luma frame view at {x0},{y0}");
            for c in 0..2 {
                assert_eq!(window(&pair, None, Some(c), x0 / 2, y0 / 2), window(&progressive, None, Some(c), x0 / 2, y0 / 2));
            }
            for p in 0..2 {
                assert_eq!(window(&pair, Some(p), None, x0, y0 / 2), window(&progressive, Some(p), None, x0, y0 / 2), "field {p}");
                for c in 0..2 {
                    assert_eq!(window(&pair, Some(p), Some(c), x0 / 2, y0 / 4), window(&progressive, Some(p), Some(c), x0 / 2, y0 / 4));
                }
            }
        }
        // a field view line is every other frame line
        let top = window(&progressive, Some(1), None, 0, 3);
        assert_eq!(top[0], (7 * 3) as u16);
        // output interleaves the fields
        let crop = (2, 3, 28, 57);
        assert_eq!(pair.copy_cropped_with(crop, Vec::with_capacity), progressive.copy_cropped_with(crop, Vec::with_capacity));
        // a frame-coded frame has no fields; publishing into a field pair's frame rows is ignored
        assert!(progressive.field(0).is_none());
        pair.publish(0, Frame::make_row(&frame, 0, &blank, &|_, _, _| u32::MAX));
        assert!(pair.field(2).is_none());
    }

    #[test]
    fn hostile_bit_depth_is_clamped() {
        let p = Planes::<u16>::with_depth(16, 16, 200, false);
        assert_eq!(p.cb[0], 1 << 13);
        let p = Planes::<u8>::with_depth(16, 16, 0, false);
        assert_eq!(p.cb[0], 128);
    }
}
