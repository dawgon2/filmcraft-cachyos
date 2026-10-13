//! Cutting an export range into chunks for the lanes (fork 6E.3, DESIGN-6E3 §2.1). Pure.
//!
//! Chunks are whole numbers of GOPs (`keyint` frames) from the range start, so a chunk starts
//! exactly where a single-device export would place an IDR, except that a boundary moves to a
//! nearby cut (an IDR at a scene change costs nothing and needs no source re-decode) and never
//! lands inside a transition. The last chunks shrink (guided self-scheduling) so that the slower
//! lane cannot hold the finish for long.

/// Output frames `[start, end)` encoded by one lane as one closed piece (IDR at `start`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// Position in file order (0, 1, …).
    pub index: usize,
    pub start: i64,
    pub end: i64,
}

impl Chunk {
    /// Frames in the chunk.
    pub fn len(&self) -> i64 {
        self.end.saturating_sub(self.start).max(0)
    }

    /// True when the chunk has no frames.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The chunks of output frames `[f0, f1)`: `keyint` frames per GOP (0 counts as 1), `nominal`
/// frames per chunk at most (rounded down to whole GOPs, at least one GOP), `lanes` lanes sharing
/// the work (for the tail rule; 0 counts as 1), `cuts` the output frames where a shot starts or
/// ends (any order; outside the range ignored), `transitions` the `[start, end)` frame ranges no
/// boundary may fall inside. The chunks cover the range exactly, in order, none empty.
pub fn plan(f0: i64, f1: i64, keyint: i64, nominal: i64, lanes: usize, cuts: &[i64], transitions: &[(i64, i64)]) -> Vec<Chunk> {
    let mut out = Vec::new();
    if f1 <= f0 {
        return out;
    }
    let keyint = keyint.max(1);
    let nominal = (nominal / keyint).max(1).saturating_mul(keyint);
    let lanes = i64::try_from(lanes.max(1)).unwrap_or(i64::MAX);
    let mut cuts: Vec<i64> = cuts.iter().copied().filter(|&c| c > f0 && c < f1).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut start = f0;
    while start < f1 {
        let remaining = f1.saturating_sub(start);
        // guided self-scheduling: a share of what is left, whole GOPs, between one GOP and nominal
        let share = (remaining / lanes.saturating_mul(2)).clamp(keyint, nominal);
        let len = (share / keyint).max(1).saturating_mul(keyint);
        let mut end = start.saturating_add(len);
        if end < f1 {
            end = snap_to_cut(end, start, keyint, &cuts);
            end = out_of_transition(end, transitions);
        }
        // never empty, never past the range; a short remainder joins the last chunk
        if end <= start || end >= f1 || f1.saturating_sub(end) < keyint.min(remaining) / 2 {
            end = f1;
        }
        out.push(Chunk { index: out.len(), start, end });
        start = end;
    }
    out
}

/// The cut nearest to `end` within `keyint` frames (strictly after `start`), else `end`.
fn snap_to_cut(end: i64, start: i64, keyint: i64, cuts: &[i64]) -> i64 {
    cuts.iter().copied().filter(|&c| c > start && c.abs_diff(end) <= keyint.unsigned_abs()).min_by_key(|&c| (c.abs_diff(end), c)).unwrap_or(end)
}

/// `end` moved to the end of the transition it falls inside (repeatedly, for chained ones).
fn out_of_transition(mut end: i64, transitions: &[(i64, i64)]) -> i64 {
    // each move goes strictly forward to a range end, so at most one move per range
    for _ in 0..=transitions.len() {
        match transitions.iter().find(|&&(a, b)| a < end && end < b) {
            Some(&(_, b)) => end = b,
            None => break,
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_cover(chunks: &[Chunk], f0: i64, f1: i64) {
        assert_eq!(chunks.first().map(|c| c.start), Some(f0));
        assert_eq!(chunks.last().map(|c| c.end), Some(f1));
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.index, i);
            assert!(!c.is_empty(), "{c:?}");
        }
        for w in chunks.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn whole_gops_and_a_shrinking_tail() {
        // 60 s at 29.97 (1798 frames), keyint 60, 8 s nominal (240 → 240), two lanes
        let c = plan(0, 1798, 60, 240, 2, &[], &[]);
        check_cover(&c, 0, 1798);
        assert_eq!(c[0].len(), 240);
        for ch in &c[..c.len() - 1] {
            assert_eq!(ch.start % 60, 0, "{ch:?}");
            assert_eq!(ch.len() % 60, 0, "{ch:?}");
        }
        // the tail shrinks to a GOP or two
        assert!(c[c.len() - 2].len() <= 120, "{c:?}");
    }

    #[test]
    fn boundaries_move_to_cuts_and_out_of_transitions() {
        let c = plan(0, 1000, 60, 240, 1, &[250, 5000, -3], &[]);
        check_cover(&c, 0, 1000);
        assert_eq!(c[0].end, 250, "snapped to the cut 10 frames later");
        let c = plan(0, 1000, 60, 240, 1, &[], &[(230, 270)]);
        check_cover(&c, 0, 1000);
        assert_eq!(c[0].end, 270, "moved past the dissolve");
        // a cut too far away is ignored
        let c = plan(0, 1000, 60, 240, 1, &[400], &[]);
        assert_eq!(c[0].end, 240);
    }

    #[test]
    fn hostile_ranges_and_parameters() {
        assert!(plan(10, 10, 60, 240, 2, &[], &[]).is_empty());
        assert!(plan(10, -5, 60, 240, 2, &[], &[]).is_empty());
        assert_eq!(plan(0, 1, 60, 240, 2, &[], &[]), [Chunk { index: 0, start: 0, end: 1 }]);
        // keyint / nominal / lanes of 0, nominal below and keyint above the range
        for (k, n, l) in [(0, 0, 0), (1000, 240, 2), (60, 0, 2), (60, 1, 2), (-5, -5, 2), (i64::MAX, i64::MAX, usize::MAX)] {
            let c = plan(0, 500, k, n, l, &[], &[]);
            check_cover(&c, 0, 500);
        }
        // extreme frame numbers
        let c = plan(i64::MAX - 1000, i64::MAX, 60, 240, 2, &[i64::MIN, i64::MAX], &[(i64::MIN, i64::MAX)]);
        check_cover(&c, i64::MAX - 1000, i64::MAX);
        // overlapping, chained transitions terminate
        let c = plan(0, 2000, 60, 240, 1, &[], &[(200, 300), (290, 400), (399, 2500)]);
        check_cover(&c, 0, 2000);
    }
}
