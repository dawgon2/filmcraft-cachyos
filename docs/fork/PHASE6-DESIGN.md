# Phase 6 design notes (2026-10-11, scheduled run)

Read-only scouting (Sonnet) and a Fable design pass, written before any Phase 6 code. Line
numbers are as of `6f324786`; re-check before relying on them.

## 6.1 — 4:2:2 on the built-in HEVC decoder (`crates/hevc`): readiness

- **Gate:** `Sps::check_supported` rejects `chroma_format_idc != 1` (`hevc/src/params.rs:637`)
  and range extensions (:643). `sub_width_c` / `sub_height_c` / `crop_rect` already handle 4:2:2.
  The SPS / PPS range-extension flags are parsed only as one bool: store each flag
  (implicit/explicit RDPCM, extended precision, intra smoothing disabled, persistent Rice,
  CABAC bypass alignment; PPS chroma_qp_offset_list, log2_sao_offset_scale) and **check which ones
  the xm5 4:2:2 clip sets first** (x265 usually sets none).
- **Decode core is 4:2:0-only** (est. 600–1000 changed lines):
  - intra chroma mode 4 derivation: add the 4:2:2 remap table (spec Table 8-3), `slicedec.rs:881`;
  - transform tree / unit (`slicedec.rs:894`, `:932`): each chroma TB is two vertically stacked
    square blocks with a second cbf_cb / cbf_cr (ChromaArrayType == 2); intra prediction and
    residual run twice; 8×8 CU with 4×4 luma handles chroma at blkIdx 3 as two 4×4 blocks;
  - chroma QP: `qpc_420` (`tables.rs:61`, used in `slicedec.rs:628`, `filter.rs:178/220`) →
    `QpC = Min(qPi, 51)` for 4:2:2;
  - deblocking chroma edge grid (horizontal edges every 8 chroma rows, vertical every 8 chroma
    columns), SAO and PCM chroma sizes (`parse_sao` :473, `pcm_sample` :791);
  - MC: chroma block is `(xp/2, yp, w/2, h)` (`slicedec.rs:1447`); vertical MV in 1/4 units
    (fraction `(mv & 3) << 1`), horizontal 1/8 (`inter.rs` `mc_chroma`);
  - frame alloc / output (`picture.rs`, `decoder.rs:587-604`): full-height chroma planes.
- **Threading:** frame-level already (rayon, one job per picture, row-wise reference waits,
  `decoder.rs:481-503`). No WPP / tile parallelism (entry points parsed, unused). Frame threads
  likely suffice for long-GOP clips; WPP is a DECODER-PLAN D2 item.
- **Plumbing:** `codecs/src/video.rs` `HevcDecoder::convert` hardcodes `Chroma::C420` (:369,
  :374): pick `C422` when chroma height == height. Hardware rejects 4:2:2
  (`platform/src/vaapi/mod.rs:117`, `va_hevc.rs:61`), so the software factory is the path.
  `frame::Chroma::C422` / `PixelData::Yuv16` exist; renderer support for C422 is unverified.
- **Tests:** `hevc/tests/conformance.rs` (bit-exact vs ffmpeg `.yuv`, ~40 fixtures) and
  `tests/common/mod.rs` hardcode the 4:2:0 frame size. Add small x265 4:2:2 10-bit fixtures
  (`-pix_fmt yuv422p10le`, intra-only + long-GOP + B-frames + odd size) via `filmcraft_testkit`.

## 6.2 / 6.3 — proxies and render cache (Fable design)

**Reuse:** proxy transcode / attach / poll / ingest (`engine/src/proxies.rs`), media-pool proxy
switch + `ProxySource` scaling (`media_pool.rs`), export always full-res (`export_tools.rs:468`,
`previews.rs:639`), `PreviewStore` (content-hashed segments, parts `<hash>-<off>-<len>.mov`,
undo-safe invalidation), segment hash incl. grade and source identity (`render/src/preview.rs`),
per-frame preview swap in playback (`ui-egui/src/frames.rs:1260`), the in-process VA HEVC
encoder + zero-copy render-into, stepped `Exporter::step`, `cancel` pacing / hwgate (no-deadline
work served last).

