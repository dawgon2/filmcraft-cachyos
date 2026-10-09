# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**,
§5 phases, §7 progress log), then this file. Updated 2026-10-09 in session 5 (Phase 4.1–4.6 done; next Phase 4 acceptance).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` (`bc2771c`), `phase-2` (`baa98f0`),
  `phase-3` (`0909c2c`).
- **Phase 0, 1, 2, 3: done.** Builds in `~/Apps/filmcraft-builds/phase-1`, `phase-2`, `phase-3`;
  `latest` → `phase-3`; launcher `~/.local/share/applications/filmcraft-fork.desktop` (valid).
- **Phase 2 in detail (all done):** in-process VA-API HEVC decode (bit-exact on 39 conformance
  fixtures), zero-copy display through the Vulkan DMA-BUF import, fallback chain in-process →
  ffmpeg pipe → software, VRAM budget, `FrameServer` drop fix. Acceptance (synthetic, full
  suite, `docs/fork-benchmarks.md` "Phase 2"): decode 40.9 fps = 95 % of the 43.2 fps ceiling at
  1.2 ms CPU/frame ✓; play-full 1798/0 at 1.8 ms CPU/frame ✓; **scrub p50 358 / p95 743 ms**
  (target ≤ 1.1 s) ✓; **fallback with the hardware pulled** (Settings Off, `FILMCRAFT_VAAPI_DEVICE=/dev/null`
  → software, mid-stream failure → pipe → software, Vulkan import off → read-back) ✓; **2-hour
  soak** (214 314 shown / 3 dropped, no RAM/VRAM growth) ✓; **`cargo xtask ci`** green ✓;
  **Phase 2 build** made ✓. Not done: the **independent review of the unsafe / FFI code**
  (§1a.5: the instruction arrived after tagging) → **done in session 3** (three rounds, ADR 0002).
- **Phase 3: done** (session 4, tag `phase-3`; acceptance in `docs/fork-benchmarks.md`). 3.1 committed (`cf4b841`): open-GOP seek fix, skipping unshown
  pictures at cuts, decode priority gate, eviction by request span. **3.2 JKL committed**
  (session 4, FORK-PLAN §7): skimming grid with an adaptive step, skimming decode, reverse GOP
  playback, reverse decode pacing. L 2× 299/1, J 1× 296/4 (start-up), L 4× / J 2× steady
  skipping (warm max gap 268 / 300 ms). Zero drops at cuts and at 2× on originals: Phase 6
  (owner). Phase 3's remaining acceptance: audio underruns 0 over 10 min, plus the JKL checks
  in a quick pass.

## Exact next step

1. ~~Unsafe / FFI review (§1a.5)~~ **done** in session 3 (rounds 1–3 committed; record in ADR
   0002 "Review of the unsafe / FFI code" and FORK-PLAN §7). Nothing left from it.
2. ~~Phase 3.2 JKL~~ **done** (session 4). Possible later polish (not required by §1a.15): J 1×
   still drops 1–4 frames in the first ~0.5 s (the first GOP and the one before it decode at
   once); cold 4× climbs the step ladder for ~2 s (a 0.7 s gap); J 2× peaks at 10.4 GB VRAM.
3. ~~3.3~~ **done** (session 4): `box_decimated` leaves GPU frames alone (draft 1/2: 105.8 →
   1.9 ms CPU/frame).
4. ~~3.4 / 3.5~~ **done** (session 4): shuttle audio up to 2× (varispeed), 24-bit LPCM exact,
   10 min at 1× with 0 underruns, PulseAudio host on PipeWire.
5. ~~Phase 3 acceptance, CI, tag, build~~ **done** (session 4).
6. **Phase 4 in progress** (design: `docs/adr/0003-gpu-color-ops.md`; steps in FORK-PLAN §5
   Phase 4). Done: 4.1 F-Log / F-Log2 (`d79060f`), 4.2a colour ops + fused GPU pass
   (`33c6c07`), 4.2b input transforms on the GPU (`c4e6aa9`) and the DWG / DI working space with
   the GPU output stage (`b594e20`), **4.3a** Lumetri Basic + Creative + Vignette + Sharpen on the
   GPU (session 5; `effects::lumetri_grade`, `lumetri_gpu`; oracle in `lumetri_chain_tests`).
   **4.3b** Curves, Wheels, Input LUT, Look as `ColorOp`s (`effects::lumetri_ops`). **4.3c**
   HSL Secondary (`FxOp::HslSecondary`: key / median / blur / masked chain). All of Lumetri is
   on the GPU: quick pass play-graded 240/0 at 3.4 ms CPU/frame, GPU 87 % (FORK-PLAN §7).
   **4.3d** effect masks / opacity masks on the GPU (`FxOp::Masked`, `OpacityMask`), ⚡ badge =
   `filmcraft_project::effect::GPU_EFFECTS`; keys stay CPU (not in the workflow). **4.4**
   scopes sampled from the Program monitor's composite on the GPU (`filmcraft_gpu::scopes`,
   `GpuState::poll_scopes`); graded 6.2K with scopes open 241/0, scopes every frame. **4.5**
   Lanczos-3 for Maximum Render Quality (export, paused Program monitor), sequence presets
   (`sequence.presets`). **4.6**
   1.6× anamorphic (desqueeze in `motion_matrix_par`, `PIXEL_ASPECTS`). Fujifilm LUT check done
   (tone exact; colours differ by design, FORK-PLAN §7). **Next: Phase 4 acceptance**: full
   `xm5` suite, `cargo xtask ci`, tag `phase-4`, build; 4.4 scopes; 4.5 reframing; 4.6 anamorphic; full suite.
   Bench cases added: `play-flog2`, `play-dwg`, `play-half-draft` (`grade()` now also
   interprets as F-Log2). Known flaky test (pre-existing): engine
   `settings_tests::hardware_decoding_setting_drives_the_decoder_switch` (global switch shared
   by parallel tests); passes alone.

## Added after the session

- 2026-10-08: owner asked for a **1.6× anamorphic** pixel aspect (8:5) in every pixel-aspect list,
  where it fits best → FORK-PLAN §1a.13 and Phase 4 item 6 (with reframing / clip interpretation).
- 2026-10-08 (in `CLAUDE.md`, recorded in session 3): **Phase 4 colour design** — small composable
  GPU operations, Lumetri as a fixed chain of them (a later node grader reuses them), working space
  DaVinci Wide Gamut / DaVinci Intermediate, F-Log2 / F-Gamut as input transform → §1a.14.
  **2× with zero drops on original 6.2K media moves to Phase 6**; Phase 3 accepts a steady,
  gracefully skipping 2× / 4× → §1a.15.

## Environment and gotchas

- **Disk rule (FORK-PLAN §1a.12):** keep ≥ 20 GB free on `/home`; `df -h /home` at session start
  and after each build; clean `target/debug/incremental` / `target/debug` / stale build dirs in
  the repo when needed; never delete `target/fixtures`, `target/fork-bench`, `target/bench`,
  `~/Apps/filmcraft-builds` (keep `target/release`: the MCP config runs
  `target/release/filmcraft-cli`); can't stay above 20 GB → reason in `.fork-plan-blocked`, stop.
  2026-10-08 after cleanup: 182 GB free (`target/debug` deleted; `target/` 31 GB).

- **Rust** 1.99 via rustup in `~/.cargo` (not pacman): `export PATH="$HOME/.cargo/bin:$PATH"`.
  `bindgen-cli` 0.73.2 is installed there (user-level) for regenerating `va_sys.rs`.
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
  `FILMCRAFT_XM5_DIR` (where the xm5 clips live; real clips can be linked there by name).
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
- Lumetri and most effects run on the CPU; graded export 0.8 fps, 18 GB RSS → Phase 4–5.
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T) — not fixed.
