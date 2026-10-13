# PREP-Y3: compositing and extensibility (audit before the work)

Prepared 2026-10-13 by the PC run (Y3-PREP from CLOUD-QUEUE); code facts as of commit 58eff262. Docs only.

No code was changed. Line numbers are from that commit. Guesses are marked "(guess)".

## 1. What Y3 asks for (YOUTUBE-PLAN.md, "Phase Y3", about 25-50 h, tag `yt-3`)

- **Y3.1 Keys on the GPU.** Ultra Key / chroma key and luma key as GPU ops (CPU-only today), spill
  suppression, matte cleanup (choke / soften), "show matte". Oracle: the existing CPU keyers.
- **Y3.2 Attach to a track.** The existing tracker drives any layer's position / scale / rotation
  (text, stickers, callouts), plus tracked blur / mosaic for faces, plates and screens.
- **Y3.3 Screen replacement.** A 4-point track drives the existing Corner Pin.
- **Y3.4 Custom shader effects.** The user drops a WGSL file with annotated parameters into the
  effects folder. naga validates it, controls are built from the annotations, it runs in the GPU
  effect pass with a time limit, and a failing one is disabled with a message. Ship examples
  (glitch, chromatic aberration, VHS, light leak). No native code.
- **Y3.5 Look and template packs.** Import / export of effect presets, LUT folders and graphic
  templates as one shareable pack file.
- **Y3.6 Moved out.** Node compositor = COMPOSITOR-PLAN.md (C1, C2 right after Y3; C3/C4 after
  H3). Plugins = PLUGIN-PLAN.md (WASM, deferred, needs the owner). Not Y3 work. Two hooks matter:
  C1's Key node reuses Y3.1 ("Key (Y3's GPU keys)", COMPOSITOR-PLAN.md), and PLUGIN-PLAN says Y3.4
  WGSL effects are "the first, shader-only form" of plugin shaders.
- **Accept:** green-screen 4K keys in real time at Full on the GPU; text on a tracked point follows
  it for 5 s; tracked face blur holds; screen replacement holds; a WGSL example loads and renders
  on AMD and Intel; a broken one is rejected with a message, no crash.

Headline: the CPU keyers, Corner Pin, Mosaic, the mask tracker, effect presets, LUT import and
graphic templates all exist. None of the Y3.1 targets has a GPU path, the tracker only writes
mask keyframes (it never drives a layer or a Corner Pin), and there is no user-shader hook at all.

## 2. What exists today

