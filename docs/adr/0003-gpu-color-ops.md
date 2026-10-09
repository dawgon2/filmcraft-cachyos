# ADR 0003: Composable colour operations on the GPU, DaVinci Wide Gamut working space (fork)

- **Status:** accepted for the fork (2026-10-08, session 4). Design set by the owner
  (`FORK-PLAN.md` §1a.14); this records how it is built.
- **Context documents:** `FORK-PLAN.md` Phase 4, `docs/fork-benchmarks.md`.

## Context

After Phase 3 a 6.2K clip plays from VRAM at ~2 ms of CPU per frame, but anything colour managed
or graded leaves the GPU path:

- a clip whose colour space needs more than the default decode (log, HDR, any Interpret Footage
  override, a wide-gamut or HDR sequence) is a "CPU layer": `colorman::decode` reads the frame
  back and converts it per pixel on the CPU (`render/src/plan.rs`, `needs_management`);
- Lumetri (`render/src/effects.rs` `lumetri`, `lumetri_advanced`, `hsl_secondary`) has no GPU
  implementation, so a graded clip is rendered on the CPU too. `xm5-play-graded` showed nothing
  in Phase 2 (218 ms of CPU per frame; graded 4K export 0.8 fps, 18 GB RSS).

The GPU effect stage (`gpu/src/fx.rs`, `fx.wgsl`, `render/src/gpufx.rs`) already has the shape
the owner asked for: an effect is evaluated once into plain numbers (`FxOp::eval`), the CPU
`FxOp::apply` is the reference, the WGSL reproduces it and parity tests compare the two
(`gpu/src/fx_tests.rs`). It runs **one compute pass per op** on `Rgba32Float` textures; at 6.2K
(26 Mpx, 416 MB per texture) a fifteen-step colour chain would move ~12 GB per frame.

The owner's design (§1a.14): small composable operations, each tested against its CPU oracle,
behind one interface; Lumetri is a fixed chain of them; a later node grader reuses the same
operations, so no grading maths is written twice. Working space DaVinci Wide Gamut / DaVinci
Intermediate (Blackmagic's published specification); F-Log2 / F-Gamut as an input transform.

## Decision

### Operations (`filmcraft_color::ops`)

One enum of **per-pixel** operations, `ColorOp`, with plain `f32` parameters, defined in the
colour crate (no GPU dependency, builds for wasm). The state an operation sees is `rgb` (three
`f32`), a `mask` (one `f32`, 1 by default) and one saved `rgb` register, so qualifiers and masked
corrections compose without special cases. The set (grown as Lumetri needs it):

| op | does |
|---|---|
| `Matrix { m, offset }` | `rgb = m·rgb + offset`: gamut conversion (colour space transform), white-balance gains, exposure, channel mixing, affine levels |
| `Decode(TransferFn)` / `Encode(TransferFn)` | signal ↔ linear: every camera log curve (F-Log2 …), DaVinci Intermediate, sRGB, BT.1886 (γ 2.4), PQ, HLG |
| `Lggo { lift, gamma, gain, offset }` | lift / gamma / gain / offset per channel (Resolve primaries) |
| `Saturation { sat, vibrance, weights }` | saturation around luma, vibrance |
| `Curve { table }` | 1D curves (master and R, G, B) sampled into a table, linearly interpolated |
| `HueCurve { kind, table }` | hue vs saturation / hue / luma, luma vs saturation, saturation vs saturation |
| `Qualify { … }` | HSL qualifier: writes the mask |
| `Save` / `MixMasked` | keep the current `rgb`; blend the saved and current `rgb` by the mask |
| `Lut3d { table, size, shaper }` | 3D LUT, tetrahedral (the CPU `lut.rs` maths, not the texture unit's trilinear) |
| Lumetri-specific steps | where Lumetri's published behaviour is not one of the above (tone regions, contrast S-curve, specular, faded film, split tone, vignette): each its own small op with the same CPU / GPU / test treatment |

Each op has a CPU `apply` (the oracle) and one WGSL branch. Tables (curves, LUTs) travel in a
storage buffer of `f32` and are interpolated by the same arithmetic on both sides.

Operations that look at other pixels (blurs of the HSL key's Refine, Sharpen, the existing
spatial `FxOp`s) are not `ColorOp`s; a chain is a list of **stages**: a run of per-pixel ops, or a
spatial `FxOp`.

### Execution

- **CPU**: `ops::apply_chain` per pixel (rayon), used by the CPU render and the export path.
- **GPU**: a run of per-pixel ops is **one compute pass** that loops over the ops (header `u32`s
  and parameters in a storage buffer) — fused, so a Lumetri grade costs one read and one write of
  the working texture, not one per op. Spatial stages keep their own passes. The same pass applies
  input transforms (log decode + gamut) when the layer is drawn from its YUV source.
- **Parity tests** per op and per chain (`gpu/src/colorops_tests.rs`, like `fx_tests.rs`), on the
  AMD and the Intel GPU, tolerance ~1e-5 (1e-4 where `pow` / `log` differ between the CPU and
  the GPU's transcendental functions).

### Working space

A new working space **DaVinci Wide Gamut / DaVinci Intermediate** (`WorkingSpace`, id `dwg-di`):
the compositor works in scene-linear DaVinci Wide Gamut, grading operations (Lumetri, later the
node grader) see the DaVinci Intermediate signal, and the output transform converts to the
delivery space (Rec. 709 / BT.1886 γ 2.4 for SDR) with the tone and gamut mapping of
`transform.rs`. Input transforms (F-Log2 / F-Gamut, every other log and HDR source) are `Decode`
+ `Matrix` ops into it. Constants from Blackmagic, *DaVinci Resolve 17 Wide Gamut Intermediate*
(2021): primaries R 0.8000 0.3130, G 0.1682 0.9877, B 0.0790 −0.1155, D65; DI encode
`y = (log2(x + 0.0075) + 7.0)·0.07329248` above `x = 0.00262409`, else `y = 10.44426855·x`.
Existing sequences keep their working space (Rec. 709 stays the default, as upstream); the
X-M5 sequence presets use DWG / DI.

### Lumetri

Lumetri becomes a fixed chain built from its parameters (`lumetri_chain(params, space) →
Vec<Stage>`), run by the CPU and the GPU alike. The refactor keeps the current CPU output: the
existing CPU implementation stays in the tests as the oracle until the chain matches it, then is
removed.

## Consequences

- A graded, colour-managed 6.2K layer stays on the GPU: decode → input transform → grade →
  composite → output transform without a read-back.
- The node grader (a later plan) is a different arrangement of the same `ColorOp`s and stages.
- Every new operation costs a CPU implementation, a WGSL branch and a parity test; the GPU path
  is only taken for chains whose every op the shader implements (as `FxOp::gpu_ok` today).
- Lumetri in a DWG / DI sequence grades the DI signal; it looks different from the same settings
  in a Rec. 709 sequence (as in Resolve, where the timeline space changes how controls respond).
