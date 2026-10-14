//! Decoded picture buffer: reference marking (8.2.5), reference list construction (8.2.4) and output
//! ordering / bumping (C.4).
//!
//! The buffer only does bookkeeping on picture headers; what a picture is (`F`: the software
//! decoder's [`crate::picture::FrameRef`], a hardware decoder's surface) is up to the decoder, so
//! hardware decoders driven by the same headers (VA-API) share this logic and output order.
//!
//! Field pictures (PAFF): an entry is a frame buffer holding a frame, a complementary field pair
//! or a single field, with a reference mark and POC per field ([`FieldState`]). A first field is
//! stored on its own and its second field joins the same entry ([`Dpb::is_second_field`]);
//! marking (sliding window, MMCOs) works on fields as 8.2.5 describes. `DpbEntry::mark` stays the
//! frame-level view (both fields marked alike), which is what frame decoding refers to.

use crate::error::{Result, ensure};
use crate::slice::{Mmco, SliceHeader, SliceType};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefMark {
    Unused,
    Short,
    Long,
}

/// Output-related metadata attached to a decoded frame.
#[derive(Clone, Debug)]
pub struct OutputMeta {
    pub pts: i64,
    pub key: bool,
    pub crop: (u32, u32, u32, u32),
    pub full_range: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub sar: (u16, u16),
    /// Decoded without deblocking (draft mode, non-reference picture).
    pub draft: bool,
}

/// Field parity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    Top,
    Bottom,
}

impl Parity {
    pub fn of(sh: &SliceHeader) -> Self {
        if sh.bottom_field { Parity::Bottom } else { Parity::Top }
    }
    pub fn opposite(self) -> Self {
        match self {
            Parity::Top => Parity::Bottom,
            Parity::Bottom => Parity::Top,
        }
    }
}

/// One field of a DPB entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldState {
    /// The field has been decoded (frames and complete pairs have both).
    pub present: bool,
    pub poc: i32,
    pub mark: RefMark,
}

impl FieldState {
    const ABSENT: FieldState = FieldState { present: false, poc: 0, mark: RefMark::Unused };
    fn new(poc: i32, mark: RefMark) -> Self {
        FieldState { present: true, poc, mark }
    }
    fn is(&self, mark: RefMark) -> bool {
        self.present && self.mark == mark
    }
}

/// Top and bottom field order counts of a picture (both equal the frame POC for callers that
/// only know a frame POC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldPocs {
    pub top: i32,
    pub bottom: i32,
}

pub struct DpbEntry<F> {
    pub frame: F,
    /// Output order: the frame POC (smaller of the present fields' POCs).
    pub poc: i32,
    pub frame_num: u32,
    pub long_term_frame_idx: u32,
    /// Frame-level marking: `Short` / `Long` only when both fields are present and marked so
    /// (8.2.4.1: what frame decoding may refer to). Kept in step with `top` / `bottom`.
    pub mark: RefMark,
    pub needed_for_output: bool,
    pub non_existing: bool,
    pub meta: Arc<OutputMeta>,
    pub top: FieldState,
    pub bottom: FieldState,
    /// A first field whose second field may still follow (the last picture stored).
    pub awaiting_second_field: bool,
}

impl<F> DpbEntry<F> {
    pub fn field(&self, p: Parity) -> &FieldState {
        match p {
            Parity::Top => &self.top,
            Parity::Bottom => &self.bottom,
        }
    }
    fn field_mut(&mut self, p: Parity) -> &mut FieldState {
        match p {
            Parity::Top => &mut self.top,
            Parity::Bottom => &mut self.bottom,
        }
    }
    /// The marking of a lone first field (its only present field).
    fn mark_of_first(&self) -> RefMark {
        if self.top.present { self.top.mark } else { self.bottom.mark }
    }
    /// Any field used for reference (8.2.5.3 counts such frames against max_num_ref_frames).
    pub fn is_reference(&self) -> bool {
        self.top.is(RefMark::Short) || self.top.is(RefMark::Long) || self.bottom.is(RefMark::Short) || self.bottom.is(RefMark::Long)
    }
    fn has_field_marked(&self, mark: RefMark) -> bool {
        self.top.is(mark) || self.bottom.is(mark)
    }
    /// Mark every present field (frame-level marking).
    fn set_mark(&mut self, mark: RefMark) {
        for f in [&mut self.top, &mut self.bottom] {
            if f.present {
                f.mark = mark;
            }
        }
        self.sync();
    }
    fn set_field_mark(&mut self, p: Parity, mark: RefMark) {
        let f = self.field_mut(p);
        if f.present {
            f.mark = mark;
        }
        self.sync();
    }
    /// Recompute the frame-level `mark` and `poc` from the fields.
    fn sync(&mut self) {
        self.mark = if self.top.is(RefMark::Short) && self.bottom.is(RefMark::Short) {
            RefMark::Short
        } else if self.top.is(RefMark::Long) && self.bottom.is(RefMark::Long) {
            RefMark::Long
        } else {
            RefMark::Unused
        };
        self.poc = match (self.top.present, self.bottom.present) {
            (true, true) => self.top.poc.min(self.bottom.poc),
            (true, false) => self.top.poc,
            (false, true) => self.bottom.poc,
            (false, false) => self.poc,
        };
    }
}

pub struct Dpb<F> {
    pub entries: Vec<DpbEntry<F>>,
    /// None = "no long-term frame indices".
    pub max_long_term_frame_idx: Option<u32>,
    pub capacity: usize,
    pub max_reorder: usize,
}

/// A picture leaving the DPB for output.
pub struct Output<F> {
    pub frame: F,
    pub poc: i32,
    pub meta: Arc<OutputMeta>,
}

impl<F: Clone> Dpb<F> {
    pub fn new() -> Self {
        Dpb { entries: Vec::new(), max_long_term_frame_idx: None, capacity: 16, max_reorder: 16 }
    }

    fn frame_num_wrap(e: &DpbEntry<F>, cur_frame_num: u32, max_frame_num: u32) -> i32 {
        if e.frame_num > cur_frame_num { e.frame_num as i32 - max_frame_num as i32 } else { e.frame_num as i32 }
    }

    fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_reference()).count()
    }

    /// Remove entries not needed for output and not used for reference.
    fn prune(&mut self) {
        self.entries.retain(|e| e.needed_for_output || e.is_reference());
    }

    /// Output the picture with the smallest POC (C.4.5.3 "bumping"). Returns false if none waits.
    fn bump(&mut self, out: &mut Vec<Output<F>>) -> bool {
        let Some(i) = self.entries.iter().enumerate().filter(|(_, e)| e.needed_for_output).min_by_key(|(_, e)| e.poc).map(|(i, _)| i) else {
            return false;
        };
        let Some(e) = self.entries.get_mut(i) else { return false };
        e.needed_for_output = false;
        out.push(Output { frame: e.frame.clone(), poc: e.poc, meta: e.meta.clone() });
        self.prune();
        true
    }

    /// Output everything in POC order and clear non-reference entries.
    pub fn flush(&mut self, out: &mut Vec<Output<F>>) {
        while self.bump(out) {}
        self.prune();
    }

    /// IDR: mark all references unused; output (or drop) prior pictures.
    pub fn idr(&mut self, no_output_of_prior_pics: bool, out: &mut Vec<Output<F>>) {
        for e in &mut self.entries {
            e.set_mark(RefMark::Unused);
            e.awaiting_second_field = false;
        }
        if no_output_of_prior_pics {
            self.entries.clear();
        } else {
            self.flush(out);
        }
        self.max_long_term_frame_idx = None;
    }

    /// Sliding window marking (8.2.5.3).
    fn sliding_window(&mut self, cur_frame_num: u32, max_frame_num: u32, max_num_ref_frames: usize) {
        let limit = max_num_ref_frames.max(1);
        while self.num_refs() >= limit {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.has_field_marked(RefMark::Short))
                .min_by_key(|(_, e)| Self::frame_num_wrap(e, cur_frame_num, max_frame_num))
                .map(|(i, _)| i);
            match victim.and_then(|i| self.entries.get_mut(i)) {
                // Short-term fields of the victim become unused; a long-term sibling field stays.
                Some(e) => {
                    for p in [Parity::Top, Parity::Bottom] {
                        if e.field(p).is(RefMark::Short) {
                            e.set_field_mark(p, RefMark::Unused);
                        }
                    }
                }
                None => break,
            }
        }
        self.prune();
    }

    /// Insert "non-existing" frames for a frame_num gap (8.2.5.2). `make_frame` gives each one's
    /// picture and POC.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_frame_num_gap(
        &mut self,
        prev_ref_frame_num: u32,
        frame_num: u32,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        make_frame: &mut dyn FnMut(u32) -> (F, i32, Arc<OutputMeta>),
        poc_state: &mut dyn FnMut(u32),
    ) {
        let mut unused = (prev_ref_frame_num + 1) % max_frame_num;
        let mut guard = 0;
        while unused != frame_num && guard < max_frame_num {
            self.sliding_window(unused, max_frame_num, max_num_ref_frames);
            let (frame, poc, meta) = make_frame(unused);
            poc_state(unused);
            self.entries.push(DpbEntry {
                poc,
                frame,
                frame_num: unused,
                long_term_frame_idx: 0,
                mark: RefMark::Short,
                needed_for_output: false,
                non_existing: true,
                meta,
                top: FieldState::new(poc, RefMark::Short),
                bottom: FieldState::new(poc, RefMark::Short),
                awaiting_second_field: false,
            });
            unused = (unused + 1) % max_frame_num;
            guard += 1;
        }
    }

    /// Initial reference picture lists (8.2.4.2) + modification (8.2.4.3) for a frame slice, each
    /// entry made by `to_ref` from the DPB entry it refers to.
    pub fn build_ref_lists<R>(&self, sh: &SliceHeader, cur_poc: i32, max_frame_num: u32, to_ref: impl Fn(&DpbEntry<F>) -> R) -> Result<[Vec<R>; 2]> {
        let mut lists: [Vec<R>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type.is_intra() {
            return Ok(lists);
        }
        let cur_fn = sh.frame_num;
        let pic_num = |e: &DpbEntry<F>| Self::frame_num_wrap(e, cur_fn, max_frame_num);
        let short: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark == RefMark::Short).collect();
        let mut long: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark == RefMark::Long).collect();
        long.sort_by_key(|e| e.long_term_frame_idx);
        let mut entry_lists: [Vec<&DpbEntry<F>>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type != SliceType::B {
            let mut s = short.clone();
            s.sort_by_key(|e| std::cmp::Reverse(pic_num(e)));
            entry_lists[0] = s.into_iter().chain(long.iter().copied()).collect();
        } else {
            let mut before: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc < cur_poc).collect();
            let mut after: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc > cur_poc).collect();
            before.sort_by_key(|e| std::cmp::Reverse(e.poc));
            after.sort_by_key(|e| e.poc);
            entry_lists[0] = before.iter().chain(after.iter()).chain(long.iter()).copied().collect();
            entry_lists[1] = after.iter().chain(before.iter()).chain(long.iter()).copied().collect();
            if entry_lists[1].len() > 1 && entry_lists[0].len() == entry_lists[1].len() {
                let same = entry_lists[0].iter().zip(entry_lists[1].iter()).all(|(a, b)| std::ptr::eq(*a, *b));
                if same {
                    entry_lists[1].swap(0, 1);
                }
            }
        }
        let nlists = if sh.slice_type == SliceType::B { 2 } else { 1 };
        for l in 0..nlists {
            let n = sh.num_ref_idx_active[l] as usize;
            let mut list: Vec<Option<&DpbEntry<F>>> = entry_lists[l].iter().copied().map(Some).collect();
            list.truncate(n);
            list.resize(n, None);
            if !sh.ref_pic_list_mod[l].is_empty() {
                let max_pic_num = max_frame_num as i32;
                let curr_pic_num = cur_fn as i32;
                let mut pred = curr_pic_num;
                let mut ref_idx = 0usize;
                for m in &sh.ref_pic_list_mod[l] {
                    ensure!(ref_idx < n, "too many ref_pic_list_modification entries");
                    list.push(None); // temporarily one longer
                    let (target, is_long): (Option<&DpbEntry<F>>, bool) = match m.idc {
                        0 | 1 => {
                            let d = m.value as i32 + 1;
                            let no_wrap = if m.idc == 0 {
                                if pred - d < 0 { pred - d + max_pic_num } else { pred - d }
                            } else if pred + d >= max_pic_num {
                                pred + d - max_pic_num
                            } else {
                                pred + d
                            };
                            pred = no_wrap;
                            let pn = if no_wrap > curr_pic_num { no_wrap - max_pic_num } else { no_wrap };
                            (short.iter().copied().find(|e| pic_num(e) == pn), false)
                        }
                        2 => (long.iter().copied().find(|e| e.long_term_frame_idx == m.value), true),
                        _ => continue,
                    };
                    let Some(target) = target else {
                        // Missing picture: keep the list unchanged (error concealment).
                        list.pop();
                        ref_idx += 1;
                        continue;
                    };
                    for c in (ref_idx + 1..=n).rev() {
                        list[c] = list[c - 1];
                    }
                    list[ref_idx] = Some(target);
                    ref_idx += 1;
                    let mut nidx = ref_idx;
                    for c in ref_idx..=n {
                        let keep = match list[c] {
                            None => true,
                            Some(e) => {
                                if is_long {
                                    !(e.mark == RefMark::Long && e.long_term_frame_idx == target.long_term_frame_idx)
                                } else {
                                    !(e.mark == RefMark::Short && std::ptr::eq(e, target))
                                }
                            }
                        };
                        if keep {
                            list[nidx] = list[c];
                            nidx += 1;
                        }
                    }
                    list.truncate(n);
                }
            }
            // Fill missing entries (should not happen in conforming streams) with the first valid entry.
            let fallback = list.iter().flatten().next().copied().or_else(|| self.entries.last());
            let out: Vec<R> = list.iter().filter_map(|e| e.or(fallback)).map(&to_ref).collect();
            lists[l] = out;
        }
        Ok(lists)
    }

    /// Initial reference picture lists for a field slice (8.2.4.2.2 P, 8.2.4.2.4 B, alternated by
    /// 8.2.4.2.5) and their modification (8.2.4.3 with field picture numbers), each entry made by
    /// `to_ref` from the DPB entry and the parity of the field it refers to. `cur_poc` is the
    /// current field's POC. The first field of the current frame is a candidate like any other.
    pub fn build_field_ref_lists<R>(
        &self,
        sh: &SliceHeader,
        cur_poc: i32,
        max_frame_num: u32,
        to_ref: impl Fn(&DpbEntry<F>, Parity) -> R,
    ) -> Result<[Vec<R>; 2]> {
        let mut lists: [Vec<R>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type.is_intra() || !sh.field_pic {
            return Ok(lists);
        }
        let cur = Parity::of(sh);
        let cur_fn = sh.frame_num;
        let wrap = |e: &DpbEntry<F>| i64::from(Self::frame_num_wrap(e, cur_fn, max_frame_num));
        // POC of an entry over its short-term fields only (8.2.4.2.4).
        let short_poc = |e: &DpbEntry<F>| {
            let t = e.top.is(RefMark::Short).then_some(e.top.poc);
            let b = e.bottom.is(RefMark::Short).then_some(e.bottom.poc);
            t.into_iter().chain(b).min().unwrap_or(e.poc)
        };
        let short: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.has_field_marked(RefMark::Short)).collect();
        let mut long: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.has_field_marked(RefMark::Long)).collect();
        long.sort_by_key(|e| e.long_term_frame_idx);
        let mut frame_lists: [Vec<&DpbEntry<F>>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type != SliceType::B {
            let mut s = short.clone();
            s.sort_by_key(|e| std::cmp::Reverse(wrap(e)));
            frame_lists[0] = s;
        } else {
            let mut before: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| short_poc(e) <= cur_poc).collect();
            let mut after: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| short_poc(e) > cur_poc).collect();
            before.sort_by_key(|e| std::cmp::Reverse(short_poc(e)));
            after.sort_by_key(|e| short_poc(e));
            frame_lists[0] = before.iter().chain(after.iter()).copied().collect();
            frame_lists[1] = after.iter().chain(before.iter()).copied().collect();
        }
        let mut field_lists: [Vec<(&DpbEntry<F>, Parity)>; 2] = [Vec::new(), Vec::new()];
        let nlists = if sh.slice_type == SliceType::B { 2 } else { 1 };
        for (l, fl) in field_lists.iter_mut().enumerate().take(nlists) {
            let Some(frames) = frame_lists.get(l) else { continue };
            *fl = alternate_fields(frames, cur, RefMark::Short);
            fl.extend(alternate_fields(&long, cur, RefMark::Long));
        }
        if let [l0, l1] = &mut field_lists
            && nlists == 2
            && l1.len() > 1
            && l0.len() == l1.len()
            && l0.iter().zip(l1.iter()).all(|(a, b)| std::ptr::eq(a.0, b.0) && a.1 == b.1)
        {
            l1.swap(0, 1);
        }
        let same = |a: (&DpbEntry<F>, Parity), b: (&DpbEntry<F>, Parity)| std::ptr::eq(a.0, b.0) && a.1 == b.1;
        for (l, init) in field_lists.into_iter().enumerate().take(nlists) {
            let n = sh.num_ref_idx_active.get(l).copied().unwrap_or(0).min(32) as usize;
            let mut list: Vec<Option<(&DpbEntry<F>, Parity)>> = init.into_iter().map(Some).collect();
            list.truncate(n);
            list.resize(n, None);
            let mods = sh.ref_pic_list_mod.get(l).map(Vec::as_slice).unwrap_or(&[]);
            let max_pic_num = 2 * i64::from(max_frame_num);
            let curr_pic_num = 2 * i64::from(cur_fn) + 1;
            let mut pred = curr_pic_num;
            let mut ref_idx = 0usize;
            for m in mods {
                ensure!(ref_idx < n, "too many ref_pic_list_modification entries");
                let target = match m.idc {
                    0 | 1 => {
                        let d = i64::from(m.value) + 1;
                        let no_wrap = if m.idc == 0 {
                            if pred - d < 0 { pred - d + max_pic_num } else { pred - d }
                        } else if pred + d >= max_pic_num {
                            pred + d - max_pic_num
                        } else {
                            pred + d
                        };
                        pred = no_wrap;
                        let pn = if no_wrap > curr_pic_num { no_wrap - max_pic_num } else { no_wrap };
                        self.find_short(pn, cur_fn, max_frame_num, Some(cur))
                    }
                    2 => self.find_long(i64::from(m.value), Some(cur)),
                    _ => continue,
                };
                let target = target.and_then(|(i, ps)| Some((self.entries.get(i)?, *ps.first()?)));
                let Some(target) = target else {
                    // Missing picture: keep the list unchanged (error concealment).
                    ref_idx += 1;
                    continue;
                };
                // Insert at ref_idx, then drop later copies of the same field (8.2.4.3.1 / .2).
                list.insert(ref_idx, Some(target));
                ref_idx += 1;
                list = list.into_iter().enumerate().filter(|(c, e)| *c < ref_idx || !e.is_some_and(|e| same(e, target))).map(|(_, e)| e).collect();
                list.truncate(n);
            }
            let fallback = list.iter().flatten().next().copied();
            if let Some(out) = lists.get_mut(l) {
                *out = list.iter().filter_map(|e| e.or(fallback)).map(|(e, p)| to_ref(e, p)).collect();
            }
        }
        Ok(lists)
    }

    /// Adaptive memory control (8.2.5.4) for a frame (`field` None) or a field of the given
    /// parity. Returns true if MMCO 5 was executed.
    ///
    /// Picture numbers (8.2.4.1): for frames PicNum = FrameNumWrap and LongTermPicNum =
    /// LongTermFrameIdx; for fields, 2 × FrameNumWrap + 1 (or 2 × LongTermFrameIdx + 1) for a
    /// field of the current parity and 2 × … for the opposite parity.
    fn apply_mmcos(&mut self, mmcos: &[Mmco], cur_frame_num: u32, max_frame_num: u32, field: Option<Parity>, cur_long: &mut Option<u32>) -> bool {
        let mut had5 = false;
        let curr_pic_num: i64 = match field {
            None => i64::from(cur_frame_num),
            Some(_) => 2 * i64::from(cur_frame_num) + 1,
        };
        for m in mmcos {
            match m.op {
                1 | 3 => {
                    let pn = curr_pic_num - (i64::from(m.difference_of_pic_nums_minus1) + 1);
                    let Some((i, parities)) = self.find_short(pn, cur_frame_num, max_frame_num, field) else { continue };
                    if m.op == 1 {
                        if let Some(e) = self.entries.get_mut(i) {
                            for p in parities {
                                e.set_field_mark(p, RefMark::Unused);
                            }
                        }
                        continue;
                    }
                    // MMCO 3: free long_term_frame_idx unless it belongs to the other field of
                    // the same frame (8.2.5.4.3), then make the picture long-term.
                    let idx = m.long_term_frame_idx;
                    for (j, e) in self.entries.iter_mut().enumerate() {
                        if e.long_term_frame_idx == idx && e.has_field_marked(RefMark::Long) && (j != i || field.is_none()) {
                            e.set_mark(RefMark::Unused);
                        }
                    }
                    if let Some(e) = self.entries.get_mut(i) {
                        if e.has_field_marked(RefMark::Long) && e.long_term_frame_idx != idx {
                            // The sibling field is long-term under another index: it can't keep it.
                            for p in [Parity::Top, Parity::Bottom] {
                                if e.field(p).is(RefMark::Long) {
                                    e.set_field_mark(p, RefMark::Unused);
                                }
                            }
                        }
                        e.long_term_frame_idx = idx;
                        for p in parities {
                            e.set_field_mark(p, RefMark::Long);
                        }
                    }
                }
                2 => {
                    let Some((i, parities)) = self.find_long(i64::from(m.long_term_pic_num), field) else { continue };
                    if let Some(e) = self.entries.get_mut(i) {
                        for p in parities {
                            e.set_field_mark(p, RefMark::Unused);
                        }
                    }
                }
                4 => {
                    let max = m.max_long_term_frame_idx_plus1;
                    for e in &mut self.entries {
                        if e.has_field_marked(RefMark::Long) && (max == 0 || e.long_term_frame_idx > max - 1) {
                            e.set_mark(RefMark::Unused);
                        }
                    }
                    self.max_long_term_frame_idx = if max == 0 { None } else { Some(max - 1) };
                }
                5 => {
                    for e in &mut self.entries {
                        e.set_mark(RefMark::Unused);
                    }
                    self.max_long_term_frame_idx = None;
                    had5 = true;
                }
                6 => {
                    // Free the index unless it is held by the first field of the current frame.
                    for e in &mut self.entries {
                        let own_first_field = field.is_some() && e.awaiting_second_field;
                        if e.long_term_frame_idx == m.long_term_frame_idx && e.has_field_marked(RefMark::Long) && !own_first_field {
                            e.set_mark(RefMark::Unused);
                        }
                    }
                    *cur_long = Some(m.long_term_frame_idx);
                }
                _ => {}
            }
        }
        self.prune();
        had5
    }

    /// The short-term picture with PicNum `pn` (8.2.4.1): its entry index and the fields it
    /// covers (both for a frame).
    fn find_short(&self, pn: i64, cur_frame_num: u32, max_frame_num: u32, field: Option<Parity>) -> Option<(usize, Vec<Parity>)> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let wrap = i64::from(Self::frame_num_wrap(e, cur_frame_num, max_frame_num));
            match field {
                None => (e.mark == RefMark::Short && wrap == pn).then(|| (i, vec![Parity::Top, Parity::Bottom])),
                Some(cur) => [cur, cur.opposite()]
                    .into_iter()
                    .find(|&p| {
                        let num = if p == cur { 2 * wrap + 1 } else { 2 * wrap };
                        e.field(p).is(RefMark::Short) && num == pn
                    })
                    .map(|p| (i, vec![p])),
            }
        })
    }

    /// The long-term picture with LongTermPicNum `lpn` (8.2.4.1).
    fn find_long(&self, lpn: i64, field: Option<Parity>) -> Option<(usize, Vec<Parity>)> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let idx = i64::from(e.long_term_frame_idx);
            match field {
                None => (e.mark == RefMark::Long && idx == lpn).then(|| (i, vec![Parity::Top, Parity::Bottom])),
                Some(cur) => [cur, cur.opposite()]
                    .into_iter()
                    .find(|&p| {
                        let num = if p == cur { 2 * idx + 1 } else { 2 * idx };
                        e.field(p).is(RefMark::Long) && num == lpn
                    })
                    .map(|p| (i, vec![p])),
            }
        })
    }

    /// Whether a field slice with header `sh` is the second field of the first field stored last
    /// (7.4.1.2.4 / 8.2.1: opposite parity, same frame_num, the first field still unpaired).
    /// Callers ask before running [`Dpb::idr`] or a frame_num gap fill for the picture: a second
    /// field joins its first field's entry and does neither.
    pub fn is_second_field(&self, sh: &SliceHeader) -> bool {
        sh.field_pic
            && self.entries.iter().any(|e| {
                e.awaiting_second_field && e.frame_num == sh.frame_num && !e.field(Parity::of(sh)).present && e.field(Parity::of(sh).opposite()).present
            })
    }

    /// Reference marking for the just-decoded frame and insertion into the DPB with output
    /// bumping; [`Dpb::store`] with both field POCs equal to `poc`. Returns pictures ready for
    /// output.
    #[allow(clippy::too_many_arguments)]
    pub fn store_picture(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        poc: i32,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) {
        self.store(sh, frame, FieldPocs { top: poc, bottom: poc }, max_frame_num, max_num_ref_frames, meta, out);
    }

    /// Reference marking (8.2.5) for the just-decoded frame or field and its storage (C.4.4 /
    /// C.4.5) with output bumping. For a field, `pocs` holds its POC under its parity (the other
    /// value is ignored); a second field ([`Dpb::is_second_field`]) joins its first field's entry
    /// and its `frame` replaces the entry's (the same picture buffer, now holding both fields).
    /// A first field is not output before its second field arrives or the next picture shows it
    /// is unpaired.
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        pocs: FieldPocs,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) {
        let field = sh.field_pic.then(|| Parity::of(sh));
        let second = self.is_second_field(sh);
        if !second {
            for e in &mut self.entries {
                e.awaiting_second_field = false;
            }
        }
        // The first field of the current frame when this is its second field.
        let first_mark = if second { self.entries.iter().find(|e| e.awaiting_second_field).map(|e| (e.mark_of_first(), e.long_term_frame_idx)) } else { None };
        let mut mark = RefMark::Unused;
        let mut long_idx = 0;
        let mut had5 = false;
        if sh.nal_ref_idc != 0 {
            if sh.idr {
                if sh.long_term_reference {
                    mark = RefMark::Long;
                    self.max_long_term_frame_idx = Some(0);
                } else {
                    mark = RefMark::Short;
                    self.max_long_term_frame_idx = None;
                }
            } else {
                let mut cur_long = None;
                if sh.adaptive_ref_pic_marking {
                    had5 = self.apply_mmcos(&sh.mmcos, sh.frame_num, max_frame_num, field, &mut cur_long);
                } else if !matches!(first_mark, Some((RefMark::Short | RefMark::Long, _))) {
                    // 8.2.5.3: no sliding window for the second field of a reference pair.
                    self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                }
                if let Some(i) = cur_long {
                    mark = RefMark::Long;
                    long_idx = i;
                } else if let Some((RefMark::Long, i)) = first_mark.filter(|_| !had5) {
                    // 8.2.5.1: the second field of a long-term first field is long-term too.
                    mark = RefMark::Long;
                    long_idx = i;
                } else {
                    mark = RefMark::Short;
                    // make room if MMCOs left the DPB over-full (non-conforming streams)
                    if !second && self.num_refs() >= max_num_ref_frames.max(1) && sh.adaptive_ref_pic_marking {
                        self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                    }
                }
            }
        }
        if had5 {
            // All earlier pictures are output before the current one (C.4.4).
            self.flush(out);
        }
        let frame_num = if had5 { 0 } else { sh.frame_num };
        if let Some(p) = field.filter(|_| second) {
            // MMCO 5 may have flushed the first field: then this field is stored on its own.
            if let Some(e) = self.entries.iter_mut().find(|e| e.awaiting_second_field) {
                e.awaiting_second_field = false;
                e.frame = frame;
                let poc = match p {
                    Parity::Top => pocs.top,
                    Parity::Bottom => pocs.bottom,
                };
                *e.field_mut(p) = FieldState::new(poc, mark);
                if mark == RefMark::Long {
                    e.long_term_frame_idx = long_idx;
                }
                e.sync();
                self.low_delay_output(out);
                return;
            }
        }
        let (top, bottom) = match field {
            None => (FieldState::new(pocs.top, mark), FieldState::new(pocs.bottom, mark)),
            Some(Parity::Top) => (FieldState::new(pocs.top, mark), FieldState::ABSENT),
            Some(Parity::Bottom) => (FieldState::ABSENT, FieldState::new(pocs.bottom, mark)),
        };
        let mut entry = DpbEntry {
            frame,
            poc: 0,
            frame_num,
            long_term_frame_idx: long_idx,
            mark,
            needed_for_output: true,
            non_existing: false,
            meta,
            top,
            bottom,
            awaiting_second_field: field.is_some(),
        };
        entry.sync();
        let poc = entry.poc;
        // Bumping until there is space (C.4.5.1 / C.4.5.2).
        if mark == RefMark::Unused {
            loop {
                let waiting = self.entries.len();
                if waiting < self.capacity {
                    break;
                }
                // A non-reference frame with the smallest POC is output directly (a first field
                // isn't: its second field still goes into the same buffer).
                let min_poc = self.entries.iter().filter(|e| e.needed_for_output).map(|e| e.poc).min();
                if field.is_none() && min_poc.is_none_or(|m| poc < m) {
                    out.push(Output { frame: entry.frame, poc, meta: entry.meta });
                    return;
                }
                if !self.bump(out) {
                    break;
                }
            }
        } else {
            while self.entries.len() >= self.capacity {
                if !self.bump(out) {
                    break;
                }
            }
        }
        self.entries.push(entry);
        if field.is_none() {
            self.low_delay_output(out);
        }
    }

    /// Low-delay output: honour max_num_reorder_frames.
    fn low_delay_output(&mut self, out: &mut Vec<Output<F>>) {
        while self.entries.iter().filter(|e| e.needed_for_output).count() > self.max_reorder {
            if !self.bump(out) {
                break;
            }
        }
    }
}