**Bugs to fix first:** `hash_source` hashes `MediaClip.proxy` (`preview.rs:329`), so attaching a
proxy invalidates every preview; `transcode()` / `render()` default `gpu_rendering: Off`
(`export/src/lib.rs:577`) = CPU compositing at 6.2K; `PreviewStore::open` reads whole files
(`previews.rs:538`).

**Proxies (6.2):** preset `hevc_half` (half size, HEVC Main10 4:2:0, MOV/hvc1, VA in-process,
VBR max(8 Mbps, source/8), keyint 1 s, GPU rendering Auto, no audio: audio from the original).
`<stem>_Proxy.<fingerprint16>.mov` in `proxy_dir()`; identity mismatch → detach + requeue.
Auto-create on import / project open for 4:2:2 (always) and ≥4K long-GOP (pref, default on for
6.2K). New `engine/src/background.rs`: one worker, one job at a time (proxies before cache, 4:2:2
first), batches of 8 under `with_background(true)` + pacing; blocks while
`Activity{playing, scrubbing, export_running}` is set, resumes 500 ms after stop (watch the
VCN-burst GPU stall). Pref `media.proxies = auto|on|off`; auto = playing / prefetch / scrubbing
use proxies, a paused frame uses the original; `FrameKey` gets a proxy bit.

**Render cache (6.3):** key = `Segment::hash` without the proxy, HASH_VERSION → v2. Segments split
at every clip edge; long segments in 2 s parts, each a closed GOP (keyint = part, CQP 20, Main10
4:2:0, sequence size): the first frame after a cut is an IDR of a small file (1 picture instead of
a 30-picture decode burst). Pref `mediaCache.renderCacheGb` (20), `index.json` with last use;
evict stale hashes, then LRU. Renders when idle ≥ 1 s and no export, nearest the playhead first;
mode `smart|all|off`. Pre-open the next part during prefetch.

**Commands / UI:** `media.proxyMode`, `media.autoProxies`, `cache.setMode`, `cache.status`,
`cache.pause`, `cache.resume`, `cache.clearUnused`; Settings ▸ Media / Media Cache rows; media
badge (proxy present / building %), timeline bar state Building, status chip; es + zh-cn.

**Tests / bench:** hash stability on proxy attach; hash changes on grade / size / identity; part
naming; eviction; worker pauses while playing; 4:2:2 import auto-creates a proxy. Bench:
`xm5-multicut proxies|cached`, `xm5-jkl L 2x proxies|cached`, `xm5-422-play proxies`,
`xm5-cache-while-playing` (stall guard). Accept: 0 drops in all.

**Lanes:** A first (`engine/src/background.rs`, `Activity` hook in `ui-egui/src/lib.rs`,
`preview.rs` hash fix + v2, `hevc_half` preset in `proxies.rs`); then B proxies (`proxies.rs`,
`autosave.rs`, `media_pool.rs`, `relink.rs`, Settings Media rows, `project_views.rs`,
`commands.rs`, tests, bench) and C cache (`previews.rs`, tests, Settings Media Cache rows,
`timeline.rs`, `frames.rs`, bench) in parallel; they share only `settings.rs` / `xm5.rs`
(separate functions).

## 6.7 Temporal noise reduction (design, Fable designer, 2026-10-12)

**Facts found.** Spatial NR is `FxOp::Denoise` (`crates/render/src/denoise.rs`, `OP_DENOISE[_YCC]`
in `crates/gpu/src/fx.wgsl`); node params are a `BTreeMap` with identity-for-missing
(`crates/project/src/grade.rs`). GPU effects are per-frame; the CPU path already has temporal
access (`FxEnv::source_frame` / `base_layer`). `track.rs` has only sparse LK + RANSAC;
`crates/render/src/flow.rs` has a **dense pyramidal Lucas–Kanade** (640 px work width, forward /
backward, median, `agree()` guard): reuse that. Decoded frames are LRU-cached by pts in the GOP
cache, so t±k fetches are hits. Export and render-cache parts both go through `plan_frame` /
`render_sequence` (`crates/export/src/pipeline.rs`): one hook serves both.

