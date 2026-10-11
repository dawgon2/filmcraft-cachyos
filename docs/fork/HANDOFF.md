# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**,
§5 phases, §7 progress log), then **all of [`GRADE-PLAN.md`](../../GRADE-PLAN.md)** (§1 audit:
don't rebuild what exists; §7 subagents), then this file. Rewritten 2026-10-11 in session 9
(G0 accepted, tagged `grade-0`, built). Phase order (FORK-PLAN §1a.6): 5 (done) → G0 (done) →
**G1** → 6 → G2 → 7 → 8.

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` … `phase-5`, `grade-0`.
  Builds in `~/Apps/filmcraft-builds/phase-1` … `phase-5`, `grade-0`; `latest` → `grade-0`;
  launcher `~/.local/share/applications/filmcraft-fork.desktop` (valid).
- **Phases 0–5 done** (history in FORK-PLAN §7 and `docs/fork-benchmarks.md`): in-process VA-API
  decode + zero-copy display (2), JKL / cuts (3), GPU colour pipeline with DWG / DI working space
  and all of Lumetri on the GPU (4, ADR 0003), in-process VA-API HEVC encode, zero-copy export
  (5). Playback plain / graded 6.2K 1798/0 over 60 s; export 4K 24.6 fps, graded 4K 27.5, 6.2K
  18.7 (synthetic).
- **G0 done** (session 9, FORK-PLAN §7):
  - ADR [`0004-grade-model.md`](../adr/0004-grade-model.md): a grade is `TrackItem.grade:
    Option<Grade>` (schema v13), `Grade { nodes: Vec<GradeNode>, bypass }`, `GradeNode { id,
    label, enabled, params: BTreeMap<String, Param> }`. Keys + identity + ranges in
    `filmcraft_project::grade::NODE_PARAMS` (`lift` / `gamma` / `gain` / `offset` as
    `Color([r, g, b, master])` offsets around 0, `contrast`, `pivot` (DI 0.336), `saturation`,
    `temperature`, `tint`, `curve.master|r|g|b`). Accessors clamp (`float_at`, `wheel_at`,
    `curve`); `MAX_NODES` 64 compiled; `next_id()` always unique.
  - `filmcraft_render::grade::compile(&Grade, t, working) -> Vec<ColorOp>` (encode to the grading
    signal, nodes, decode; empty = identity), `item_ops`, `changes_picture`. Wired: GPU plan
    (`plan.rs` `push_item`: `[input transform, grade, effect chain]` fused), CPU `item_layer`,
    adjustment layers (CPU). Identity grades keep every shortcut (titles vector-sharp, plain nests).
    Tests: `render/src/grade.rs`, end of `render/src/tests.rs`, `nest_tests.rs`,
    `adjustment_tests.rs`, `gpu/src/colorops_tests.rs` `gpu_compiled_grades_match_cpu` (AMD /
    Intel 2.1e-6).
  - Widgets: `crates/ui-egui/src/widgets/color.rs` (`curve_editor`, `color_wheel` + `WheelChange`).
    Reviewer's suggestion for G1.2: make the wheel's lightness slider optional / replaceable by a
    master ring and return the `egui::Response` (for automation ids / tooltips); neither widget
    registers an automation id yet (pre-existing).
  - `effects.move {"clip","index","to"}` (engine `commands.rs`, `movable_run`).
  - G0 acceptance (synthetic, quick pass): play-graded 240/0 at 3.4 ms CPU/frame, play-full 240/0
    at 1.9 ms (unchanged); `cargo xtask ci` green.

## Exact next step: GRADE-PLAN Phase G1 ("Lite")

Steps (GRADE-PLAN §3 G1; acceptance there): 1 node stack + Color workspace, 2 primaries panel
(LGGO wheels with master rings → the existing node keys), 3 curves per node, 4 copy / paste /
append grade, 5 stills + wipe / split compare, 6 film grain kernel, 7 scopes follow the grade
(check only), 8 Settings ▸ Workflow: Premiere / Resolve (pages, Color page layout, Resolve keymap
preset from the public manual only, node grader as default colour panel).

Suggested order and lanes (GRADE-PLAN §7; scheduled-task prompt §3 for models / limits):
1. **Main, first:** `grade.*` commands in `crates/engine` (new `grade.rs` module next to
   `color.rs`): `addNode`, `insertNode`, `removeNode`, `moveNode`, `toggleNode`, `labelNode`,
   `setParam` (mirror `effects.setParam`'s `json_to_param` + keyframes at the playhead),
   `resetNode`, `bypass`, each through `Session::edit` (`engine/src/lib.rs:745`), with an engine
   test per command (undo / hostile ids). Commit; that unblocks the lanes.
2. **Leaf builder (opus, `crates/gpu` + `crates/render`):** film grain `FxOp` (luma-weighted,
   size / strength / softness, deterministic seed per frame) with CPU oracle + GPU parity; node
   key `grain.*` compiled in `render/src/grade.rs` (the grade then emits `FxOp`s, not only
   `ColorOp`s: change `compile`'s output or add a second list; keep identity empty).
3. **Main:** Color workspace (`ui-egui/src/dock.rs` `workspace()` :232) with node strip (painter
   widget) + primaries / curves panels using `widgets/color.rs`; automation ids on every widget.
4. **Leaf builder (sonnet):** copy / paste / append grade (`clip_ops.rs:1024` copy attributes
   learns `grade`), then stills (grab frame + grade into the project) and wipe / split in the
   Program monitor Comparison mode (`panels/monitor.rs`, `monitor_view.rs`).
5. **Leaf builder (sonnet, engine only):** Resolve keymap preset in `crates/engine/src/keyboard.rs`
   (next to the Premiere preset), from Resolve's public manual; then the Workflow setting.
6. Acceptance via MCP / control (3-node grade on a synthetic F-Log2 clip, copy to 5 clips, still
   + wipe, Workflow switch screenshots), play-graded quick ≥ 95 % of Phase 5's shown frames,
   `cargo xtask ci`, tag `grade-1`, build `grade-1`, rewrite this file.

**After G1: Phase 6** (FORK-PLAN §5). Start with 6.5: extend `xm5-multicut` with realistic cut
spacing (2 / 3 / 4 / 5 s, mid-GOP entries), record drops per spacing; first find why multicut
regressed to 302–338 drops vs 237 at Phase 4 (same with the old GOP cache code; saved binaries
`target/fork-bench/bench-phase{4,5}`). Then 6.1 4:2:2 software decode + badge, 6.2 auto proxies
with the in-process encoder (export reads originals), 6.3 render cache (like Resolve's Smart
Cache), 6.6 L 2× zero drops on 6.2K originals via proxies / cache. Don't start the 6.2K
optimisation phase (§1a.16) without the owner.

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
- `Grade::with_node()` and an empty grade render nothing (identity); G1 node commands must keep
  ids unique (`Grade::next_id`).
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T) — not fixed.
