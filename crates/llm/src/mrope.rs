//! Multimodal rotary positions for Qwen3-VL's text model (AI-PLAN AI4 item 2,
//! `docs/fork/DESIGN-AI4-TEXT.md` step 4), as the published reference implementation lays them
//! out: every token has a (t, h, w) position. Text tokens count up with t = h = w, so text-only
//! prompts get ordinary rotary positions. A picture's merged tokens, in reading order, all share
//! t = start and take h = start + row, w = start + column; the text after it continues from
//! start + max(rows, columns). The rotary angles then take each frequency pair's position from
//! h or w on the interleaved sections (pairs 1, 4, 7, … for h; 2, 5, 8, … for w, each over three
//! times its section) and from t otherwise.

/// Most tokens one prompt may have.
pub const MAX_TOKENS: usize = 1 << 20;

/// One run of the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// This many text tokens.
    Text(usize),
    /// A picture's merged tokens: `rows` × `cols` (the patch grid divided by the merge size).
    Picture { rows: usize, cols: usize },
}

/// The (t, h, w) position of every token, and the position the next token would take. None for
/// an empty picture or a prompt over [`MAX_TOKENS`].
pub fn positions(spans: &[Span]) -> Option<(Vec<[u32; 3]>, u32)> {
    let mut out = Vec::new();
    let mut next = 0usize;
    for span in spans {
        match *span {
            Span::Text(n) => {
                if out.len().checked_add(n)? > MAX_TOKENS {
                    return None;
                }
                for i in 0..n {
                    let p = u32::try_from(next.checked_add(i)?).ok()?;
                    out.push([p, p, p]);
                }
                next = next.checked_add(n)?;
            }
            Span::Picture { rows, cols } => {
                if rows == 0 || cols == 0 || out.len().checked_add(rows.checked_mul(cols)?)? > MAX_TOKENS {
                    return None;
                }
                let t = u32::try_from(next).ok()?;
                for r in 0..rows {
                    for c in 0..cols {
                        out.push([t, u32::try_from(next + r).ok()?, u32::try_from(next + c).ok()?]);
                    }
                }
                next = next.checked_add(rows.max(cols))?;
            }
        }
    }
    Some((out, u32::try_from(next).ok()?))
}

/// The `head_dim / 2` rotary angles of one token at `pos`: pair j turns at
/// `theta^(-2j / head_dim)` times the position on its axis. `sections` are the (t, h, w) section
/// sizes of the config (`mrope_section`, e.g. [24, 20, 20] for head 128).
pub fn angles(pos: [u32; 3], head_dim: usize, sections: [usize; 3], theta: f32) -> Vec<f32> {
    let half = head_dim / 2;
    (0..half)
        .map(|j| {
            let axis = if j % 3 == 1 && j < 3 * sections[1] {
                1
            } else if j % 3 == 2 && j < 3 * sections[2] {
                2
            } else {
                0
            };
            let inv = theta.powf(-((2 * j) as f32) / head_dim as f32);
            pos[axis] as f32 * inv
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_only_prompts_get_ordinary_positions() {
        let (p, next) = positions(&[Span::Text(4)]).unwrap();
        assert_eq!(p, vec![[0, 0, 0], [1, 1, 1], [2, 2, 2], [3, 3, 3]]);
        assert_eq!(next, 4);
        // every pair turns with the one position: plain rotary angles
        let a = angles([3, 3, 3], 128, [24, 20, 20], 5_000_000.0);
        for (j, v) in a.iter().enumerate() {
            let want = 3.0 * 5_000_000f32.powf(-((2 * j) as f32) / 128.0);
            assert!((v - want).abs() <= 1e-6 * want.abs().max(1.0), "{j}");
        }
    }

    #[test]
    fn a_picture_takes_grid_positions_and_text_resumes_after_it() {
        let (p, next) = positions(&[Span::Text(2), Span::Picture { rows: 2, cols: 3 }, Span::Text(2)]).unwrap();
        assert_eq!(p, vec![[0, 0, 0], [1, 1, 1], [2, 2, 2], [2, 2, 3], [2, 2, 4], [2, 3, 2], [2, 3, 3], [2, 3, 4], [5, 5, 5], [6, 6, 6]]);
        assert_eq!(next, 7);
    }

    #[test]
    fn interleaved_sections_pick_the_axis() {
        // t = 1, h = 10, w = 100: pair 0 → t, 1 → h, 2 → w, 3 → t; past 3 × 20 everything is t
        let a = angles([1, 10, 100], 128, [24, 20, 20], 1.0);
        assert_eq!(&a[..4], &[1.0, 10.0, 100.0, 1.0]);
        assert_eq!(a[58], 10.0);
        assert_eq!(a[59], 100.0);
        assert_eq!(a[61], 1.0);
        assert_eq!(a[62], 1.0);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hostile_spans_are_refused() {
        assert!(positions(&[Span::Picture { rows: 0, cols: 4 }]).is_none());
        assert!(positions(&[Span::Picture { rows: usize::MAX, cols: 2 }]).is_none());
        assert!(positions(&[Span::Text(MAX_TOKENS), Span::Text(1)]).is_none());
        assert!(positions(&[Span::Text(usize::MAX)]).is_none());
        assert_eq!(positions(&[]), Some((vec![], 0)));
    }
}
