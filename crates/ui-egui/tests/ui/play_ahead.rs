//! Audio mixed ahead (`play_ahead`): driven by a wall-clock device. Its own test binary, because one
//! test panics a mixer on purpose and the crash hook's tests read the process-wide last panic.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use filmcraft_ui_egui::play_ahead::{AudioStats, BLOCK_FRAMES, LEAD_S, spawn};

const SR: u32 = 48_000;
const PERIOD: usize = 512;

/// A mixer whose every sample is its own device frame number, so the output shows which frame
/// played where.
fn frame_numbers(delay: Duration, calls: Arc<AtomicU64>) -> impl FnMut(i64, &mut [f32], usize) + Send + 'static {
    move |cursor, buf, ch| {
        calls.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(delay);
        for (i, f) in buf.chunks_mut(ch).enumerate() {
            f.fill((cursor + i as i64) as f32);
        }
    }
}

/// Plays `callbacks` device periods, one every period on the wall clock as a device does, and
/// returns the interleaved output.
fn play(fill: &mut Box<dyn FnMut(&mut [f32], usize) + Send>, ch: usize, callbacks: usize) -> Vec<f32> {
    let period = Duration::from_secs_f64(PERIOD as f64 / SR as f64);
    let t0 = Instant::now();
    let mut out = Vec::new();
    for k in 0..callbacks {
        let mut buf = vec![f32::NAN; PERIOD * ch];
        fill(&mut buf, ch);
        out.extend_from_slice(&buf);
        if let Some(wait) = (period * (k as u32 + 1)).checked_sub(t0.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    out
}

#[test]
fn plays_every_frame_in_order_from_the_start() {
    let stats = Arc::new(AudioStats::default());
    let mut fill = spawn(frame_numbers(Duration::ZERO, Arc::default()), 1000, SR, 2, stats.clone());
    std::thread::sleep(Duration::from_millis(30));
    let out = play(&mut fill, 2, 60);
    for (p, f) in out.chunks(2).enumerate() {
        assert_eq!(f, [(1000 + p) as f32; 2], "frame {p}");
    }
    assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    assert_eq!(stats.callbacks.load(Ordering::Relaxed), 60);
}

#[test]
fn a_mixer_slower_than_the_device_plays_silence_and_stays_in_time() {
    // each 1024-frame block takes 40 ms to mix, about twice its 21 ms of sound
    let stats = Arc::new(AudioStats::default());
    let mut fill = spawn(frame_numbers(Duration::from_millis(40), Arc::default()), 1, SR, 2, stats.clone());
    let out = play(&mut fill, 2, 120);
    assert!(stats.underruns.load(Ordering::Relaxed) > 0);
    assert!(stats.missing_frames.load(Ordering::Relaxed) > 0);
    let mut heard = 0;
    for (p, f) in out.chunks(2).enumerate() {
        assert!(f.iter().all(|x| x.is_finite()), "frame {p} left unwritten");
        if f[0] != 0.0 {
            // every frame that plays is the one due at that moment: no drift after an underrun
            assert_eq!(f[0], (1 + p) as f32, "frame {p}");
            heard += 1;
        }
    }
    assert!(heard > PERIOD, "some sound still plays");
}

#[test]
fn a_mixer_that_panics_leaves_silence_not_a_crash() {
    let stats = Arc::new(AudioStats::default());
    let mut first = true;
    let mix = move |cursor: i64, buf: &mut [f32], ch: usize| {
        if !first {
            panic!("mixer bug");
        }
        first = false;
        frame_numbers(Duration::ZERO, Arc::default())(cursor, buf, ch);
    };
    let mut fill = spawn(mix, 1, SR, 2, stats.clone());
    let out = play(&mut fill, 2, 20);
    assert!(out.iter().all(|x| x.is_finite()));
    assert_eq!(out.get(..2), Some(&[1.0, 1.0][..]), "the block mixed before the panic plays");
    assert!(stats.underruns.load(Ordering::Relaxed) > 0);
}

#[test]
fn a_device_with_other_channels_plays_the_channels_both_have() {
    for dev_ch in [1, 6, 10, 16] {
        let stats = Arc::new(AudioStats::default());
        let mut fill = spawn(frame_numbers(Duration::ZERO, Arc::default()), 1, SR, 2, stats.clone());
        // let the mixer get ahead first, as a device would before its first callback
        std::thread::sleep(Duration::from_millis(30));
        let out = play(&mut fill, dev_ch, 4);
        for (p, f) in out.chunks(dev_ch).enumerate() {
            let want: Vec<f32> = (0..dev_ch).map(|c| if c < 2 { (1 + p) as f32 } else { 0.0 }).collect();
            assert_eq!(f, &want[..], "{dev_ch} channels, frame {p}");
        }
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0, "{dev_ch} channels");
    }
}

#[test]
fn stopping_the_stream_stops_the_mixer() {
    let calls = Arc::new(AtomicU64::new(0));
    let fill = spawn(frame_numbers(Duration::ZERO, calls.clone()), 1, SR, 2, Arc::default());
    std::thread::sleep(Duration::from_millis(50));
    drop(fill);
    std::thread::sleep(Duration::from_millis(20));
    let after_stop = calls.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::Relaxed), after_stop);
}

