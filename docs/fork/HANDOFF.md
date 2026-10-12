# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
incl. §1a.6 order, §1a.16 6E, §1a.17 grader, §1a.18 YouTube gaps, §1a.19–24; §5 phases; §7
progress log), then [`GRADE-PLAN.md`](../../GRADE-PLAN.md), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md),
[`EXPORT-PLAN.md`](../../EXPORT-PLAN.md), [`H264-PLAN.md`](../../H264-PLAN.md),
[`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md), [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md),
[`DECODER-PLAN.md`](../../DECODER-PLAN.md), [`docs/guide/README.md`](../guide/README.md),
`docs/fork/OWNER-QUESTIONS.md` (Q8 open: x86-64-v3 build flag, provisional), then this file.
Rewritten 2026-10-11 22:45 (live session: G2b done, tagged `grade-2b`, built). Phase order
(FORK-PLAN §1a.6, owner): G0 ✓ → G1 ✓ → U0 ✓ → G2 ✓ → G2b ✓ → **6** (incl. 6.1; temporal NR
is Phase 6 item 7) → G2c (WB picker, §1a.38) → Y1 → 6E → U0.2 → A1 → Y3 → C1 → C2 → AI1 → A2 → G3 → 7 (+ Y4) → Y2 → H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 → R2 → D → 8 (A / G3 / G4 / C3–C4 / R added by the owner 2026-10-12, §1a.35–36, [`AUDIO-PLAN.md`](../../AUDIO-PLAN.md), [`RAW-PLAN.md`](../../RAW-PLAN.md), [`AI-PLAN.md`](../../AI-PLAN.md) (AI1, §1a.37); fork-only code rule §1a.36, [`FORK-ONLY.md`](FORK-ONLY.md))
(corrected and revised by the owner 2026-10-12, FORK-PLAN §1a.6 / §1a.34: D after H3, Phase 8
deferred to the end). **Publish to `cachyos` after every phase (§1a.33).**

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` … `phase-5`, `grade-0`,
  `grade-1`, `upstream-sync-1`, `grade-2`, `grade-2b`. Builds in `~/Apps/filmcraft-builds/`
  (… `grade-2`, `grade-2b`); `latest` → `grade-2b`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Bench binaries
  `target/fork-bench/bench-{phase0,1,2,4,5,grade0,grade1,u0,grade2}`.
- **Build flag:** `.cargo/config.toml` builds x86-64 Linux for `x86-64-v3` (AVX2; candle's
  quantized kernels need it at compile time; OWNER-QUESTIONS Q8, provisional). Float results
  unchanged; all target dirs were rebuilt once.
- **G2b done** (live session 2026-10-11, FORK-PLAN §7):
  - `crates/llm` (filmcraft-llm, L2): candle GGUF inference (qwen2 / llama), GGUF
    pre-validator, guarded `generate`, `compact_facts` prompt, catalogue (4 stock Apache-2.0
    models + 2 local FilmCraft-tuned entries, `is_local()`, `recommended_options()` = repeat
    penalty 1.0 for tunes), examples `llm_eval`, `llm_quantize` (safetensors → GGUF).
  - `crates/render/src/grade_explain/{check.rs,grammar.ebnf,cases.rs}`: the fact checker is a
    strict controlled grammar (`GRAMMAR_DOC`, `VOCABULARY`, `check_explanation_labeled`,
    `check_grammar`), 36 held-out cases (`eval_cases`), 101 adversarial must-fail rows; examples
    `explain_traindata` (training set) and `explain_score`.
  - `crates/engine/src/ai_explain.rs`: optional AI explanations (`ExplainBackend`, cached
    model, checker gate, fixed reason codes, `ai.*` commands, `grade.explainNode` + `aiKey`),
    prefs `color.ai.{enabled (off), model ("" = best installed tune, else DEFAULT_MODEL)}`,
    Settings ▸ Color ▸ AI explanations. Engine features `llm` / `llm-download` (apps default
    `llm`).
  - Grading Coach: `crates/engine/src/{coach.rs, coach_ask.rs, coach_hints.rs}`,
    `assets/lessons/*.toml` (7), `assets/coach/{hints.toml, questions.toml}`,
    `crates/ui-egui/src/{panels/coach.rs, coach_ids.rs}`; Help ▸ Grading Lessons…; guide
    `docs/guide/color/coach.md` (every `{ui:}` id there and in the whole guide is checked by
    `coach_ui` walk tests; new guide pages must be added to `GUIDE_PAGES`).
  - Training (owner Q6/Q7): `tools/explainer-train/` + `docs/fork/TRAINING.md`; venv
    `~/Apps/filmcraft-train/.venv` (torch ROCm, `HSA_OVERRIDE_GFX_VERSION=10.3.0`); tuned
    models in `~/.local/share/filmcraft-fork/models/tuned/` and hard-linked into
    `models/<id>/`. Tuned Qwen 0.5B Q8_0: 35/36 checker passes at 2.4 s (best stock 5/36).
    Publishing to dawgon2/filmcraft-cachyos releases only when the owner says "publish".
    Release folders `tuned/{qwen05-v1,smol360-v1}/` are ready (FORK-PLAN §1a.26 done 2026-10-11):
    `NOTICE`, `LICENSE`, path-free `manifest.json` (`manifest.private.json` keeps the local
    paths; never publish it).
    **Owner 2026-10-12 (FORK-PLAN §1a.27): SmolLM2 is the explainer base going forward;
    `qwen05-v1` is frozen (kept, not developed or published). Second held-out set: smol360-v1
    72/75, qwen05-v1 74/75 (TRAINING.md).**
- Acceptance (GRADE-PLAN "Phase G2b"): AI text passes the fact check on the held-out set with
  the tuned model (35/36; the miss falls back) and falls back cleanly without a model; every
  lesson completes end to end through the control channel (coach_ui); 30 beginner questions pass
  the control-name check (29/30 top-1 section, 8/8 off-topic fall back). `cargo xtask ci` green
  (3215 tests, 39 min after the full rebuild).
- Follow-ups noted, not done: lesson / hint / AI texts English only; a second held-out set on
  another frame / seed; own quantized forward pass with gemm prefill (≈ 2 s); pure-Rust candle
  LoRA trainer; `explainGrade` doesn't use the AI path yet; earlier G2 list (Match tab renders on
  the UI thread, node strip summary, mark pair intended, identical-grade badge, preview cost
  ignores the grade, linear masks in older builds, mask tracking key time, effects.setParam
  keyframe time, stills gallery UI-thread renders, app-wide grade keys, Gallery empty-state
  clipping, `cropped()` read-back, two VA displays, HDR→Main 10 note).
