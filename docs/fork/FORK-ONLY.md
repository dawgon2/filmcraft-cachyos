# Fork-only code manifest

Everything here must be left out of any branch prepared for upstream (UPSTREAM-PLAN "Contributing
back", FORK-PLAN §1a.25 / §1a.36). New fork-only work follows FORK-PLAN §1a.36: its own crate or
module, one `#[cfg(feature = "fork-ffmpeg")]` hook marked `// FORK-ONLY`, commits prefixed
`FORK-ONLY:`, and an entry below. `grep -rn "FORK-ONLY" crates` finds every hook.

| What | Where | Hook | Since |
|---|---|---|---|
| ProRes RAW via system ffmpeg (planned, RAW-PLAN R2) | `crates/fork-prores-raw` | one line in the app crate, feature `fork-ffmpeg` | — |
| Resolve as the default workflow + one-time prefs migration (RESOLVE-UI-PLAN RS1) | `crates/engine/src/fork_resolve.rs` (+ `fork_resolve_tests.rs`); `Workflow` default in `crates/engine/src/settings.rs` | `cfg_attr` lines on `Workflow` and `cfg(feature = "fork-resolve-default")` hooks in `autosave.rs` (`Preferences::load` / `from_value`) and `lib.rs` (`start_autosave`), all marked `// FORK-ONLY`; engine feature `fork-resolve-default` enabled by `apps/filmcraft` | RS1 |

The RS1 commit also adds generic, upstream-safe pieces (not behind the feature): `PREFS_VERSION` 3,
the `workflowReturnTheme` preference, the `resolve` dark theme (`ThemeKind::Resolve`, View ▸
Appearance ▸ Resolve) and `set_workflow` swapping the dark theme with the keymap.

## Older fork-only code (before §1a.36, not yet isolated)

The pipeline's existing "remove fork-only parts" step still applies to these by hand: ffmpeg
fallbacks in `crates/platform/src/ffmpeg.rs`, `vaapi_pipe.rs`, `hevc_export.rs`,
`crates/engine/src/media_pool.rs`; FORK-PLAN / HANDOFF / `*-PLAN.md` / `docs/fork/`; the X-M5
benches in `crates/ui-egui/examples/bench/`.