| Where | What it does | Y3 step |
|---|---|---|
| `crates/render/src/vfx/color.rs:159` `ultra_key` | CPU Ultra Key: Y'CbCr colour-difference matte, tolerance / gain / highlight / shadow, cleanup, spill suppression (`:217-231`), colour correction. Dispatched at `vfx/mod.rs:464`. | Y3.1 (the oracle) |
| `crates/render/src/effects.rs:95` and `:331` | `vfx::apply` is tried first; `effects.rs` keeps the older `ultra_key` / `color_key` path (`key()` at `:533`) and `luma_key` (threshold / cutoff on sRGB luma, `:331`). Which of the two key paths wins for `ultra_key` is (guess) the vfx one. | Y3.1 |
| `crates/render/src/vfx/color.rs:262` `track_matte` | Multiplies the layer by another track's alpha or luma via `apply_matte` (`:273`). CPU only. | Y3.1 (matte reuse) |
| `crates/project/src/effect/vfx.rs:453-470` | Keying definitions (`alpha_adjust`, `logo_cutout`...); `ultra_key_setting` presets at `:1020`. `luma_key`, `track_matte`, `mosaic` listed near `:1136`. | Y3.1 |
| `crates/project/src/effect.rs:857-893` `GPU_EFFECTS` | List of effects with a GPU path (colour ops, blurs, crop, transform, lumetri, `video_limiter`...). Contains none of `ultra_key`, `luma_key`, `corner_pin`, `mosaic`, `track_matte`. A test in `gpufx.rs:~425` forces this list to equal what the GPU draws. | Y3.1 |
| `crates/render/src/gpufx.rs:34` `FxOp` | The evaluated-parameter enum shared by CPU and GPU ("evaluate once, run on either"; `gpu_ok()` falls back to CPU). Has `ColorPass` (keep one colour, `:~58`), `LeaveColor`, `ChangeToColor`, no key variant. | Y3.1 |
| `crates/gpu/src/fx.rs:199` `steps()` | Maps each `FxOp` to compute `Step`s (opcode + 4 ints + floats). `Fx::new` (`:418`) builds one WGSL module from `include_str!("fx.wgsl")` and a pipeline. Per-pass budget / texture ring: `targets` `:572`, `dispatch` `:694`. | Y3.1, Y3.4 |
| `crates/gpu/src/fx.wgsl` (1044 lines) | The single effect shader: helpers (`sample_bilinear`, `rgb_to_hsl`, `luma709`...), `color_op` (`:313`), blur, limiter, noise-reduction. | Y3.1 |
| `crates/gpu/src/mask.rs` + `mask.wgsl`; `fx.rs:188` `mask_program` | Masks run on the GPU (flat mask program per effect). | Y3.2 |
| `crates/gpu/src/lut.rs` + `lut.wgsl`; `colorops.rs` + `colorops.wgsl` (ADR 0003) | GPU LUT and the composable colour-op chain. | Y3.5 (looks) |
| `crates/gpu/src/lib.rs:1339` | `push_error_scope(Validation)` around pipeline creation. `export_renderer.rs:119-120` marks the device failed on uncaptured errors and device loss. | Y3.4 (error handling) |
| `crates/gpu/Cargo.toml:30` | `wgpu = "30"` with the `wgsl` feature (naga comes in transitively). No direct `naga` dependency in any manifest. | Y3.4 |
| `crates/render/src/track.rs` (527 lines) | Mask tracker: Shi-Tomasi features, pyramidal Lucas-Kanade with forward-backward check, RANSAC fit, returns an `Affine` frame A to B (`track_step` `:405`, `fit_robust` `:329`). Methods: Position, PositionRotation, PositionScaleRotation (`project/src/mask.rs:364`). No perspective (homography) fit; no multi-point output. | Y3.2, Y3.3 |
| `crates/engine/src/masks.rs:707` `track` | Command `masks.track`: runs a background job on frames at most 960 px wide, applies results as merged "Track Mask" undo steps via `put_keyframe(.., ParamValue::Path)` (`:853-870`). Forward / backward, already-tracking guard. Output is only mask path keyframes. | Y3.2 |
| `crates/engine/src/masks_tests.rs:328` | Mask-tracking accuracy test (max vertex error over -8..+12 frames). | Y3.2 |
| `crates/render/src/vfx/distort.rs:48` `corner_pin` | CPU Corner Pin: four point params, homography via `square_to_quad` / `h_inverse` / `warp_opt`. Point params exist in `effect/vfx.rs:60-63`. | Y3.3 |
| `crates/render/src/vfx/distort.rs:535` `mosaic`, `effects.rs:937` `mosaic()` | CPU Mosaic (blocks). `preview.rs:434` lists it among effects that need whole-frame treatment. | Y3.2 (tracked mosaic) |
| `crates/render/src/mask.rs`, `crates/project/src/mask.rs` | Mask shapes, feather, invert; effects can carry masks (so Gaussian Blur + tracked mask is already a blur-a-region setup). | Y3.2 |
| `crates/engine/src/presets.rs:198` `builtin_presets`, `:548` `export`, `:570` `import` | Effect presets: parameters, keyframes, masks; JSON `filmcraft.effect-presets` v1; user file `<data dir>/effect-presets.json`; commands `presets.*` incl. export / import. Point params scaled for frame size. | Y3.5 |
| `crates/engine/src/graphic_templates.rs:64` `user_templates_dir`, `:77` `library`, `:102` `find_template` | `.fcgt` templates (`crates/project/src/gtemplate`), user folder `<data dir>/Graphics Templates/*.fcgt`, Install copies a file, `.mogrt` refused unopened. | Y3.5 |
| `crates/engine/src/color.rs:91` `import_lut`; `crates/color/src/lut.rs`; `crates/render/src/luts.rs` | `.cube` / `.3dl` parse + write, tetrahedral interpolation, project LUT library (`lut.*` commands). Single-file import; no folder import found. | Y3.5 |
| `crates/render/src/lumetri_presets.rs:72` `presets()` | Built-in Lumetri looks in code ("original work", thumbnails from a procedural preview). | Y3.5 |
| `crates/engine/src/stills.rs` | Grade stills (picture PNG + grade metadata in the project). | Y3.5 (guess: could ride in a pack) |
| `docs/fork/DESIGN-C1.md`, `docs/adr/0005-upstream-sync.md` | C1 design exists; next free ADR number is 0006 (0005 is the last in `docs/adr/`; COMPOSITOR-PLAN says "U0 took 0005"). | Y3.6 |

