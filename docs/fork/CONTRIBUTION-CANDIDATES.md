# Contribution candidates vs upstream main

Checked against upstream `78fe010` (2026-10-11), on 2026-10-12 (cloud, REVIEW-1). Method: read upstream's
code with `git show` / `git grep` on `refs/remotes/upstream-ro/main`, compared with the fork's code and
FORK-PLAN.md §7. No code is copied here.

| Candidate | Upstream status | Recommendation |
|---|---|---|
| 1a. GOP-cache memory bound (`MIN_FRAMES`) | Not in upstream | Still a candidate (small PR) |
| 1b. Store-time eviction (black frame) fix | Partly: upstream evicts around the requested frame | Check, probably drop |
| 2. `FrameServer` drop fix | Not in upstream | Still a candidate |
| 3. 1.6x anamorphic PAR | Not in upstream | Still a candidate |
| 4. `effects.move` | Not in upstream | Still a candidate |
| 5. Lanczos-3 for Maximum Render Quality | Not in upstream | Still a candidate (needs GPU part trimmed) |
| 6. Sequence presets | Not in upstream | Still a candidate |
| 7a. Zero-copy VA-API to Vulkan DMA-BUF display | Not in upstream | Needs maintainers first |
| 7b. GPU ColorOps, DWG / DI, all of Lumetri on GPU | Partly: Basic / Creative / Vignette only | Needs maintainers first |
| 7c. GPU scopes | Not in upstream | Needs maintainers first |
| 7d. Node grader (`grade.*`) | Not in upstream | Needs maintainers first |

## 1. GOP cache

- Upstream `crates/codecs/src/gop.rs` still has `MIN_FRAMES = 64`. The store function sizes its budget as
  the larger of the byte budget and 64 times one frame's size, so a cache in use holds 64 frames of any
  size (about 5 GB at 6.2K). Nothing bounds that in bytes.
- The fork changed it: `MIN_FRAMES_BYTES` (1 GiB) and `MIN_FRAMES_LARGE` (24) give a frame count
  between 24 and 64 depending on frame size (`gop.rs` around lines 649-663 in the fork).
- Eviction: upstream's store function already evicts the frame furthest from the request (`pin`), not
  the newest decode, with a comment about frame-threaded decoders bursting past the playhead. That
  looks like the same black-frame / wanted-picture-evicted problem. Before proposing, diff the fork's
  store-time fix against this; likely only the byte bound remains.
- Recommendation: 1a is a small, focused PR. 1b: drop unless the diff shows a case upstream misses.

## 2. FrameServer drop

- Upstream `crates/ui-egui/src/frames.rs` defines `FrameServer` with worker threads but has no
  `impl Drop` for it (grep finds none). A dropped server keeps its workers, caches and GPU surfaces alive.
- Fork: `impl Drop for FrameServer` in the same file (about line 1141) stops the workers and empties the caches.
- Recommendation: still a candidate. Matters mostly for tests and benches that make many servers.

## 3. 1.6x anamorphic PAR

- Upstream `PARS` in `crates/ui-egui/src/panels/export_mode.rs` has 6 entries and tops out at
  "Anamorphic 2:1 (2.0)". No 8:5 entry anywhere; `crates/project/src/lib.rs` has no pixel-aspect list
  with it either.
- Fork: `anamorphic-1.6` "Anamorphic 1.6x (1.6)" (8:5) in the shared list in `crates/project/src/lib.rs`
  (about line 179), feeding the Export dialog.
- Note: upstream already has anamorphic handling (`par_tests.rs` in engine and render, 4:3 and 2:1),
  so the PR is just the extra list entry plus translations (i18n `.tsv` files list labels).
- Recommendation: still a candidate, smallest of all.

## 4. `effects.move`

- Upstream has `effects.apply`, `effects.remove`, `effects.reset`, `effects.moveKeyframe(s)` in
  `crates/engine/src/commands.rs`. No command that reorders the effect stack.
- Fork: `effects.move` (`8ca59ba`, G0.5) with a test `effects_move_reorders_standard_effects_only` in
  `crates/engine/src/clip_ops_tests.rs`; one undo step, never moves past the intrinsic effects.
- Recommendation: still a candidate. Check whether upstream's Effect Controls has drag-reorder using
  another path before opening.