**Neighbours.** The clip's frames at timeline `t ± k·frame` (k ≤ radius), clamped to the item;
`source_time_at` handles speed / reverse / holds; deduped by snapped source frame (holds /
slow motion collapse to fewer neighbours, down to identity). Never across a cut. Media / subclip
items only (nested / graphic / adjustment: op ignored). CPU: `base_layer(…, t_k, want)`. GPU:
`plan.rs push_item` fetches `clip_frame(src, item, t_k, want)` and attaches them to the op
(`FxOp::TemporalDenoise { luma, chroma, radius: u8, motion, inputs: Option<Arc<TemporalInputs>> }`).
Flow at ≤ 640 px luma, cached per adjacent pair both ways; ±2 by chaining
`f(t→t+2)(p) = f1(p) + f2(p + f1(p))`. GPU frames: small luma draw + ~1 MB read-back, cached per
frame.

**Algorithm (own code from published methods).** Motion: dense pyramidal LK (Lucas & Kanade
1981; Bouguet 2001). Warp: backward bilinear fetch at `p + f(t→k)(p)`. Blend: motion-compensated
temporal filtering (Dubois & Sabri 1984) with per-pixel range weights, a temporal bilateral
(Bennett & McMillan 2005) in the same encoded YCC space as spatial NR:
`w_k = exp(−Δ²/2σ²) · agree(fb_error) · inside`, Δ = 3×3 SAD of luma (per pixel for chroma),
σ = 0.05·strength, `agree` = forward–backward consistency (Kalal et al. 2010) scaled by
`tnr.motion`; out = (c_t + Σ w_k c_k) / (1 + Σ w_k). Luma / chroma separate; temporal runs before
spatial at the node's input.

**Data model.** Node params `tnr.luma` (0, 0–1), `tnr.chroma` (0, 0–1), `tnr.frames` (1, 1–2),
`tnr.motion` (0.5, 0–1); missing = identity (schema v16 unchanged). Existing
`grade.setParam` / `grade.get` / `grade.params`; Noise tab rows (ids `grade.param.tnr.*`).
Compiles in `grade.rs denoise()` to `FxOp::TemporalDenoise` before `Denoise`.
**Provisional (owner question Q9): v1 honours it on the first active node only.**
`RenderOptions.temporal` (default false): export (and so render-cache parts) true; frame server
true only for the paused on-screen frame; never while playing / scrubbing (the cache plays it).

**Tests.** Synthetic fixture (320×240, 7 frames, textured disc moving (3.2, −1.7) px/frame over a
textured background, encoded-domain Gaussian noise σ = 0.02, fixed seed). Accept: σ ratio ≤ 0.70
at ±1, ≤ 0.60 at ±2 (luma 1.0); ghost index (mean |out − clean| in disc(t±k) \ disc(t),
normalised by the naive mean) ≤ 0.25; edge 10–90 % width growth ≤ 0.5 px; static input = plain
mean within 1e-5. Hostile params / neighbours / flows; CPU vs GPU parity ≤ 1e-3 rel on AMD +
Intel; export = viewer; cache part = export frame; segment hash changes on a tnr edit.

**Estimated cost at 6.2K (RX 6700 XT, unmeasured):** ±2 → 25–40 ms GPU + 40–100 ms CPU flow per
frame; export ≈ 4–8 fps with TNR on (vs ~18); a 2 s cache part +5–10 s. Fallbacks: radius 1,
GPU LK later, Rgba16Float neighbours if VRAM is tight.

**Lanes.** A (kernel): `render/src/{flow.rs, tnr.rs, gpufx.rs}`, `gpu/src/{fx.rs, fx.wgsl,
fx_tests.rs}`. B (integration): `project/src/grade.rs`, `render/src/{grade.rs, lib.rs, plan.rs,
preview_tests.rs}`, `export/src/pipeline.rs`, `ui-egui/src/{frames.rs, panels/grade.rs,
i18n/*.tsv}`, `docs/guide/color/noise.md`, bench `export-tnr`, `cache-tnr`.
