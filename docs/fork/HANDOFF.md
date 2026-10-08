# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

Read `FORK-PLAN.md` first (§1 fork rules, §1a standing instructions, §7 progress log), then this.
Keep this file current at the end of every session.

## Where we are

- **Phase 1 done** (`bc2771c`, tag `phase-1`, build in `~/Apps/filmcraft-builds/phase-1/`,
  launcher "FilmCraft (fork)"). **Now: Phase 2** (in-process zero-copy hardware decode),
  2.1–2.4 done (in-process VA-API, zero-copy display, VRAM budget); now Phase 2 acceptance items.
- **Branch:** `hevc-vaapi`. Commits: `da32409` plan, `5d5b9a8` F0, `9e0b3f3` handoff files,
  `bc2771c` F1, then the Phase 8 plan change.

## Done

- Phase 0: system recorded, synthetic fixtures, `xm5` bench section, baseline
  (`docs/fork-benchmarks.md`), the "59 fps" explanation.
- Phase 1: `PixelData::Yuv16Bi` (P010/P210) read straight from the ffmpeg pipe; 10-bit planes
  uploaded as `R16Unorm`/`Rg16Unorm` (`filmcraft_gpu::wanted_features()`, app via
  `filmcraft_ui_egui::wgpu_configuration()`); GPU 10/12-bit range bug fixed; RAM-scaled decode
  cache (`gop::auto_limits`, Settings ▸ Memory ▸ Decoded source frames). Quick pass: CPU/frame
  −59 to −66 %; 6.2K still ~20 fps (pipe ceiling ≈ 22 fps).

## Decisions in force