- Schema unchanged (v16). New user-visible strings need es + zh-cn entries.
- Cloud: merged up to `55f8639c` (2026-10-12 02:35: EXPORT-UI-1, KEYMAP-1, REVIEW-1, GUIDE-4/5/6;
  tested on PC). Owed: export-page screenshot with an HDR sequence. Merge `origin/cloud-work`
  first if it has new commits (CLOUD-QUEUE: GUIDE-7/8, COACH-WALK-1, Y1-PREP, I18N-2).

## Exact next step: FORK-PLAN Phase 6, step 6.1 (merge the 4:2:2 decoder lane), then 6.2 / 6.3

**New, owner 2026-10-12 (FORK-PLAN §1a.39): [`RESOLVE-UI-PLAN.md`](../../RESOLVE-UI-PLAN.md).**
Resolve style becomes the fork's default (Premiere stays a full option) and the app opens on a
Resolve-style **Project Manager home screen** (pick a project to go into), with Resolve terminology
(Sequence → Timeline, …; RS2b). Start RS1 → RS2 → RS2b as a
leaf-builder lane next to Phase 6 when a lane slot is free (UI / engine only); RS3 later.
References: `plan/resolve/screenshots/` (local only, never commit).

Temporal NR is Phase 6 item 7, after the render cache 6.3. Design notes for 6.1–6.3:
[`PHASE6-DESIGN.md`](PHASE6-DESIGN.md).

Order (scheduled-task prompt §2, FORK-PLAN §5 Phase 6):
1. ~~6.5~~ **done 2026-10-12** (FORK-PLAN §7): no code regression (noise sd 31 + Mesa 26.2.4
   ~10 %); cut-spacing cases `xm5-cutspacing 2s|3s|4s|5s` + per-cut `missed` accounting; root
   cause of cut drops fixed in `gop.rs` (multicut ~258 → 63–70 dropped, synthetic). Compare
   multicut numbers only as means of ≥ 2–3 runs.
