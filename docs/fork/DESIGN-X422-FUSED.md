# Design note: fused graded passes (X422 step 2b, FORK-PLAN §1a.71)

Status: design, 2026-10-14 (scheduled run, MINIMAL mode, self-reviewed). Implementation follows in the
steps of §5, each its own commit with parity tests on the RX 6700 XT and the UHD 750.

## 1. Why

A graded 6.2K layer runs its effects as compute passes that ping-pong between `Rgba32Float` working
textures (`crates/gpu/src/fx.rs`). One pass at 6240 × 4160 reads ~415 MB and writes ~415 MB
(~830 MB of memory traffic). The bench grade (`crates/ui-egui/examples/bench/xm5.rs` `grade`:
F-Log2 input transform, Lumetri Basic + luma curve + HSL Secondary + vignette, Sharpen 40) costs
about 8 such passes plus the source draw and the final draw into the accumulator. The colour maths
is cheap next to that traffic on the AMD card, and on the Intel UHD 750 (shared DDR4) bandwidth is the
whole story. Aims (§1a.71, full-length bench, median of 2): UHD 750 6.2K graded ~8–10 fps (from
~3.5), 6.2K graded export ≥ 24 fps with both GPUs, lower render time in X422 (a).

## 2. Where the passes come from today

`fx.rs` `steps()` lowers each `FxOp` to `Step`s; `route()` assigns textures; one `Step` = one
dispatch:

| FxOp (bench grade) | Steps |
|---|---|
| `ColorChain` (input transform + Lumetri Basic / curves / vignette `CO_VIGNETTE`, merged by `gpufx::fuse_color_chains`) | 1 (`colorops.wgsl`, specialised to its op codes, U0.2.3b) |
| `HslSecondary` | key chain 1 + box passes (`boxes_for_gauss(σ, 3)`: up to 3 per axis, radius 0 skipped) + masked-correct combine 1 |
| `Unsharp` (Sharpen) | box passes (up to 3 per axis) + combine 1 |

That is ~10–13 passes. Lumetri's vignette is a chain op, so the bench grade has **no** separate point
pass: F1 below gains nothing on it; F2 and F3 are what count (`FxOp::Vignette` is the standalone
Vignette effect).

So the per-pixel work is already specialised (no interpreter switch); what is left is the number of
full-frame round trips. Every pass in a run of *point* operations (output at p reads only input at p)
and every box pass of a cascade is a round trip that fusion removes.

**Point ops** (safe to fuse; `pixel()` in `fx.wgsl` reads only `ld(p)`): every `color_op` code (the
`default` arm), `OP_INVERT_ALPHA`, `OP_ALPHA_ADJUST`, `OP_CROP` (position-dependent but point),
`OP_VIGNETTE`, `OP_VIDEO_LIMITER`, and the `colorops.wgsl` chains (`ColorChain`; HSL's key and
masked-correct chains are point ops on their inputs). **Not point ops:** `OP_BOX`, `OP_MEDIAN_MASK`,
`OP_DIRECTIONAL`, `OP_DENOISE*`, `OP_TEMPORAL_*`, `OP_RESAMPLE`, flips, `OP_MIRROR`, `OP_OFFSET`,
save / restore (masked effects).

## 3. The fusions

**F1. Point runs → one pass.** A maximal run of adjacent point steps becomes one generated compute
shader: load `ld(p)` once, apply each step's function in order, store once. Code generation reuses
U0.2.3b's machinery: `colorops::specialise` already turns a chain into straight-line calls of
`chain_op_<code>`; F1 adds `fx_point_<code>` functions cut from `pixel()`'s `case` bodies the same way
(`case_body`), with `ld(p)` replaced by the running value. Parameters: one storage buffer holding each
step's `p` block and chain program back to back, with per-step offsets baked into the generated code
as constants (so the shader is keyed by the op-code sequence only, like today's cache; values change
per frame without a rebuild). Cap: the existing `MAX_SPECIALISED_OPS` (64) per fused shader; longer
runs split. Build failure → the unfused steps (as `build_specialised` does).

