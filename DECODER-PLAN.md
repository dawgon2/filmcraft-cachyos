# DECODER-PLAN.md: Phase D, a 4:2:2 HEVC decoder that beats ffmpeg on this machine

Written 2026-10-11 (session 8, with the owner). Scheduled **after Phase 7, before Phase 8** (owner):
… → C1 → C2 → 7 (+ Y4) → **D** → 8 → Y2 → H1 → H2 → H3 (FORK-PLAN §1a.6). All FORK-PLAN rules apply:
never crash, no `unsafe` outside `crates/platform` and `crates/gpu/src/interop.rs`, layering, pure
Rust, ffmpeg only as a test oracle and benchmark ceiling (§1a.24).

Starting point: Phase 6.1 gives `crates/hevc` correct Main 4:2:2 10 decoding with frame threading,
bit-exact against ffmpeg (test oracle only). The X-M5 records 4:2:2 occasionally; the RX 6700 XT's
VCN can't decode it (silicon), so it's CPU work today.

## What "better than ffmpeg" means here

- **Correctness:** identical, not better: a conforming decoder's output is fully defined, so the
  target is bit-exact on every conformance vector and fixture. Better *robustness*: fuzzed, never
  crashes, conceals damaged data instead of stopping.
- **Speed on this PC (i5-11600K + RX 6700 XT) at 6.2K 4:2:2 10-bit:** clearly faster than ffmpeg's
  software decoder, mainly by using the GPU's compute units, which ffmpeg's software path can't.
- **Editing-aware:** decode only what the editor needs, which a general-purpose decoder can't know.

## Stages

### D1: measure (≈ 2–4 h; machine idle, no builders compiling)
ffmpeg's software HEVC decode of the 6.2K 4:2:2 10-bit fixtures (`target/fixtures/xm5/`) on this CPU
with all threads (`-threads 0`, frame + slice threading), and the Phase 6.1 decoder, both to fps
and CPU %; a per-stage profile of ours (entropy decode, prediction, inverse transform, deblocking,
SAO, output conversion). Record in `docs/fork-benchmarks.md` "Phase D" (synthetic). This sets the
number to beat and decides which of D2–D4 matter most.

### D2: safe SIMD and threading on the CPU (≈ 10–20 h)
- Vectorise the hottest kernels found in D1 (inverse transforms, interpolation filters, deblocking,
  SAO, 10-bit output packing) with a **safe** SIMD library with runtime CPU dispatch (candidates:
  `pulp`, `wide`; an ADR picks one), so no `unsafe` in `crates/hevc`; scalar paths stay as the
  reference and fallback.
- Better parallelism: WPP / tile decode when the stream has it, frame threads with finer
  reference-row waits, a work-stealing pool shared with the rest of the media stack.
- Target: within ~1.2–1.5× of ffmpeg's CPU-only speed, bit-exact.

### D3: editing-aware decode (≈ 6–12 h)
- **Draft quality** for previews and proxy creation (optionally skip deblocking / SAO on
  non-reference pictures; clearly marked, never used for export or full-quality viewing).
- **Keyframe-only fast path** for thumbnails, skimming and scrub-ahead.
- **Cut-aware:** skip pictures that won't be shown (already done on the hardware path, Phase 3.1),
  decode-ahead priority from the playback scheduler (`hwgate`-style ordering).
- **Output in the GPU compositor's format** directly, no extra conversion pass.

### D4: GPU-assisted reconstruction (≈ 20–40 h; only if D1–D3 leave 6.2K 4:2:2 below real time)
- The CPU keeps the sequential part (CABAC entropy decoding, syntax, motion vectors); the GPU
  (wgpu compute shaders) does the parallel part: dequantisation and inverse transforms,
  intra / inter prediction (motion compensation with the 8 / 4-tap filters), deblocking and SAO,
  writing the reconstructed picture straight into a GPU texture the compositor uses (no read-back).
- Intra prediction's dependencies are handled in wavefronts; reference pictures stay on the GPU.
- **Bit-exact against the CPU path** on every fixture and conformance vector, on AMD and Intel;
  any mismatch, device loss or missing feature falls back to the CPU path automatically (never
  crash), with the reason in `perf.stats`.
- ADR for the design; Fable designer and reviewer throughout.

### D5: robustness (≈ 4–8 h, alongside)
Fuzz-style tests on mutated 4:2:2 streams (never crash, bounded memory), error concealment for
damaged slices (copy from the previous picture, logged), truncated-file handling.

## Accept
Bit-exact on the 4:2:2 conformance set and fixtures on every path (CPU scalar, CPU SIMD, GPU on AMD
and Intel); **6.2K 4:2:2 10-bit decode clearly faster than ffmpeg's software decoder on this PC**
(target: real time at 23.98 / 25 fps for playback at Full, measured and recorded, synthetic);
`xm5-422-play` without proxies drops nothing, or the remaining gap is documented; fuzz tests pass;
a 30-minute 4:2:2 soak through the control channel. `cargo xtask ci`, tag `decoder-1`, build per
§1a.9. If D4 isn't needed (D1–D3 already reach real time), record why and skip it.

## Estimate
D1 2–4 h · D2 10–20 h · D3 6–12 h · D4 20–40 h (conditional) · D5 4–8 h · total ≈ 40–80 h.