## 5. Lanczos-3 for Maximum Render Quality

- Upstream stores `max_render_quality` (`crates/project/src/lib.rs`, export settings) and uses it in
  `crates/export/src/pipeline.rs` only to render at full size and then scale with "an area filter"
  (doc comment in `crates/export/src/lib.rs`). The only Lanczos in upstream is the app icon in `brand.rs`.
- Fork: `Image::transformed_hq` (CPU) and `layer_color_hq` in `composite.wgsl` (GPU), used by export and
  the paused Program monitor.
- Recommendation: still a candidate. The CPU side fits upstream's layering as is; the GPU shader part
  touches `composite.wgsl`, which upstream changes often, so rebase carefully and consider two PRs.

## 6. Sequence presets

- Upstream has no sequence presets. `crates/engine/src/presets.rs` is effect presets (`presets.apply`),
  and "preset" in `commands.rs` is the export preset name. `file.newSequence` takes explicit settings.
- Fork: `file.newSequence {preset}` and list `sequence.presets` in `crates/engine/src/commands.rs`
  (UHD, vertical, plus X-M5 entries in DaVinci Wide Gamut).
- Recommendation: still a candidate, but drop the `xm5-*` camera-specific presets and the DWG ones, which
  depend on fork-only color work. Keep the generic UHD / vertical ones.

## 7. Larger items (talk to maintainers first)

- **Zero-copy VA-API to Vulkan DMA-BUF display.** Upstream has VA-API decode in `crates/platform/src/vaapi/`
  (read back to the CPU) and `docs/gaps.md` / `docs/hardware-parity.md` still list zero-copy as missing.
  No DMA-BUF code in `crates/gpu`. The fork's `crates/gpu/src/interop.rs` needs `unsafe` in the gpu crate
  plus an ADR; that conflicts with upstream's "unsafe only in `crates/platform`" rule, so ask first.
- **GPU ColorOps / DWG / DI / Lumetri.** Upstream's GPU path (`crates/gpu/src/fx.rs`, `fx.wgsl`,
  `OP_LUMETRI`) covers Lumetri Basic, Creative and Vignette (UPSTREAM-PLAN: GPU3-GPU7). No Curves, Wheels,
  HSL secondary or composable ops, and no DaVinci Wide Gamut / Intermediate space in `crates/color`
  (its `grade.rs` is the grading-signal definition used by the CPU path). The fork rebuilt the same area, so a
  merge is a redesign, not an add-on.
- **GPU scopes.** Upstream scopes (`crates/scopes`, `crates/engine/src/scopes.rs`) are CPU; no scope code in
  `crates/gpu`. Fork: `filmcraft_gpu::scopes`.
- **Node grader (`grade.*`).** No `grade.*` commands in upstream and no node-based grading in docs.
- **Local AI models (checked 2026-10-12).** Upstream already ships local, pure-Rust candle 0.9 inference
  (`crates/speech`: Whisper behind an off-by-default `whisper` feature, on-demand checksummed download;
  Kokoro TTS) and lists model-based features as wanted (`docs/gaps.md` G12: Object Mask #406 / #694,
  model-based Enhance Speech, auto colour). So the fork's AI work fits upstream if it follows the same
  pattern: optional feature, off by default, models downloaded on demand, never bundled. Best fits:
  **GRADE-PLAN G3 Magic Mask** (= upstream's Object Mask) and **AUDIO-PLAN A2 voice isolation** (= their
  model-based Enhance Speech). The Grading Coach / explainer LLM (`crates/llm`) is a bigger scope ask (no
  LLM upstream yet) and must drop the fork's `x86-64-v3` build flag (OWNER-QUESTIONS Q8): gate candle's
  quantized kernels at run time or behind the feature. The FilmCraft-tuned explainer weights need a
  stable public host and a licence note (derived from Apache-2.0 bases).

## Items UPSTREAM-PLAN says are already fixed upstream

Spot-check, only the open-GOP one: upstream `gop.rs` has open-GOP support (leading pictures of an open GOP,
tests "open gop" with an iPhone-HEVC-style stream, around lines 127, 1447, 1512). `load4` exists in upstream
`composite.wgsl` (line 53) and was not checked for the snap fix. The Linux keymap fix was not checked.
