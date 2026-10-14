# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–67 incl. §1a.6 order; §5 phases; only the last ~80 lines of §7), then
[`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md) (U0 stage 2, next) and
[`PREP-Y1-6E-U02.md`](PREP-Y1-6E-U02.md) **§3 (U0.2 map, step table, recommendations)**, then
[`FORK-ONLY.md`](FORK-ONLY.md), [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), this file. Other plan
files (GRADE, RESOLVE-UI, YOUTUBE, AUDIO, AI, COMPOSITOR, STORAGE, H264, RAW, DECODER) only when a
step points at them. Rewritten 2026-10-13 10:55 after the 6E group acceptance (scheduled run).

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → Y1 ✓ → 6E ✓ → **U0.2** → **AI1** →
**X422** → Y3 → C1 → C2 → AI2 → A2 → S1 → S2 → G3 → 7 (+ Y4) → Y2 → H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 →
R2 → D → 8. Small phases are accepted in groups (§1a.41). AI1 moved right after U0.2 and the
4:2:2 export cases (X422) right after AI1 by the owner 2026-10-13 (§1a.69).
**A1 runs on the Dell lane (§1a.60)** after L1 / L2 part 2 and after U0.2 merges; the main runner skips it.
**L1 / L2 part 2 (Dell lane, scheduled task `filmcraft-dell-lane`, lock `.dell-lane.lock`; the main
runner leaves it alone):** L1.5 portal dialogs and L2.6 left. **Flatpak build ✓ (`18d523e4`,
2026-10-13):** from-source manifest built offline on the Dell (`cargo vendor`, 25.08 SDK +
rust-stable, ~7 min; scratch setup `~/flatpak-build/` there) and feature-checked in the sandbox in
a virtual KWin: native Wayland, VA-API iHD + DMA-BUF, playback 0 dropped with PipeWire audio, MPRIS
Pause, portal Inhibit, taskbar progress, export, Notifications / FileManager1 / KDE Connect /
UDisks2 reachable (test app uninstalled again; SDK kept, 7.2 GB in the Dell's
`~/.local/share/flatpak`, see Q17). Not seen live: an export notification popping (the window was
focused and the job short: by design no popup) and a real card eject. L1.8 / L2.4 / L2.10 and the
text-scale hint (Q15 b, `2e67f732`; owed on a real desktop: Plasma font 11 pt → 110 %) already merged.
Worktree `.claude/worktrees/dell-lane`, branch `dell-lane` (= hevc-vaapi, nothing unmerged). Part 2 rides with the **U0.2 group** (tags `linux-1b` /
`linux-2b`); its real-desktop checks (GlobalShortcuts dialog, card insert → Copy → Eject, KDE
Connect send / receive / Multimedia control) join U0.2's acceptance. The lane waits for U0.2 to
merge before L1.5 / L2.6. **The Dell lane's schedule should go back to every 30 min now that part 1
is accepted (§1a.56): the owner or a live session sets it; the runner can't.**

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, the yt-1 group (`resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`,
  `coach-1`, `yt-1`) and the **6E group on `d866cb4c` (2026-10-13): `phase-6e`, `linux-1a`,
  `linux-2a`**. Builds in `~/Apps/filmcraft-builds/`; `latest` → `phase-6e`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public (§1a.33): dawgon2/filmcraft-cachyos
  branch `fork` snapshot `94e88700` (of `e7a7b374` = the tag plus a fixture-name scrub; parent
  `c72dcac2`), tags `fork-phase-6e`, `fork-linux-1a`, `fork-linux-2a`, release `fork-phase-6e`
  (scrubbed Dell build of `d866cb4c`, NOTES.md).
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

## Exact next step: U0.2 (UPSTREAM-PLAN U0 stage 2: one VA-API stack), step U0.2.5

1. **Weekly upstream sync** is due from 2026-10-18 (last: Sync 1, 2026-10-11; upstream had 565 new
   commits on 2026-10-13, still none in `vaapi/`). Re-check `vaapi/` drift before each U0.2 step.
2. **U0.2.1 done (Y1). U0.2.2 done (2026-10-13, merge `18fddc60`):** HEVC Main / Main10 4:2:0 goes
   through upstream's `vaapi/hevc.rs::Front` with `ZeroCopyAccel` first in `vaapi_factory`
   (`NAME_HEVC_ZERO_COPY`); `va_hevc::NativeVaDecoder` only when `VaDecoder::front_takes` refuses
   (4:2:2, range extensions, Rext 4:2:0 intra profiles). Grey stand-ins for missing references on
   the zero-copy path only (`Front::set_conceal_missing`); hostile tile layouts (`tile_sizes`) and
   entry points / zero-data slices refused in the front end; `ZeroCopyAccel` decodes on the
   thread's decode node (6E lanes) and frames keep the context alive (`Arc<Mutex<Context>>`: fixes
   an iHD `vaSyncSurface` SIGSEGV after the decoder was dropped, H.264 too). Gates (synthetic):
   conformance parity AMD + Intel (`va_native` 37 fixtures via the front end, 22 via the fork
   decoder), `vaapi.rs` 20/20 both GPUs; xm5 quick: play-full 240/0 1.96 ms, graded 240/0 3.41 ms
   (= U0); export quick dual A/B vs a base binary (`target/fork-bench/bench-phase6e` vs
   `bench-u022`): 22.1 / 26.2 / 17.2 fps vs 22.2 / 11.0 / 17.2 (graded quick dual is very noisy);
   multicut A/B drops 50, 49 vs 60, 22 (noise) **but CPU per frame 2.9–3.2 vs 2.2–2.5 ms and ~8 %
   fewer frames due**: look at it in U0.2.3 (suspect: CPU grey fills for RASL / missing refs at
   open-GOP entries, 78 MB per picture at 6.2K, review finding 4: reuse the grey surface).
3. **U0.2.3 done (2026-10-13, merge `8777f478`):** every HEVC stream VA-API takes goes through the
   front end; `vaapi_factory` = zero-copy → read-back → ffmpeg pipe (no `NativeVaDecoder`; still
   used by `crates/ui-egui/tests/ui/va_interop.rs`: re-point in U0.2.5). Hand-written Rext / Scc /
   Extension structs in `vaapi/ffi.rs` with ABI tests against `va_sys` and GCC; Y210 pool,
   Intel→AMD 4:2:2 hand-off or read-back, `NAME_HEVC_422_ZERO_COPY`; grey stand-in surfaces reused
   while unwritten (`Accel::pick_gray`, flag cleared before any write); on iHD (no concealment)
   pictures with missing references are refused up front as `CodecError::Unsupported` (iHD fails
   any picture predicted from a grey stand-in), which neither stops a 4:2:2 decoder nor sets the
   4:2:2 failure badge; RASL after a CRA at decode start is skipped (not the multicut CPU cause).
   AMD concealment decodes but differs from software (max 112; VA can't flag generated refs).
   Tests (both GPUs): lib 90, vaapi 23, va_native (59 bit-exact), va_hevc_422 6, va_422_handoff 5,
   vaapi_displays 1. Opus review: no blockers; both should-fix items fixed (220c498a).
   **Owed:** (i) **done 2026-10-13 (U0.2.3b run):** an up-front `Unsupported` goes straight to software
   (`hybrid.rs` skips the next hardware backends; the Intel case takes 0.11 s instead of ~10 s, the test
   asserts < 5 s); (ii) a test that a failed decode / copy
   into a grey slot gets re-greyed (needs failure injection in `ZeroCopyAccel`); (iii) **done
   2026-10-13 (DC1 `intel-1`, UHD 770, 1335df19):** vaapi 23/23, va_hevc_422 6/6, va_422_handoff 5/5
   (no AMD on the Dell: hand-off cases there are Intel-only); (iv) `va_dual_lanes` / `va_dual_export` not re-run; (v) pre-existing:
   `va_dual_export::two_gpus_keep_the_colours_of_one` fails only with the Intel device forced.
4. **U0.2.3b, speed floor: CLOSED 2026-10-13 18:49** (step e full length, `docs/fork-benchmarks.md`
   "U0.2.3b step e": 4K 32.5 / 35.1, 4K graded 31.1 / 30.5, 6.2K 25.5, 6.2K graded 19.6 = gap,
   owner question Q21 FYI, choice (a) accept; Q20 = Auto's scaled hand-off). Owed from it: Opus
   review of the d plumbing (parts 1–3) when the mode allows; U0.2.3 owed (ii) and (v) unchanged.
   History below. **U0.2.3b (FORK-PLAN §1a.58, §1a.62–65; ≈ 10–14 h), before U0.2.4.** Done
   2026-10-13: the export regression U0.2.2 / U0.2.3 caused (zero-copy surfaces capped at the GOP
   budget → read-back decoder; 4K 34 → 8 fps) is fixed; bench has `xm5-export-6k graded` and
   `backends` in export rows. Numbers (`docs/fork-benchmarks.md` "U0.2.3b"): 4K 34.2 / 37.6,
   6.2K 24.4 / 24.2 (thin; a build overlapped), 4K graded 21.1 / 21.4, 6.2K graded 19.1 / 19.2,
   multicut CPU 2.49–2.68 ms (~U0). **Remaining, in order (owner 2026-10-13):**
   a. **Done (2026-10-13):** export always renders f32 (`GpuCompositor::for_export`, used by
      `gpu_export.rs` and `export_renderer.rs`; tests `export_compositors_never_use_half_floats` in
      the gpu and platform crates, both GPUs). Half-float preview is opt-in only
      (`FILMCRAFT_FX_FORMAT=f16`; `HALF_ON_INTEGRATED = false`, owner Q18: Sharpen 3.4–3.8 codes
      off f32, colour ops 0.3–0.7). Diagnosis so far (`docs/fork-benchmarks.md` "U0.2.3b part 2",
      ignored test `graded_export_ops_timing_6k`, `--release --ignored --nocapture`, GPU via
      `MESA_VK_DEVICE_SELECT`): at 6.2K Intel UHD 750 spends 498 ms on the 9-op colour chain
      (`colorops.wgsl` `run_chain` interpreter: ~50 ms per op, ALU-bound, same in f16), 308 ms on
      HSL Secondary, 279 ms on Unsharp; AMD 23 / 19 / 11 ms (GPU ~12 ms per export frame: AMD's
      lane is VCN-bound, 6.2K graded encode 44 ms / frame). Lever for (c): straight-line chain
      shaders instead of the interpreter, fused passes, Xe workgroup sizes.
   b. **Done (2026-10-13):** `lanes/lane.rs::auto_keeps_one_gpu` / `uses_effect_stage` (effects,
      grade, TNR, opacity masks, an interpretation colour space; nested sequences ≤ 8 deep) keep
      Auto on one GPU; Dual still forces both; 4:2:2 keeps its Intel encoder. Test
      `auto_keeps_graded_and_effect_exports_on_one_gpu`; "?" help next to Export engines
      (`export.video.exportEngines.help`, es + zh-cn); guide `export.md` "Two GPUs". Full-length
      bench of ungraded speeds still owed (with the step e bench). Was: **Auto rule (§1a.63):** Export engines = Auto runs on the RX 6700 XT alone; the Intel lane
      joins only for a reason: the 4:2:2 master needs it, or the export is one where Intel
      measurably helps at full precision (ungraded today; graded once (c) makes it a net gain).
      Dual / Single stay manual overrides; slow-lane rule stays the guard. Tests per case;
      tooltip; guide `export.md` "Two GPUs".
   c. **Intel faster on graded frames at f32** (2–4 fps vs 15–19 on AMD): where its time goes,
      fewer passes / less bandwidth, fused ops, workgroup sizes for the Xe iGPU.
      **Part 1 done (2026-10-13):** colour chains run as WGSL specialised to their op codes
      (`colorops::specialise`, cache in `FxStage::special_pipeline`, interpreter fallback for > 64
      ops / wasm / failed builds / `FILMCRAFT_CHAIN_SPECIALISE=0`). 6.2K f32 per frame: Intel all
      graded ops 980 → 332 ms (chain 473 → 82, HSL Secondary 315 → 53), AMD 34 → 23 ms. Tests
      `specialised_chains_match_the_interpreter` (both GPUs), `every_specialised_op_validates`
      (naga, no GPU), `specialised_source_calls_each_op_in_order`. **Part 2 done:** per-op
      `fx_px` pipelines (`override PX_OP`, `FxStage::px_pipeline`; test
      `per_op_pipelines_match_the_generic_pass`), output pass specialised, timing test now GPU-only
      (`stage_ms`). 6.2K f32 GPU time: Intel 960 → 286 ms, AMD 23.9 → 20.8 ms. Intel is now near its
      bandwidth floor (~30–40 ms per f32 pass, ~8 passes): ~3.5 fps, so graded stays on AMD; the
      remaining Intel lever is pass fusion (gap logged as owner question Q19, FYI). **c is closed
      unless Q19 says otherwise; next: d (AMD graded speed, diagnosis below).**
   d. **AMD faster on graded frames at f32:** trace with `FILMCRAFT_EXPORT_TRACE=1` /
      `FILMCRAFT_EXPORT_GPU_PROBE=1`, overlap render and encode, cut redundant passes.
      **Diagnosis so far (2026-10-13, quick runs, bench `target/fork-bench/bench-u023b-c`):**
      4K graded (Auto → AMD only) 21.3 / 20.6 fps; 4K ungraded forced single
      (`FILMCRAFT_BENCH_ENGINES=single`) 27.7 / 28.8 fps. Difference ≈ 12 ms per frame ≈ the AMD GPU
      work per graded frame, so render does **not** overlap the decode. Stack samples (`eu-stack`,
      python `PR_SET_PTRACER` parent, see memory `fork-lessons`): export workers wait on the clip's
      `GopCache` state lock (`frame_skim_locked` → `lock_contended`) while one worker decodes
      (`HybridDecoder` → `Front::read_out` → `ZeroCopyAccel::read` → `vaSyncSurface` →
      `drmSyncobjWait`). Lumetri op evaluation is 0.02 ms (not it). **Pitfall:**
      `FILMCRAFT_EXPORT_GPU_PROBE` waits for the GPU while holding the compositor lock: it inflates
      "gpu lock wait"; don't combine it with throughput numbers. **Correction (full length,
      single GPU, `FILMCRAFT_BENCH_ENGINES=single`, bench `bench-u023b-d`):** 4K ungraded 23.3 fps,
      4K graded 23.1 / 22.0 fps: **grading costs nothing on one AMD lane**; the quick-run gap was
      noise (quick graded runs swung 18 → 27 fps). One lane is decode-bound: new trace counter
      "va decode sync" (`trace::DEC_SYNC`, around `ZeroCopyAccel::read`'s sync) ≈ 70 s of 77 s
      wall, and it decodes 2400 pictures for 1798 frames (GOP seeks 22, evicted 2127: ~33 % decoded
      twice). So d = a faster single lane. The re-decodes are mostly inherent: 22 GOP seeks × ~27
      pictures ≈ the ~600 extra (cuts entering clips mid-GOP decode from the GOP start), so not a
      cache bug. **Tried: two chunk lanes both on the RX 6700 XT** (diagnostic
      `FILMCRAFT_EXPORT_SECOND_NODE=/dev/dri/renderD129` + `FILMCRAFT_BENCH_ENGINES=dual`, bench
      `bench-u023b-d2`): 4K graded 18.5 fps (lanes 8.6 + 9.9), worse than one lane (23); ungraded
      22.3 (lane 2 took no chunks). **Reading: AMD alone is VCN-bound** — one media engine does both
      decode (~29 ms sync per picture, 2400 pictures) and encode (~28 ms per frame), ≈ the 77 s wall;
      graded or not makes no difference. **Split chosen from data (2026-10-13, no designer needed):** Intel
      decodes (69.7 fps at 6.2K vs AMD 35.4), AMD composites and encodes (Intel encodes at only 18 fps).
      **Built (`22d348fe`):** 4:2:0 hand-off: `hw::set_thread_decode_handoff(Some(node))` (or the
      diagnostic `FILMCRAFT_DECODE_NODE=/dev/dri/renderD128` for threads without a node) makes
      `ZeroCopyAccel` decode on that node and copy each picture into linear NV12 / P010 memory the
      compositor imports (`va::linear_layout`, `linear_width`: 256-byte rows; mapped reads of planar
      linear surfaces); linear copies are described at their visible size (`VaPicture::visible`: the
      right-most column used to take chroma from the padding, 4:2:2 too). Test `tests/va_420_handoff.rs`
      (bit-exact + drawn in place = read back, Intel → AMD). Also fixed: an up-front `Unsupported`
      refusal no longer stops a 4:2:2 decoder (the U0.2.3 review fix was only a comment;
      `va_hevc_422` was failing). **Measured (`docs/fork-benchmarks.md` "Intel decodes, AMD renders and
      encodes"):** 4K graded single 24.7 / 25.0 fps vs 24.9 decoding on AMD: AMD's encode wait fell
      62 → 13.6 s but the serial decode chain now spends ~12 ms per picture in Intel's decode sync and
      **~20 ms in the hand-off copy** (trace "va hand-off copy", `trace::DEC_COPY`), although one copy
      alone takes 4.3 ms (ignored test `va_accel::tests::handoff_copy_time`, `--release`).
      **Later the same run:** the copy's time is its sync (trace "copy sync", lock wait 0); handing
      the copy out unsynced (first reader waits) was **tried and reverted** (22.7 fps: the wait moved
      into the compositor's GPU lock and Intel's decode slowed, decode and copy share the iGPU). Inside
      the export Intel's decode + copy reach ~30 pictures/s at 4K vs ~100 alone (ignored test
      `handed_off_4k_decode_time` in `tests/va_420_handoff.rs`), details in fork-benchmarks.
      **iGPU clock checked (not it):** `/sys/class/drm/card1/gt_act_freq_mhz` during the export: median
      1100 of 1300 MHz. **Explained:** `xm5-export-4k` scales the **6.2K** clip, so Intel decodes
      and copies 6240×4160 P010 pictures (78 MB each: copy ~13 ms + decode ~12 ms, serial ≈ the ~30
      pictures/s measured; `docs/fork-benchmarks.md`). **Next (design; Opus designer when the mode
      allows):** (1) a *scaling* hand-off copy: when the export frame is smaller than the source and
      the clip is a plain fit-to-frame (no scale / crop / position the compositor must apply at full
      size), the VPP copy scales to the export size (frame carries its new size, PAR kept): 2.6×
      fewer bytes at 4K; re-measure 4K graded with Intel decode. **Platform half done
      (2026-10-13 17:14):** `LinearCopy::copy_scaled` / `Context::process_scaled` (VPP scales the
      region to the copier's size; test `scaled_handoff_copy_keeps_values`; ignored timing test
      `handoff_scaled_copy_time`, run alone): Intel 6.2K P010 held copy 14.5 ms plain vs **7.6 ms**
      scaled to 3240×2160 (AMD 1.1 / 0.4 ms). Worth it: **next is the plumbing**: a thread hint
      (like `set_thread_decode_handoff`) carrying the export's target size from the export lane to
      `ZeroCopyAccel::hand_off`, set only for a plain fit-to-frame clip (no scale / crop / position /
      effects that need full size); the frame reports the scaled size, PAR kept; re-measure 4K graded.
      **Design settled (2026-10-13, self, MINIMAL):** the renderer already asks for reduced frames
      (`FrameRequest.scale` = `want`, from the Motion matrix; Maximum Render Quality asks 1.0) and
      the contract is "sources may return a larger frame", never smaller; the GPU input path
      already resamples a 6.2K frame to the drawn size (`plan.rs` ~l.520), so a smaller frame
      needs no renderer change. Decoders of the main pool are shared with preview, so the scale
      is fixed **per source, only in a `MediaPool::lane_copy` pool** (export-only decoders; the
      Intel-decode AMD lane needs its own decoders anyway): the export computes per media item
      `s = max over every clip using it of its static Motion scale` (1.0 when any clip has
      keyframed Motion, TNR — `tnr.rs:103` needs `source_size` frames — a nest, a crop, or
      Maximum Render Quality); the lane pool sets it as a thread hint around opening that source
      (check whether `ZeroCopyAccel` is made at open or lazily at the first decode); `hand_off`
      makes its copier at `round(crop × s)` (even sizes) and `frame()` reports that size.
      **Built (2026-10-13 17:51), diagnostic only:** `hw::set_thread_handoff_scale` /
      `hw::handoff_scale` (env `FILMCRAFT_HANDOFF_SCALE`), `ZeroCopyAccel` hands off at
      `scaled_size(crop, s)` (falls back to unscaled when copies cannot be mapped), VPP filter
      `FILMCRAFT_HANDOFF_FILTER` (makes no difference on iHD). Tests `scaled_sizes_are_even_and_bounded`,
      `scaled_handoff_gives_smaller_pictures_of_the_same_content` (0.5 codes vs reference 2×2).
      **Measured (full length, synthetic, `bench-u023b-d4/d5`, Intel decode, scale 0.5192308):**
      4K graded 37.3 / 39.6 / 37.8 / 37.1 / 38.7 fps (unscaled 24.7), 4K ungraded 41.8 (AMD 26.0).
      Unscaled hand-off is bit-identical to AMD decode; scaled differs in fine texture only
      (PSNR 42.6 dB ungraded, 32.9 graded): **owner question Q20, choice (a) use it**.
      **Next:** the plumbing per the design above (per-source scale in `MediaPool::lane_copy` pools,
      set as the thread hint inside `Mp4Video::make_decoder`, since decoders are made lazily by
      the GOP cache), plus Auto's graded plan: Intel decode + AMD lane (`set_thread_decode_handoff`
      in `VaLanes::bind_thread`). Then step e. **Plumbing part 1 done (2026-10-13 17:52):**
      `MediaSource::set_handoff_scale` (default no-op), `Mp4Source` stores it and sets
      `hw::with_handoff_scale` around `make_decoder` (test `handoff_scale_is_scoped_and_sane`).
      **Next, part 2:** `MediaPool` gets a per-`ItemId` scale map used only by lane copies
      (`resolve` calls `set_handoff_scale` on a newly opened source), a render-crate function
      computing the map for an export (rules above), and the export lane that decodes on Intel
      asks its `lane_copy` for it. **Part 2 done (2026-10-13 18:02):**
      `filmcraft_render::handoff::handoff_scales(project, seq, opts)` (3 tests: fitted 6.2K in 4K →
      2160/4160, largest use wins, keyframed Motion / TNR / nests / Maximum Render Quality → full
      size), `SourceProvider::lane_copy_scaled` (default = `lane_copy`), `MediaPool::lane_copy_scaled`
      (sane scales < 1 only, capped 4096; `resolve` calls `set_handoff_scale` on a newly opened
      file; test `scaled_lane_copies_keep_sane_scales_only`). **Next, part 3 (Auto's graded plan):**
      in `export/src/lanes/lane.rs::start`, when `auto_keeps_one_gpu` says one GPU and a second
      node decodes the source codec with a video processor, run **one lane** on the primary GPU
      whose decoders run on the Intel node (`LaneDevices` gets a "bind thread, decode on `n`, hand
      off" method → `hw::set_thread_decode_handoff`; `LaneCtx` a `decode_on`), sources from
      `lane_copy_scaled(handoff_scales(...))`. Check first that `Coordinator` and `chunker::plan`
      work with one lane (slow-lane rule, joiner, in-band). Report row "Intel decodes". Then a
      full-length bench (graded 4K / 6.2K, ungraded) and step e.
      **Part 3 done (2026-10-13 18:21):** `lane::start` → `decode_only_plan` (Auto + effect stage +
      `handoff_scales` not empty + not `FILMCRAFT_HANDOFF_SCALE≥1`) runs **one lane** (node None, all
      threads) whose workers `LaneDevices::bind_thread_handoff(second)` (→ `set_thread_decode_handoff`),
      sources `lane_copy_scaled`; `Dual::decode_only` carries the scales; report note "one GPU renders
      and encodes, the other decodes (hand-off at 62 %)"; a failed lane falls back to the single path
      (`lanes_failed`). Test `the_second_gpu_decodes_only_for_sources_drawn_smaller`. Guide
      `export.md` updated. **Measured (full length, plain Auto, `bench-u023b-d7`):** 4K graded
      **30.2 / 30.6 / 30.8 fps** (was 24.7); the bench clip has a 120 % zoom, so the scale is 62 %
      (the 37–40 diagnostic used 52 %, too small: Q20 corrected). 64 s chunks tried: no change
      (30.9), reverted. PSNR vs single-chip 33.8 dB graded (texture). `va_dual_lanes` 2/2 and
      `va_dual_export` 1/1 pass (owed iv done). **Owed:** Opus review of parts 1–3 (FFI-adjacent
      VA hand-off + lane threads) when the mode allows; 6.2K graded and ungraded unaffected (6.2K
      export: no scaling → no decode-only plan). **Next: step e** (full-length before/after bench,
      median of 2, idle machine), then U0.2.4. (2) If that doesn't reach ≥ 30 (or
      for 6.2K exports, where it can't help), accept: log the gap (owner question, FYI), keep the
      hand-off code (tested, diagnostic env) and go to step e
      (full-length before / after bench) then U0.2.4. Otherwise decide Auto's graded plan (Intel
      decode + AMD lane when an Intel node decodes the stream; wire `set_thread_decode_handoff` into
      `VaLanes::bind_thread` for
      the AMD lane). Risky code (`crates/platform` VA): Opus review when the mode allows. Older notes:
      the GOP re-decodes (~25–33 %) are mostly inherent (cuts entering mid-GOP).
   e. Output unchanged throughout (bit-exact or within the existing AMD / Intel parity tolerance).
      Full-length bench, idle machine, median of 2: 4K graded ≥ 30, 6.2K graded aim ≥ 24, 4K ≥ 30,
      6.2K ≥ 24 (not slower). (The two 4:2:2 cases of §1a.68 moved out of
      U0.2.3b to their own step X422 right after AI1, §1a.69.) Record before / after in `docs/fork-benchmarks.md`; then close
      U0.2.3b. A lever short of its share in budget: keep the gain, log the gap in OWNER-QUESTIONS,
      continue with U0.2.4.
   Saved binaries: `target/fork-bench/bench-phase6e`, `bench-u022`, `bench-u023`, `bench-u023b`;
   a worktree-built bench needs its `target/fixtures` symlinked to the main one.
5. **Next: U0.2.4** (part 1 done 2026-10-13: `render_nodes()` moved to `va.rs`, callers use
   `crate::va::render_nodes`; remaining: upstream's `Session` / `Display` / `shared()` under the
   front ends replaced by the fork's `va::Display`, PREP §3.2 item 4. **Part 2 done:** the
   read-back `VaDecoder::new` is `ZeroCopyAccel::new_read_back` (pictures read back with
   `read_back`; HEVC conceals missing references like the zero-copy path, shared `hevc_front`);
   the `Front::H264` / `Front::Hevc` Session variants are gone; `vaapi`, `va_native`,
   `va_hevc_422`, `vaapi_displays` pass on AMD, `vaapi` + `va_native` on Intel. **Part 3 next:**
   delete upstream's `Session`, `Display`, `shared()`, `replace_shared`, `displays_opened` from
   `vaapi/va.rs` (keep what the front ends / `ffi.rs` still need), `vaapi::va::probe()` callers
   (`lib.rs:152`) → `va::Display::get()`, update `tests/vaapi_displays.rs`. **Part 3 done
   (2026-10-13): U0.2.4 complete.** Upstream's `vaapi/va.rs` deleted (its own libva loader,
   `Display`, `shared()`, `Session`, `probe`); `crate::va::displays_opened()` counts the fork's
   displays; `tests/vaapi_displays.rs` asserts at most one per render node; startup log reads the
   fork display. Platform tests all green (AMD; `vaapi` + `va_native` on Intel after part 2);
   `cargo check --workspace --all-targets` clean. **Next: U0.2.5** (below).)
   **U0.2.5 removals done (2026-10-13):** `va_hevc.rs`, `crates/hevc/src/accel.rs` + its test
   deleted; `ui/va_interop.rs` re-pointed to `VaDecoder::zero_copy` (2/2 pass, drawn, not
   skipped); ADR 0002 / 0005 and FORK-ONLY.md updated. The factory already is one in-process
   entry (zero-copy → read-back) → pipe. **ci run 1 (2026-10-13 19:20–20:05): 6 failures** = a
   crash: `filmcraft-cli help` aborted (heap corruption) in ~25 % of runs. Cause (core dumps): the
   short command exited while the 4:2:2 warm probe was inside Intel's `vaInitialize`, so the
   driver's exit handlers ran under it; upstream's always-open display (removed in U0.2.4) had
   hidden it. **Fixed:** every VA display the fork opens is kept for the process's life
   (`Display::open` → `KEPT`), opens are serialised, VA set-up work is counted (`va::Busy`), and
   `filmcraft_platform::quiesce()` (CLI exits, app exit; plus an `atexit` fallback) waits for it
   (≤ 3 s). 0 / 300 runs fail; regression test `short_commands_exit_cleanly_while_hardware_probes_run`
   fails without the fix. `teardown_ui` failed once under ci load, passes alone (3 / 3): flake.
   **Remaining for U0.2.5:** `cargo xtask ci`, tag
   `upstream-sync-2`, phase end (§1a.9 build, §1a.33 publish `fork-upstream-sync-2`, HANDOFF
   rewrite for A1→… per §1a.6; check §1a.69: AI1 comes right after U0.2). (one display layer; `render_nodes` out of
   `ffmpeg.rs`), U0.2.5 (chain cleanup, removals, FORK-ONLY.md / ADR updates, `cargo xtask ci`,
   tag `upstream-sync-2`, with L1 / L2 part 2 if the lane is ready: `linux-1b` / `linux-2b`).
   Bindings: take PREP §3.2 item 5's recommendation **B** (keep `ffi.rs` for the front ends'
   buffers, `va_sys` for the rest) unless a step proves it wrong; log the choice.

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
