# Cloud queue

Jobs for the cloud routine (branch `cloud-work`, only while the PC is down; FORK-PLAN §1a.23).
CPU-verifiable, chosen not to overlap with what the PC run is doing. Take them top to bottom; note
each in `docs/fork/CLOUD-LOG.md` with what is UNVERIFIED ON GPU.

**PC is doing now (2026-10-12 20:00, scheduled run): RS2, RS2b and RS3 are merged; next 6.7b** (was RESOLVE-UI-PLAN: Project
Manager home screen with `projects.*` commands, the Resolve terminology overlay
`crates/ui-egui/src/i18n/terms-resolve.tsv`, then page chrome / layouts), then **6.7b** (Temporal NR moves to the clip, §1a.40), then **Y1** (VA-API
H.264). **Don't touch** `crates/ui-egui/src/{app.rs,menus.rs,theme.rs,workspaces*,i18n/mod.rs}`,
new `projects*` files, the autosave / project file code, `crates/platform`, `crates/hevc`, the
grade model / compiler, `crates/llm` or `.cargo/config.toml`. New user-visible strings need es +
zh-cn entries. The fork build defaults to the **Resolve** workflow (feature
`fork-resolve-default`): a new UI test about Premiere behaviour must set
`prefs.workflow = Premiere`. **Guide pages are read by the Grading Coach at compile time:** a new
page must also be added to `GUIDE_PAGES` in `crates/engine/src/coach_ask.rs`, and its `{ui:}` ids
must be known to the `coach_ui` walk tests.

Done earlier (merged and checked on the PC): GUIDE-1…6, GUIDE-CHECK (covered by
`coach_ask_tests::every_indexed_section_has_provenance`), REVIEW-1, I18N-1, EXPORT-UI-1, KEYMAP-1.

1. **GUIDE-7, `docs/guide/editing.md` (docs only):** cutting on the timeline, JKL shuttle (each
   press doubles the speed), trimming, multicam; link to `proxies.md` / `render-cache.md` (the PC
   writes those) instead of explaining them. Every command id must exist (grep `crates/engine/src`).
   Done = page written with provenance tags, in `GUIDE_PAGES`, README status updated, `coach_ask`
   tests green. GPU owed: none.
2. **GUIDE-8, `docs/guide/color/finishing.md` scopes section (docs only):** waveform, parade,
   vectorscope, histogram: what each shows, which to look at for exposure / balance / skin, the
   real panel labels and ids. Done = section written and tagged, `coach_ask` tests green. GPU owed:
   none.
3. **~~Y1-PREP~~ DONE 2026-10-12 (side lane, `docs/fork/PREP-Y1-6E-U02.md` §1.1: no parser field missing; don't redo).** Was: **Y1-PREP, `crates/h264` only (CPU):** check that the parser exposes every SPS / PPS /
   slice-header field a VA-API H.264 decode needs (`VAPictureParameterBufferH264`,
   `VASliceParameterBufferH264`, `VAIQMatrixBufferH264`: names from the public libva header, no
   code copied), for Constrained Baseline / Main / High, 8-bit 4:2:0 progressive. Add any missing
   fields with unit tests on hand-built or fixture bitstreams (fixtures generated with ffmpeg into
   `target/fixtures/`, never committed). Write the field map to `docs/fork/Y1-FIELDS.md`. Done =
   map + tests, `cargo test -p filmcraft-h264` green. GPU owed: none (Y1 itself is PC work).
4. **I18N-2, i18n catalogs only (CPU), after 6.2 / 6.3 are on hevc-vaapi:** diff en vs es / zh-cn
   for strings added by the proxy and render-cache UI and fill the gaps. Done = i18n tests green.
   GPU owed: none.

Removed 2026-10-12: COACH-WALK-1, G2C-CLIP, RS1-HEADLESS (an owner side lane fixes them on the PC,
FORK-PLAN §1a.40 (c)). Also don't touch the TNR / grade compiler code (step 6.7b is next on the PC).
