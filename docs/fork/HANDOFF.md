# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**,
§5 phases, §7 progress log), then this file. Updated 2026-10-10 in session 7 (Phase 5.2c zero-copy done; next 5.2d encode on the Intel media engine).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags: `phase-1` (`bc2771c`), `phase-2` (`baa98f0`),
  `phase-3` (`0909c2c`), `phase-4` (`476371c`).
- **Phase 0, 1, 2, 3, 4: done.** Builds in `~/Apps/filmcraft-builds/phase-1` … `phase-4`;
  `latest` → `phase-4`; launcher `~/.local/share/applications/filmcraft-fork.desktop` (valid).
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
   (tone exact; colours differ by design, FORK-PLAN §7). Phase 4 accepted (play-graded
   1798/0 at 2.4 ms CPU/frame), tagged `phase-4`, built. **Phase 5.1 done:** export frames
   composited and converted to 4:2:0 on a headless GPU (`platform::gpu_export`,
   `GpuCompositor::read_yuv420`): 4K 16.5 fps, graded 13.0, 6.2K 8.2 (10-bit; before: 4.9 / 0.6 /
   4.0 at 8-bit; `bitDepth: "10"` used to be ignored). **5.2a** writer queue sized to overlap
   render and encode: 4K 19.0 fps, graded 13.6, 6.2K 9.3 (`af5b731`). **5.2b done** (session 6): in-process VA-API
   HEVC encode (`va_enc.rs`, `filmcraft_hevc::write`, `Backend::VaNative` first), GPU pack pass for
   the read-back (28 → 11 ms per 4K frame); 4K 23.7 fps, graded 15.4, 6.2K 12.3 (FORK-PLAN §7).
   **Batch overlap done** (`Exporter::step` renders the next batch on a scoped thread while the
   current one encodes; native only): **4K 24.8 fps ✓ (≥ 24)**, graded 15.8, 6.2K 14.7.
   **5.3a done:** converted media graded at the size drawn (graded 4K 15.8 → 21.5 fps,
   `FILMCRAFT_FULL_SIZE_GRADE=1` = old behaviour). **Next:** export decodes at ~29 fps where the
   decode bench does 41 (threads wait on decoded frames): find why (1080p export as a probe:
   GPU/VCN contention vs the decode path), target graded ≥ 30 fps. Probe done: 1080p ~34 fps,
   4K ~25 with the same decode → output-side cost. **5.4 done** (presets + ffprobe/mpv test).
   **5.2c zero-copy done** (session 7, `6600295`): render targets over the encoder's surfaces,
   no wait on the render side (frames in flight; the encoder thread waits per surface), parity
   AMD + Intel; GOP-cache black-frame race fixed. 60 s: 4K 23.3, graded 22.9, 6.2K 18.2.
   **The limit is now the one VCN** (decode 6.2K + encode share it; ≈ 31 fps ceiling, AMD encode
   10.5 → ~54 ms/frame with the decoder flat out; FORK-PLAN §7 session 7).
   **5.2d measured (session 7, FORK-PLAN §7):** Intel encode while AMD decodes is slower (4K
   18.5 fps); 6.2K ceiling on one VCN ≈ 18–20 fps; graded 4K ≈ 31. `FILMCRAFT_VAAPI_ENCODE_DEVICE`
   exists for experiments. Owner (session 7): continue the current plan; a possible later phase
   for heavy 6.2K optimisation after Phase 8 (FORK-PLAN §1a.16). **Phase 5 acceptance in progress (session 7):** colour vs Program monitor ✓,
   independent unsafe review ✓ + fixes (`f2f2af9`). **Exact next step:** full xm5 suite (idle,
   `target/bench/xm5-phase5.json`), then `cargo xtask ci`, tag `phase-5`, build (§1a.9). Earlier plan text kept for reference: encode on the Intel UHD 750 (`/dev/dri/renderD128`, iHD, 4K Main10
   27 ms/frame incl. upload) while AMD decodes and composites: a separate encode VA display
   (`Display::get` is the decode one; add an encode display chosen by `FILMCRAFT_VAAPI_ENCODE_DEVICE`
   or Auto = another render node with HEVC EncSlice when the decode node's engine is shared).
   Measure first with the read-back path (zero-copy can't cross devices: Intel surfaces are
   Y-tiled), then consider rendering into a linear DMA-BUF in system memory that the Intel VA
   imports. Target graded 4K ≥ 30 fps. Then Phase 5 acceptance (full xm5 suite idle, colour vs
   Program monitor, independent review of the new unsafe in va.rs / interop.rs → ADR 0002,
   `cargo xtask ci`, tag `phase-5`, build).
   Bench cases added: `play-flog2`, `play-dwg`, `play-half-draft` (`grade()` now also
   interprets as F-Log2). Known flaky test (pre-existing): engine
   `settings_tests::hardware_decoding_setting_drives_the_decoder_switch` (global switch shared
   by parallel tests); passes alone.

## Phase 5.2b plan (in-process VA-API HEVC encode), written at the end of session 5

Where export time goes now (60 s, 10-bit): ungraded 4K 19 fps = decode 6.2K + composite + 25 MB
read-back + pipe to ffmpeg, which uploads again and encodes (`hevc_vaapi`, ceiling 29.9 fps);
graded is GPU-bound by the full-size 6.2K grade (~29 ms/frame). Acceptance needs ungraded 4K
≥ 24 fps (80 % of the ceiling) and graded faster than real time (≥ 30 fps).

1. **Bindings:** `va_sys.rs` lacks every encode struct (`VAEncSequenceParameterBufferHEVC`,
   `VAEncPictureParameterBufferHEVC`, `VAEncSliceParameterBufferHEVC`, `VAEncPackedHeader*`,
   `VACodedBufferSegment`, `VAEncMiscParameter*` for rate control). Regenerate with bindgen 0.73.2
   adding `va/va_enc_hevc.h`; keep dynamic loading. **Done:** `crates/platform/gen-va-sys.sh`.
2. **Headers:** write VPS / SPS / PPS / slice headers ourselves as packed headers
   (`filmcraft_bitstream::BitWriter`; `crates/hevc/src/synth_tests.rs` already writes HEVC
   parameter sets for tests, `crates/h264enc` is the model for structure). I/P only (no B) first,
   CTB 64 or what Mesa reports, one slice per picture, Main / Main 10, VUI with the colour tags.
3. **Encode wrapper** in `crates/platform` (`va_enc.rs`, unsafe confined there with `// SAFETY:`):
   config (VAProfileHEVCMain10, EncSlice, check `VAConfigAttribRTFormat`, rate-control and
   packed-header attributes), recon / reference surfaces, coded buffer, per frame the seq / pic /
   slice / misc (rate control: CQP / VBR / CBR) buffers, `vaBeginPicture` … `vaEndPicture`,
   `vaSyncSurface`, map the coded buffer segments → Annex B. Implement `VideoEncoder` and put it
   first in `hevc_export`'s backend order (VA in process → ffmpeg `hevc_vaapi` → `libx265`), with
   the probe-and-fall-back logic already there. **Independent unsafe review** of the new FFI before
   tagging Phase 5 (§1a.5 spirit; ADR 0002 gets a section).
4. **Input:** first upload the GPU path's 4:2:0 planes (`GpuCompositor::read_yuv420`, already
   done) into the VA surface (`vaDeriveImage` / map / copy P010), removing the ffmpeg pipe and its
   second upload; then zero-copy: export the encode surface as DMA-BUF and write P010 into it from
   the compositor (import the two planes as storage images in `crates/gpu/src/interop.rs`), so no
   frame leaves VRAM.
