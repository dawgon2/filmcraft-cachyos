# Cloud queue

Jobs for the cloud routine (branch `cloud-work`, only while the PC is down; FORK-PLAN §1a.23).
CPU-verifiable, chosen not to overlap with what the PC run is doing. Take them top to bottom; note
each in `docs/fork/CLOUD-LOG.md` with what is UNVERIFIED ON GPU.

**PC is doing now (2026-10-11, end of session 13): G2 is done (tag `grade-2`); next the PC
starts Phase G2b** (G2.11 local AI explanations in a new crate beside `crates/speech`, G2.12
Grading Coach: lesson engine in `crates/ui-egui`, ask-the-coach over `docs/guide/`). **Don't
touch** the grade model / compiler / panels (`crates/render/src/grade*.rs`,
`crates/project/src/grade.rs`, `crates/engine/src/grade*.rs`, `crates/engine/src/masks.rs`,
`ui-egui/src/panels/grade.rs`, `ui-egui/src/panels/masks.rs`), `crates/speech`, or
`ui-egui/src/automation.rs`. New user-visible strings need es + zh-cn entries.

0. **GUIDE-CHECK, `xtask` only (CPU):** a `cargo xtask guide` subcommand (and a call from
   `cargo xtask ci`) that parses every page under `docs/guide/` and fails when a named command id
   (backticked `area.name` matching the engine's command list: read the ids from
   `crates/engine/src/*.rs` `spec(` / `CommandSpec { id:` literals) or automation id
   (`{ui:…}`; ids are registered at runtime, so check them against a list the UI exports: if no
   such list exists, only check that `{ui:…}` ids use the documented id patterns and report them
   as "unverified") doesn't exist, or a `##` section lacks a provenance tag ([Resolve-style] /
   [FilmCraft] / [Fork] / [Standard]). Done = subcommand + tests, the current guide passes (fix
   the guide, not the check, when a real id is wrong). GPU owed: none. (G2b's coach depends on it.)
1. **GUIDE-1, `docs/guide/` (docs only):** `glossary.md`: plain-language definitions (log, gamut,
   working space, node, serial node, wheel, curve, still, wipe, key, matte, LUT, grain, proxy,
   render cache), each with the Resolve and Premiere names side by side, our own words.
   Done = page written, every term tagged with its provenance (README rules), README status
   updated. GPU owed: none.
2. **GUIDE-2, `docs/guide/hardware.md` (docs only):** what runs where on this machine (VA-API
   decode / encode on the AMD VCN, Intel as second device, zero-copy display, CPU fallbacks via the
   ffmpeg pipe, the Settings switches and env vars from `docs/fork/HANDOFF.md` "Environment"), in
   user language. Done = page written with provenance tags. GPU owed: none.
3. **GUIDE-3, `docs/guide/color/recipes.md` (docs only):** 4–6 recipes using only G0 / G1 features
   (normalise F-Log2 with a starter node tree, warm a shot with the wheels, match two shots with a
   still and the wipe, copy a grade to a scene, add grain). Every command id must exist (grep
   `crates/engine/src`). Done = page written. GPU owed: none.
4. **REVIEW-1 (no code change):** read `UPSTREAM-PLAN.md` "Contributing back" and list, per
   candidate, whether upstream `main` already has an equivalent (by reading upstream's code on
   GitHub, never copying), in `docs/fork/CONTRIBUTION-CANDIDATES.md`. GPU owed: none.
5. **I18N-1, `crates/ui-egui` i18n catalogs only:** zh-cn entries for the fork's HEVC export
   presets and node-command labels that still fall back to English (find them by diffing the en
   and zh-cn catalogs). Done = the catalogs match, `cargo test -p filmcraft-ui-egui` i18n tests
   green. GPU owed: none.
6. **EXPORT-UI-1, export dialog (`crates/ui-egui`, export panel only):** when HDR is on with
   H.265, say in the dialog that the export is promoted to Main 10 (one line, en / es / zh-cn).
   Done = text shown (unit / ui test), crate tests green. GPU owed: a screenshot check of the
   dialog on the PC.
7. **GUIDE-4, `docs/guide/export.md` (docs only):** export presets, hardware HEVC (VA-API) and
   when it falls back, CRF vs bitrate modes (upstream's model since U0; old fork presets with a CRF
   load as CRF), HDR → Main 10. Every command id must exist. Done = page written with provenance
   tags, README status updated. GPU owed: none.