#[test]
fn the_ring_keeps_no_more_than_the_lead() {
    let calls = Arc::new(AtomicU64::new(0));
    let _fill = spawn(frame_numbers(Duration::ZERO, calls.clone()), 1, SR, 2, Arc::default());
    std::thread::sleep(Duration::from_millis(100));
    let lead_blocks = (SR as f64 * LEAD_S / BLOCK_FRAMES as f64).ceil() as u64;
    assert!(calls.load(Ordering::Relaxed) <= lead_blocks + 1, "mixed {} blocks", calls.load(Ordering::Relaxed));
}

/// Fork A1.2: one block whose mixing panics plays as silence and the mixer carries on in time; a
/// mixer that panics block after block stops and says so (`mixerStopped`).
#[test]
fn a_panicking_block_is_silence_and_only_repeated_panics_stop_the_mixer() {
    let stats = Arc::new(AudioStats::default());
    let mut blocks = 0;
    let mix = move |cursor: i64, buf: &mut [f32], ch: usize| {
        blocks += 1;
        if blocks == 3 {
            panic!("one bad block");
        }
        frame_numbers(Duration::ZERO, Arc::default())(cursor, buf, ch);
    };
    let mut fill = spawn(mix, 1, SR, 2, stats.clone());
    std::thread::sleep(Duration::from_millis(50));
    let out = play(&mut fill, 2, 16);
    assert_eq!(stats.mixer_panics.load(Ordering::Relaxed), 1);
    assert!(!stats.mixer_stopped.load(Ordering::Relaxed));
    // the third block (frames 2049..=3072) is silent, the fourth plays its own frames: in time
    let frame = |p: usize| out.get(p * 2).copied();
    assert_eq!(frame(2048), Some(0.0));
    assert_eq!(frame(3072), Some(3073.0));
    assert_eq!(frame(4000), Some(4001.0));

    let stats = Arc::new(AudioStats::default());
    let mut first = true;
    let mix = move |cursor: i64, buf: &mut [f32], ch: usize| {
        if !first {
            panic!("mixer bug");
        }
        first = false;
        frame_numbers(Duration::ZERO, Arc::default())(cursor, buf, ch);
    };
    let _fill = spawn(mix, 1, SR, 2, stats.clone());
    // (a loaded machine symbolising backtraces can take a while per panic)
    let deadline = Instant::now() + Duration::from_secs(5);
    while !stats.mixer_stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(stats.mixer_stopped.load(Ordering::Relaxed));
    assert_eq!(stats.mixer_panics.load(Ordering::Relaxed), 4);
    assert_eq!(stats.to_json(SR)["mixerStopped"], true);
}

/// Fork A1.2: a device with large buffers gets a lead of a few of them, and every underrun grows
/// the lead, up to MAX_LEAD_S; a hostile sample rate or channel count doesn't size the ring by it.
#[test]
fn the_lead_follows_the_device_buffer_and_grows_after_underruns() {
    use filmcraft_ui_egui::play_ahead::MAX_LEAD_S;
    let stats = Arc::new(AudioStats::default());
    let mut fill = spawn(frame_numbers(Duration::ZERO, Arc::default()), 1, SR, 2, stats.clone());
    assert_eq!(stats.lead_frames.load(Ordering::Relaxed), (SR as f64 * LEAD_S) as u64);
    let mut buf = vec![0.0; 4096 * 2];
    fill(&mut buf, 2);
    assert_eq!(stats.lead_frames.load(Ordering::Relaxed), 4 * 4096, "four of the device's 4096-frame buffers");

    // a mixer faster than real time that stalls longer than the lead now and then (a slow file
    // open at a cut; 40 blocks = 0.85 s of sound between stalls):
    // each underrun grows the lead, up to the cap, never past it
    let stats = Arc::new(AudioStats::default());
    let mut blocks = 0u32;
    let mut numbers = frame_numbers(Duration::ZERO, Arc::default());
    let stalling = move |cursor: i64, buf: &mut [f32], ch: usize| {
        blocks += 1;
        if blocks.is_multiple_of(40) {
            std::thread::sleep(Duration::from_millis(320));
        }
        numbers(cursor, buf, ch);
    };
    let mut fill = spawn(stalling, 1, SR, 2, stats.clone());
    std::thread::sleep(Duration::from_millis(50));
    let _ = play(&mut fill, 2, 600);
    let lead = stats.lead_frames.load(Ordering::Relaxed);
    assert!(stats.underruns.load(Ordering::Relaxed) > 1, "a stall underruns several callbacks");
    assert!(lead > (SR as f64 * LEAD_S) as u64, "grew: {lead}");
    // one step per stall (not per short callback): it stops growing once the lead covers the
    // 320 ms stall, well short of the cap
    assert!(lead < (SR as f64 * MAX_LEAD_S) as u64, "grew once per stall, not to the cap: {lead}");

    // hostile device values: still plays, no runaway allocation
    for (rate, ch) in [(u32::MAX, 64), (0, 2), (1, 1)] {
        let stats = Arc::new(AudioStats::default());
        let mut fill = spawn(frame_numbers(Duration::ZERO, Arc::default()), 1, rate, ch, stats.clone());
        let mut buf = vec![f32::NAN; 64 * ch];
        fill(&mut buf, ch);
        assert!(buf.iter().all(|x| x.is_finite()), "{rate} Hz, {ch} ch");
    }
}
