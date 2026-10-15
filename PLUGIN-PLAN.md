# PLUGIN-PLAN.md: a safe Rust plugin system (and OpenFX later)

Written 2026-10-11 (session 8, with the owner). **Deferred: scheduled after everything else and
not started without the owner's go-ahead.** All FORK-PLAN rules apply; above all **never crash**:
no plugin, however broken, may take FilmCraft down or corrupt a project.

## Design

- **Plugins are Rust compiled to WebAssembly** (`wasm32`), run in a sandboxed runtime inside
  FilmCraft (e.g. `wasmtime`, written in Rust). A trap, timeout, memory cap or bad result disables
  that plugin with a clear message; FilmCraft keeps running. Native `.so` / `.dll` plugins are
  rejected by design (no stable Rust ABI, and one bad plugin would crash the app).
- **GPU work through WGSL:** a plugin bundles shaders for per-pixel work (validated with naga, run
  in the existing GPU effect pass with a time limit); the WASM side handles parameters, logic and
  analysis. YOUTUBE-PLAN Y3.4's WGSL effects are the first, shader-only form of this.
- **What a plugin can add:** effects, transitions, generators (titles, patterns), analysis tools
  (e.g. scopes, measurements), compositor nodes (COMPOSITOR-PLAN), and **commands** (FilmCraft's
  "everything is a command" model), so plugins can automate editing too.
- **Capabilities, not ambient access:** a plugin declares what it needs (frames in / out, params,
  timeline read, timeline edit via commands, files in its own folder); the user approves on
  install; no network or arbitrary file access.
- **Format:** one `.fcplugin` file (manifest + `.wasm` + shaders + icons with attribution), a
  `filmcraft-plugin-sdk` crate with typed parameters, example plugins, and docs.

## Phases (all deferred)

- **P1:** host runtime, manifest, capabilities, sandbox limits, effect and generator plugins,
  parameter UI from the manifest, SDK + 3 example plugins. Accept: examples run on AMD and Intel;
  a deliberately broken / hostile plugin set (infinite loop, OOM, trap, bad output sizes) is
  disabled without a crash.
- **P2:** transitions, analysis tools, compositor nodes, command plugins; plugin manager UI
  (install, update, disable), signing / checksums.
- **P3 (optional): OpenFX bridge.** OpenFX is the open C plugin standard used by Resolve, Nuke,
  Vegas and Natron (Sapphire, Neat Video, FilmConvert, …). Native plugins can crash, so they run in
  a **separate helper process** that hosts them and exchanges frames with FilmCraft through shared
  memory; a crash there costs one frame or one effect, never the app. Large and `unsafe`-heavy
  (FFI confined to its own crate with an ADR); only with the owner.

## Native plugins: the owner's decisions (explained live 2026-10-15)

Hosting VST3 / CLAP / LV2 (audio) or OpenFX (video) means loading other people's compiled code.
Before any of it starts, the owner decides: (1) **whether and when** (after which phase);
(2) **which format(s)**: VST3 (largest catalogue; Steinberg's SDK is MIT-licensed since 3.8,
2025), CLAP (open, MIT, good on Linux) or LV2 (open, ISC, the Linux-native one); (3) **an
exception to the pure-Rust / no-`unsafe` rules** for one new FFI crate with an ADR, and plugins
running in a **separate helper process** so a crashing plugin can't take the project down (costs
some latency and work); (4) plugins are **outside FilmCraft's control**: they aren't reviewed,
some are paid, many VST3s are Windows / macOS only.

## Rough size
P1 ≈ 30–50 h · P2 ≈ 25–45 h · P3 ≈ 50–100 h.
