# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–70 incl. §1a.6 order; §5 phases; only the last ~80 lines of §7), then
[`AI-PLAN.md`](../../AI-PLAN.md) (phase AI1, next) and `TRAINING.md` where AI1.6 needs it, then
[`FORK-ONLY.md`](FORK-ONLY.md), [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), this file. Other plan
files (GRADE, RESOLVE-UI, YOUTUBE, AUDIO, AI, COMPOSITOR, STORAGE, H264, RAW, DECODER) only when a
step points at them. Rewritten 2026-10-13 21:00 after the U0.2 acceptance (scheduled run).

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → Y1 ✓ → 6E ✓ → U0.2 ✓ → **AI1** →
**X422** → Y3 → C1 → C2 → AI2 → A2 → S1 → S2 → G3 → 7 (+ Y4) → Y2 → H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 →
R2 → D → 8. Small phases are accepted in groups (§1a.41). AI1 moved right after U0.2 and the
4:2:2 export cases (X422) right after AI1 by the owner 2026-10-13 (§1a.69).
**A1 (Dell lane, §1a.60) is ready for acceptance** (below); its tag `audio-1` rides with the next phase group.
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
**A1 ready for acceptance (Dell lane, 2026-10-14 00:55; AUDIO-PLAN Phase A1, §1a.59–60).** Steps merged:
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
`.dell-a1-complete` exists: the Dell lane's scheduled task has nothing left to do (the owner may
turn it off). Worktree
`.claude/worktrees/dell-lane`, branch `dell-lane` (= hevc-vaapi, nothing unmerged).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, the yt-1 group (`resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`,
  `coach-1`, `yt-1`) and the **6E group on `d866cb4c` (2026-10-13): `phase-6e`, `linux-1a`,
  `linux-2a`**, and **`upstream-sync-2` on `e090c0c9` (2026-10-13, U0.2 + U0.2.3b)**. Builds in
  `~/Apps/filmcraft-builds/`; `latest` → `upstream-sync-2`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public (§1a.33): dawgon2/filmcraft-cachyos
  branch `fork` snapshot `45ea3549` (of `f7a04384` = the tag plus a docs name scrub; parent
  `94e88700`), tag `fork-upstream-sync-2`, release `fork-upstream-sync-2` (scrubbed Dell build of
  `e090c0c9`, NOTES.md).
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

## Exact next step: AI1 (AI-PLAN.md, portable fast inference), step AI1.1

U0.2 is closed (tag `upstream-sync-2`; its working notes are archived at the end of
[`PROGRESS-LOG-ARCHIVE.md`](PROGRESS-LOG-ARCHIVE.md), "HANDOFF notes of U0.2"). Next phase per
§1a.6 / §1a.69: **AI1** (≈ 8–15 h), then **X422**, then Y3.

1. **AI1-PREP done** (2026-10-13): read [`PREP-AI1.md`](PREP-AI1.md) first. Key finding: candle's
   quantized matmul never uses `gemm` (batch-1 decode is a per-row quantized `vec_dot`), so the hot
   path is our own Q8_0 / Q4_K / Q6_K dots with run-time dispatch through `pulp` (already in
   Cargo.lock via gemm); `gemm` only for prefill f32 attention. Start by measuring one Q8_0 row loop
   (pulp, no flag) against candle-with-flag before porting the rest.
