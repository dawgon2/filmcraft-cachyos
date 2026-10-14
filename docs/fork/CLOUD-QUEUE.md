# Cloud queue

Jobs for the cloud routine (branch `cloud-work`, only while the PC is down; FORK-PLAN §1a.23).
CPU-verifiable, chosen not to overlap with what the PC run is doing. Take them top to bottom; note
each in `docs/fork/CLOUD-LOG.md` with what is UNVERIFIED ON GPU.

**PC is doing now (2026-10-13 21:00, scheduled run): AI1 (AI-PLAN, `crates/llm` own quantized forward pass); don't touch `crates/llm`, `crates/engine/src/ai_explain*`, `.cargo/config.toml`**. Also don't touch `crates/gpu/src/{fx,colorops}*`. after the 6E group was accepted and published. **Don't touch** `crates/platform`,
`crates/hevc`, `crates/codecs` (hybrid / hw factory), `crates/export/src/lanes/`, or the Dell lane's
Linux desktop files (`apps/filmcraft/src/{desktop,mpris,cards,kdeconnect,portal*}.rs`). Earlier rules:
**Don't touch** `crates/ui-egui/src/{app.rs,menus.rs,theme.rs,workspaces*,i18n/mod.rs}`,
new `projects*` files, the autosave / project file code, `crates/platform`, `crates/hevc`, the
grade model / compiler, `crates/llm` or `.cargo/config.toml`. New user-visible strings need es +
zh-cn entries. The fork build defaults to the **Resolve** workflow (feature
`fork-resolve-default`): a new UI test about Premiere behaviour must set
`prefs.workflow = Premiere`. **Guide pages are read by the Grading Coach at compile time:** a new
page must also be added to `GUIDE_PAGES` in `crates/engine/src/coach_ask.rs`, and its `{ui:}` ids
must be known to the `coach_ui` walk tests.

Done earlier (merged and checked on the PC): GUIDE-1…6, GUIDE-CHECK (covered by
`coach_ask_tests::every_indexed_section_has_provenance`), REVIEW-1, I18N-1, EXPORT-UI-1, KEYMAP-1.

1. **~~GUIDE-7~~ DONE 2026-10-13 (PC run): `docs/guide/editing.md` written, in `GUIDE_PAGES`.** Was: **GUIDE-7, `docs/guide/editing.md` (docs only):** cutting on the timeline, JKL shuttle (each
   press doubles the speed), trimming, multicam; link to `proxies.md` / `render-cache.md` (the PC
   writes those) instead of explaining them. Every command id must exist (grep `crates/engine/src`).
   Done = page written with provenance tags, in `GUIDE_PAGES`, README status updated, `coach_ask`
   tests green. GPU owed: none.
2. **~~GUIDE-8~~ DONE 2026-10-13 (PC run):** scopes section of `color/finishing.md`; eval question `uncovered-scopes` now covered.
3. **~~Y1-PREP~~ DONE 2026-10-12 (side lane, `docs/fork/PREP-Y1-6E-U02.md` §1.1: no parser field missing; don't redo).** Was: **Y1-PREP, `crates/h264` only (CPU):** check that the parser exposes every SPS / PPS /
   slice-header field a VA-API H.264 decode needs (`VAPictureParameterBufferH264`,
   `VASliceParameterBufferH264`, `VAIQMatrixBufferH264`: names from the public libva header, no
   code copied), for Constrained Baseline / Main / High, 8-bit 4:2:0 progressive. Add any missing
   fields with unit tests on hand-built or fixture bitstreams (fixtures generated with ffmpeg into
   `target/fixtures/`, never committed). Write the field map to `docs/fork/Y1-FIELDS.md`. Done =
   map + tests, `cargo test -p filmcraft-h264` green. GPU owed: none (Y1 itself is PC work).
4. **~~I18N-2~~ DONE 2026-10-13 (PC run, nothing to add):** every proxy / render-cache string is a
   `tl!` / `tlf!` literal, and `i18n::tests::{spanish_translates_every_tl_literal,
   chinese_translates_every_menu_label_and_tl_literal}` already fail on any gap; don't redo.

Removed 2026-10-12: COACH-WALK-1, G2C-CLIP, RS1-HEADLESS (an owner side lane fixes them on the PC,
FORK-PLAN §1a.40 (c)). Also don't touch the TNR / grade compiler code (step 6.7b is next on the PC).

5. **~~A1-PREP~~ DONE 2026-10-13 (PC run): `docs/fork/PREP-A1.md`; don't redo.** Was: **A1-PREP, docs only (CPU):** read AUDIO-PLAN.md A1 (Linux playback reliability) and audit the
   playback path it names (`crates/audio`, the cpal output in the app, underrun handling, device
   switching); write `docs/fork/PREP-A1.md`: what exists, what A1's steps need per file, risks,
   suggested test list. No code changes. GPU owed: none.
6. **~~Y3-PREP~~ DONE 2026-10-13 (PC run): `docs/fork/PREP-Y3.md`; don't redo.** Was: **Y3-PREP, docs only (CPU):** same for YOUTUBE-PLAN Y3 (GPU keys, attach-to-track / tracked
   blur, screen replacement, WGSL custom shaders, look packs): what exists (keyers, tracker,
   effect registry), the gaps, and a lane split; write `docs/fork/PREP-Y3.md`. No code changes.
   GPU owed: none.
