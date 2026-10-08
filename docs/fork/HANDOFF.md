# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

Read `FORK-PLAN.md` first (§1 fork rules, §1a standing instructions, §7 progress log), then this.
Keep this file current at the end of every session.

## Where we are

- **Phase:** 1 (quick wins on the ffmpeg path), being finished in session 1 (2026-10-08).
- **Branch:** `hevc-vaapi`. Commits: `da32409` plan, `5d5b9a8` F0 (baseline). Phase 1 changes
  are being committed now (see §7 of the plan for the latest).
- **Next step:** see "Exact next step" at the bottom.

## Done

- Phase 0: system recorded, synthetic fixtures, `xm5` bench section, baseline numbers
  (`docs/fork-benchmarks.md`), the "59 fps" explanation.
- Phase 1 (in progress at the time of writing):
  - 1.1 `PixelData::Yuv16Bi` (P010 / P210, MSB-aligned `y` + interleaved `uv`), CPU paths via
    `VideoFrame::to_planar()`; VA-API reader fills it straight from the pipe.
  - 1.2 10-bit planes upload as `R16Unorm` / `Rg16Unorm` when the device has
    `TEXTURE_FORMAT_16BIT_NORM` (app: `filmcraft_ui_egui::wgpu_configuration()`; bench and tests
    request it too); half floats stay the fallback. Fixed an upstream GPU bug on the way: 10/12-bit
    limited-range offsets were scaled by 2^bits/255 instead of 2^(bits-8) (≈ 1–2 % dim).
  - 1.3 pooled buffers read whole (`pool::take_u16_filled`, no re-zeroing).
  - 1.4 `gop::auto_limits` from installed RAM (budget RAM/5 in 1–16 GiB, decoders RAM/8 GiB in
    2–8), per-cache floor 24–64 frames by size (was 64 always = 5 GB at 6.2K), Settings ▸ Memory
    ▸ "Decoded source frames (0 = Auto)" (`memory.decodeCacheMb`), `perf.stats`
    `decode.cacheBudgetMB` / `maxLiveDecoders`.

## Decisions in force

All of `FORK-PLAN.md` §1 and §1a (owner's). Agent decisions so far, all logged in §7:
- Phase codes in commit messages: `F0:`, `F1:` … (CLAUDE.md asks for a task id per commit).
- Pre-existing Linux CI failures fixed (shortcuts test assumed the Mac keymap; GPU 1:1 sampling
  snap in `composite.wgsl`).
- Phase 1 acceptance judged against the ffmpeg pipe ceiling (§1a.5, agent's interpretation).

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
  automation (`/loop`, timers running `claude`); the owner starts those.

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

(Updated at the end of the session.)