Not found anywhere: a user effects folder, any WGSL loaded from disk, a direct naga use, a pack or
zip container for presets / LUTs / templates, a link from a tracker result to a clip's Motion
parameters, and a homography / 4-point tracking mode.

## 3. Gaps per step

**Y3.1 Keys on the GPU** (about 8-14 h; the most mechanical)
- `crates/render/src/gpufx.rs`: add `FxOp::UltraKey {..}` and `FxOp::LumaKey {..}` (evaluated
  numbers, `gpu_ok` true for finite values), reuse the CPU maths as the CPU reference, route in
  `apply_ops`. Note `ultra_key` is dispatched from `vfx/mod.rs:464`, not `effects.rs`, so the
  `FxOp` evaluation needs to be added where the vfx effects get their `FxOp` (guess: `gpufx.rs`
  `FxOp::eval`).
- `crates/gpu/src/fx.rs` + `fx.wgsl`: new opcodes in `steps()`; the key needs a neighbourhood for
  choke / soften (a blur of the matte), so it is two or three passes (matte, matte blur, despill
  and composite), like the existing box-blur `boxes()` helper at `fx.rs:179`.
- `crates/project/src/effect.rs:857` `GPU_EFFECTS`: add `ultra_key`, `luma_key` (the badge test
  will then require the GPU draw them).
- "Show matte": check the parameter exists in the `ultra_key` definition (`vfx.rs`, not read in
  full); if not, add a "Output: Composite / Alpha / Matte" choice to both paths.
- Choke / soften: the CPU "cleanup" stage exists (`color.rs:~200-215`); mirror it.
- Real-time 4K needs a measurement (`perf.stats`), not a guess; see §5.

**Y3.2 Attach to a track + tracked blur** (about 8-14 h; the design decision is here)
- `crates/engine/src/masks.rs`: today a job only writes mask path keyframes. Add a way to turn the
  same `Affine` stream into keyframes on a clip's Motion position / scale / rotation (a new
  command, e.g. `tracking.attach` (guess at the name), with a target clip and a source tracked
  region). Reuse `track_step` as is; keep its 960 px working width and convert to clip pixels.
- Tracked blur / mosaic needs no new tracker: a masked Gaussian Blur or Mosaic with a tracked
  mask already works. The gap is the mask-on-effect for Mosaic on the GPU (mosaic is CPU only) and
  a one-click "Track blur" command that adds the effect, a mask and starts the track.
- UI (`crates/ui-egui`): the tracker panel, new buttons need automation ids (project rule).
- Rotation and scale from a similarity fit are available now; perspective is not (see Y3.3).

**Y3.3 Screen replacement** (about 6-10 h)
- `crates/render/src/track.rs`: add a homography fit (`fit_ls` / `fit_robust` handle only
  translation / similarity). Needs 4+ good features in the region, RANSAC with a 4-point minimal
  sample, a determinant and reprojection sanity check, and a "lost track" result that stops.
- Output: four Corner Pin point keyframes (`effect/vfx.rs:60-63` point params) written the way
  `masks.rs:853` writes mask keyframes. A new command, undo-merged like "Track Mask".
- `corner_pin` is CPU only (`distort.rs:48`); at Full on 4K that is slow. A GPU Corner Pin is
  an optional add (a fragment of `fx.wgsl` using the same inverse homography). Not required by
  Accept, but worth it for playback (guess).
- Compositing the replacement: place the new picture on the track above, with Corner Pin on that
  clip. Check masks / edge feather work on the pinned layer.

