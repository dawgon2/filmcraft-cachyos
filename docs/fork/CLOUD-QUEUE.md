# Cloud queue

Jobs for the cloud routine (branch `cloud-work`, only while the PC is down; FORK-PLAN §1a.23).
CPU-verifiable, chosen not to overlap with what the PC run is doing. Take them top to bottom; note
each in `docs/fork/CLOUD-LOG.md` with what is UNVERIFIED ON GPU.

**PC is doing now (2026-10-12 02:40, scheduled run): Phase 6** (merging 6.2 auto proxies and
6.3 render cache lanes, then 6.1b 4:2:2 decode on the Intel VA engine, then 6.6 2× on 6.2K
originals, then Y1). **Don't touch** `crates/hevc`, `crates/platform` (VA-API),
`crates/engine/src/{proxies.rs,proxy_jobs*.rs,previews.rs,background.rs,playback*.rs}`,
`crates/render/src/{decode*,gop*,frame_server*}.rs`, `crates/ui-egui/src/frames.rs`,
`crates/ui-egui/examples/bench/`, `docs/guide/{proxies,render-cache}.md`, the grade model /
compiler / panels, `crates/llm` or `.cargo/config.toml`. New user-visible strings need es + zh-cn
entries. **Guide pages are read by the Grading Coach at compile time:** a new page must also be
added to `GUIDE_PAGES` in `crates/engine/src/coach_ask.rs`, and its `{ui:}` ids must be known to
the `coach_ui` walk tests.

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
3. **COACH-WALK-1, `crates/ui-egui/tests/ui/coach_ui.rs` only (CPU):** extend the walk so it opens
   the Export page (and the Lumetri match section) and sees their ids, then drop those ids from
   `NOT_WALKED`. Done = the list is shorter, `--test ui -- coach` green. GPU owed: none.
4. **Y1-PREP, `crates/h264` only (CPU):** check that the parser exposes every SPS / PPS /
   slice-header field a VA-API H.264 decode needs (`VAPictureParameterBufferH264`,
   `VASliceParameterBufferH264`, `VAIQMatrixBufferH264`: names from the public libva header, no
   code copied), for Constrained Baseline / Main / High, 8-bit 4:2:0 progressive. Add any missing
   fields with unit tests on hand-built or fixture bitstreams (fixtures generated with ffmpeg into
   `target/fixtures/`, never committed). Write the field map to `docs/fork/Y1-FIELDS.md`. Done =
   map + tests, `cargo test -p filmcraft-h264` green. GPU owed: none (Y1 itself is PC work).
5. **I18N-2, i18n catalogs only (CPU), after 6.2 / 6.3 are on hevc-vaapi:** diff en vs es / zh-cn
   for strings added by the proxy and render-cache UI and fill the gaps. Done = i18n tests green.
   GPU owed: none.