5. **Graded speed:** convert log media at the export size instead of the 6.2K source size when
   downscaling (the plan converts converted layers at full size, `plan.rs` `n = 1`), or run the
   FX stage on `Rgba16Float` working textures; measure both against parity first.
6. Then the rest of Phase 5: presets (YouTube 4K HEVC, 6.2K Master, Archive x265, ProRes 422 HQ,
   Vertical 2160×3840) with ffprobe checks of the colour tags, mpv playback, colour vs monitor;
   full `xm5` suite; tag; build.

Test / bench notes from 5.1: `bench --section xm5 --only export` (60 s cases, ~8 min);
`FILMCRAFT_EXPORT_CPU=1` forces the CPU render path; a 3 s graded comparison via
`filmcraft-cli run` scripts (`file.import` → `clip.interpretFootage {items:[1]}` →
`file.newSequence {fromItem:1}` → `sequence.settings {width,height}` → `edit.selectAll` →
`clip.fitToFrame` → `effects.apply` → `file.exportMedia {range:"custom",…,wait:true}`), then
`ffmpeg … -lavfi psnr` (the CPU path once wrote a black frame; GPU did not).

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
- Exports render on the CPU (graded 4K 0.6 fps) → Phase 5. Playback grading is on the GPU.
- First play after loading drops frames for ~0.5–1 s (cold start) → Phase 7.
- Fujifilm's F-Log2 LUT changes gamut on the log signal; built-in is linear light (FORK-PLAN §7).
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T) — not fixed.