**Y3.4 WGSL custom effects** (about 10-18 h; the biggest and the riskiest)
- Nothing to build on in the loader sense. `fx.rs:418` compiles one fixed module at start-up
  from `include_str!`. A user shader is a separate module + pipeline created at runtime.
- New code, suggested split (guess): a parser / validator in a pure crate or module (the L-level
  must stay below L5; likely `crates/gpu` or a new small leaf crate), parsing `// @param` style
  annotations into a parameter list; validation by `naga` (add as a direct dependency, pin to the
  version `wgpu 30` uses, `wgsl-in` feature only); reject on: parse / validation error, too-large
  source, forbidden bindings, workgroup size over limits, loops can't be proven bounded (naga
  can't prove that; so the "time limit" must be a runtime one: see §4).
- Registration: effect definitions are built in code (`project/src/effect.rs` `build_effects`,
  `OnceLock`), so user effects need a second, dynamic registry; `EffectDef` has static ids
  (`&'static str`) (guess from `find_effect` returning `&'static EffectDef`). Needs a design
  decision (leak strings, or an owned def type). This is the main API risk.
- Files: a `<data dir>/Effects/*.wgsl` folder, read at start and on "Reload"; the UI builds the
  controls from the parameter list through the normal inspector; keyframes via the usual `Param`.
- Examples: four small WGSL files with `.attribution` sidecars and an `ATTRIBUTION.md` entry
  (`cargo xtask assets` checks it); original code only.
- Failure path: wrap pipeline creation in `push_error_scope` (as `gpu/src/lib.rs:1339` does), mark
  the effect "disabled" with a message, and render the layer unchanged.

**Y3.5 Look and template packs** (about 4-8 h)
- A pack = one file. Suggested: a zip-like container holding `manifest.json`, `presets.json`
  (the existing `filmcraft.effect-presets` payload), `luts/*.cube`, `templates/*.fcgt`. No zip
  crate was seen in the engine dependencies I checked (guess; verify `Cargo.toml` first), so the
  container may need a small hand-written format or a pure-Rust zip dependency (check the licence;
  no GPL).
- Commands (`packs.export`, `packs.import`) next to `presets.export/import` (`presets.rs:548,570`),
  reusing `graphic_templates` install and `color::import_lut`; import must be hostile-input safe
  (size caps, no path escape in names, count caps, `.mogrt` refused, LUT size caps already in
  `lut.rs`).
- Collisions and undo: import adds to the libraries; define rename-on-clash.

## 4. Risks

- **GPU parity, AMD vs Intel.** Key maths (divides, `pow`, YCbCr) must match the CPU oracle within
  a tolerance on both the RDNA3 VA-API/Vulkan path and the Intel UHD 770 (Dell lane). Existing
  GPU-vs-CPU tests are in `crates/gpu/src/fx_tests.rs`; add the key cases there. WGSL
  undefined behaviour on NaN / zero divisors differs by driver; clamp every denominator. User
  shaders (Y3.4) multiply this risk: the same file can behave differently per driver.
- **Never-crash.** No `unwrap` / `expect` / `panic!` in new code. Hostile input: WGSL source from
  the effects folder and packs (size caps, parse errors as messages), keyframes with NaN, tracker
  regions of zero area, homographies with a near-zero determinant (`h_apply` already guards
  `|w| < 1e-12`), project files with absurd parameter counts. A shader that loops forever can hang
  the GPU: naga does not bound loops, so the "time limit" needs a measured per-frame cost
  and disable-after-N-slow-frames rule, and ideally a watchdog (device-lost handling is at
  `export_renderer.rs:119`). This is a real design problem, not just a detail (guess: cannot be
  fully solved on all drivers; document the limit honestly).
- **Layering** (`cargo xtask layers`). Parsing and validating shaders must not live above where
  render / gpu live; no egui, winit, rfd in any new core code. L0 codec crates depend only on
  `filmcraft-bitstream`: none of Y3 should touch them. A pack container belongs in
  `crates/engine` or a small leaf crate, not the UI.
- **Clean-room.** Key maths is textbook colour difference; don't mirror Adobe's Ultra Key
  internals beyond what is there. No Blackmagic or Adobe look files, no `.mogrt` loading (already
  refused). Example shaders and packs must be original and have attribution sidecars.