2. **6.1** part 1 **merged 2026-10-12** (`92054234`, 4:2:2 decode bit-exact, ~14 fps at 6.2K).
   Owed: renderer check of C422 frames (`--only 422-play`), the "4:2:2: software decode, proxies
   recommended" badge (lane B, `media_badge` in `panels/project_views.rs`), 4:2:2 proxies (6.2).
   Lane A of 6.2 / 6.3 **merged 2026-10-12** (background worker `engine/src/background.rs`,
   preview hash v2, `HEVC_HALF`; see FORK-PLAN §7). At 02:35 two Opus lanes started in
   worktrees: **lane B** (auto proxies, `media.*` proxy commands, 4:2:2 badge, Settings ▸ Media,
   guide `proxies.md`, bench `… proxies` cases) and **lane C** (render cache on the worker,
   `cache.*` commands, budget / eviction, Settings ▸ Media Cache, guide `render-cache.md`, bench
   `… cached` + `xm5-cache-while-playing`). If a run died with them unmerged: their branches are
   `worktree-agent-*` under `.claude/worktrees/`; review (Opus in STEADY) and merge one at a
   time, or note and remove them. Also owed: the frame-server hook for "proxies while playing,
   originals when paused" if lane B couldn't do it within its files; 4:2:2 software-decode
   memory cap (RSS ~8 GB in `xm5-422-play`).
   **02:23: the run died (systemd-oomd killed the Claude app; §1a.31).** Lanes B and C were
   not merged: lane B (auto proxies) is all uncommitted in `.claude/worktrees/agent-acb14297…`,
   lane C (render cache) has `f5bfc6a3` plus uncommitted UI / settings / guide changes in
   `.claude/worktrees/agent-ae7b714f…`. Check each builds and passes its crate tests (through
   `fc-cargo.sh`, one UI-crate run at a time), review, commit in the worktree, merge one at a time.
   **03:28 (scheduled run): lanes B (6.2, `2798066e`) and C (6.3, `cd0bb40b`) merged, review
   fixes in, tests green (FORK-PLAN §7). 6.1b runs as an Opus lane: edits in a
   `.claude/worktrees/agent-*` worktree here, builds / Intel tests on the Dell (owner request:
   branch `lane-6-1b` pushed to the Dell, worktree `~/Projects/fc-6-1b` there). If a run dies:
   check the lane's last commit here and on the Dell, review (Opus: FFI), merge.**
   **05:15 state: two unmerged lanes, both committed in `.claude/worktrees/agent-*`:**
   (a) **DMA-BUF hand-off** `worktree-agent-a97b975e5d7c97018` (`447a7ee8`; Opus review: approve
   after fixes — HOST_COHERENT non-device-local memory type, external-buffer query, process-wide
   "4:2:2 import refused" flag, mmap read-back to drop the triple copy, pool stress test; the
   lane is applying them). Timings (synthetic): decode→GPU frame 44–47 ms hand-off vs 103–146 ms
   read-back. (b) **6.7 lane B** `worktree-agent-afcc51ee9cfb7445f` (`073d5b40`, TNR
   integration; Opus review running). If a run dies: finish the reviews / fixes, merge one at a
   time (re-run touched crates' tests), then the remaining Phase 6 acceptance item — **export
   from 4:2:2 originals keeps the source chroma** (no test yet: e.g. a 4:2:2 fixture → ProRes 422
   HQ export → decode → compare Cb/Cr planes, PSNR threshold) — a quick `xm5` pass, `cargo xtask
   ci`, tag `phase-6`, build, publish (§1a.33), rewrite this file for **G2c**.**
   **6.1b (owner §1a.29, not started): 4:2:2 decode on the Intel media engine through our own
   `va_hevc.rs`** (FORK-PLAN §5 Phase 6 item 1, "6.1b"): own `Display` for Main 4:2:2 10 on the
   node advertising it (`renderD128`, iHD), bit-exact vs the software decoder, read-back cost at
   6.2K, fallback chain + badge per path. Do it after lanes B / C merge and before 6.6 (it is
   what makes 4:2:2 play without proxies). Owner asked about it 2026-10-12. **§1a.30: the
   DMA-BUF hand-off Intel → AMD is part of 6.1b** (measure read-back first as the baseline).
3. **6.2** auto proxies on import with the in-process hardware encoder (`crates/engine/src/
   proxies.rs`, fast decode paths; export always reads originals).
4. **6.3** render cache like Resolve's Smart Cache (`crates/engine/src/previews.rs`; background
   render of graded segments with the hardware encoder, played instead of re-grading; watch the
   GPU-stall-while-decoding gotcha below). Fable designer for the cache design.
5. **6.6** L 2× with zero drops on 6.2K originals via proxies / render cache; zero drops at cuts.
6. Acceptance (FORK-PLAN §5 Phase 6 "Accept", quick passes per §1a.7, synthetic), `cargo xtask
   ci`, tag `phase-6`, build `phase-6`, then rewrite this file for **Y1** (YOUTUBE-PLAN, hard limit
   8 agent hours; re-scoped by U0).

Lanes that fit (prompt §3): 6.5 runs alone (benchmarks); proxies (6.2) and the render cache
(6.3) share the hardware encoder and `previews.rs` — design the cache first, then build them in
separate worktrees. Benchmarks only in the main session with no builder compiling.

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
