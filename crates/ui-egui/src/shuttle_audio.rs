//! Sound while shuttling (JKL): the sequence mix played faster or backwards, tape-style. Used by
//! the desktop's play-ahead mixer and by the web build's callback alike.

/// Fastest shuttle (JKL) speed that plays sound; faster ones play silence, like slow motion.
pub const MAX_SHUTTLE_AUDIO_SPEED: f64 = 2.0;

/// Whether playback at `speed` plays sound: normal speed forwards and backwards, and shuttling
/// up to [`MAX_SHUTTLE_AUDIO_SPEED`] either way.
pub fn audible(speed: f64) -> bool {
    let a = speed.abs();
    (a - 1.0).abs() < 1e-9 || (a > 1.0 && a <= MAX_SHUTTLE_AUDIO_SPEED)
}

/// `mix` played at `speed`, tape-style (the pitch follows the speed; negative plays backwards):
/// device frame `cursor` plays source frame `start + (cursor − start) · speed`, and each device
/// frame is the mean of the source frames it covers (a box filter, against aliasing when faster).
/// At `speed` 1 it is `mix` itself. Source frames before 0 are silence.
pub fn varispeed<M>(mut mix: M, start: i64, speed: f64) -> impl FnMut(i64, &mut [f32], usize) + Send + 'static
where
    M: FnMut(i64, &mut [f32], usize) + Send + 'static,
{
    let speed = if speed.is_finite() && speed != 0.0 { speed.clamp(-MAX_SHUTTLE_AUDIO_SPEED, MAX_SHUTTLE_AUDIO_SPEED) } else { 1.0 };
    let mut src: Vec<f32> = Vec::new();
    move |cursor: i64, buf: &mut [f32], ch: usize| {
        if speed == 1.0 {
            mix(cursor, buf, ch);
            return;
        }
        let ch = ch.max(1);
        let frames = buf.len() / ch;
        // source span of this block, lowest frame first
        let s0 = start as f64 + cursor.saturating_sub(start) as f64 * speed;
        let s1 = s0 + frames as f64 * speed;
        let lo = s0.min(s1).floor() as i64;
        let n = ((s0.max(s1).ceil() as i64).saturating_sub(lo).max(1) as usize).min(frames * MAX_SHUTTLE_AUDIO_SPEED as usize + 2);
        src.clear();
        src.resize(n * ch, 0.0);
        // before the start of the sequence: silence
        let skip = (-lo).clamp(0, n as i64) as usize;
        if let Some(part) = src.get_mut(skip * ch..)
            && !part.is_empty()
        {
            mix(lo + skip as i64, part, ch);
        }
        for (j, out) in buf.chunks_exact_mut(ch).enumerate() {
            let p0 = s0 + j as f64 * speed;
            let (q0, q1) = if speed > 0.0 { (p0, p0 + speed) } else { (p0 + speed, p0) };
            let i0 = ((q0.floor() as i64).saturating_sub(lo)).clamp(0, n as i64 - 1) as usize;
            let i1 = ((q1.ceil() as i64).saturating_sub(lo)).clamp(i0 as i64 + 1, n as i64) as usize;
            let k = (i1 - i0) as f32;
            for (c, o) in out.iter_mut().enumerate() {
                let sum: f32 = (i0..i1).filter_map(|i| src.get(i * ch + c)).sum();
                *o = sum / k;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{audible, varispeed};

    /// A ramp: source frame i plays the value i in every channel.
    fn ramp(cursor: i64, buf: &mut [f32], ch: usize) {
        for (j, f) in buf.chunks_exact_mut(ch).enumerate() {
            f.fill((cursor + j as i64) as f32);
        }
    }

    #[test]
    fn shuttle_speeds_that_play_sound() {
        assert!(audible(1.0) && audible(-1.0) && audible(2.0) && audible(-2.0) && audible(1.5));
        assert!(!audible(4.0) && !audible(-8.0) && !audible(0.25) && !audible(0.0));
    }

    #[test]
    fn twice_as_fast_averages_pairs_and_backwards_plays_in_reverse() {
        let mut v = varispeed(ramp, 100, 2.0);
        let mut buf = vec![0.0; 8];
        v(100, &mut buf, 2);
        // frames (100,101), (102,103), … averaged
        assert_eq!(buf, [100.5, 100.5, 102.5, 102.5, 104.5, 104.5, 106.5, 106.5]);
        // the next block continues where the source left off
        v(104, &mut buf, 2);
        assert_eq!(buf.first(), Some(&108.5));
        // backwards from position 100: the frame before it first
        let mut r = varispeed(ramp, 100, -1.0);
        let mut buf = vec![0.0; 4];
        r(100, &mut buf, 1);
        assert_eq!(buf, [99.0, 98.0, 97.0, 96.0]);
        r(104, &mut buf, 1);
        assert_eq!(buf, [95.0, 94.0, 93.0, 92.0]);
    }

    #[test]
    fn backwards_past_the_start_is_silence() {
        let mut r = varispeed(ramp, 2, -2.0);
        let mut buf = vec![9.0; 4];
        r(2, &mut buf, 1);
        // frames (1, 0), then nothing but silence
        assert_eq!(buf, [0.5, 0.0, 0.0, 0.0]);
    }
}