/// 8.2.4.2.5: fields from an ordered frame list, alternating parity starting with the current
/// field's, each parity's fields taken in list order (skipping frames whose field of that parity
/// isn't marked `mark`); when one parity runs out the rest of the other follows in order.
fn alternate_fields<'a, F>(frames: &[&'a DpbEntry<F>], cur: Parity, mark: RefMark) -> Vec<(&'a DpbEntry<F>, Parity)> {
    let of = |p: Parity| frames.iter().copied().filter(move |e| e.field(p).is(mark)).map(move |e| (e, p));
    let mut same = of(cur);
    let mut opp = of(cur.opposite());
    let mut out = Vec::new();
    loop {
        match (same.next(), opp.next()) {
            (Some(a), Some(b)) => {
                out.push(a);
                out.push(b);
            }
            (Some(a), None) => {
                out.push(a);
                out.extend(same);
                break;
            }
            (None, Some(b)) => {
                out.push(b);
                out.extend(opp);
                break;
            }
            (None, None) => break,
        }
    }
    out
}

impl<F: Clone> Default for Dpb<F> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::slice::RefPicListMod;

    fn meta() -> Arc<OutputMeta> {
        Arc::new(OutputMeta {
            pts: 0,
            key: false,
            crop: (0, 0, 16, 16),
            full_range: false,
            colour_primaries: 2,
            transfer_characteristics: 2,
            matrix_coefficients: 2,
            sar: (1, 1),
            draft: false,
        })
    }

    /// A slice header: `field` None = frame, Some(bottom) = field of that parity.
    pub(crate) fn hdr(frame_num: u32, field: Option<bool>, nal_ref_idc: u8, idr: bool) -> SliceHeader {
        SliceHeader {
            nal_ref_idc,
            nal_unit_type: if idr { 5 } else { 1 },
            idr,
            first_mb_in_slice: 0,
            slice_type_raw: 2,
            slice_type: SliceType::I,
            pps_id: 0,
            colour_plane_id: 0,
            frame_num,
            field_pic: field.is_some(),
            bottom_field: field.unwrap_or(false),
            idr_pic_id: 0,
            pic_order_cnt_lsb: 0,
            delta_pic_order_cnt_bottom: 0,
            delta_pic_order_cnt: [0; 2],
            redundant_pic_cnt: 0,
            direct_spatial_mv_pred: false,
            num_ref_idx_active: [0; 2],
            ref_pic_list_mod: [Vec::<RefPicListMod>::new(), Vec::new()],
            pred_weight_table: None,
            no_output_of_prior_pics: false,
            long_term_reference: false,
            adaptive_ref_pic_marking: false,
            mmcos: Vec::new(),
            cabac_init_idc: 0,
            slice_qp_delta: 0,
            sp_for_switch: false,
            slice_qs_delta: 0,
            disable_deblocking_filter_idc: 0,
            slice_alpha_c0_offset_div2: 0,
            slice_beta_offset_div2: 0,
            slice_group_change_cycle: 0,
            header_bits: 0,
        }
    }

    fn mmco(op: u32, diff_minus1: u32, long_term_pic_num: u32, long_term_frame_idx: u32) -> Mmco {
        Mmco { op, difference_of_pic_nums_minus1: diff_minus1, long_term_pic_num, long_term_frame_idx, max_long_term_frame_idx_plus1: 0 }
    }

    const MAX_FN: u32 = 16;

    /// Stores a picture whose `frame` is its id; returns the ids output.
    fn store(dpb: &mut Dpb<u32>, sh: &SliceHeader, id: u32, top: i32, bottom: i32, refs: usize) -> Vec<u32> {
        let mut out = Vec::new();
        if sh.idr && !dpb.is_second_field(sh) {
            dpb.idr(sh.no_output_of_prior_pics, &mut out);
        }
        dpb.store(sh, id, FieldPocs { top, bottom }, MAX_FN, refs, meta(), &mut out);
        out.into_iter().map(|o| o.frame).collect()
    }

    fn entry(dpb: &Dpb<u32>, frame_num: u32) -> &DpbEntry<u32> {
        dpb.entries.iter().find(|e| e.frame_num == frame_num && !e.non_existing).expect("entry")
    }

    #[test]
    fn frames_keep_their_output_order_and_marking() {
        let mut dpb = Dpb::new();
        dpb.max_reorder = 1;
        let mut out = store(&mut dpb, &hdr(0, None, 1, true), 0, 0, 0, 4);
        out.extend(store(&mut dpb, &hdr(1, None, 1, false), 1, 4, 4, 4));
        out.extend(store(&mut dpb, &hdr(2, None, 0, false), 2, 2, 2, 4));
        let mut rest = Vec::new();
        dpb.flush(&mut rest);
        out.extend(rest.into_iter().map(|o| o.frame));
        assert_eq!(out, vec![0, 2, 1]);
        let e = entry(&dpb, 1);
        assert_eq!((e.mark, e.top.mark, e.bottom.mark), (RefMark::Short, RefMark::Short, RefMark::Short));
        assert!(e.top.present && e.bottom.present && !e.awaiting_second_field);
    }

    #[test]
    fn complementary_field_pair_shares_one_entry_and_is_output_once() {
        let mut dpb = Dpb::new();
        dpb.max_reorder = 0;
        // 8.2.1: a first field is never output on its own while its second field may follow.
        assert!(store(&mut dpb, &hdr(0, Some(false), 1, true), 7, 0, 0, 4).is_empty());
        assert!(dpb.is_second_field(&hdr(0, Some(true), 1, false)));
        assert!(!dpb.is_second_field(&hdr(0, Some(false), 1, false)), "same parity is not a second field");
        assert!(!dpb.is_second_field(&hdr(1, Some(true), 1, false)), "other frame_num is not a second field");
        assert_eq!(store(&mut dpb, &hdr(0, Some(true), 1, false), 7, 0, 1, 4), vec![7]);
        assert_eq!(dpb.entries.len(), 1);
        let e = entry(&dpb, 0);
        assert_eq!((e.top.poc, e.bottom.poc, e.poc), (0, 1, 0));
        assert_eq!(e.mark, RefMark::Short);
        let mut rest = Vec::new();
        dpb.flush(&mut rest);
        assert!(rest.is_empty());
    }

    #[test]
    fn bottom_first_pair_takes_the_smaller_poc() {
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(0, Some(true), 1, true), 1, 0, 0, 4);
        store(&mut dpb, &hdr(0, Some(false), 1, false), 1, 3, 0, 4);
        let e = entry(&dpb, 0);
        assert_eq!((e.top.poc, e.bottom.poc, e.poc), (3, 0, 0));
    }

    #[test]
    fn unpaired_first_field_stays_alone() {
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(0, Some(false), 1, true), 0, 0, 0, 4);
        store(&mut dpb, &hdr(1, Some(false), 1, false), 1, 4, 0, 4);
        assert_eq!(dpb.entries.len(), 2);
        let e = entry(&dpb, 0);
        assert!(!e.awaiting_second_field && e.top.present && !e.bottom.present);
        // A lone short-term field is a reference, but not a frame reference.
        assert!(e.is_reference());
        assert_eq!(e.mark, RefMark::Unused);
        assert!(entry(&dpb, 1).awaiting_second_field);
    }

    #[test]
    fn sliding_window_counts_frames_and_spares_the_current_first_field() {
        let mut dpb = Dpb::new();
        // max_num_ref_frames = 1: the pair of frame 0, then frame 1's first field evicts it.
        store(&mut dpb, &hdr(0, Some(false), 1, true), 0, 0, 0, 1);
        store(&mut dpb, &hdr(0, Some(true), 1, false), 0, 0, 1, 1);
        store(&mut dpb, &hdr(1, Some(false), 1, false), 1, 4, 0, 1);
        assert!(!entry(&dpb, 0).is_reference());
        // 8.2.5.3: the second field of a short-term first field doesn't run the window.
        store(&mut dpb, &hdr(1, Some(true), 1, false), 1, 4, 5, 1);
        let e = entry(&dpb, 1);
        assert_eq!((e.top.mark, e.bottom.mark, e.mark), (RefMark::Short, RefMark::Short, RefMark::Short));
    }

    #[test]
    fn mmco1_on_a_field_unmarks_only_that_field() {
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(0, Some(false), 1, true), 0, 0, 0, 4);
        store(&mut dpb, &hdr(0, Some(true), 1, false), 0, 0, 1, 4);
        store(&mut dpb, &hdr(1, Some(false), 1, false), 1, 4, 0, 4);
        store(&mut dpb, &hdr(1, Some(true), 1, false), 1, 4, 5, 4);
        // Current: bottom field, frame_num 2 -> CurrPicNum 5. Frame 1's bottom (same parity) has
        // PicNum 2*1+1 = 3, its top 2*1 = 2; frame 0's bottom 1, top 0.
        let mut sh = hdr(2, Some(true), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(1, 1, 0, 0), mmco(1, 4, 0, 0)]; // picNumX 3, then 0
        store(&mut dpb, &sh, 2, 0, 8, 4);
        let e1 = entry(&dpb, 1);
        assert_eq!((e1.top.mark, e1.bottom.mark, e1.mark), (RefMark::Short, RefMark::Unused, RefMark::Unused));
        assert!(e1.is_reference());
        let e0 = entry(&dpb, 0);
        assert_eq!((e0.top.mark, e0.bottom.mark), (RefMark::Unused, RefMark::Short));
    }

    #[test]
    fn mmco3_makes_a_field_long_term_and_the_sibling_may_share_the_index() {
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(0, Some(false), 1, true), 0, 0, 0, 4);
        store(&mut dpb, &hdr(0, Some(true), 1, false), 0, 0, 1, 4);
        let mut out = Vec::new();
        dpb.max_long_term_frame_idx = Some(1);
        // Top field, frame_num 1: CurrPicNum 3; frame 0's top PicNum 1 (diff 2), bottom 0 (diff 3).
        let mut sh = hdr(1, Some(false), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(3, 1, 0, 0)];
        dpb.store(&sh, 1, FieldPocs { top: 4, bottom: 0 }, MAX_FN, 4, meta(), &mut out);
        let e0 = entry(&dpb, 0);
        assert_eq!((e0.top.mark, e0.bottom.mark, e0.long_term_frame_idx), (RefMark::Long, RefMark::Short, 0));
        let mut sh = hdr(1, Some(true), 1, false);
        sh.adaptive_ref_pic_marking = true;
        // Bottom field: CurrPicNum 3; frame 0's bottom (same parity) PicNum 1 -> diff 2.
        sh.mmcos = vec![mmco(3, 1, 0, 0)];
        dpb.store(&sh, 1, FieldPocs { top: 4, bottom: 5 }, MAX_FN, 4, meta(), &mut out);
        let e0 = entry(&dpb, 0);
        assert_eq!((e0.top.mark, e0.bottom.mark, e0.mark), (RefMark::Long, RefMark::Long, RefMark::Long));
        // MMCO 2 on a field: LongTermPicNum of frame 0's top seen from a top field is 2*0+1.
        let mut sh = hdr(2, Some(false), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(2, 0, 1, 0)];
        dpb.store(&sh, 2, FieldPocs { top: 8, bottom: 0 }, MAX_FN, 4, meta(), &mut out);
        let e0 = entry(&dpb, 0);
        assert_eq!((e0.top.mark, e0.bottom.mark, e0.mark), (RefMark::Unused, RefMark::Long, RefMark::Unused));
    }

    #[test]
    fn second_field_of_a_long_term_idr_field_is_long_term() {
        let mut dpb = Dpb::new();
        let mut sh = hdr(0, Some(false), 1, true);
        sh.long_term_reference = true;
        store(&mut dpb, &sh, 0, 0, 0, 4);
        store(&mut dpb, &hdr(0, Some(true), 1, false), 0, 0, 1, 4);
        let e = entry(&dpb, 0);
        assert_eq!((e.mark, e.long_term_frame_idx), (RefMark::Long, 0));
    }

    #[test]
    fn mmco6_on_a_second_field_keeps_the_first_fields_index() {
        let mut dpb = Dpb::new();
        dpb.max_long_term_frame_idx = Some(0);
        let mut out = Vec::new();
        let mut sh = hdr(1, Some(false), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(6, 0, 0, 0)];
        dpb.store(&sh, 1, FieldPocs { top: 4, bottom: 0 }, MAX_FN, 4, meta(), &mut out);
        let mut sh = hdr(1, Some(true), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(6, 0, 0, 0)];
        dpb.store(&sh, 1, FieldPocs { top: 4, bottom: 5 }, MAX_FN, 4, meta(), &mut out);
        let e = entry(&dpb, 1);
        assert_eq!((e.mark, e.long_term_frame_idx), (RefMark::Long, 0));
    }

    #[test]
    fn hostile_lone_fields_never_overfill_or_panic() {
        let mut dpb = Dpb::new();
        dpb.capacity = 2;
        dpb.max_reorder = 2;
        let mut outputs = 0;
        for i in 0..100u32 {
            // Non-reference fields of one parity: none ever pairs; a few reference ones too.
            let sh = hdr(i % MAX_FN, Some(i % 7 == 3), u8::from(i % 5 == 0), i == 0);
            outputs += store(&mut dpb, &sh, i, (2 * i) as i32, (2 * i + 1) as i32, 1).len();
            assert!(dpb.entries.len() <= 3, "{} entries", dpb.entries.len());
        }
        let mut rest = Vec::new();
        dpb.flush(&mut rest);
        assert!(outputs + rest.len() <= 100);
        // A second field with no first field, and an MMCO 5 second field: stored on their own.
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(3, Some(true), 1, false), 0, 0, 0, 4);
        let mut sh = hdr(3, Some(false), 1, false);
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos = vec![mmco(5, 0, 0, 0)];
        let out = store(&mut dpb, &sh, 1, 0, 0, 4);
        assert_eq!(out, vec![0]);
        assert_eq!(dpb.entries.len(), 1);
        assert!(entry(&dpb, 0).top.present);
    }

    use Parity::{Bottom as B, Top as T};

    /// Field slice header of the given type with `n` active references per list.
    fn field_slice(frame_num: u32, bottom: bool, ty: SliceType, nal_ref_idc: u8, n: u32) -> SliceHeader {
        let mut sh = hdr(frame_num, Some(bottom), nal_ref_idc, false);
        sh.slice_type = ty;
        sh.num_ref_idx_active = [n, n];
        sh
    }

    /// Two reference field pairs: frame_num 0 (POCs 0 / 1) and 1 (POCs 8 / 9), frame ids 0 and 1.
    fn two_pairs() -> Dpb<u32> {
        let mut dpb = Dpb::new();
        store(&mut dpb, &hdr(0, Some(false), 1, true), 0, 0, 0, 4);
        store(&mut dpb, &hdr(0, Some(true), 1, false), 0, 0, 1, 4);
        store(&mut dpb, &hdr(1, Some(false), 1, false), 1, 8, 0, 4);
        store(&mut dpb, &hdr(1, Some(true), 1, false), 1, 8, 9, 4);
        dpb
    }

    fn lists(dpb: &Dpb<u32>, sh: &SliceHeader, cur_poc: i32) -> [Vec<(u32, Parity)>; 2] {
        dpb.build_field_ref_lists(sh, cur_poc, MAX_FN, |e, p| (e.frame, p)).unwrap()
    }

    #[test]
    fn p_field_list_alternates_parity_by_frame_num_wrap() {
        let dpb = two_pairs();
        let [l0, l1] = lists(&dpb, &field_slice(2, false, SliceType::P, 1, 4), 16);
        assert_eq!(l0, vec![(1, T), (1, B), (0, T), (0, B)]);
        assert!(l1.is_empty());
        // Truncated to num_ref_idx_active.
        let [l0, _] = lists(&dpb, &field_slice(2, true, SliceType::P, 1, 2), 17);
        assert_eq!(l0, vec![(1, B), (1, T)]);
    }

    #[test]
    fn second_p_field_may_refer_to_its_own_first_field() {
        let mut dpb = two_pairs();
        store(&mut dpb, &field_slice(2, false, SliceType::P, 1, 0), 2, 16, 0, 4);
        let sh = field_slice(2, true, SliceType::P, 1, 5);
        assert!(dpb.is_second_field(&sh));
        // Same parity (bottom) runs out after frames 1 and 0; frame 2 only has a top field.
        let [l0, _] = lists(&dpb, &sh, 17);
        assert_eq!(l0, vec![(1, B), (2, T), (0, B), (1, T), (0, T)]);
    }

    #[test]
    fn b_field_lists_order_by_poc_with_equal_pocs_before() {
        let dpb = two_pairs();
        let [l0, l1] = lists(&dpb, &field_slice(2, false, SliceType::B, 0, 4), 4);
        assert_eq!(l0, vec![(0, T), (0, B), (1, T), (1, B)]);
        assert_eq!(l1, vec![(1, T), (1, B), (0, T), (0, B)]);
        // A POC equal to the current one counts as "before" for fields (8.2.4.2.4).
        let [l0, l1] = lists(&dpb, &field_slice(2, true, SliceType::B, 0, 4), 8);
        assert_eq!(l0.first(), Some(&(1, B)));
        assert_eq!(l1.first(), Some(&(1, T)), "lists equal and longer than one: first two swapped");
        assert_eq!(l1.get(1), Some(&(1, B)));
    }

    #[test]
    fn long_term_fields_follow_short_term_ones_and_skip_unmarked_fields() {
        let mut dpb = two_pairs();
        // Frame 0 becomes long-term index 0 with only its bottom field still marked.
        if let Some(e) = dpb.entries.iter_mut().find(|e| e.frame_num == 0) {
            e.long_term_frame_idx = 0;
            e.set_field_mark(T, RefMark::Unused);
            e.set_field_mark(B, RefMark::Long);
        }
        let [l0, _] = lists(&dpb, &field_slice(2, false, SliceType::P, 1, 3), 16);
        assert_eq!(l0, vec![(1, T), (1, B), (0, B)]);
    }

    #[test]
    fn field_list_modification_uses_field_pic_nums() {
        let mut dpb = two_pairs();
        store(&mut dpb, &field_slice(2, false, SliceType::P, 1, 0), 2, 16, 0, 4);
        let mut sh = field_slice(2, true, SliceType::P, 1, 5);
        // CurrPicNum 5; abs_diff 1 -> picNum 4 = 2 * FrameNumWrap(2), the opposite-parity (top)
        // field of frame 2. Then abs_diff 4 -> picNum 0: frame 0's top.
        sh.ref_pic_list_mod[0] = vec![RefPicListMod { idc: 0, value: 0 }, RefPicListMod { idc: 0, value: 3 }];
        let [l0, _] = lists(&dpb, &sh, 17);
        assert_eq!(l0, vec![(2, T), (0, T), (1, B), (0, B), (1, T)]);
        // Long-term: frame 1 long-term index 1; LongTermPicNum of its bottom seen from a bottom
        // field is 2 * 1 + 1 = 3.
        if let Some(e) = dpb.entries.iter_mut().find(|e| e.frame_num == 1) {
            e.long_term_frame_idx = 1;
            e.set_mark(RefMark::Long);
        }
        sh.ref_pic_list_mod[0] = vec![RefPicListMod { idc: 2, value: 3 }];
        let [l0, _] = lists(&dpb, &sh, 17);
        assert_eq!(l0.first(), Some(&(1, B)));
        assert_eq!(l0.iter().filter(|r| **r == (1, B)).count(), 1);
    }

    #[test]
    fn hostile_field_list_requests_never_panic() {
        let dpb = two_pairs();
        let mut sh = field_slice(9, true, SliceType::B, 1, 32);
        sh.ref_pic_list_mod[0] = vec![
            RefPicListMod { idc: 0, value: u32::MAX },
            RefPicListMod { idc: 1, value: 77 },
            RefPicListMod { idc: 2, value: 99 },
            RefPicListMod { idc: 9, value: 0 },
        ];
        sh.ref_pic_list_mod[1] = (0..40).map(|v| RefPicListMod { idc: v % 3, value: v }).collect();
        // Too many modifications is an error, never a panic; missing pictures are concealed.
        let r = dpb.build_field_ref_lists(&sh, i32::MIN, MAX_FN, |e, p| (e.frame, p));
        assert!(r.is_err());
        sh.ref_pic_list_mod[1].clear();
        let [l0, l1] = lists(&dpb, &sh, i32::MAX);
        assert_eq!((l0.len(), l1.len()), (32, 32), "padded with the first valid entry");
        let empty: Dpb<u32> = Dpb::new();
        let [l0, l1] = lists(&empty, &sh, 0);
        assert!(l0.is_empty() && l1.is_empty());
        // A frame slice asks the frame builder; the field builder returns nothing for it.
        let mut frame = hdr(2, None, 1, false);
        frame.slice_type = SliceType::P;
        frame.num_ref_idx_active = [2, 0];
        let [l0, _] = lists(&dpb, &frame, 0);
        assert!(l0.is_empty());
    }
}
