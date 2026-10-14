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

## 2026-10-14 12:17–12:25 UTC (cloud run; stopped because the PC heartbeat came back)

origin/cloud-work was already merged into hevc-vaapi, so cloud-work was fast-forwarded to
`730e95df`. Worked CLOUD-QUEUE:

- **C1-PREP (item 7): not done, superseded.** `docs/fork/DESIGN-C1.md` (2026-10-12) already covers
  what exists, the per-step gaps and the files, so a PREP-C1.md would duplicate it. The PC can strike it.
- `fbf60f1d` **MKV-HANDOFF (item 11)**: `MkvSource` and `MxfSource` store the hand-off scale
  (same sanitising as `Mp4Source`: finite, in (0, 1), else 1.0) and make their decoders inside
  `hw::with_handoff_scale`. MXF was included because its VC-3 / ProRes decoders also come from
  `make_video_decoder` (VideoToolbox could take ProRes); MPEG-TS / APV build our software
  decoders directly, so they have nothing to scale. `hw.rs` and `mp4.rs` were not touched
  (don't-touch list). New `crates/codecs/tests/handoff_scale.rs`: a recording factory (registered
  ahead of the built-ins, declines, so our decoders still decode) checks MP4 / MKV / MXF:
  unset → 1.0, 0.5 → 0.5, NaN / ∞ / 0 / −0.5 / 1 / 2 → 1.0, and the scale doesn't leak past
  decoder creation. MKV and MXF failed before the change (checked by stashing it).

**Verified on CPU:** `cargo test -p filmcraft-codecs --lib --test mxf_oracle --test
handoff_scale` (113 + 9 + 3 ok), `cargo clippy -p filmcraft-codecs --all-targets -D warnings`,
`cargo fmt --check`. No dependency changes, so `xtask layers` is unaffected.

**UNVERIFIED ON GPU / owed by the PC:** one 4K-from-6.2K **MKV** export on the PC with Intel
(VA-API) decoding, checking the hand-off copies are made at the export scale (as for MP4 in
U0.2.3b d). Full `cargo xtask ci` at the next boundary.

**Questions for the owner:** none.

**Known problems:** none found.

**Cloud's exact next step:** CLOUD-QUEUE item 9 **R1-PREP** (`docs/fork/PREP-R1.md`, docs only),
then item 10 **TEARDOWN-FLAKE** (`tests/ui/teardown_ui.rs`, needs 3 parallel runs of the ui test
binary with `RUST_TEST_THREADS=12`; long build, start it early).
