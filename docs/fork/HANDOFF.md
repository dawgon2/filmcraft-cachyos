# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–39+ incl. §1a.6 order; §5 phases; §7 progress log), then [`GRADE-PLAN.md`](../../GRADE-PLAN.md)
(G2c done; G3 later), [`RESOLVE-UI-PLAN.md`](../../RESOLVE-UI-PLAN.md) (§1a.39), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md),
[`EXPORT-PLAN.md`](../../EXPORT-PLAN.md), [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md),
[`AUDIO-PLAN.md`](../../AUDIO-PLAN.md), [`AI-PLAN.md`](../../AI-PLAN.md), [`RAW-PLAN.md`](../../RAW-PLAN.md),
[`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md), [`H264-PLAN.md`](../../H264-PLAN.md),
[`DECODER-PLAN.md`](../../DECODER-PLAN.md), [`FORK-ONLY.md`](FORK-ONLY.md), [`docs/guide/README.md`](../guide/README.md),
[`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md) (Q9 open: temporal NR on the first node only, provisional),
then this file. Rewritten 2026-10-12 after G2c (scheduled run: tagged `grade-2c`, built, published).
Phase order (FORK-PLAN §1a.6, the only valid order): … → 6 ✓ → G2c ✓ → **RS3** (after RS2 / RS2b) → Y1 → 6E → U0.2 → A1 → Y3 →
C1 → C2 → AI1 → A2 → G3 → 7 (+ Y4) → Y2 → H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 → R2 → D → 8;
RESOLVE-UI-PLAN RS1 + RS2 run as a lane alongside (§1a.39).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0`,
  `grade-1`, `upstream-sync-1`, `grade-2`, `grade-2b`, `grade-2c`. Builds in
  `~/Apps/filmcraft-builds/`; `latest` → `grade-2c`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public snapshots / releases on
  dawgon2/filmcraft-cachyos branch `fork` (§1a.33); latest `fork-grade-2c`.
- **G2c done** (FORK-PLAN §7, 2026-10-12): `crates/color/src/wb.rs` (shared Temperature / Tint
  gains, exact inverse, box average, grey-world Auto, CIEDE2000), commands `grade.wb.pick` /
  `grade.wb.auto` / `lumetri.wb.pick` / `lumetri.wb.auto`, buttons under Primaries ▸ Tint and in
  Lumetri ▸ Basic Correction (click or drag a box in the Program picture). Graphic / mask
  overlays stand aside while any picker is armed (they used to swallow picker clicks).
- **RS1 done (merged, part of tag `resolve-ui-1` later):** feature `fork-resolve-default`
  (enabled by `apps/filmcraft`, so `cargo xtask ci` runs every test with Resolve as the
  default: **new UI tests that assume Premiere must set `prefs.workflow = Premiere` or run
  `settings.setWorkflow premiere`**), one-time prefs migration (`crates/engine/src/fork_resolve.rs`,
  marker file `fork-resolve-migrated` in the data dir), `ThemeKind::Resolve`. The owner's own
  prefs migrate on the first launch of the grade-2c build.
- **Owed (not blocking):** G2c: clipped highlights aren't detected in DWG / HDR sequences; the
  Coach walk doesn't open Lumetri (`lumetri.wb.*` in `NOT_WALKED`, cloud job COACH-WALK-1).
  RS1: a session without a data dir (headless / MCP) reports Resolve but keeps the default
  keymap; the Resolve theme chosen in Premiere isn't remembered across a workflow round trip.
  From Phase 6: frame server per-frame proxy hint; `cache.claimFolder`; Export Frame without
  TNR; automated 4:2:2 export chroma test; decode-to-screen fps with hardware 4:2:2 in the app;
  software 4:2:2 decoder memory cap / multithreading (ask first); TNR / cache / proxy 6.2K bench
  cases; VPP context leak on purpose (iHD crash on destroy).
- Schema unchanged (v16); prefs version 3. New user-visible strings (and engine command labels)
  need es + zh-cn entries. New guide pages go in `GUIDE_PAGES` (`crates/engine/src/coach_ask.rs`)
  and their `{ui:}` ids must be known to the `coach_ui` walk tests.
- Cloud: merged up to `55f8639c`; CLOUD-QUEUE has GUIDE-7/8, COACH-WALK-1, Y1-PREP, I18N-2.
  Merge `origin/cloud-work` first if it has new commits.

## Exact next step: RESOLVE-UI-PLAN RS2 → RS2b (tag `resolve-ui-1`), then RS3 (tag `resolve-ui-2`), then Y1

§1a.6 puts RS3 right after G2c, and RESOLVE-UI-PLAN says RS3 comes after RS2 (RS1 + RS2 + RS2b
were a lane alongside Phase 6 that only got to RS1). So:
1. **RS2, Project Manager home screen** (RESOLVE-UI-PLAN RS2 items 1–7): `projects.*` commands
   in the engine (folder listing, summaries + poster frames written on save, hostile `.fcproj`
   handling with garbage-file regression tests), the full-window screen in ui-egui, the
   "At startup show" preference (generic; Resolve workflow defaults to Project Manager), File ▸
   Project Manager / Media Management relabel in the Resolve workflow, guide
   `docs/guide/projects.md` ([Resolve-style]). Reference `plan/resolve/screenshots/resolveHomePage.png`
   (never commit it). Lanes: engine `projects.*` + summaries (one builder) and the UI screen once
   the command shapes are committed.
2. **RS2b, Resolve terminology overlay** (`i18n/terms-resolve.tsv` consulted before the
   catalogue in the Resolve workflow; on-screen words only; tests: no dead keys, live relabel,
   Premiere byte-identical). Then acceptance (both workflows), tag `resolve-ui-1`.
3. **RS3, page chrome and layouts** (its own phase, tag `resolve-ui-2`, build + publish per
   §1a.9 / §1a.33): toolbar row, restyled page bar, per-page workspace presets; screenshots
   against `plan/resolve/screenshots/` in both workflows.
4. Then **Y1** (YOUTUBE-PLAN, hard limit 8 agent hours; Intel half can run on the Dell;
   CLOUD-QUEUE Y1-PREP may have mapped the H.264 fields already).

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