**F2. Box cascades → one pass per axis.** The ≤ 3 box passes along one axis run in one dispatch with
workgroup shared memory: a workgroup loads a tile of a row (column) plus an apron of Σ radii on each
side, runs box 1, 2, 3 in shared memory and stores the centre. Edge handling must equal the cascade:
each stage reads its predecessor through `clamp(j, 0, n − 1)` (repeat) or zero (no repeat) *at image
coordinates*, so the tile computes each intermediate only for image coordinates and clamps reads
against the image bounds, not the tile. Sums keep `fx_px`'s order (i = −r … r) so the result is the
same arithmetic. Only when every radius ≤ `BOX_PER_PIXEL_MAX` (32; bigger radii keep the running-sum
`fx_run` path) and the tile fits (`Rgba32Float`, tile 256 + 2 × 96 apron ≈ 7 KB, well inside 16 KB).

**F3. Combines join the last pass; keys join the first.** Unsharp's combine reads (blurred,
original) per pixel, so it runs at the end of the vertical F2 pass (the original is the pass's aux
input already routed by `unsharp_blur` / `combine`). HSL Secondary: the key chain is a point op on the
source, so it runs while the horizontal F2 pass loads its tile (key computed on load); the masked
correct chain runs at the end of the vertical pass. With a Denoise median on the key (`median_r > 0`)
the key stays its own pass (median isn't separable). A point run right after a combine (here the
vignette, and the next effect's chain) continues in that pass (F1 applied to the combine's output).

**F4 (measure first). Lead-in into the source draw.** The first point run could be folded into the
source draw (YUV → RGBA, `composite.wgsl` `fs`), saving one more round trip. That pass is a render
pass, not compute, and shares code with plain layers, so F4 is only built if §5's measurement after
F1–F3 shows the source draw + first pass still matter on Intel.

Bench grade after F1–F3: HSL H (key + boxes), HSL V (+ masked correct + any following point run),
Unsharp H, Unsharp V (+ combine), with the input transform + Lumetri chain fused into the HSL H pass
when it directly precedes it: **~4 full-frame passes instead of ~10–12**, i.e. roughly 3 GB less
traffic per 6.2K frame.

## 4. Correctness and parity

- **Reference:** the unfused path stays, behind `FILMCRAFT_FX_FUSE=0` (A/B, tests) and as the
  fallback for any shader that fails to build. The CPU renderer stays the oracle.
- **Tests per step:** fused vs unfused on both GPUs (`a_renderer_on_each_gpu`) for single ops, every
  point op code, runs crossing chain / px boundaries, boxes with repeat and without, radius mixes
  including 0 and `BOX_PER_PIXEL_MAX`, edges (tiles at x = 0 and the last partial tile, widths not a
  multiple of the tile), images narrower than the apron, Unsharp / HSL with and without denoise, alpha
  premultiplied input. Tolerance: the existing AMD / Intel parity tolerance; expect bit-exact on the
  same GPU except where the compiler contracts differently (then ≤ the documented tolerance).
- **Hostile input:** radii, tile sizes and offsets come from checked arithmetic; a shader too large
  or a run longer than the cap splits; never a panic (AGENTS §0).
- **Half floats:** fused passes write the same storage format as today (`wgsl_for`), so the preview
  f16 option (Q18) keeps working; export stays f32 (§1a.64). Fusing *reduces* f16 rounding steps.

## 4b. Measured after 2b.1 (2026-10-14, synthetic, `graded_export_ops_timing_6k`, GPU time per op at 6240 × 4160)

| | AMD f32 | Intel f32 | Intel f16 |
|---|---|---|---|
| source draw | 3.5 ms | 27 ms | 20 ms |
| ColorChain (1 pass) | 1.2–3.5 ms | 98 ms | 93 ms |
| HslSecondary (key + masked correct, **no blur in the bench grade**) | 4.8–5.7 ms | 66 ms | 46 ms |
| Unsharp (Sharpen 40: **one 1-px box per axis** + combine = 3 passes) | 8.5 ms | 121 ms | 62 ms |
| all ops | 19.1 ms | 286 ms (same fused / unfused) | 201 ms |
| one-op chain (≈ one round trip) | 2.3 ms | 33 ms | 17 ms |