2. **AI1.1a done** (`crates/llm/src/quant.rs`, 2026-10-13): Q8_0 parse / quantize / dequantize /
   dot / matvec in safe Rust, dispatched by `pulp` through `WithSimd` structs with `#[inline(always)]`
   (a plain closure is *not* vectorized: measured no gain), `SimdLevel::{Auto, Baseline}` +
   `FILMCRAFT_LLM_SIMD=baseline` override; all levels bit-identical (tested). Synthetic, 960 × 2560
   matvec, 1 thread: built **without** the flag ours 147 µs (baseline 202), candle 1 342 µs; with the
   flag candle 110 µs. **AI1.1b done:** rayon row split
   (64 rows / task, bit-identical to one thread); 12 threads: no-flag ours 41.8 µs, candle no-flag 332,
   candle with flag 41.7 → parity reached when threaded (a 4-rows-per-pass kernel gave nothing;
   the 1-thread 1.34× gap is the i8 multiply shape, candle's `vpmaddubsw`; optional later).
   **AI1.1c done** (`crates/llm/src/kquant.rs`): Q4_K / Q6_K parse (6-bit scales unpacked at load,
   weights stay packed), dequantize (bit-equal to candle's on random blocks), Q8_K quantize, dots /
   threaded matvecs via a zero-sized `Kernel` type (a `fn` pointer blocked inlining: measured).
   Synthetic 1024 × 2560, 12 threads, no flag: Q4_K 44.5 µs / Q6_K 52.4 µs vs candle no-flag 139 / 149,
   candle with flag 31 / 37 (we are ~1.4× behind it; optional tuning later).
   **AI1.1d-1 done:** `gguf.rs` now keeps the tensor directory (`tensor_infos`, `tensor()`) and
   finite float metadata (`floats`, `float()`); `weights.rs` `Weight::read` (F32 / F16 / Q8_0 / Q4_K /
   Q6_K, dims checked, NaN F32 refused) with `row`, `vector`, `matvec` + `Scratch` (deliberately no
   activation cache: a pointer-keyed cache goes stale on in-place buffers; quantizing is ~1 %).
   **AI1.1d-2 done:** `forward.rs` (`Config::from_gguf`, `Forward::load / forward(&[u32], pos) /
   reset`): both families, matches candle on a synthetic tiny GGUF (both archs) and on the real
   SmolLM2-360M tune (steps 0–1 bit-identical, later steps same top token, ≤ 8 %: attention-sum
   order × 8-bit activation rounding). Synthetic timing, 10 tokens: **20 ms / token without the
   flag** (candle no-flag 74, flagged 36, both incl. load). Prefill is still token-by-token.
   **AI1.1e done:** Q5_0 widened exactly to Q8_0 at load (`quant::parse_q5_0_as_q8_0`); K-quant
   kernels now keep the reference's 8-lane float order, `rms_norm` divides and attention divides by
   √d as candle does → **step 0 bit-identical to candle's scalar path on all six local models**
   (Q8_0 models and Qwen 0.5B Q4_K_M even through step 1; later steps differ by attention-sum order
   × 8-bit rounding, worst SmolLM2-1.7B ~15 %; the fact checker is the gate). Synthetic, 10 tokens,
   no flag: 20–26 ms / token (360M / 0.5B), 69–76 ms (1.5B / 1.7B), all faster than candle no-flag.
   Real-model check: `FILMCRAFT_LLM_GGUF=<file> RUSTFLAGS="-C target-cpu=x86-64" … test --release
   -p filmcraft-llm --features infer --lib real_model -- --ignored --nocapture`.
   **Owed (optional, AI1.1g):** the exact-order Q4_K kernel vectorizes badly with AVX2 (1 thread
   786 µs vs 228 for the old inexact one; 12 threads 120 vs 44.5 µs, candle flagged 31); only the
   1.5B / 1.7B options lean on Q4_K.
   **AI1.1f-1 done:** one generic row kernel (`quant::rows_at<K: Kernel>`, weights × t activation
   rows; matvec = t 1) + `matmul_q8_0 / _q4k / _q6k`, `Weight::matmul`, batched `Forward::chunk`
   (64-token prompt chunks, attention parallel over tokens), all bit-identical to token-by-token
   (tested across a chunk boundary). Synthetic 300-token prefill, SmolLM2-360M tune: **ours 1.86 s
   without the flag** vs candle 15.9 s no-flag / 2.75 s flagged. Prefill is now compute-bound on
   the Q8_0 dot (~17 GMAC/s per thread; `vpmaddubsw`-shaped kernel would be the next big win,
   AI1.1g with the Q4_K item).
   **AI1.1f-2 done:** `sample.rs`: greedy (first max, = candle's argmax) and repeat penalty equal
   to candle's (tested against it); temperature sampling with our own seeded splitmix64 (not
   candle's random sequence; greedy is the default and the evaluated setting).
   **AI1.1h done:** `infer.rs` now runs on `Forward` + `Sampler` (public API unchanged; candle is a
   dev-dependency: test oracle + `llm_quantize`). Real models, no flag (`tests/model.rs`, synthetic
   prompts): tuned smol360-v1 **1.57 s**, qwen05-v1 1.58 s, stock 0.5B 1.90 s per answer (target
   ≤ 2.5 s); engine `ai_explain` tests green.
   **AI1.1g-1 done (Q8_0 on AVX2):** `Kernel::dot_v3` override + `dot_q8_0_v3` (sign trick,
   `vpmaddubsw` / `vpmaddwd`, f32 lanes across blocks) via pulp's safe V3 intrinsics + bytemuck
   casts, chosen at run time (`V3::try_new`, off under `FILMCRAFT_LLM_SIMD=baseline`). Synthetic
   960 × 2560: 1 thread **91.5 µs** (portable 205, candle flagged 110), 12 threads **26 µs**
   (candle flagged 41.7). 958-token prefill on the 360M tune 8.3 → **5.2 s**. Exactness vs candle
   now only at Baseline (tests adjusted). **AI1.1g-2 done:** attention `dot` in 8 lanes and a
   4-token register-blocked `dot4_v3` (bit-identical per lane): 958-token prefill 5.2 → **4.2 s**
   (profile before: MLP + wo 2.9 s, qkv 0.8 s, attention 1.0 s; candle no-flag 54.7 s). Remaining:
   **AI1.1g-3 done (Q4_K / Q6_K on AVX2):** Q4_K straight `vpmaddubsw` on the unsigned nibbles;
   Q6_K on the unsigned 0..63 values with the −32 offset through `bsums` (the sign trick would
   overflow: Q8_K activations reach −128). Synthetic 1024 × 2560, 12 threads: Q4_K **24.3 µs**,
   Q6_K **31.7 µs** (candle flagged 31 / 37). Qwen2.5-1.5B 958-token prefill **50 → 11.9 s**
   (candle flagged 17 s), decode 69 → 45 ms / token; SmolLM2-1.7B top token 10 / 10 steps vs
   candle (real-model bound now 20 %). Tests: portable loop bit-identical to candle's scalar path,
   AVX2 within 1e-5; the global SIMD override is no longer touched by tests (it raced with
   `FILMCRAFT_LLM_SIMD=baseline` runs).
   The AI1.1g regression is closed. Optional later: `dot4_v3` for K-quants, AVX-512 / NEON paths
   (the portable loop covers them today).
   **Held-out evaluation on our pass done** (`runs/ai1-own-pass/`, no flag, synthetic): qwen05-v1
   74/75 (identical), smol360-v1 70/75 (recorded 72; set 2 flips 36 / 37 / 38 with summation
   order on threshold cases) at **1.62 s mean per answer** (was ~3 s): OWNER-QUESTIONS **Q22**,
   provisional (a) accept within ±2. This CPU also has AVX-512 (a later V4 path is possible).
   **AI1.2 done (2026-10-13):** the `x86-64-v3` block is gone (Q8 resolved). No Whisper / Kokoro
   weights are on this PC, so their candle f32 ops were timed synthetically
   (`crates/speech/examples/candle_ops_bench.rs`, with vs without the flag): gemm-backed ops equal;
   candle's `conv_transpose1d` (6.7 → 45 ms) and last-dim mean (0.2 → 2 ms) regress without it, so
   Kokoro's `ConvT` (groups 1) and `norm_last` now use gemm (`layers::conv_transpose1d_gemm`,
   `mean_last`: 8.0 / 0.09 ms, tested against candle). Owed: time one real transcription / TTS line
   when weights are on disk (no download without the owner's yes).
   **AI1.3a done (crates/llm):** `Model::generate_streaming(system, user, opts, &mut |so_far| bool)`
   (whole answer so far, half-UTF-8 held back; `false` → `StopReason::Cancelled`; panicking
   callback → `Panicked`), and **prompt-prefix KV reuse** (`Forward::truncate`, `Model.cached`; any
   error / panic forgets the cache; answers identical, tested). Synthetic, real models, only the
   system prompt cached: **first words 0.29–0.31 s** on the 0.5B / 360M models (was ~0.6 s; 1.5B
   0.9 s, 1.7B 1.15 s).
   **AI1.3b done (engine + UI):** `ExplainBackend::generate_streaming` (default: no streaming);
   the job publishes each longer run of **complete sentences that pass the fact check** as
   `Answer::Pending { partial }` (`grade.explainNode` / `ai.explanation` JSON field `partial`);
   unchecked text is never shown, the final answer is still checked whole (a later wrong sentence
   → template). Generation stops when nobody waits (timeout / forgotten). Match tab shows the
   partial. Guide `docs/guide/color/matching.md` updated.
   **AI1.3c done:** background pre-generation: `AiExplain::prefetch` / `can_prefetch`,
   `grade.explainNode {"prefetch": true}`; speculative pending entries, a newer prefetch drops
   older un-asked ones (queued skip, running stop via `wanted()`), an explicit request adopts a
   pending one; the grade panel fires it 0.6 s after the last edit / node change, not while
   playing (`prefetch_explanation`, debounce in egui temp data keyed on revision, clip, node).
   Tested in engine (`a_newer_prefetch_supersedes_and_a_request_adopts`); UI path not
   screenshot-checked (MINIMAL).
   **AI1.4 approved (owner, 2026-10-14, Q24 (a)):** download SmolLM2-135M-Instruct and do AI1.4
   (keep the "fast" tune only if it passes; doesn't block `ai-1`).
   **AI1.5 done:** new L1 crate `crates/ml` (`filmcraft-ml`): `simd::level()`, `job::{guarded,
   Cancel, JobError, panic_message}`, feature `candle` → `ops::{conv_transpose1d, mean_last}`
   (moved from Kokoro; tts's `kokoro` feature enables it). README says how to time a new
   model's candle ops. A2 / G3 build on it.
   **AI1.6 in progress:** `explain_traindata --subtle`, held-out set 3 (`eval_cases_v3`,
   `--set 3`), data-v2 and the training run are described in TRAINING.md "smol360-v2"
   (COACH-QUESTIONS has no entries yet; 6.7b TNR is clip-level, not something the node
   explainer describes). Started 23:02 EDT: `~/Apps/filmcraft-train/runs/smol360-v2/`
   (`session.log`, `train.log`); weights → `models/tuned/smol360-v2/`. If a run died, rerun
   with `STAGES="quantize eval score"` (or `train …` if no `model.safetensors`). Then evaluate
   v1 and v2 on all sets in one `llm_eval` run (`explain_score --set all --dump-cases`,
   `--dedupe <data-v2>/train.jsonl`), and if v2 ≥ v1 on sets 1 and 2 and passes set 3: add it to
   the catalogue (`crates/llm/src/models.rs`, `TUNED_PREFERENCE` first) and to the engine tests.
   Then acceptance (both held-out sets, ≤ 2.5 s mean,
   first words < 0.5 s ✓ 0.3 s, no crash without / with a corrupt model) → tag `ai-1`.
   **AI1.1 (rest):** own quantized forward pass in `crates/llm` (from candle's MIT / Apache-2.0 code,
   attribution kept), heavy maths through `gemm` (run-time AVX-512 / AVX2 / SSE / NEON dispatch), no
   `unsafe` in our code; results identical to the old path within the fact checker; a forced-SSE
   override for tests.
3. AI1.2 delete the `x86-64-v3` block in `.cargo/config.toml` (resolve Q8); AI1.3 streaming +
   background pre-generation; AI1.4 SmolLM2-135M "fast" option (keep only if it passes); AI1.5
   shared runtime for A2 / G3; AI1.6 retrain `smol360-v2` on the RX 6700 XT (~1 h per run).
   Accept (AI-PLAN): same scores on both held-out sets (75 cases), ≤ 2.5 s mean answer, first words
   < 0.5 s, no crash without / with a corrupt model; tag `ai-1`.
4. **Model downloads:** AI1 uses the models already on disk / published; any new download needs the
   owner's yes (memory `fork-owner-questions`).

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
