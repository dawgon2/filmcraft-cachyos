# Upstream syncs (fork)

How the fork (`hevc-vaapi`) takes in upstream [storytold/filmcraft](https://github.com/storytold/filmcraft).
Plan: [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md); strategy: [ADR 0005](../adr/0005-upstream-sync.md).
Newest sync first. All benchmark numbers are synthetic (generated `xm5` fixtures, quick passes).

## Sync 1 (Phase U0, 2026-10-11): upstream `4e2d0bc2`, tag `upstream-sync-1`

**Merge commit:** `b2ef7689` on `hevc-vaapi` (tree = the sync branch's final tree `12e53006`).

**Base.** The fork's `master` is a one-commit snapshot whose tree equals upstream `8fcad73`, so a
local `git replace --graft master 8fcad73` (never pushed) gave the merge its true base. The result
is one merge commit on `hevc-vaapi` (parents: fork tip, upstream `4e2d0bc2`); after it, later syncs
are ordinary merges and need no graft. The work happened on the local branch
`sync/upstream-2026-10-11` (checkpoint `972a90ba`, fixes up to `12e53006`); that branch was
local only (kept, not pushed).

**What came in.** 806 upstream commits since `8fcad73` (563 files, about +97k / −7k lines), all of
it except what is listed under "Resolutions". Highlights: Spanish and Chinese interface, VA-API
H.264 / HEVC hardware decode on Linux (upstream's own `crates/platform/src/vaapi/`), Windows
Media Foundation / DXVA decode, NVENC work, E-AC-3 and MP2 audio, narrations / text to speech, notes,
multi-stream audio, Morph Cut along optical flow, shared docking and many timeline / trim / bin /
caption fixes (#259, #411, #483, #648, #653, #712, …), bounded file-descriptor pool, hardened MP4
`rtmd` parsing, `cargo xtask parity`.

**Conflicts.** 55 files conflicted textually (93 files were changed on both sides). Four
resolvers worked by cluster: platform / decode, GPU / render / colour, export / format,
engine / UI / apps. Then compile fixes (cpal 0.18 lock, duplicate definitions, missing match arms
and fields), clippy / fmt, per-crate test fixes, and an independent review in three sweeps
(nothing lost on either side).

**Resolutions** (owner questions Q1–Q4 in [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), all answered):
- **Project schema:** upstream's numbering wins (v13 multi-stream audio, v14 narrations, v15
  notes); the fork's grades and stills are **v16**. Fork files saved as v13 / v14 are detected and
  upgraded through upstream's chain (Q1).
- **Export CRF:** upstream's bitrate-mode model; fork presets with a numeric `crf` and no
  `bitrateMode` load as CRF mode, with a test (Q2, `5d2869a3`). HDR H.265 export is always Main 10
  (`8ea67999`).
- **VA-API:** both stacks compiled in (stage 1). Linux decoder chain: the fork's HEVC decoder with
  the zero-copy path → upstream's VA-API decoder (H.264 and anything the fork's refuses) → ffmpeg
  pipe (renamed `vaapi_pipe` to end the module clash). One stack is **U0.2**, right before Phase 8
  (owner, Q3).
- **Lumetri:** upstream's single GPU `OP_LUMETRI` op is retired; Lumetri runs as the fork's
  ColorOps chain (ADR 0003) with upstream's vignette roundness fix (#719) ported and upstream's
  Lumetri tests run against the chain (Q4).
- **Render plans:** a new `PlanStep::Output` carries the output transform; DWG / DI stays on the
  GPU for the monitor and SDR export; HDR export frames render on the CPU in a pool renderer (Q4).
- **`media.setInterpretation` `pixelAspect`:** accepts `[n,d]`, `"N:D"`, an id, `"file"` or
  `"auto"`; the response is upstream's `[n,d]` plus `pixelAspectOverride` (Q4).
- **Audio:** the fork's cpal 0.18 + PulseAudio kept, upstream's device fallback / input formats
  ported onto it; shuttle audio keeps the fork's reverse / forward varispeed up to 2× (Q4).
- **GPU texel snap:** `load4` keeps the fork's 1/512 px snap (needed on RADV and Intel).
- **i18n:** Spanish and Chinese entries added for the fork's pages, node grader, HEVC presets and
  PAR strings (`5279b4d3`); zh-cn still lacks a few HEVC preset / node-command labels (falls back to
  English).

**Gates** (all on the sync branch's final tree):
- `cargo xtask ci` green (fmt, clippy `-D warnings`, workspace tests in release, layers, assets,
  wasm: 258 test binaries) on 2026-10-11 12:01. The first ci run failed one upstream test,
  `frames::tests::dropping_the_server_stops_its_workers`: it counted every `filmcraft-frame`
  thread in the process and saw other tests' frame servers (27 vs 3); it now counts its own
  server's workers through their references to the shared state.
- GPU tests (gpu, render, platform VA-API) green on AMD (renderD129, 1002:73df) and Intel
  (renderD128, 8086:4c8a).
- xm5 quick pass, synthetic (`target/bench/xm5-u0-quick-*.json`), against grade-1:

  | Scenario | grade-1 | U0 |
  |---|---|---|
  | play-full (frames / drops, frame time) | 240/0, 2.0 ms | 240/0, 2.0 ms |
  | play-graded | 240/0, 3.8 ms | 240/0, 3.4 ms |
  | multicut quick (frames / drops) | 323/37 (same session) | 340/20 |
  | export 4K / graded 4K / 6.2K (fps) | 24.6 / 27.5 / 18.7 | 26.7 / 27.1 / 18.9 |

**Follow-ups** (not blockers): `cropped()` on GPU frames reads back (`clap`-cropped MP4s lose
zero-copy, `crates/frame/src/lib.rs`); two VA displays are opened at startup (U0.2); the export
dialog doesn't say HDR H.265 is promoted to Main 10; after a hybrid falls back to software the GOP
cache still counts it as hardware (pre-existing).