So on the bench grade **F2 finds nothing to fuse** (no box cascades), and on Intel the cost is ~33 ms
per round trip plus ~65 ms of chain ALU. The passes that can go: Unsharp's 3 → 1 and chain + HSL's
3 → 1, about 4 round trips (~130 ms of 286 on Intel f32). That is what 2b.2 now does (F3 made
concrete):

- **F3a, one-pass small Unsharp:** when each axis's cascade is small (Σ radii ≤ 3 per axis), the
  combine pass sums the 2-D box directly ((2R+1)² taps with the cascade's integer weights, weights
  computed on the CPU into the uniforms; repeat-edge clamp like the boxes). Same maths, different
  summation order: within the f32 parity tolerance, not bit-exact.
- **F3b, HSL without blur / denoise in one pass:** key and masked correct in one invocation (key k
  from the pixel, then the correct chain mixed by k), and the ColorChain right before it run in front
  of the key in the same shader (`colorops::specialise` over the concatenated op codes, with the
  segment boundaries baked in as constants).

## 5. Steps (one commit each, tests with each)

Ordered by gain on the bench grade (F2 and F3 remove ~8 of its passes; F1 none):

1. **2b.1 F2 ✓ (2026-10-14)** shared-memory box cascade per axis (`fx_box3`, `fuse_boxes`, switch
   `FILMCRAFT_FX_FUSE=0`): f32 output within 1e-6 of the unfused passes on AMD and Intel (every effect
   case, tile-edge sizes); no gain on the bench grade (§4b), a gain for real blurs.
2. **2b.2 F3a ✓ (2026-10-14)** one-pass Unsharp for at most one box per axis, radius ≤ 3 (`OP_UNSHARP_DIRECT`,
   `fused_steps`): f32 within 1e-5 of the passes (≤ 4e-6, Sharpen 4000), AMD + Intel. Timing: Intel f32 all ops
   286 → 207 ms (Unsharp 121 → 44), f16 201 → 175; AMD f32 19.1 → 13.5 ms.
   **F3b ✓ (2026-10-14)** HSL Secondary without Blur / Denoise as one chain (GPU-only codes `CO_KEY_BEGIN` /
   `CO_KEY_END`), adjacent plain chains merged (`merge_chains`, within `MAX_PROGRAM` / `MAX_SPECIALISED_OPS`):
   bench grade and HSL variants within 1e-5 of the passes, AMD + Intel. **Graded ops per 6.2K frame: Intel f32
   286 → 178 ms, f16 201 → 165; AMD f32 19.1 → 8.2 ms.** On Intel the merged chain + HSL pass (~134 ms) is
   ALU-bound (merging saved little there): the colour chain's own maths (~60 ms) is what is left, so the
   ~8–10 fps Intel aim needs cheaper chain arithmetic (e.g. baking the input transform + Basic into a 3-D LUT on
   the GPU), not fewer passes. Exports: 6.2K graded 19.6 → 19.7 fps, X422 (a) 17.1 → 19.9 fps; aims short,
   OWNER-QUESTIONS Q28 (FYI). 2b.3 / 2b.4 optional.
3. **2b.3 F1** point-run fusion (generator + cache) for grades that stack separate point effects.
4. **2b.4** measure; F4 only if it pays; record (a) / (b) / Intel graded in `docs/fork-benchmarks.md`.
   Short of an aim: keep the gain, OWNER-QUESTIONS FYI (§1a.71), go on to X422's final benches.

Risks: Intel shader compile time for long generated shaders (U0.2.3b saw seconds when inlining
whole switches; F1 emits only the cases used, as `specialise` does); shared-memory tiling being
slower than the plain per-pixel box on the AMD card for small radii (measure per GPU; the switch can
be per device type if needed).
