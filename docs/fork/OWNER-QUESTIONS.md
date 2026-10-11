# Owner questions

Decisions the runs could not settle on their own (FORK-PLAN §1a.20). Each has the provisional,
reversible choice the run made so the work could continue. **Answer under the question** (or in
the planning chat); the next run applies the answer and marks it resolved. Never delete answers.

## Q1 (2026-10-11, U0 step 2): project schema numbers collided with upstream

Upstream and the fork both used project schema v13 and v14: upstream v13 = multi-stream audio,
v14 = narrations, v15 = project notes; the fork's v13 = colour grades, v14 = stills gallery.
- **Provisional choice:** upstream's numbers win. Grades + stills become **v16** (a no-op step).
  Fork files saved as v13 / v14 are recognised on load (old single-stream audio object, a
  `stills` list or a clip `grade`) and upgraded from v12 through upstream's chain, keeping the
  grades and stills. New saves carry an envelope marker `"lineage": "fork/hevc-vaapi"`; a v16 file
  *without* the marker (i.e. a future upstream v16 meaning something else) is refused with a clear
  error instead of being misread (`FormatError::OtherLineage`, `crates/format/src/lib.rs`).
- **Options:** (a) the above; (b) keep the fork's numbers and renumber upstream's (breaks loading
  projects saved by upstream builds); (c) refuse ambiguous v13 / v14 files.
- **To reverse:** change the detection in `crates/format/src/lib.rs` (`fork_shape`) and the
  refusal; tests in the same crate.

**Owner's answer (2026-10-11, via the planning chat): keep (a).** Resolved; nothing to change.

## Q2 (2026-10-11, U0 step 2): export CRF setting model

Both sides added CRF: the fork as `crf: Option<u8>` (set = constant quality), upstream as
`bitrate_mode: Crf` + `crf: f32`.
- **Provisional choice:** upstream's model (`bitrate_mode` decides; the fork's HEVC encoders take
  `crf` when the mode is CRF; old `"crf": null` reads as the default). Consequence: a user preset
  saved by the fork with `"crf": 22` but no `"bitrateMode": "crf"` loads as bitrate mode; re-pick
  CRF once.
- **To reverse:** map a numeric `crf` without `bitrateMode` to CRF mode in `deserialize_crf` /
  the preset loader (`crates/export`).

**Owner's answer (2026-10-11): keep upstream's model, plus the compatibility fix:** a numeric `crf` without `bitrateMode` loads as CRF mode (`deserialize_crf` / the preset loader in `crates/export`), with a test, so no fork preset silently changes. Apply it, then mark resolved.

## Q3 (2026-10-11, U0 step 2): two VA-API decoders during the merge

Upstream has its own VA-API H.264 / HEVC decoder (`crates/platform/src/vaapi/`); the fork has its
own HEVC decoder with the zero-copy display path (`va.rs`, `va_hevc.rs`). The fork's ffmpeg pipe
module was renamed `vaapi_pipe` to end the name clash.
- **Provisional choice (stage 1):** both compiled in; the decoder chain on Linux is the fork's
  HEVC decoder → upstream's VA-API decoder (H.264 and anything the fork's refuses) → ffmpeg pipe;
  `hw_backend` reports plain "VA-API" (upstream's convention). UPSTREAM-PLAN asks for one stack
  (upstream's decoder + the fork's zero-copy path on top): that consolidation is U0 stage 2 and is
  only done if it keeps the fork's 6.2K numbers and conformance; otherwise the fork's HEVC decoder
  stays and this question stays open.
- **To reverse:** the order of the chain in `vaapi_factory` (`crates/platform/src/lib.rs`).

**Owner's answer (2026-10-11): keep the staged approach, and go ahead with stage 2** (one VA-API stack: upstream's decoder + the fork's zero-copy path) as UPSTREAM-PLAN plans, with the same 6.2K and conformance gate. **New owner direction: phase out ffmpeg from the product** (FORK-PLAN §1a.24): don't add new ffmpeg dependencies, and the ffmpeg pipe leaves the decoder chain in Phase 7.7.

**Owner update (2026-10-11, live in the runner chat): stage 2 moves to right before Phase 8** (U0.2,
FORK-PLAN §1a.6). U0 merges with both stacks (stage 1) once its gates are green. Resolved for U0.

## Q4 (2026-10-11, U0 step 2): smaller merge calls

Recorded so they can be undone; all keep both sides working.
- **Lumetri:** upstream's single GPU `OP_LUMETRI` op was removed; Lumetri runs as the fork's
  ColorOps chain (ADR 0003) with upstream's vignette roundness fix (#719) ported and upstream's
  Lumetri tests run against the chain.
- **Render plans:** new `PlanStep::Output` carries the output transform in upstream's step plans;
  DWG / DI stays on the GPU for monitor and SDR export; HDR export frames render on the CPU inside
  a pool renderer (correct, slightly slower).
- **`media.setInterpretation` `pixelAspect`:** accepts `[n,d]`, `"N:D"`, an id, `"file"` or
  `"auto"`; the response is upstream's `[n,d]` plus `pixelAspectOverride`.
- **Audio:** the fork's cpal 0.18 + PulseAudio (fixes the fork's issue #23) kept, with upstream's
  device fallback / input formats ported onto it. Shuttle audio keeps the fork's varispeed
  (reverse and forward up to 2×) instead of upstream's forward-only mix rate.

**Owner's answer (2026-10-11): fine, all accepted.** Resolved.
