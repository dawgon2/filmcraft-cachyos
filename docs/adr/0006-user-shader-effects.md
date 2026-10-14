# ADR 0006: User shader effects (WGSL files) and the user-effect registry (fork)

- **Status:** accepted for the fork (2026-10-14, Y3.4 step 1; written in a MINIMAL-usage run without a
  separate designer: **provisional**, open to the owner's change like any §1a choice). Scope set by
  [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md) Phase Y3.4; audit in
  [`docs/fork/PREP-Y3.md`](../fork/PREP-Y3.md) §3–§5.
- **Builds on:** the GPU effect stage (`crates/gpu/src/fx.rs`, `FxOp`), the effect definitions
  (`filmcraft_project::effect`). **Leads to:** C1's node definitions (COMPOSITOR-PLAN) and the
  shader-only first form of PLUGIN-PLAN (deferred; the owner decides on plugins).

## Context

Y3.4 lets a user drop a `.wgsl` file with annotated parameters into an effects folder; FilmCraft
validates it, builds its controls from the annotations, runs it in the GPU effect pass with a time
limit, and disables a failing one with a message. No native code. Four constraints shape the design:

1. Every effect today is an `EffectDef` with `&'static str` ids and names, found by
   `find_effect(id)` from `EffectInstance::def()`, which dozens of call sites use (panels, Effect
   Controls rows, presets, project load, badges). User effects appear and change at run time.
2. WGSL from a file is hostile input: syntax and type errors, bindings we don't provide, and loops
   that never end (naga validates types, not termination).
3. There is no CPU interpreter for WGSL; the CPU renderer cannot draw a user effect.
4. A project that uses a user effect must still open on a machine without that file.

## Decision

**Files.** User effects live in `<data dir>/Effects/*.wgsl` (at most 256 files, each at most 256 KB;
more are ignored with a message). FilmCraft ships its examples (glitch, chromatic aberration, VHS,
light leak; original work with `.attribution` sidecars) in `assets/effects/`, loaded the same way.
A file's effect id is `user.<slug of the file stem>`; a bundled example's is `example.<slug>`.

**Annotations** are line comments the user writes above their code:

```wgsl
// @effect "Chromatic Aberration"
// @category "Stylize"
// @param amount: float = 4.0 [0.0, 50.0] "Amount"
// @param center: point = auto "Center"
// @param tint: color = #ff0000 "Tint"
// @param radial: bool = true "Radial"
```

Kinds: `float` (default, range), `bool`, `color` (`#rrggbb`), `point` (`auto` = layer centre, or
`x, y` in layer pixels). At most 16 parameters. Unknown kinds, duplicate names, a missing default,
a default outside its range or a name that isn't `[a-z_][a-z0-9_]*` are errors with the line number.

**The shader contract.** The user writes one function:

```wgsl
fn effect(px: vec2<f32>) -> vec4<f32>
```

`px` is the output pixel's centre in layer pixels. FilmCraft prepends a prelude and appends the
entry point: the prelude declares the bindings (input image, output image, a uniform block),
`P.<name>` for each parameter (points in layer pixels, colours as straight sRGB-encoded
`vec3<f32>`), `size: vec2<f32>`, `time: f32` (seconds into the clip), and helpers
`sample(p: vec2<f32>) -> vec4<f32>` (bilinear, edges clamped) and `load(p: vec2<i32>)`. Colours the
user sees and returns are **straight, display-referred (sRGB-encoded)**, the way most effects
think; the wrapper converts from and to the working image's linear premultiplied form. User code
may not declare bindings, entry points, `var<storage>`, `var<workgroup>`, `override`s or
`enable` directives (refused before naga sees it). The composed module is validated by naga
(a direct `naga = "30"` dependency, the version wgpu 30 uses) in a new leaf crate
`filmcraft-usershader` (L2: depends on `naga` only; the engine turns its result into an `EffectDef`), which returns the parsed
parameters and the composed WGSL, or errors that point at the user's own line numbers.

**Registry.** `filmcraft_project::effect` gains a run-time registry next to the static list:
`register_user_effects(Vec<UserEffect>)` replaces the user set; `find_effect(id)` looks there after
the built-ins. To keep `&'static EffectDef` (constraint 1) user definitions are **interned**: a
definition with the same id and the same source hash returns the existing `&'static`; a changed
one is leaked once. The registry refuses more than 4096 interned definitions per process (a few MB
at most), so reloading in a loop can't grow memory without bound. User effects carry
`category: ["Video Effects", "User"]` (or their `@category` under `User`) and `accelerated: true`; they
are told apart from built-ins by their id prefix (`is_user_effect`: `user.` / `example.`), not a new
`EffectDef` field (19 literal constructions stay unchanged).

**Projects.** An instance stores only `effect: "user.<slug>"` and its parameters, like any effect.
Opening a project without that file keeps the instance unchanged (it round-trips), Effect Controls
shows it as "Missing effect: <id>" with its saved values, and it renders as **pass-through**. No
schema change.

**Rendering.** `FxOp::User { id, hash, params }` (evaluated parameter values). `gpu_ok()` is true; the
GPU stage compiles the composed module once per (id, hash) inside a validation error scope (a
failure marks that effect broken for the session with the error message) and caches the pipeline
like the specialised `fx_px` pipelines. The CPU reference is pass-through: a layer with a user
effect never falls back to the CPU path silently: when no GPU stage is available the effect is
skipped and Effect Controls shows "needs the GPU".

**Time limit.** WGSL cannot be preempted, so the limit is measured and enforced after the fact:
each user pass's GPU time is read (timestamp queries where the adapter has them, else the
submit-to-done wall time of a frame with only that pass); a pass slower than **50 ms at 1080p**
(scaled by pixel count) on **3 frames in a row** disables that effect for the session with a
message. A device loss while a user effect was in the frame writes the effect's id and hash to
`<data dir>/Effects/quarantine.json`; quarantined effects load as broken (message, pass-through)
until the user re-enables them. This is honest about the limit: a shader that never ends can still
freeze the GPU until the driver's own timeout resets it; FilmCraft survives that (device-loss
handling exists) and won't run that shader again automatically.

## Consequences

- Every existing call site of `find_effect` / `def()` keeps working; panels list user effects with
  no special cases beyond the "Missing effect" row.
- Leaked definitions are bounded (4096) and only grow when a file's content changes.
- User effects are GPU-only by design; exports and the program monitor use the GPU in this fork.
- C1's nodes can reuse `EffectDef` + the registry for a "Shader" node later; PLUGIN-PLAN's WASM
  form (deferred) would register through the same function.
- Reversal: the registry and `FxOp::User` are additive; removing the feature leaves projects with
  `user.*` instances that load as missing effects (pass-through), which is the same as a machine
  without the files.

## Steps (Y3.4)

1. This ADR.
2. `filmcraft-usershader`: annotation parser + forbidden-construct check + composition + naga
   validation; table tests (valid, each error kind with its line, size caps, hostile characters).
3. Registry in `filmcraft_project::effect` (interning, cap) + engine loading from the data dir and
   `assets/effects/` at start and on `effects.reloadUser`; project round trip with a missing effect.
4. `FxOp::User` + GPU compile / cache / run + the slow-pass rule + quarantine; parity of an example
   against a hand-written expected image on AMD and Intel; a broken file rejected with a message.
5. The four examples, UI (Effects ▸ User folder, "Missing effect" / "needs the GPU" rows, Reload,
   Open Effects Folder), guide page.