- **Project compatibility.** New effect ids and params must be `#[serde(default)]`-safe so old
  projects load; user effects missing on another machine must load as "missing effect" not an
  error.
- **Everything is a command** and every new widget needs an automation id.
- **Tracker honesty.** Tracking fails on blur, occlusion, flat areas; show "lost track" and stop,
  never extrapolate silently.

## 5. Suggested lane split

| Lane | Steps | Crates | GPU check? | Independent? |
|---|---|---|---|---|
| A | Y3.1 keys (CPU maths already done; port + 3 passes) | `render/gpufx.rs`, `gpu/fx.rs` + `fx.wgsl`, `project/effect.rs` | Yes, AMD + Intel (Dell lane) and a 4K timing | Yes |
| B | Y3.3 homography fit + Corner Pin keyframing | `render/track.rs`, `engine/masks.rs` (or a new tracking module) | No (CPU); GPU Corner Pin optional later | Yes, but shares `masks.rs` with C |
| C | Y3.2 attach-to-track, tracked blur / mosaic command | `engine`, `ui-egui` | Light (GPU Mosaic only if added) | After B (shares the job code) or before it (guess: either, agree who touches `masks.rs` first) |
| D | Y3.4 WGSL effects: parser + naga validator as a leaf module/crate first, then the runtime pipeline, then the UI | new leaf + `gpu/fx.rs` + `project` registry + `ui-egui` | Yes, AMD + Intel; the registry design needs an ADR (0006 is the next number) | Parser / validator: pure leaf, can run in parallel with A. Runtime: after the registry decision |
| E | Y3.5 packs | `engine` (+ small container code) | No | Yes, pure CPU and file work, good for a cloud lane |
| F | Docs: guide page for keying, tracking, shader effects, packs | `docs/guide` | No | After A-E |

Parser/validator (D, first half), E, and the CPU parts of B are leaf jobs suited to a cloud lane.
A and D (runtime) need GPU machines. Do D's registry decision early, because C1 (COMPOSITOR-PLAN)
will want the same effect-def shape for its nodes (guess).

## 6. Suggested test list

- **Keys (A):** GPU vs CPU on a synthetic green ramp, per-channel tolerance (like the other
  `fx_tests.rs` cases), both `ultra_key` and `luma_key`; spill suppression removes a known green
  cast; choke and soften change the matte edge; "show matte" returns grey alpha; NaN and zero-size
  inputs return the layer unchanged; the badge test (`gpufx.rs` list test) passes after the list
  change; a timing test or `perf.stats` capture for a 4K green-screen frame at Full.
- **Homography (B):** synthetic quad moved by a known homography over 30 frames; recovered corners
  within 0.5 px; fewer than 4 features gives `None`; degenerate (collinear) points give `None`; a
  frame that goes blank stops the job without a panic.
- **Attach (C):** a synthetic textured square moved by a known path; the attached layer's Motion
  keyframes follow it within 1 px over 5 s (150 frames at 30 fps); undo restores; the job cancels
  cleanly; the existing `masks_tests.rs:328` accuracy test still passes.
- **Tracked blur (C):** after tracking, a pixel inside the mask is blurred and outside is not, at
  frames 0, 60 and 149.
- **Shaders (D):** annotation parser table tests (valid, missing default, duplicate names,
  oversize, hostile characters); naga rejects a syntax error and a type error with a line
  number; a shader with an unsupported binding is rejected; an example effect renders and matches a
  golden within tolerance on AMD and Intel; a deliberately slow shader is disabled with a message
  and the frame still renders; creating a pipeline from a bad shader inside an error scope returns
  an error, not a panic.
- **Packs (E):** export then import round trip equals the original presets, LUTs and templates; a
  pack with `../` names, a huge entry count or a `.mogrt` inside is refused with a message; a
  name clash renames; `import_lut` size caps still apply to LUTs from a pack.
- **Project load:** an older project file with no user effects still loads; a project that uses a
  missing user effect loads and shows it as unavailable.
- Gates for every lane: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo xtask layers`, `cargo xtask assets`, `cargo xtask wasm`.