All of `FORK-PLAN.md` §1 and §1a (owner's). Key current ones: never ask, no sudo, don't touch
master; synthetic footage, mark results "synthetic"; quick bench passes except Phase 2/4/5/7
acceptance; after every working step commit + progress log + this file; build at each phase
boundary; keep going through phases until asked to stop. **Phase 2/5: only the primary hardware
backend** (VA-API unless Vulkan Video is ≥ 20 % faster and matches; decided by the 2.1 ffmpeg
comparison), behind an interface ready for the other backend (Phase 8) and NVIDIA later;
fallback primary → ffmpeg pipe → software.

Agent decisions (logged in §7): commit prefixes `F0:`, `F1:` …; Phase 1 accepted against the
pipe ceiling (§1a.5, the owner's rule text never arrived; interpretation recorded); mixer perf
test floor 2.5× on this CPU.

## Environment notes and gotchas

- Rust 1.99 via rustup in `~/.cargo` (not pacman): `export PATH="$HOME/.cargo/bin:$PATH"`.
- Bash tool shell is **zsh**: unquoted globs that match nothing are errors (`--include='*.rs'`
  needs quotes); no `bc` (use python3); `set -- $var` does not split.
- `pgrep -f "<pattern>"` inside `bash -c '…'` matches its own command line: use `pgrep -x cargo`.
- Foreground tool commands time out at 10 min: run long things with `run_in_background` and wait
  with `timeout 590 bash -c 'until …; do sleep 10; done'`.
- GPUs: AMD RX 6700 XT = `/dev/dri/renderD129`, Vulkan GPU0; Intel UHD 750 iGPU =
  `renderD128`, Vulkan GPU1. Force one for wgpu tests: `MESA_VK_DEVICE_SELECT=1002:73df
  MESA_VK_DEVICE_SELECT_FORCE_DEFAULT_DEVICE=1` (AMD) or `8086:4c8a` (Intel). Run GPU parity
  tests on both. VA-API device override: `FILMCRAFT_VAAPI_DEVICE`; ffmpeg: `FILMCRAFT_FFMPEG`.
- Installed: Mesa 26.2.2, libva 2.24.1, vainfo, vulkaninfo, ffmpeg n9.0.1, kernel 7.2.4-cachyos.
  No `perf`, no `bc`.
- Disk: `/home` is ~90 % full (≈ 30 GB free); `target/` is ~90 GB. Deleting
  `target/debug/incremental` is safe. Fixtures take 12 GB in `target/fixtures/xm5/`.
- `cargo xtask ci` runs tests in release; a full workspace test takes ~30 min. Run CI on an idle
  machine: `mixer_tests::perf_24_tracks_3_effects_realtime_factor` needs ≥ 4× real time on one core
  (4.7× idle, 3.0× while x265 or a build runs). Use `cargo test --workspace --release
  --no-fail-fast` to see every failure.
- Bench: `target/release/examples/bench --section xm5 [--only CASE] [--quick] --json out.json`
  (cases: ceilings, decode, play-full, play-graded, jkl, scrub, multicut, export, 422-play).
  A full run takes ~65 min (the graded 4K export alone ~41 min at baseline). The Phase 0 binary is
  kept as `target/fork-bench/bench-phase0`; results in `target/bench/xm5-phase0.json`.
- The auto-mode permission classifier blocks the agent from setting up self-restarting
  automation (`/loop`, timers running `claude`); the owner starts those (a `/loop` line inside a
  longer message does not activate it).
- Builds: `~/Apps/filmcraft-builds/phase-N/`, `latest` symlink, icon
  `~/Apps/filmcraft-builds/ai.storyteller.filmcraft.png`, launcher
  `~/.local/share/applications/filmcraft-fork.desktop` (exists; don't recreate). App data for the
  fork build: `~/.local/share/filmcraft-fork`. Smoke test: run with `--control 9876 --data-dir
  <scratch>`, JSON lines `{"id":1,"method":"perf.stats","params":{}}`, `ui.screenshot`, `app.quit`.
- Phase 2 groundwork: wgpu-hal 30 `vulkan::Adapter::open_with_callback` can add device
  extensions and queue create infos (video decode family 2 on RADV) before wgpu wraps the device;
  `texture_from_raw` wraps a `vk::Image`; eframe accepts our device via
  `egui_wgpu::WgpuSetup::Existing`. `crates/hevc` has SPS/PPS/slice-header/RPS parsers to fill
  VA-API / Vulkan Video parameter structures. `cros-libva` (BSD-3) links libva at build time;
  loading libva / Vulkan at runtime (`libloading`, `ash`) keeps the app starting without them.
- Phase 2.1 comparison script: `vkcheck.py` in the session scratchpad (lost between sessions;
  re-create: 3 timed `-f null` runs per backend + `framemd5` of 60 downloaded P010 frames vs
  software decode).

## Open problems

- The playback scheduler shows **nothing** (not fewer frames) when decode is slower than real
  time: every job ~550 ms late is cancelled (Phase 3 item; also hides Phase 1/2 gains in
  `play-*` numbers until decode is faster than real time).
- The ffmpeg pipe caps 6.2K P010 at ~22 fps (ffmpeg's own download + write is serial).
- 4:2:2 HEVC has no decoder at all on Linux (FilmCraft's HEVC decoder is 4:2:0 only; VCN3 has
  none): Phase 6.1 must add the ffmpeg software path.
- Export renders on the CPU: 4K graded export 0.7 fps with a 23 GB RSS peak (Phases 4–5).
- Multicut peaked at 11.3 GB VRAM of 12 at baseline (four ffmpeg VA-API processes).
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T); not fixed.

## Exact next step

Phase 2 acceptance work (zero-copy decode → display is done and committed; see §7):
1. `xm5-multicut` drops ~24/360 at cuts: the next clip must be decoded from its keyframe to the
   cut point before the playhead arrives; look at how `FrameServer::schedule_playback` prefetches
   across a cut and give the next clip's first frame enough lead (GPU decode is cheap: ~24 ms per
   picture, up to 29 pictures from the keyframe).
2. `xm5-scrub` p95 must be ≤ 1.5× the time to decode one GOP (≈ 30 pictures at 41 fps ≈ 0.73 s →
   p95 ≤ 1.1 s; now 1.54 s).
3. Fallback with the hardware path pulled (Settings Off; `FILMCRAFT_VAAPI_DEVICE=/dev/null`; libva
   missing) and a mid-stream failure (`NativeVaDecoder::fail_after`) → ffmpeg pipe → software.
4. 2-hour playback soak without VRAM/RAM growth (control channel or bench loop).
5. Full `xm5` suite (Phase 2 acceptance is a full-suite phase), `cargo xtask ci`, then the Phase 2
   build (tag phase-2, `~/Apps/filmcraft-builds/phase-2`, NOTES.md, latest symlink).
Useful: bench prints `VA surfaces` and `cache VRAM`; `FILMCRAFT_VRAM_CACHE_MB` overrides the VRAM
budget; `FILMCRAFT_NO_GPU_IMPORT` disables the Vulkan import (read-back path).
