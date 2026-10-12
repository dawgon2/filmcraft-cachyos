# Cloud log

Notes from the hourly cloud routine (branch `cloud-work`, works only while the PC is down;
FORK-PLAN §1a.23). The PC reconciles this into FORK-PLAN §7, HANDOFF and OWNER-QUESTIONS.


## 2026-10-12 04:17–04:45 UTC (cloud run; stopped because the PC heartbeat came back)

Merged origin/hevc-vaapi (7514a0b, fast-forward) into cloud-work, then worked CLOUD-QUEUE top
to bottom. Commits (all on `cloud-work`, pushed):

- `a256a0a` **EXPORT-UI-1**: Export ▸ Video, H.265: when the sequence's working space is HDR,
  SDR isn't asked for, the profile is Main (8-bit) and `hdr_available(Hevc)` says yes, a dim line
  under Profile says "HDR export: written as Main 10 (10-bit)" (automation id
  `export.video.hevcMain10Note`; es + zh-cn entries). Logic in `hevc_promoted_to_main10`
  (`panels/export_mode.rs`), mirroring `export::job::hevc_bit_depth_for` / `hdr_output`; unit
  tests (incl. that the HDR probe is only asked when everything else holds).
- `f14ab36` **KEYMAP-1**: the conflicts themselves were already fixed upstream
  (`drop_control_collisions` in `Shortcuts::builtin_for`, merged in U0). Added
  `every_builtin_keymap_has_no_conflicts_on_any_platform` (all 5 built-in presets × Mac / Windows
  / Linux); it passes, so no bindings changed and no guide shortcut list needed edits.
- `14a8887` **REVIEW-1**: `docs/fork/CONTRIBUTION-CANDIDATES.md`, checked against upstream main
  `78fe010` (shallow fetch, read-only, nothing copied). Not in upstream: GOP `MIN_FRAMES` byte
  bound, FrameServer drop fix, 1.6× PAR, `effects.move`, Lanczos-3, sequence presets; store-time
  eviction fix: partly there (probably drop); larger items need maintainers first. The load4 snap
  and Linux keymap "already upstream" claims were not re-checked (keymap: see KEYMAP-1, consistent).
- `3731229` **GUIDE-5**: recipes.md recipe 8 "Fix grey or lifeless skin" (only existing ids).
- `ab2f315` **GUIDE-4**: `docs/guide/export.md` (open the page, presets incl. the five fork H.265
  presets, Auto / VA-API / x265 encoder choice and fallbacks, CBR / VBR / CRF and the pre-U0 CRF
  preset compatibility, HDR → Main 10), added to `GUIDE_PAGES`; export-page ids added to
  `coach_ui.rs` `NOT_WALKED` (the walk never opens Export). **Touched a coach file:**
  `assets/coach/questions.toml` off-topic question "export a video for youtube" is now covered by
  the guide (the test failed with it, correctly), so it was replaced by "how do I stabilise shaky
  footage?" (not covered by any page).
- `830404d` **GUIDE-6**: `docs/guide/color/lumetri.md` (why two tools, order nodes → Lumetri, the
  panel's sections and commands, Color Match vs Shot Match, which to use, what doesn't carry
  over); in `GUIDE_PAGES`; primaries.md link updated; two `lumetri.match.*` ids in `NOT_WALKED`.

Queue item 0 (GUIDE-CHECK remainder): already covered by `coach_ask_tests::
every_indexed_section_has_provenance` (every `##` section of every indexed page); nothing done.

**Verified on CPU:** `cargo test -p filmcraft-ui-egui --lib` (142 ok), `--test ui -- coach_ask`
(3 ok, incl. `coach_ask_every_guide_ui_id_is_known` and the 30 questions), `--test ui -- export_ui
i18n` (14 ok), `cargo test -p filmcraft-engine --lib -- coach shortcuts workflow` (56 ok), clippy
`-D warnings` on ui-egui + engine (all targets), `cargo fmt --check`, `cargo xtask layers`.

**UNVERIFIED ON GPU / owed by the PC:** a screenshot of the Export page with an HDR (Rec. 2100 PQ)
sequence, H.265, Profile Main (8-bit), to check the note's placement; full `cargo xtask ci` at the
next boundary. No GPU code was changed.

**Questions for the owner:** none new.

**Known problems:** Lumetri's sliders have no automation ids (upstream gap), so lumetri.md names
them by label only. The export dialog has no SDR switch for HDR sequences (only `file.exportMedia`
`sdr`); noted in export.md.

**Cloud's exact next step:** CLOUD-QUEUE is empty apart from what the PC adds. Next CPU-safe
work: backfill `docs/guide/editing.md` (cutting, JKL, multicam; proxies / render cache once 6.2 /
6.3 land) and the scopes section of `color/finishing.md`; or a walk extension in `coach_ui.rs`
that opens the Export page so its ids are seen instead of listed in `NOT_WALKED`.
