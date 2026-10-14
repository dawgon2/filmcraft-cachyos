# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–74 incl. §1a.6 order; §5 phases; only the last ~80 lines of §7), then
[`AI-PLAN.md`](../../AI-PLAN.md) (phase AI1, next) and `TRAINING.md` where AI1.6 needs it, then
[`FORK-ONLY.md`](FORK-ONLY.md), [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), this file. Other plan
files (GRADE, RESOLVE-UI, YOUTUBE, AUDIO, AI, COMPOSITOR, STORAGE, H264, RAW, DECODER) only when a
step points at them. Rewritten 2026-10-13 21:00 after the U0.2 acceptance (scheduled run).

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → Y1 ✓ → 6E ✓ → U0.2 ✓ → **AI1** →
**X422** → Y3 → C1 → **AI3** (§1a.73–74; knowledge work on the Dell first) → C2 → AI2 → S1 → S2 → G3 → 7 (+ Y4) → Y2 → C3 → C4 → G4 → A3 → R1 →
R2 → D → 8. Small phases are accepted in groups (§1a.41). AI1 moved right after U0.2 and the
4:2:2 export cases (X422) right after AI1 by the owner 2026-10-13 (§1a.69). **A2 and then
H1 → H2 → H3 run on the Dell lane (owner 2026-10-14, §1a.72); the main runner skips them.**
**A1 (Dell lane, §1a.60) accepted by the owner 2026-10-14 ~01:20 (§1a.71)**; its tag `audio-1` rides with the next phase group.
**L1 / L2 part 2 accepted by the owner (live, 2026-10-14 ~01:10; was ready 2026-10-13 22:55, Dell lane; scheduled task `filmcraft-dell-lane`,
lock `.dell-lane.lock`):** the last two steps merged: **L1.5** `d0c68898` (upstream #260 off-thread
file dialogs cherry-picked, Q23; dialogs parented to the window, each kind starts in its last
folder, close waits for an open dialog) and **L2.6** `7f6d79dc` (Settings ▸ Playback ▸ Graphics
device by PCI ids, VA-API tries the chosen GPU's render node first, `graphics.listDevices`).
Checklist on the Dell (CachyOS, UHD 770, 2026-10-13): fmt ✓, clippy ✓ (workspace minus
`filmcraft-speech`, see below), engine / app / ui-egui lib ✓, every `--test ui` module one per
process ✓ (86 / 86), `native_dialogs_ui` ✓, platform lib ✓, layers ✓, wasm ✓; both steps had a
fresh-context Opus review, findings fixed. Earlier part 2 items: Flatpak build ✓ (`18d523e4`,
feature-checked in a virtual KWin; SDK kept on the Dell, Q17), L1.8 / L2.4 / L2.10 and the
text-scale hint (`2e67f732`). **Tags `linux-1b` / `linux-2b` ride with the next phase group (AI1).**
**Owed at acceptance:** on the PC: Graphics device Auto vs a chosen GPU on the RX 6700 XT + UHD 750
(window, monitors and decode on the same GPU; `graphics.listDevices` `inUse`), Program monitor 1:1
at fractional scale; on a real desktop: a file dialog stacked over the window (KDE portal, Wayland)
and opening in the last folder, GlobalShortcuts dialog, card insert → Copy → Eject, KDE Connect send /
receive / Multimedia control, Plasma font 11 pt → 110 %, an export notification popping.
**Seen on the Dell, not the lane's:** `cargo clippy --workspace --all-targets` fails in
`crates/speech/examples/candle_ops_bench.rs` since `a2cd09d4` (AI1.2): the example uses candle without
`required-features = ["whisper"]` in `crates/speech/Cargo.toml`.
**A1 accepted by the owner (live, 2026-10-14 ~01:20, §1a.71; was ready 00:55, Dell lane; AUDIO-PLAN Phase A1, §1a.59–60).** Steps merged:
A1.1 `a7b753c2` (upstream #840 stream recovery, ported to cpal 0.18), A1.3a `39f4439a` (failed reopen
retried within the budget; restart keeps the running format), A1.2 / A1.2b (play-ahead: per-block
panic recovery, `mixerStopped` → status + wall clock in both monitors, adaptive lead ≥ 4 device
buffers +50 ms per underrun episode ≤ 0.5 s, checked mix access, start-failure status), A1.3b
`a9077873` (mix rebuilt at the rate the output opened), A1.4a `c0a81628` (clock counts frames heard:
cpal timestamps), A1.4b `cdd0dc69` (sync probe `tests/ui/av_sync_ui.rs`: 0 frames off after 100
seeks, at 1× and 2×), A1.5 `c008e455` (soak harness `tools/fork/a1-audio-soak.sh`, docs/testing.md
§6). **Acceptance checklist (Dell, CachyOS, PipeWire 1.6.9):** 30-minute soak, 20 tracks with
overlapping clips, 78 seeks, 48 2× shuttles: **0 underruns** ✓, **drift ≤ 1 frame** (worst 1, 87 383
readings) ✓; **simulated device loss** (null sink removed / restored, card profile off / on, both
automated) recovered in 0.50–0.56 s each without a restart ✓, profile restored ✓; regression tests
per report: #336 overlapping-clip cut-outs and #617 Linux stutter → soak + `play_ahead` stall /
lead tests; #602 no audio → start-failure status, rate agreement, #840 recovery tests; #605 (Xonar
DX output) → sample-format conversion and running-format restart tests (the card itself isn't
here); #410 ("buggy audio", no repro steps) → covered only by the soak. **Owed on the PC at
acceptance:** the soak once on the PC's own output (`tools/fork/a1-audio-soak.sh 300`; it uses a null
sink and the first card's profile), a listen at 1× / 2× for pitch after a default-sink change, and
the real output latency (`perf.stats` `playback.audio`). Tag `audio-1` rides with the next group.
**A2 in progress on the Dell lane (§1a.72; scheduled task `filmcraft-dell-lane`, lock
`.dell-lane.lock`; worktree `.claude/worktrees/dell-lane`, branch `dell-lane` = hevc-vaapi, nothing
unmerged).** Audit `docs/fork/PREP-A2.md` (step map §5). Merged: A2.1 `26d2e7c5` (new L2 crate
`crates/voice`: mixed-radix FFT, DFN3 STFT / ERB / features / deep filter), A2.2 `93e0eb8b` (checked
checkpoint loader, feature `dfn`), A2.3 `e9e95f2e` (the network as plain-Rust streaming kernels +
`Denoiser`; parity with DeepFilterNet's own `deep-filter` 68.2 dB; real-time factor 0.071 on one
Dell core), A2.4 `e66e056d` (the `voice_isolate` effect: audio-dsp external-effect registry +
DeNoise fallback, `filmcraft_voice::effect::install`, project def, one render arm, guide
`docs/guide/audio.md`), A2.5a `12721bed` (`audio.voiceIsolate.status / download / cancelDownload /
installFile / remove`, pinned package, lazy loading; app + CLI feature `voice-isolation` on by
default; real download checked end to end on the Dell), A2.5b `28edd384` (Settings ▸ Audio ▸ Voice
isolation row, UI test in both workflows), A2.7 `3e635592` (test set + SI-SDR harness
`voice_isolation_eval`; **on synthetic formant speech the model fails the gate (2.1 dB on clean
speech): real speech needed, OWNER-QUESTIONS Q26 is open, recommendation (b) LibriSpeech
test-clean subset**), A2.6 `3cb31fd0` (live in the mixer: one network step per track on the
channels' mean, 0.2 s pre-roll; Dell soak 300 s: 1 Voice Isolation track 0 underruns, 4 tracks 21
in the first 2× shuttle's first 0.2 s; no offline cache needed). DeepFilterNet3 weights (pinned commit 82b0c7ad, SHA-256 in PREP-A2 §3) are on the Dell
under `~/.local/share/filmcraft-fork/models/deepfilternet3/`; the oracle is
`~/fork-oracles/target/release/deep-filter` (built from source with `RUSTC_BOOTSTRAP=1`, `time`
pinned to 0.3.36). Dell tests: `FILMCRAFT_DFN3_CKPT=<ckpt> FILMCRAFT_DFN_ORACLE=<oracle> cargo test
-p filmcraft-voice --features dfn`. **Next: A2.8** (one-click clean-up `audio.cleanupVoice` + the written manual
process + the gate; its loudness / true-peak half can be measured now, the SI-SDR half waits for
Q26), then **A2.9** (per-take review). When Q26 is answered: put 48 kHz mono WAVs in a folder on the
Dell and run `FILMCRAFT_EVAL_SPEECH=<dir> FILMCRAFT_DFN3_CKPT=<ckpt> cargo test --release -p
filmcraft-engine --features voice-isolation voice_isolation_eval -- --ignored --nocapture`.
**After A2 (lane order to confirm):** FORK-PLAN §1a.74 puts **AI3-PREP** on this lane after A2;
the lane's scheduled task still lists H1 → H2 → H3 after A2 (§1a.72). The lane will take AI3-PREP
first (the owner wants the Dell to do AI3's knowledge work "before the heavy lifting picks up"),
then H1 → H3, unless the task file or the owner says otherwise.
44.1 kHz sequences use the DeNoise fallback for now (resampling around the model is a later step). Q26 (FYI):
oracle choice, formant-voice test speech. Reviews so far self-reviews (weekly usage above pace, no
subagents); a fresh-context review of A2.1–A2.3 is owed before acceptance. **Owed on the PC at
acceptance:** live voice isolation during playback on the i5-11600K.

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, the yt-1 group (`resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`,
  `coach-1`, `yt-1`) and the **6E group on `d866cb4c` (2026-10-13): `phase-6e`, `linux-1a`,
  `linux-2a`**, **`upstream-sync-2` on `e090c0c9` (2026-10-13, U0.2 + U0.2.3b)** and **`ai-1` on
  `bcb5f713` (2026-10-14)**. Builds in `~/Apps/filmcraft-builds/`; `latest` → `ai-1`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public (§1a.33): dawgon2/filmcraft-cachyos
  branch `fork` snapshot `6e3428fc` (of `ai-1`, parent `45ea3549` = `fork-upstream-sync-2`), tag
  `fork-ai-1`, release `fork-ai-1` (scrubbed Dell build of `bcb5f713`, no `target-cpu` flag,
  `strings`: only /home/user; NOTES.md).
- **U0.2 as shipped:** one VA-API stack: upstream's H.264 / HEVC front ends on the fork's
  `va::Display` with `va_accel::ZeroCopyAccel` (zero-copy, hand-off incl. scaled, read-back mode);
  fork `va_hevc.rs`, `crates/hevc/src/accel.rs` and upstream's `vaapi/va.rs` deleted. Displays are
  kept for the process's life and `filmcraft_platform::quiesce()` runs before exit (a CLI crash at
  exit, fixed with a regression test). Exports (U0.2.3b, synthetic, full length): 4K 32.5–35.1,
  4K graded 30.5–31.1 (Auto: Intel decodes, scaled hand-off, Q20), 6.2K 25.5, 6.2K graded 19.6 (Q21).
- **6E as shipped:** two-GPU HEVC export (`crates/export/src/lanes/`: chunker, scheduler,
  coordinator with the **slow-lane rule** `retire_slow_lanes`, spill files; `hev1` when the
  drivers' headers differ), Export engines setting, encoder speed preset, 4:2:2 10-bit master on
  Intel. Synthetic: 6.2K 25.8 fps dual vs 19.4 single, 4K 33.9 vs 24.6, graded 4K 24.2 vs 22.3
  (`docs/fork-benchmarks.md` "Phase 6E"). Diagnostic env vars: `FILMCRAFT_EXPORT_TRACE=1`,
  `FILMCRAFT_EXPORT_GPU_PROBE=1` (per-frame GPU time per device), `FILMCRAFT_BENCH_ENGINES=dual|single`,
  `FILMCRAFT_BENCH_KEEP=<dir>`.
- **Schema v17**, prefs version 3. New user-visible strings need es + zh-cn rows. New guide pages go
  in `GUIDE_PAGES` (`crates/engine/src/coach_ask.rs`) and their `{ui:}` ids must be known to the
  `coach_ui` walk; dynamic automation ids (`format!`) must be listed in
  `crates/ui-egui/src/coach_ids.rs::DYNAMIC_ID_PATTERNS` (a test fails otherwise). Coach eval floor 82.
- **RS1** feature `fork-resolve-default` is on in ci: new UI tests that assume Premiere set
  `prefs.workflow = Premiere`.
- **Desktop integration is off under `--control`** unless `FILMCRAFT_DESKTOP_INTEGRATION=1`
  (MPRIS, taskbar progress, notifications, inhibit).
- **Public-snapshot hygiene:** `publish-snapshot.sh` refuses the owner's user name, e-mail name or a
  LAN address it can't rewrite (it rewrites the home path, not a `/media/…` or `/run/media/…` mount
  path, and not a name inside prose): use `user` in test fixtures and never write the owner's user
  name in docs.
- **Owed (not blocking):**
  - *U0.2:* Opus review of U0.2.3b d (per-source hand-off scale, `handoff_scales`, decode-only lane)
    and U0.2.4 / .5 (display layer, `Busy` / `quiesce`) when the usage mode allows (STEADY / LEAN);
    `teardown_ui` fails under the full parallel ci run (8 references left after 3 s) but passes alone
    (known flake since 6E: make it deterministic); hand-off scale only for MP4 / MOV sources (MKV
    etc. ignore `set_handoff_scale`); U0.2.3 owed (ii) re-grey test needs failure injection, (v)
    `va_dual_export::two_gpus_keep_the_colours_of_one` with the Intel device forced.
  - *6E:* Intel lane barely helps graded exports (retired); `Rgba16Float` effect stage on
    integrated GPUs would let it; a hung lane can still be the slow-lane comparison until the
    watchdog; (`perf.stats` lanes now report `retired` apart from `failed`, 2026-10-13);
    4:2:2 master is CPU-converted (2.7 fps; GPU → Y210 is worth a step); ~7 GB RSS for a 6-frame
    6.2K dual export in `hevc_presets`.
  - *L1 / L2 part 1, on the owner's desktop* (visual / System Settings / sudo; NOTES "Please
    check"): KWin look + icon, Program monitor 1:1 at 125 / 150 %, accent / light-dark / Animation
    speed live, window size restore, 10 min without lock, Dolphin thumbnails / double-click /
    paste, nested non-KDE compositor, `makepkg -si`. Verified here: native Wayland, MPRIS + Pause,
    portal Inhibit on play, taskbar progress + notification, journal priorities, single instance,
    X11 run.
  - *Y1:* idle zero-copy slots not trimmed / counted; AMD open-GOP-seek parity covers the I
    picture only; the fork HEVC pool may share the iHD context-lifetime hazard; Dell run of
    `tests/vaapi.rs` (U0.2.3 has a Dell run anyway).
  - *Earlier:* 6.7b TNR keyframe markers / label width; RS3 cramped Color page, Markers vs Index,
    missing panels, Fairlight strip, home-screen screenshot compare; Phase 6 frame-server proxy
    hint, `cache.claimFolder`, decode-to-screen fps with hardware 4:2:2, software 4:2:2 memory cap
    (ask first), TNR / cache / proxy 6.2K bench cases.
- **DC1 done (2026-10-13):** `fork/dell-ci/` installed on the Dell (user timer, every 2 min, only
  while the owner is logged in: Linger=no). Use: `git push ssh://dell~/Projects/filmcraft-main
  HEAD:refs/heads/dell-ci/<kind>-<id>` (kinds `ci`, `test-<crate>`, `intel` = `fork/dell-ci/intel.list`,
  `soak-<name>` = `fork/dell-ci/soaks/<name>.sh`), keep working, later
  `ssh dell 'cat ~/filmcraft-ci/results/<kind>-<id>.txt'` (3 lines; `.log` next to it, read only on FAIL).
  Self-tests: soak-selftest-fail → FAIL with its line, test-filmcraft-time → PASS. Gentle mode per
  §1a.56 (games: GPU jobs wait; MPRIS playing: 8 build jobs, soaks wait; < 3 GB free: no new job).
  `intel-1` (U0.2.3 owed iii): PASS in 3m47s.
- Cloud: CLOUD-QUEUE has C1-PREP, AI1-PREP, R1-PREP (docs only). Merge `origin/cloud-work` first
  if it has new commits.

## Exact next step: X422 (FORK-PLAN §1a.68 / §1a.69), tag `x422`

AI1 is closed (tag `ai-1`; its working notes are archived at the end of
[`PROGRESS-LOG-ARCHIVE.md`](PROGRESS-LOG-ARCHIVE.md), "HANDOFF notes of AI1"; summary: own
quantized forward pass in `crates/llm` with run-time AVX2, no build flag, streaming + prompt-prefix
KV reuse + background pre-generation of node explanations, shared `crates/ml`; `smol360-v2`
trained but not shipped, see TRAINING.md). Next per §1a.69: **X422**, then Y3 → C1 → C2 → AI2 → A2 …

X422 = the two 6.2K 4:2:2 graded export cases (read §1a.68 in full first):
**Next run starts here (2026-10-14 05:21):** design note done: [`DESIGN-X422-FUSED.md`](DESIGN-X422-FUSED.md) (F1 point-run fusion, F2 shared-memory box cascades, F3 combines / keys joined, F4 only if measured worth it; steps 2b.1–2b.4 in its §5). **2b.1 (F2) done 05:30:** `fx_box3` fuses box cascades (f32 within 1e-6, AMD + Intel), but the bench grade has no cascades (Sharpen = one 1-px box per axis, HSL without blur), so no gain there; measured per-op times in the design note §4b (Intel f32: chain 98 ms, HSL 66, Unsharp 121, one round trip ≈ 33). **2b.2 F3a done 05:32:** one-pass Sharpen-sized Unsharp (`OP_UNSHARP_DIRECT`): Intel f32 graded ops 286 → 207 ms, AMD 19.1 → 13.5 ms (synthetic, design note §5). **F3b done 05:36:** no-blur HSL as one chain + adjacent chains merged: graded ops per 6.2K frame Intel f32 286 → 178 ms, AMD 19.1 → 8.2 ms (synthetic `graded_export_ops_timing_6k`); Intel's rest is chain ALU (design note §5). **Benches done 05:42** (`docs/fork-benchmarks.md` "X422 2b"): 6.2K graded 19.7 fps (AMD decode / encode-bound), X422 (a) 17.1 → 19.9 fps, Intel lane ~4.9 fps; aims short → Q28 FYI (kept, go on). **Next: X422's final step**: median-of-2 full-length runs of (a) and (b) (binary `target/bench/bench-x422-2b` or a fresh build), `cargo xtask ci`, tag `x422` (+ riding tags `audio-1`, `linux-1b`, `linux-2b`), build per §1a.9, publish per §1a.33, HANDOFF rewrite. 2b.3 (F1) is optional and not needed for acceptance. Earlier (03:35): (b) is recorded (~7.3 fps, Q27 FYI) and AI1.4 is done.
Do **2b, the fused graded passes (§1a.71)** next: first a short design note (`docs/fork/DESIGN-X422-FUSED.md`:
which colour ops / per-pixel effects of the bench grade fuse into one full-f32 pass, how Unsharp's passes
combine, how the existing specialised shaders from U0.2.3b c are generated, parity plan AMD / Intel), then
build it in small committed steps with parity tests on both GPUs. Measure (a) and the Intel graded rate
before / after (`bench --sections xm5 --only xm5-export-422 graded`, `xm5-export-6k graded`). (a) today:
17.1 fps; its trace shows the Intel → AMD hand-off copy (~42 ms thread time per frame, Intel VPP) next to
decode on the same Intel chip, so 2b's AMD-side gain may be capped by Intel; record that honestly.
Then the final median-of-2 full-length runs of (a) and (b), ci, tag `x422` (+ the riding tags), build, publish.
**Progress (2026-10-14 02:15, scheduled run):** bench cases `xm5-export-422 graded` (a) and
`xm5-export-422 master graded` (b) added (`bench --sections xm5 --only xm5-export-422`); **(b)'s GPU
RGB → 4:2:2 done** (`YuvPass` 4:2:2 mode, `GpuCompositor::read_yuv422`, `GpuFrames::yuv422`,
read back, `frame_to_yuv422p10le` takes it as is; parity: ≤ 1 code vs the CPU oracle on AMD and Intel,
both GPUs in `a_renderer_on_each_gpu`). First numbers (synthetic, `docs/fork-benchmarks.md` "X422 first
measurements"): **(a) 17.1 fps**, **(b) 6.8 fps** (was 0.5 graded on the CPU path). AI1.4 done 03:29 (the
final X422 benches can run). 02:35: VA upload image kept per pool
(encoder input 145 → 126 ms/frame); **(b)'s limit is the read-back, 108 ms/frame serial under the
compositor lock** (benchmarks doc). 02:33: read-back pipelined (waited for outside the compositor lock):
**(b) 7.3 fps**; its limit is now the encoder thread's input, mostly Intel's `vaPutImage` (~87 ms of ~123 ms
per frame). **(b) recorded 02:56 (Q27 FYI): ~7.3 fps, limited by the
Intel UHD 750 (4:2:2 decode + hand-off + 4:2:2 encode); speed level, `vaDeriveImage`, non-blocking polling
tried without gain; ProRes 422 HQ 0.5 fps (CPU render). (b)'s final median-of-2 full-length run is owed with
(a)'s final benches (after AI1.4). Next: case (a) (overlap the hand-off copy) and 2b fused passes.**
Tried 02:37 and reverted: Intel encoding from linear Y210 memory
the AMD GPU allocated, filled by the CPU through its mapping: encodes, but wrong pictures (dead end in the
benchmarks doc). Next for (b), if pursued: `vaDeriveImage` instead of `vaCreateImage` + `vaPutImage`, or two
encoder-input threads (write surface N+1 while N encodes), then the bounded Intel levers + ProRes comparison;
else record (b) at ~7.3 fps with the OWNER-QUESTIONS FYI; for (a): overlap the Intel → AMD
copy (~42 ms thread time per frame) with decode, then 2b.
1. **(a) 6.2K 4:2:2 graded → 4:2:0 file, target ≥ 24 fps.** Intel decodes (AMD can't decode
   4:2:2) → Y210 / P210 hand-off → AMD renders + encodes. The ~104 MB frame copy must overlap
   decode (off the serial chain). Start by measuring where the time goes today with
   `FILMCRAFT_EXPORT_TRACE=1` / `FILMCRAFT_EXPORT_GPU_PROBE=1` on `xm5_422_2997_20s.mov` (or a
   full-length equivalent in `target/fixtures`); relevant code: `crates/export/src/lanes/`,
   the hand-off in `va_accel::ZeroCopyAccel` (`handoff_scales`, read-back mode).
2. **(b) 6.2K 4:2:2 graded → 4:2:2 HEVC master:** move RGB → 4:2:2 from the CPU to the GPU first
   (today 2.7 fps; `RenderMode::of` in `crates/export/src/job.rs`), then measure honestly. Only
   Intel encodes 4:2:2 (`EncSlice`, no low-power Main422_10), expect ~10–15 fps. Bounded (≈ 1–2 h)
   levers: Intel speed / quality level, two Intel encode sessions on chunks, and our CPU ProRes 422
   HQ encoder at 6.2K for comparison. Short of 24 → OWNER-QUESTIONS FYI with numbers + ProRes.
2b. **Fused graded passes (§1a.71; Q19 (b), Q21):** one full-f32 pass per clip for the colour chain
   + per-pixel effects, Unsharp's passes combined; output within the AMD / Intel parity tolerance
   of today's path. Aims: Intel 6.2K graded ~8–10 fps (from ~3.5), 6.2K graded export ≥ 24 fps
   with both GPUs, lower render time in (a). ≈ 15–25 h. Do it after measuring (a), before (a)'s
   final numbers.
3. Benchmarks need an idle machine (no builds, the owner not using the app); results synthetic in
   `docs/fork-benchmarks.md`. GPU code (WGSL RGB → 4:2:2) → GPU parity on AMD and Intel.
4. Accept: (a) ≥ 24 fps or the measured reason it can't; (b) measured and recorded; ci; tag
   `x422`; build + publish per §1a.9 / §1a.33 (the Dell's `~/fork-logs/release-build.sh` no longer
   sets `target-cpu`).

**Owed from AI1 (not blocking):** Opus review of AI1.1g–AI1.5 (own SIMD kernels, KV reuse,
streaming / prefetch concurrency in `crates/engine/src/ai_explain.rs`) when the mode allows;
real Whisper / Kokoro timing once weights are on disk; NEON kernels (Q25, provisional: Phase 8
item 7); AI1.4 SmolLM2-135M: **done 2026-10-14 03:29** (`smol135-v1`, catalogued as the "fast" option; TRAINING.md).

## Environment and gotchas

- **Disk rule (FORK-PLAN §1a.12):** keep ≥ 20 GB free on `/home`; `df -h /home` at session start
  and after each build; clean `target/debug/incremental` / `target/debug` / stale build dirs in
  the repo when needed; never delete `target/fixtures`, `target/fork-bench`, `target/bench`,
  `~/Apps/filmcraft-builds` (keep `target/release`: the MCP config runs
  `target/release/filmcraft-cli`); can't stay above 20 GB → reason in `.fork-plan-blocked`, stop.
  2026-10-08 after cleanup: 182 GB free (`target/debug` deleted; `target/` 31 GB).

- **AI explainer models (G2.11, owner-approved downloads 2026-10-11):** GGUF candidates +
  tokenizers in `~/.local/share/filmcraft-fork/models/llm/` (qwen2.5-0.5b q4_k_m, qwen2.5-1.5b
  q4_k_m, smollm2-360m q8_0, smollm2-1.7b q4_k_m; SHA-256 checked); full-precision bases for
  training in `~/.local/share/filmcraft-fork/models/llm-base/{SmolLM2-360M-Instruct,
  Qwen2.5-0.5B-Instruct}/` (pinned revision in `REVISION`). Trainer venv
  `~/Apps/filmcraft-train/.venv` (Python 3.12, torch 2.5.1+rocm6.2, transformers, peft; 18 GB);
  run it with `HSA_OVERRIDE_GFX_VERSION=10.3.0` (RX 6700 XT = gfx1031; ~6 TFLOPS bf16, 12 GB).
  Tuned models are published as releases on the public dawgon2/filmcraft-cachyos only after the
  owner says "publish" (OWNER-QUESTIONS Q7).
- **Rust** 1.99 via rustup in `~/.cargo` (not pacman): `export PATH="$HOME/.cargo/bin:$PATH"`.
  `bindgen-cli` 0.73.2 is installed there (user-level) for regenerating `va_sys.rs`.
- **Bench `--only`:** the export section is gated by `wants("export")` (substring of the case
  name in the filter, not the other way round): use `--only export` (all three cases).
- **Shell is zsh**: unmatched globs are errors (quote `--include='*.rs'`); `set -- $var` does
  not word-split (use `bash -c '…'`); no `bc` (use python3). `pgrep -f` / `pkill -f` match
  their own command line (`pkill -f gates-2.2b` killed the tool's shell): use `pgrep -x cargo`,
  `pkill -x <name>`; killing cargo leaves orphaned `rustc` processes (`pkill -x rustc`).
- **Foreground commands time out at 10 min.** Run long work detached
  (`(cmd > log 2>&1; echo "exit $?" >> log) &; disown`) and wait with
  `timeout 590 bash -c 'until grep -q "^exit" log; do sleep 15; done'`. Never chain `sleep`s.
- **Dell lane (FORK-PLAN §1a.32):** `ssh dell` (fish shell: send `bash -s` scripts) is a second
  machine with an Intel UHD 770, set up and green (full build 392 s, tests 380 s). Use it for
  Intel-path tests (6.1b 4:2:2 parity first), long tests / soaks and weaker-PC checks; sync
  with `git push ssh://dell~/Projects/filmcraft-main hevc-vaapi`.
- **GPUs:** AMD RX 6700 XT = `/dev/dri/renderD129`, Vulkan GPU0; Intel UHD 750 = `renderD128`,
  Vulkan GPU1. Force one for wgpu tests: `MESA_VK_DEVICE_SELECT=1002:73df` (AMD) or `8086:4c8a`
  (Intel) plus `MESA_VK_DEVICE_SELECT_FORCE_DEFAULT_DEVICE=1`. Run GPU parity tests on both.
- **GPU stall with the video engine flat out (session 4):** decoding pictures back to back on
  VCN made compositing wait 40–100 ms per frame (`device.poll`); spread decoding did not.
  Reverse playback paces VA submissions (`cancel::with_pace`). Not caused by DMA-BUF import,
  surface churn or VA queue depth (all measured). Watch for it wherever a burst of decoding
  happens while playing (proxies, cache rendering in Phase 6).
- **Driver quirks found:** Mesa's VA-API wants HEVC scaling lists in raster order of each 4×4 / 8×8
  matrix (not the coded diagonal order the VA header suggests); RADV/Intel interpolate 1:1
  sample positions slightly off texel centres (fixed by snapping in `composite.wgsl`); ffmpeg's
  MOV muxer writes `hvcC numTemporalLayers = 6` for single-layer x265 streams (use the SPS);
  libva reuses surface ids after destroy (key caches by our own memory id).
- **Bench:** `--only "jkl J 1x"` runs one JKL case; `L 4x warm` / `J 2x warm` play the speed
  elsewhere first (remembered step); output has `steps` and cadence (`gap_ms`).
- **Env vars:** `FILMCRAFT_VAAPI_DEVICE` (render node), `FILMCRAFT_FFMPEG`,
  `FILMCRAFT_NO_GPU_IMPORT` (app/bench: no Vulkan DMA-BUF import → read-back path),
  `FILMCRAFT_VRAM_CACHE_MB` (decode-cache VRAM budget), `FILMCRAFT_SOAK_MINUTES` (bench soak),
  `FILMCRAFT_BENCH_TRACE=1` (bench lists frames that finished after they were due),
  `FILMCRAFT_BENCH_KEEP=<dir>` (bench keeps each export's output), `FILMCRAFT_NO_VA_ENCODE`
  (export through ffmpeg), `FILMCRAFT_FULL_SIZE_GRADE` (grade converted media at full size),
  `FILMCRAFT_XM5_DIR` (where the xm5 clips live; real clips can be linked there by name),
  `FILMCRAFT_NO_ZERO_COPY` (export reads frames back instead of rendering into encoder surfaces),
  `FILMCRAFT_EXPORT_TRACE` (per-batch export stage times on stderr).
- **Worktree lanes (session 10):** leaf builders in `.claude/worktrees/agent-*` with
  `CARGO_TARGET_DIR=target/agent-<name>` (~5 GB each); remove the worktree after merging.
- **Disk:** see the disk rule above; a debug build of the workspace regrows `target/debug` to ~60 GB. Fixtures: `target/fixtures/xm5/` (12 GB synthetic), `target/fixtures/hevc/`
  (conformance, `.hevc` + ffmpeg `.yuv`), `target/fixtures/platform/`, `target/fixtures/hevc4k/`.
- **Bench:** `target/release/examples/bench --section xm5 [--only CASE] [--quick] [--json f]`;
  cases ceilings, decode, play-full, play-graded, jkl, scrub, multicut, export, 422-play, and only
  when named: soak, open. Output line includes VA surfaces, cache VRAM and GOP seeks / decoded /
  skipped / evicted. Full suite ≈ 75 min (graded 4K export alone ≈ 40 min). Saved binaries in
  `target/fork-bench/bench-phase{0,1,2}`, results in `target/bench/xm5-phase*.json`.
- **Benchmark pitfalls hit:** run on an idle machine (an x265 encode, a build or orphaned
  `rustc`s inflate everything; check `uptime` and `ps`); never run benchmarks during the soak
  (they share the decode engine); `--quick` multicut covers only 3 cuts near GOP starts, the full
  60 s run covers 20 cuts deeper into the clips (much worse); bench plays used to leak a
  `FrameServer` each (fixed) — results of a case can depend on the cases before it in one process;
  the synthetic clips are x265 `ultrafast` open-GOP (CRA + RASL), keyint 30, ~49 % disposable.
- **CI:** `cargo xtask ci` ≈ 22 min since session 13 (nextest, `[profile.ci]`, one UI test binary `crates/ui-egui/tests/ui/main.rs`: a new UI test file needs a `mod` line there; prints per-stage times); run idle — `mixer_tests::perf_24_tracks_3_effects_realtime_factor`
  needs ≥ 2.5× real time on one core (4.7× idle). `cargo test --workspace --release --no-fail-fast`
  shows every failure.
- **App smoke test:** `filmcraft --data-dir <scratch> --control <port> --empty`, JSON lines
  `{"id":1,"method":"engine.execute","params":{"command":"file.import","params":{"paths":[…]}}}`,
  `file.newSequenceFromClip` `{"items":[id]}`, `ui.playback` `{"action":"play"}`, `perf.stats`,
  `ui.screenshot` `{"path":…}`, `app.quit`. Playback resolution: `ui.set {"playbackRes":"Full"}`
  (serde wants variant names: `{"program":{"res":"Full"}}` works, `"full"` is refused). JKL:
  `engine.execute` `playback.forward` / `playback.reverse` (each call doubles the speed while
  playing), `playback.stop`; shown / dropped in `ui.inspect` → `playback`. Script used in
  session 4: a Python JSON-lines client (see FORK-PLAN §7, Phase 3.3).
- **Silent audio tests:** cpal's PulseAudio client ignores `PULSE_SINK` and sets no
  `application.process.id`; a long test once played on the owner's speakers. Find the stream by
  `application.name` (check it first) before moving it to a null sink, or mute it another way.
- **Permission classifier:** the agent cannot set up self-restarting automation (`/loop`, timers
  that start `claude`); the owner does that (the runner script).

## Dead ends (with measurements, all synthetic)

- JKL (Phase 3.2): more spare VA surfaces (48 instead of 8) against the reverse GPU stall: no
  change; limiting VA submissions in flight (1, 2, 3, 64): 12–44 drops of 300, no fix (pacing
  the rate did it); a skimming tolerance of a quarter step (instead of half): grid frames deep
  in a GOP cost every reference before them and 4× climbed to step 24–30.

- ffmpeg pipe tuning (Phase 1): a 1 MiB pipe buffer did not help (22.2 vs 21.8 fps); ffmpeg's own
  download + write is the cap.
- Vulkan Video as primary: 5 % slower than VA-API in ffmpeg (40.9 vs 43.2 fps), frames identical.
- Multicut / cuts (Phase 2–3): requesting the first frame after the next cut early (20–90 frames
  ahead, with or without the priority gate), a deeper prefetch window (20–30 frames), a per-clip
  floor of 16 frames, waiting for every picture inside the priority gate (−6 % throughput):
  all made cut drops worse or did nothing. What helped: the open-GOP seek fix (correctness) and
  skipping non-reference pictures before a clip's in-point (~98 → 25–42 drops of 360).
- VRAM: Auto budget of half the VRAM ran a 12 GB card out of memory with four 6.2K clips; a
  quarter is the default. The monitor frame cache charging GPU frames their full size left room
  for ~9 plans (fewer than the prefetch window); it charges 16 MB per GPU frame.

## Open problems

- 6.2K cuts every few seconds drop frames on original media (decoder-bound) → Phase 6.
- 4× on this synthetic stream (51 % reference pictures, open GOP) runs at the decoder's limit
  even at step 12–15: steady skipping at ~8–10 pictures/s, not more.
- 4:2:2 HEVC does not decode at all (no decoder on Linux) → Phase 6.1.
- Graded 4K export 27.5 fps (just under real time): one VCN shared by the 6.2K decode and the
  encode; a possible 6.2K optimisation phase later (FORK-PLAN §1a.16, only with the owner).
- First play after loading drops frames for ~0.5–1 s (cold start) → Phase 7.
- Fujifilm's F-Log2 LUT changes gamut on the log signal; built-in is linear light (FORK-PLAN §7).
- Grade node ids are per grade (`Grade::next_id`); the current node is clip-scoped
  (`EditorState.grade_node: Option<GradeSel>`).
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T) — not fixed.
