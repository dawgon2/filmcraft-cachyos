# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
incl. §1a.6 order, §1a.16 6E, §1a.17 grader, §1a.18 YouTube gaps, §1a.19–24; §5 phases; §7
progress log), then [`GRADE-PLAN.md`](../../GRADE-PLAN.md) (all of it; §1 audit, §3 Phase G2, §7
lanes), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md), [`EXPORT-PLAN.md`](../../EXPORT-PLAN.md),
[`H264-PLAN.md`](../../H264-PLAN.md), [`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md),
[`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md), [`DECODER-PLAN.md`](../../DECODER-PLAN.md),
[`docs/guide/README.md`](../guide/README.md), `docs/fork/OWNER-QUESTIONS.md`, then this file.
Rewritten 2026-10-11 in session 12 (U0 merged, tagged `upstream-sync-1`, built). Phase order
(FORK-PLAN §1a.6, owner): G0 ✓ → G1 ✓ → U0 ✓ → **G2** (GRADE-PLAN items 1–10, tag `grade-2`) →
G2b (G2.11 + G2.12, tag `grade-2b`) → 6 (incl. 6.1) → Y1 → 6E → U0.2 → Y3 → C1 → C2 → 7 (+ Y4) →
D → 8 → Y2 → H1 → H2 → H3. From G2b on, each phase boundary has a contribution slot (FORK-PLAN
§1a.25: up to two small upstream contributions pushed to `cachyos`, never a PR without the owner).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` … `phase-5`, `grade-0`,
  `grade-1`, `upstream-sync-1`. Builds in `~/Apps/filmcraft-builds/` (`phase-1` … `phase-5`,
  `grade-0`, `grade-1`, `upstream-sync-1`); `latest` → `upstream-sync-1`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Bench binaries
  `target/fork-bench/bench-{phase0,1,2,4,5,grade0,grade1,u0}`.
- **Phases 0–5, G0, G1 done** (FORK-PLAN §7, `docs/fork-benchmarks.md`): in-process VA-API
  decode / encode with zero-copy display and export, GPU colour pipeline (DWG / DI, ADR 0003),
  per-clip serial node grade (ADR 0004, `crates/render/src/grade.rs`, `crates/engine/src/grade.rs`,
  `stills.rs`), Color page (`ui-egui/src/panels/grade.rs`, `stills.rs`, `pages.rs`), Workflow
  Premiere / Resolve switch.
- **U0 done** (session 11–12): upstream `storytold/filmcraft` `4e2d0bc2` (806 commits) merged as
  one merge commit; summary, resolutions and numbers in
  [`docs/fork/UPSTREAM-SYNC.md`](UPSTREAM-SYNC.md); ADR 0005. **Schema is now v16** (upstream's
  v13–v15 + the fork's grades / stills). Both VA-API stacks are compiled in (fork HEVC zero-copy →
  upstream `vaapi/` → `vaapi_pipe`); one stack is U0.2. Lumetri now runs as the ColorOps chain
  (upstream's `OP_LUMETRI` retired). Upstream added Spanish / Chinese UI: **new user-visible
  strings need es + zh-cn entries** (ui-egui i18n completeness tests fail otherwise).
- Later syncs: ordinary `git merge upstream/main` (no graft), weekly, per UPSTREAM-PLAN
  (ask-first rules there). The local replace ref `refs/replace/<master>` is no longer needed.
- Follow-ups noted, not done (small): `effects.setParam` keyframes at `source_time_at` instead of
  `effect_time_at`; the stills gallery re-renders missing pictures on the UI thread; grade keys
  are app-wide; the Gallery's empty-state text is clipped in a narrow column; `cropped()` on GPU
  frames reads back (`crates/frame/src/lib.rs`); two VA displays opened at startup (U0.2); the
  export dialog doesn't say HDR H.265 is promoted to Main 10; zh-cn lacks a few HEVC preset /
  node-command labels.

## Progress in G2 (session 13)

- **Done:** G2.1 qualifier (`ColorOp::Stash` / `MixStash`, `grade::compile_fx` → FX steps),
  G2.2 windows (`GradeNode.windows`, `FxOp::Masked`; `masks.*` with `"node"`, `masks::Owner`),
  linear mask shape for all masks, G2.3 tracking (the mask tracker on node windows), G2.4 spatial
  NR (`FxOp::Denoise`, node params `nr.*`), G2.5 soft clip (`hl.*`), G2.6 node LUT
  (`GradeNode.lut`, `grade.setLut`). Guide: `color/secondaries.md`, `color/noise.md`,
  `color/finishing.md`.
- **Done too:** G2.8 / G2.9 / G2.10 (`grade_match.rs`, `grade_explain.rs`, engine
  `grade_tools.rs`: `grade.compareData`, `compareShots`, `checkScene`, `shotMatch`,
  `explainNode`, `explainGrade`; Color page Match tab; guide `matching.md`).
- Follow-ups (G2): the Match tab's requests render on the UI thread (move to a background job);
  the node strip doesn't show the explainer's one-line summary yet (needs a cache keyed by
  `grade_explain::node_state_key` + the input frame); "mark a pair intended" for shot comparison
  isn't built; no timeline badge for identical grades (the Match tab lists them).
- Temporal NR deferred to FORK-PLAN Phase 6 item 7 (owner, 2026-10-11).
- Follow-ups: the preview cost model (`crates/render/src/preview.rs` `item_fx_cost`) ignores the
  grade (NR is expensive); linear masks in a project re-saved by an older build lose their shape
  (render as a 2-point path = no effect); mask tracking writes keyframes at source time while
  edits now use the effect time (same unless a frame hold is involved).

## Exact next step: G2 acceptance (G2.1–G2.10 done)

GRADE-PLAN G2 "Accept" (synthetic, quick passes): qualifier + tracked circle window on the moving
synthetic subject held over 5 s (extend `masks_tests::grade_node_windows_are_masks_and_track` or a
control-channel script), NR σ drop ≥ 40 % with edge kept (`denoise.rs` tests) on an F-Log2-like
fixture, play-graded quick with NR on (record ms / drops), export matches the viewer, compare /
Shot Match / explainer (render tests). Then `cargo xtask ci`, tag `grade-2`, build
`~/Apps/filmcraft-builds/grade-2/` per §1a.9, rewrite this file for G2b.

GRADE-PLAN §3 "Phase G2" items 1–10, in order, one commit per item (task ids `G2.1` … `G2.10`;
item 7 doesn't exist). Start by sending a Sonnet scout to map what exists for each item (GRADE-PLAN
§1 is the audit: `HslKey`, `mask.rs` shapes, `track.rs`, the LUT op, BT.2390 output stage,
`color_match.rs` statistics and `solve`), then:
1. **G2.1 qualifier per node** (main session): `GradeNode` gains an optional qualifier (reuse
   `HslKey`; invert, sat max, highlight mode), compiled in `crates/render/src/grade.rs`; picker on
   the viewer; `grade.*` commands; serde + old-project (v16 without the field) tests; CPU / GPU
   parity on AMD and Intel.
2. **G2.2 windows** (main) + **linear window** shape (leaf builder in `crates/render` / `mask.rs`).
3. **G2.3 tracking** of node windows through `track.rs` (keyframes like effect masks).
4. **G2.4 spatial NR** (leaf builder, GPU kernel in `crates/gpu`, Opus for the WGSL).
5. **G2.5 highlight roll-off**, **G2.6 LUT node**.
6. **G2.8 compare** (`grade.compareData`, `grade.compareShots`, `grade.checkScene`),
   **G2.9 Shot Match** on node primaries, **G2.10 node explainer** (leaf builder in
   `crates/render` once the node types are committed).
7. Guide pages (FORK-PLAN §1a.21) for every visible feature, in the same or next commit.
8. Acceptance (GRADE-PLAN G2 "Accept", synthetic quick passes), `cargo xtask ci`, tag `grade-2`,
   build per §1a.9, rewrite this file for **G2b**.

## After G2: G2b (G2.11 + G2.12), then FORK-PLAN Phase 6 (owner, 2026-10-11)

Phase 6 notes, for after G2:

Order (scheduled-task prompt §2, FORK-PLAN §5 Phase 6):
1. **6.5 first: the multicut regression.** `xm5-multicut` regressed to 302–338 drops vs 237 at
   Phase 4 (same with the old GOP cache code, so not Phase 5's GOP change). Bisect with the saved
   bench binaries (`target/fork-bench/bench-phase4`, `bench-phase5`, `bench-grade0`,
   `bench-grade1`, `bench-u0`) and `--quick` / full multicut runs on an idle machine; use a Fable
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
