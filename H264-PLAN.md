# H264-PLAN.md: complete H.264 decode without ffmpeg or proxies

Written 2026-10-11 (session 8). **A separate plan, run after the primary plan is finished**
(order: … → 7 → 8 → Y2 → **H1 → H2 → H3**, FORK-PLAN §1a.6). The owner wants H.264 fully supported, with hardware decode
wherever the GPU can do it and **no dependence on ffmpeg** (not everyone has it) **or on proxies**,
but nothing beyond what that needs. YOUTUBE-PLAN Y1 (≤ 8 h) already adds in-process VA-API decode
for progressive 8-bit 4:2:0; this plan covers the rest. All FORK-PLAN rules apply.

What the hardware can do (`vainfo`, 2026-10-11): the AMD VCN and the Intel UHD 750 both decode H.264
Constrained Baseline / Main / High, **8-bit 4:2:0 only**. Neither decodes High 10 or High 4:2:2 (the
10-bit modes of Sony XAVC S / S-I, Canon XF-AVC, Panasonic and others); no current AMD or Intel GPU
does, so FilmCraft must decode those itself. Today's software decoder (`crates/h264`) is
progressive 8-bit 4:2:0 only and single-threaded.

## Phase H1: interlaced H.264 on the hardware path (≈ 6–12 h)
Extend `crates/platform/src/va_h264.rs` (from Y1) to PAFF field pictures and MBAFF (e.g. AVCHD
1080i camcorders): field POCs, field reference lists, second-field pairing, the VA interlaced
picture flags; deinterlacing stays the existing display / render path's job. Test on AMD and
Intel. **Accept:** 1080i PAFF and MBAFF fixtures decode bit-exact against ffmpeg's output (used as a
test oracle only) on both GPUs and play with zero drops. Tag `h264-1`.

## Phase H2: the built-in software decoder covers everything in scope (≈ 25–45 h)
In `crates/h264`, pure Rust:
1. **High 10 and High 4:2:2** (8/10-bit 4:2:2, Intra and Long-GOP): bit depth up to 10; 4:2:2
   chroma intra prediction, chroma DC 2×4 transform, deblocking, motion compensation; output in
   the existing 16-bit planar frame type.
2. **Interlaced** (PAFF and MBAFF) in software, for machines without VA-API.
3. **Multithreading** (slice-level, then frame-level with reference-row waits) so 4K 10-bit
   4:2:2 is usable on the i5-11600K; record fps (synthetic).
4. Never crash on hostile streams (fuzz-style tests); allocation caps from the SPS.
**Accept:** bit-exact against the conformance vectors for every in-scope profile; 4K 10-bit 4:2:2
Long-GOP and Intra fixtures (XAVC S / S-I-like) play, fps recorded. Tag `h264-2`.

## Phase H3: ffmpeg-free H.264, confirmed (≈ 2–4 h; Phase 7.7 already removes ffmpeg from the product, FORK-PLAN §1a.24)
The decode chain is hardware (VA-API) → built-in software; the ffmpeg pipe is only an optional
extra (a Settings switch, off by default for H.264). Every supported file opens and plays with
ffmpeg absent, and the UI says which path is in use. **Accept:** the full H.264 fixture set plays
with ffmpeg removed from PATH (and the HEVC set still does); fallback with the hardware pulled
works. Tag `h264-3`.

## Not in scope
H.264 encoding changes, High 4:4:4 / CAVLC 4:4:4 Intra, SVC / MVC, NVIDIA NVDEC (the backend
interface stays ready for it, FORK-PLAN §1a.4).

## Estimate
H1 6–12 h · H2 25–45 h · H3 4–8 h · total ≈ 35–65 agent hours.
