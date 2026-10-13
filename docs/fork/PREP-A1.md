# PREP-A1: Linux playback reliability (audit before the work)

Prepared 2026-10-13 by the PC run (A1-PREP from CLOUD-QUEUE); code facts as of commit 790e2c20.

Docs only. No code was changed. Line numbers are from that commit.

## 1. What A1 asks for (AUDIO-PLAN.md, "Phase A1", about 20-40 h, tag `audio-1`)

- **A1.1** Reproduce each reported problem as a test where possible: overlapping clips (#336),
  long timelines, scrubbing, J/K/L shuttle, seeks during playback, device unplug or default-device
  change, 44.1 vs 48 kHz devices, PipeWire and plain ALSA. First check what the upstream merge
  (U0.2) already brought.
- **A1.2** Fix the causes. The mixer never allocates or locks on the audio callback thread
  (lock-free ring, look-ahead sized from the device buffer). Decode and effects run ahead on a
  worker. Underruns are counted in `perf.stats` (`audio.underruns`).
- **A1.3** Devices. The output device is chosen and remembered in Settings. Device loss falls back
  to the default device and the UI says so. A sample-rate mismatch is resampled, not an error.
- **A1.4** A/V sync. Audio stays within +-1 frame of video at 2x / 4x and after 100 random seeks,
  measured from the clock.
- **Accept:** a 30-minute soak (20 tracks, overlapping clips, shuttles, seeks) has zero underruns
  and drift <= 1 frame. Every reproducible report has a regression test. Unplug and replug of a
  USB device recovers without a restart.

Headline finding: most of A1.2 and A1.3 already exists (upstream-owned, merged in U0). What is
missing is mainly live device recovery, the sync measurement, and the soak harness.

## 2. What exists today

| Where | What it does | A1 step |
|---|---|---|
| `apps/filmcraft/src/audio.rs:12` `CpalOut` | cpal output, implements the `AudioOut` trait. Holds the stream, a played-frames atomic, a `failed` flag, the rate, the channels and the hardware prefs. cpal 0.18 with the `pulseaudio` feature (`apps/filmcraft/Cargo.toml:37`). | all |
| `audio.rs:54` `host()` | Picks the host from `device_class`; unknown or empty means `default_host()`. | A1.3 |
| `audio.rs:75` `fallback_rank`, `:92` `output_device` | Named device first, then the host default. If the default cannot be opened (broken ALSA `default`), probes the others in order pipewire, pulse, other PCMs, `hw:`/`plughw:` last. Caches the pick in a `Mutex`. Sets a `Note::Fallback` for the status bar. | A1.3 (fallback exists, but only at open time) |
| `audio.rs:148` `config()` | Takes the requested rate (or the sequence rate if "force document rate" is on) and the best matching supported config. If there is none it uses the device default. | A1.3 |
| `audio.rs:161` `converted_stream` | Integer and f64 sample formats. Uses a `scratch` Vec on the callback (`try_reserve`, refuses buffers over 1 Mi samples). Allocates on first call and when the buffer grows. | A1.2 (minor) |
| `audio.rs:197` `stream_error` | Sets `failed` and prints to stderr. No other recovery. | A1.3 |
| `audio.rs:212` `start()` | Stops, opens the device, builds the stream (a Fixed buffer size only if it is inside the supported range), calls `play()`. `played` counts frames per callback. | A1.2/A1.3 |
| `audio.rs:270` `played_frames()` | The playback master clock. Returns `None` once `failed` is set. It counts frames handed to the driver and is not corrected for output latency. | A1.4 |
| `audio.rs:281` `configure()` | Applies the prefs and re-reads the rate and channels. Takes effect at the next `start`. | A1.3 |
| `audio.rs:300+` tests | Pure tests: fallback rank, note text, sample conversion, `failed` stream gives no clock. Two `#[ignore]` hardware tests (`audio_output_plays_on_every_host`, `xrun_recovers`). | A1.1 |
| `crates/ui-egui/src/lib.rs:76` `trait AudioOut` | The interface: `start(fill)`, `stop`, `sample_rate`, `channels`, `played_frames`, `devices`, `configure`, `note`. Fake outputs exist in `tests/ui/playback_clock_ui.rs:23` and `source_playback_ui.rs`. | all |
| `crates/ui-egui/src/play_ahead.rs` (193 lines) | The lock-free design A1.2 describes. A mixer thread (`filmcraft-audio-mix`) fills an atomics-only SPSC ring, 0.2 s ahead (`LEAD_S`, 1024-frame blocks). The callback only copies out of it; no lock or allocation. On an underrun it plays silence, counts it, and tells the mixer to skip the same span so the sound stays in step. The mixer runs under `catch_unwind`. | A1.2 mostly done |
| `play_ahead.rs:28` `AudioStats` | `callbacks`, `underruns`, `missing_frames`, `min_lead_frames`. Exposed as `perf.stats` `playback.audio` (`crates/ui-egui/src/perf.rs:26`, docs in `docs/control-protocol.md`). | A1.2 |
| `lib.rs:1186` `start_audio()` | Builds the mix closure, wraps it in `varispeed`, calls `play_ahead::spawn`, then `a.start`. On error: `audio_clock=false` and a log line; the user sees nothing. | A1.2/A1.3 |
| `lib.rs:1237-1275` clock polling | Elapsed time comes from `played_frames()/sr`. If `failed` gives `None`, it switches to the wall clock and sets the status "Audio output failed: playing without sound". If no frames are consumed for `AUDIO_STALL_S` = 1 s (`:243`), it also falls back to the wall clock. | A1.3/A1.4 |
| `lib.rs:2250` `playback_mix` | Per-block mix at the device rate. Uses the live project snapshot (`previews.live.project()`), resamples through `StreamResampler` when the sequence rate differs from the device rate, handles Output Mapping, the 5.1 mixdown and voice-over cues. | A1.3 |
| `crates/audio-dsp/src/resample.rs` (170 lines) | Linear streaming resampler, exact rational positions, so output is the same however the stream is cut into blocks. Rate 0 is treated as 1. | A1.3 mostly done |
| `crates/ui-egui/src/shuttle_audio.rs` | `varispeed`: tape-style sound at +-2x; faster is silent. 4x has no sound by design. | A1.4 |
| `crates/engine/src/settings.rs:217` `AudioHardwarePrefs` | `device_class`, `default_output`, `buffer_size` (512), `sample_rate` (48000), `force_document_rate`, output mapping. Saved with prefs, so "remembered" is true. | A1.3 |
| `apps/filmcraft/src/audio_in.rs` (255 lines) | cpal input for voice-over. Not part of A1 playback. | out of scope |
| Existing tests | `crates/ui-egui/tests/ui/play_ahead.rs`: ordered frames, slow mixer gives silence and stays in time, mixer panic leaves silence, channel mismatch, drop stops the mixer, ring holds at most the lead. `playback_clock_ui.rs`: stalled device does not freeze, working device drives the clock, fallback note reaches the status bar. `crates/ui-egui/examples/bench_audio.rs`; `docs/fork-benchmarks.md:226` records 0 underruns over 10 minutes (56 247 callbacks). | A1.1 |

No PipeWire-specific code exists. PipeWire and PulseAudio are reached through cpal's ALSA host
(the `pipewire`/`pulse` PCM names, only used in the fallback ranking) and its `pulseaudio`
feature. There is no hotplug or default-device-change watcher anywhere.

## 3. Gaps per A1 step

| Step | Gap | Where | Size |
|---|---|---|---|
| A1.1 | A repro matrix does not exist. The reports (#336, #410, #602, #605, #617) have no named regression tests. Needs a fake `AudioOut` / mixer harness that drives `play_ahead` with a jittery wall-clock "device". | `crates/ui-egui/tests/ui/` (new `a1_repro.rs`) | M |
| A1.1 | Check U0.2: confirm the upstream merge brought no newer fix to `audio.rs` or `play_ahead.rs`. | read-only diff | S |
| A1.2 | The ring and counters are done. Remaining: `converted_stream` still allocates `scratch` on the callback (first call). Pre-size it in `start()`. | `audio.rs:161` | S |
| A1.2 | A dead mixer thread is not detected. After a panic the thread ends, the callback plays silence forever, and nothing says so. Add an `AtomicBool` "mixer_dead" and count it in `AudioStats`, then restart or surface it. | `play_ahead.rs:158`, `perf.rs` | S-M |
| A1.2 | The lead is a fixed 0.2 s and not sized from the device buffer. Make it `max(LEAD_S, k x buffer)`; add a "lead grows after an underrun" rule (adaptive). | `play_ahead.rs:132` | S |
| A1.2 | The mix closure takes `previews.live.project()` and other locks, and `mix_layout` allocates. This is fine on the mixer thread but needs a check that no lock is held across a blocking decode. Audit `previews.live`. | `lib.rs:2250`, `crates/engine` previews | M |
| A1.2 | "Decode runs ahead on a worker": the mixer thread decodes inline. A cut with a slow file open eats lead. A soak with 20 tracks is the first real check. | measurement first | M-L |
| A1.3 | Device loss at runtime: `stream_error` only sets `failed`, so the clock goes to the wall clock and playback is silent until the user presses play again. Needs: on a `DeviceNotAvailable` error, re-resolve the device (default device, using `output_device`), restart the stream while keeping the mixer position, and post a status note. | `audio.rs:197` plus a hook in `lib.rs:1237` | M-L |
| A1.3 | No default-device-change detection while idle or playing (PipeWire moving the default sink). Cheapest option: re-resolve on every `start()` (already true) plus a periodic re-probe (a few seconds) of `default_output_device()` name while playing or idle. | `audio.rs` | M |
| A1.3 | "Says so in the UI": the fallback note exists (`Note::Fallback`) but a `start()` error is only logged (`lib.rs:1210`). Show a status message. | `lib.rs:1208` | S |
| A1.3 | A requested rate the device lacks falls back to the device default, and the mixer resamples to it. This works, but `configure()` and `start()` can disagree if the device changed between them. `start_audio` reads `a.sample_rate()` *before* `a.stop()`/`a.start()`, and `start()` may then pick a different rate: the mix runs at the old rate, so pitch and speed would be wrong. Fix: `start()` must return the rate and the mixer must be built from it (or start() restarts). Needs confirming with a test. | `lib.rs:1186-1204`, `audio.rs:228` | M |
| A1.3 | Resampler is linear (quality); fine for A1, a better kernel is later work. | `resample.rs` | skip |
| A1.4 | `played_frames()` is not latency-compensated and counts silence as played. Audio and picture are consistent with each other but both are early by the driver latency. Use the cpal timestamp in the data callback (`OutputCallbackInfo`) to compute the true playback instant, or at least store the reported latency. | `audio.rs:235-240` | M |
| A1.4 | No measurement of drift. Needs a sync probe: a synthetic timeline (a click each frame), then compare the mixer cursor against the video frame clock after 100 seeks and at 2x/4x, from the clock values rather than by ear. 4x is silent by design: decide that the "+-1 frame at 4x" criterion then means clock-only. | new test + `perf.stats` field `sync.driftFrames` | M |
| A1.4 | Seek during playback: confirm `play()` restarts audio with the new cursor (reset of `audio_seen`, stats). Test it. | `lib.rs` `play`/`start_audio` | S |
| Soak | 30-minute, 20-track synthetic soak harness (headless `bench_audio` extension) with a pass/fail on `underruns == 0` and drift. | `crates/ui-egui/examples/bench_audio.rs` | M |

## 4. Risks

- **Never-crash.** The new code must have no `unwrap`/`expect`/`panic!`/indexing in non-test
  code. Existing spots to harden: `lib.rs:2287` `mix.channels[c][i]` and `mix.channels[0]` index
  without `get()` (a short or empty channel vec would panic, and today that kills the mixer
  thread silently); `mix.channels.len() - 1` underflows if empty. `play_ahead.rs` `late - drop`
  is safe because of `min`, but check overflow on `skip` casts (`as i64`).
- **Realtime thread.** The device callback must not allocate, lock or log. Today the F32 path is
  clean. The converted path allocates once. `eprintln!` in `stream_error` runs on cpal's error
  thread (not the audio thread), which is acceptable. Do not add a `Mutex` to the callback to
  deliver device-loss events; use atomics only.
- **Device loss.** cpal error callbacks can fire from other threads, can arrive twice, and can
  race `stop()`. Dropping a `cpal::Stream` from the UI thread while the error callback runs must
  stay safe. Restarting must not loop forever: add a retry cap with backoff, then fall back to
  silent wall-clock playback.
- **Hostile values.** Device channel counts (0, 1, 64+), rates (0, 8000, 384000+), huge or tiny
  buffer sizes, `u32` buffer sizes from prefs. `ch` is clamped to 64 in `play_ahead`; `start()`
  rejects 0. Check `lead * ch` against absurd rates (a 4 GHz rate would allocate gigabytes:
  cap `sample_rate` at, say, 768 kHz and the ring size).
- **Hardware-exclusive fallback.** The `hw:` devices are last in rank on purpose (they silence
  the desktop). Keep that when adding hotplug.
- **Regression risk.** `play_ahead` carries the open-bug fixes for #23/#106/#136/#254. Keep
  those tests green.

## 5. Suggested tests

Can be done with fakes (no audio hardware), mostly `crates/ui-egui/tests/ui/`:
- Wall-clock fake device that calls the fill closure on a timer with jitter and stalls; assert
  `underruns == 0` under a healthy mixer and a bounded count under stalls (extends `play_ahead.rs`).
- Mixer that blocks 50 ms at "cuts" (overlapping clips repro #336): lead must absorb it.
- Mixer panic: `mixer_dead` flag set, counter visible in `perf.stats`.
- Fake `AudioOut` that returns `failed` after N frames, then recovers on re-resolve: assert the
  status note, the cursor continuity and the retry cap.
- Fake device whose rate changes between `configure()` and `start()` (the 44.1/48 kHz mismatch):
  assert the mix rate equals the started rate.
- Hostile device values (0 channels, absurd rate, huge buffer): `start` returns `Err`, no panic.
- Sync probe: click-per-frame synthetic sequence, 100 random seeks (seeded), 2x forward and
  backward, compare `played_frames/sr` to the video clock; assert <= 1 frame.
- Resampler: 44.1 to 48 k, no block-boundary phase jump (partly covered already).
- Unit tests for any new pure function (device re-resolve ordering, retry/backoff).

Needs real hardware (keep as `#[ignore]`, run on the PC and the Dell): USB unplug and replug,
PipeWire default-sink switch during playback, a plain ALSA `hw:` device, a Xonar-class card
(#605), true output latency, and the 30-minute soak on real decoders (the synthetic soak can run
on a fake clock; a real-clock pass is the acceptance run).

## 6. Suggested lane split

Independent, mostly single-crate:
1. **Lane A (`apps/filmcraft` only):** device loss and default-device recovery, scratch
   pre-size, rate agreement between `configure` and `start`, cpal timestamp latency. Fakes needed:
   extract the device-choice logic behind a small trait so it is testable. Largest piece (L).
2. **Lane B (`crates/ui-egui`):** `play_ahead` hardening (mixer-dead flag, adaptive lead sized
   from the device buffer), `playback_mix` indexing fixes, status message when `start` fails,
   repro tests (A1.1). M.
3. **Lane C (`crates/ui-egui` + `examples/bench_audio.rs`):** sync probe, `sync.driftFrames` in
   `perf.stats`, 100-seek test, soak harness. M. Depends on B only for the final soak.
4. Last: the real-hardware acceptance pass and tag `audio-1`.

Lanes A and B touch different crates and can run in parallel. The `AudioOut` trait is the shared
contract: if A needs a change (for example `start` returning the real rate, or a `device_lost()`
poll), land the trait change first, in one small commit, then split.