7. **C1-PREP, docs only (CPU):** read COMPOSITOR-PLAN.md C1 and C2 and audit what they build on
   (the grade node model and compiler, `effects` / FxOps, GPU compositing in the renderer, keyers,
   masks, text / title rendering, the ADR list); write `docs/fork/PREP-C1.md`: what exists, gaps
   per C1 / C2 step with the files they touch, risks (serde of the DAG, cycles, GPU memory),
   suggested tests, a lane split. No code changes. GPU owed: none.
8. **~~AI1-PREP~~ DONE 2026-10-13 (PC run): `docs/fork/PREP-AI1.md`; don't redo.** Was: *read AI-PLAN.md AI1 and audit `crates/llm` and every place that
   depends on the x86-64-v3 build flag (`.cargo/config.toml`, FORK-PLAN Q8); write
   `docs/fork/PREP-AI1.md`: the current forward pass and quantisation, what a run-time CPU dispatch
   (`is_x86_feature_detected!`, scalar fallback) needs per file, how to test parity between the
   paths, risks. No code changes. GPU owed: none.*
9. **R1-PREP, docs only (CPU):** read RAW-PLAN.md R1 (CinemaDNG, pure Rust) and audit what exists
   for TIFF / DNG parsing, lossless JPEG, demosaic and the input-transform path; write
   `docs/fork/PREP-R1.md` (exists / gaps per step / fixtures that ffmpeg or a public-domain sample
   could give / risks / tests). No code changes. GPU owed: none.

10. **TEARDOWN-FLAKE, `crates/ui-egui` test only (CPU):** `tests/ui/teardown_ui.rs`
   `dropping_the_app_stops_its_workers_and_frees_its_media` fails under the full parallel
   `cargo xtask ci` run (8 media references left after its 3 s settle) but passes alone. Find what
   still holds the media pool after the app drops under load (frame workers, render-cache job,
   thumbnail jobs) and make closing wait for / cancel it deterministically, or make the test
   measure the right thing; never just raise the timeout. Done = the test passes in 3 full
   parallel runs of the ui test binary with `RUST_TEST_THREADS=12`. GPU owed: none.
11. **MKV-HANDOFF, `crates/codecs` (CPU):** `MediaSource::set_handoff_scale` is implemented for
   MP4 / MOV only (`mp4.rs`, U0.2.3b d). Give `mkv.rs` (and any other source whose decoder comes from
   `make_video_decoder`) the same: store the scale, wrap its decoder creation in
   `hw::with_handoff_scale`. Done = a unit test per source that the scale reaches
   `hw::handoff_scale()` inside decoder creation (a test factory can record it). GPU owed: one
   4K-from-6.2K MKV export on the PC with Intel decoding.
