# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
incl. §1a.6 order, §1a.16 6E, §1a.17 grader, §1a.18 YouTube gaps; §5 phases; §7 progress log),
then [`GRADE-PLAN.md`](../../GRADE-PLAN.md), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md),
[`EXPORT-PLAN.md`](../../EXPORT-PLAN.md), [`H264-PLAN.md`](../../H264-PLAN.md),
[`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md), [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md),
[`DECODER-PLAN.md`](../../DECODER-PLAN.md),
[`docs/guide/README.md`](../guide/README.md), `docs/fork/OWNER-QUESTIONS.md` (if present), then
this file. FORK-PLAN §1a.20–23 (never stop, the fork guide, never-without-the-owner, runs and the
cloud) apply to every phase.
Rewritten 2026-10-11 in session 10 (G1 accepted, tagged `grade-1`, built). Phase order
(FORK-PLAN §1a.6, owner): G0 ✓ → G1 ✓ → **U0** ([`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md)) → 6
→ Y1 → 6E → G2 → Y3 → C1 → C2 → 7 (+ Y4) → D → 8 → Y2 → H1 → H2 → H3. Also read UPSTREAM-PLAN.md (all of
it: "Phase U0" and "U0 merge notes" are the exact steps) and FORK-PLAN §1a.19.

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` … `phase-5`, `grade-0`,
  `grade-1`. Builds in `~/Apps/filmcraft-builds/` (`phase-1` … `phase-5`, `grade-0`, `grade-1`);
  `latest` → `grade-1`; launcher `~/.local/share/applications/filmcraft-fork.desktop` (valid).
  Bench binaries `target/fork-bench/bench-{phase0,1,2,4,5,grade0,grade1}`.
- **Phases 0–5 done** (FORK-PLAN §7, `docs/fork-benchmarks.md`): in-process VA-API decode +
  zero-copy display, JKL / cuts, GPU colour pipeline (DWG / DI, ADR 0003), in-process VA-API HEVC
  encode + zero-copy export. Playback plain / graded 6.2K 1798/0 over 60 s; export 4K 24.6 fps,
  graded 4K 27.5, 6.2K 18.7 (synthetic).
- **G0 + G1 done** (sessions 9–10): per-clip serial node grade (`TrackItem.grade`, ADR 0004),
  compiled to `ColorOp`s in the fused GPU pass (`crates/render/src/grade.rs`), film grain
  `ColorOp::Grain`; `grade.*` / `stills.*` / `compare.*` commands (`crates/engine/src/grade.rs`,
  `stills.rs`); clip-scoped current node `EditorState.grade_node` (`grade.selectNode`); Color
  Grading workspace laid out like Resolve's Color page (`ui-egui/src/panels/grade.rs`,
  `stills.rs`, `dock.rs`); Settings ▸ General ▸ Workflow Premiere / Resolve (`settings.setWorkflow`,
  "DaVinci Resolve Compatible" keymap, page bar `panels/pages.rs`); schema v14 (stills). G1
  acceptance (synthetic): control-script checks all passed, play-graded quick 240/0 at 3.8 ms,
  play-full 240/0 at 2.0 ms, `cargo xtask ci` green.
- Follow-ups noted, not done (small): `effects.setParam` keyframes at `source_time_at` instead of
  `effect_time_at` (differs from the render on frame holds without Hold Filters); the stills
  gallery re-renders missing pictures on the UI thread (move to a frames job); grade keys are
  app-wide (Ctrl+D in the Edit page toggles a node); the Gallery's empty-state text is clipped in
  a narrow column.

## Exact next step: Phase U0, the first upstream merge (UPSTREAM-PLAN.md), in progress

State (2026-10-11, session 11): ADR 0005 (`docs/adr/0005-upstream-sync.md`, on the sync branch)
records the strategy: local graft `git replace --graft master 8fcad73` (already in this checkout's
refs/replace; never pushed), merge of `upstream/main` **`4e2d0bc2`** on the local branch
`sync/upstream-2026-10-11` in the worktree **`.claude/worktrees/sync-upstream`** (target dir
`target/agent-sync` inside it). All 55 conflicted files resolved by four Fable resolvers and
checkpointed (`972a90ba`, a merge commit); an Opus builder then works on `cargo check` / clippy /
fmt / per-crate tests there with "U0 (WIP): " checkpoint commits, plus the Q2 CRF compatibility
fix (numeric `crf` without `bitrateMode` → CRF mode, with a test). Owner answers Q1–Q4 in
`docs/fork/OWNER-QUESTIONS.md` (all resolved; Q2 needs that fix applied, then mark it done).
**Stage 2 (one VA-API stack) is no longer part of U0:** it is U0.2, right before Phase 8 (owner).

Next (updated 09:36, session 11 stopped at the 5-hour limit, 95 %):
- **Done on the sync branch (HEAD `8ea67999`):** everything compiles; all crates' tests green
  (1679 + ui-egui 136 + 69 integration binaries); fmt, clippy, layers, assets, wasm green; Q2 CRF
  compatibility fix (`5d2869a3`); i18n entries for the fork's strings (`5279b4d3`). Fable review
  (three sweeps) found nothing lost; fixes applied in `8ea67999` (load4 snap back to 1/512 px,
  HDR H.265 export always Main 10). GPU tests (gpu, render, platform VA-API) green on AMD
  (renderD129 / 1002:73df) and Intel (renderD128 / 8086:4c8a). xm5 quick pass (synthetic,
  `target/bench/xm5-u0-quick-*.json`): play-full 240/0 at 2.0 ms (grade-1 2.0), play-graded 240/0
  at 3.4 ms (grade-1 3.8), multicut quick 340/20 (grade-1 binary in the same session 323/37),
  export 4K 26.7 / graded 27.1 / 6.2K 18.9 fps (grade-1 24.6 / 27.5 / 18.7). The worktree's
  `target/fixtures/xm5` is a symlink to the main checkout's fixtures.
- **`cargo xtask ci` was started at 09:35 in the worktree, detached**, log
  `target/u0-ci.log` in the main checkout (last line `exit N`). If it is still running, wait for it
  (no builds meanwhile); if it failed, fix and rerun.
- Follow-ups (not blockers, for later phases): `cropped()` on GPU frames reads back
  (`crates/frame/src/lib.rs` ~327: `clap`-cropped MP4s lose zero-copy); two VA displays opened at
  startup (U0.2); the export dialog doesn't say HDR H.265 is promoted to Main 10; zh-cn lacks the
  HEVC preset / node-command labels (falls back to English); after a hybrid falls back to software
  the GOP cache still counts it as hardware (pre-existing).

4. All green → on `hevc-vaapi`: `git merge --no-commit upstream/main` (4e2d0bc2), then take the
   sync branch's final tree (first merge `hevc-vaapi` into the sync branch so plan / doc commits
   made meanwhile are kept; drop the sync branch's COMPOSITOR-PLAN edit, the owner already fixed
   the ADR number), commit one clean merge, push. Write `docs/fork/UPSTREAM-SYNC.md` + FORK-PLAN
   §7, tag `upstream-sync-1`, build per §1a.9, remove the worktree and its target dir, rewrite
   this file for **G2** (owner, 2026-10-11: G2 items 1–10 right after U0, then G2b = G2.11 + G2.12,
   then Phase 6 and 6E). Y1 note: upstream's VA-API H.264 is in the chain after U0; Y1 wires it
   into the zero-copy path (without the full U0.2 consolidation).

## After U0: G2 (GRADE-PLAN 1–10), G2b (G2.11 + G2.12), then FORK-PLAN Phase 6 (owner, 2026-10-11: §1a.6 order U0 → G2 → G2b → 6 → Y1 → 6E)

Phase 6 notes, for after G2:

Order (scheduled-task prompt §2, FORK-PLAN §5 Phase 6):
1. **6.5 first: the multicut regression.** `xm5-multicut` regressed to 302–338 drops vs 237 at
   Phase 4 (same with the old GOP cache code, so not Phase 5's GOP change). Bisect with the saved
   bench binaries (`target/fork-bench/bench-phase4`, `bench-phase5`, `bench-grade0`,
   `bench-grade1`) and `--quick` / full multicut runs on an idle machine; use a Fable
   diagnostician for the root cause (prompt §3). Then extend `xm5-multicut` with realistic cut
   spacing (2 / 3 / 4 / 5 s, clips entered mid-GOP) and record drops per spacing (synthetic).
2. **6.1** 4:2:2 software decode on the built-in HEVC decoder (FORK-PLAN §1a.24, not ffmpeg; old note: 4:2:2 software decode (ffmpeg pipe, multithreaded, 4:2:2 biplanar frames)) + the badge
   "4:2:2: software decode, proxies recommended" on the clip.
3. **6.2** auto proxies on import with the in-process hardware encoder (`crates/engine/src/
   proxies.rs`, fast decode paths; export always reads originals).
4. **6.3** render cache like Resolve's Smart Cache (`crates/engine/src/previews.rs`; background
   render of graded segments with the hardware encoder, played instead of re-grading; watch the
   GPU-stall-while-decoding gotcha below). Fable designer for the cache design.
5. **6.6** L 2× with zero drops on 6.2K originals via proxies / render cache; zero drops at cuts.
6. Acceptance (FORK-PLAN §5 Phase 6 "Accept", quick passes per §1a.7, synthetic), `cargo xtask
   ci`, tag `phase-6`, build `phase-6`, rewrite this file for **Y1** (YOUTUBE-PLAN, hard limit
   8 agent hours; re-scoped by U0).

Lanes that fit (prompt §3): 6.5 runs alone (benchmarks); proxies
(6.2) and the render cache (6.3) share the hardware encoder and `previews.rs` — design the cache
first, then build them in separate worktrees. Benchmarks only in the main session with no
builder compiling.

## Environment and gotchas

- **Disk rule (FORK-PLAN §1a.12):** keep ≥ 20 GB free on `/home`; `df -h /home` at session start
  and after each build; clean `target/debug/incremental` / `target/debug` / stale build dirs in
  the repo when needed; never delete `target/fixtures`, `target/fork-bench`, `target/bench`,
  `~/Apps/filmcraft-builds` (keep `target/release`: the MCP config runs
  `target/release/filmcraft-cli`); can't stay above 20 GB → reason in `.fork-plan-blocked`, stop.
  2026-10-08 after cleanup: 182 GB free (`target/debug` deleted; `target/` 31 GB).

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
- **CI:** `cargo xtask ci` ≈ 40 min; run idle — `mixer_tests::perf_24_tracks_3_effects_realtime_factor`
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
