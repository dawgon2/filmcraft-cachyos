# Fork-only code manifest

Everything here must be left out of any branch prepared for upstream (UPSTREAM-PLAN "Contributing
back", FORK-PLAN §1a.25 / §1a.36). New fork-only work follows FORK-PLAN §1a.36: its own crate or
module, one `#[cfg(feature = "fork-ffmpeg")]` hook marked `// FORK-ONLY`, commits prefixed
`FORK-ONLY:`, and an entry below. `grep -rn "FORK-ONLY" crates` finds every hook.

| What | Where | Hook | Since |
|---|---|---|---|
| ProRes RAW via system ffmpeg (planned, RAW-PLAN R2) | `crates/fork-prores-raw` | one line in the app crate, feature `fork-ffmpeg` | — |
| Resolve as the default workflow + one-time prefs migration (planned, RESOLVE-UI-PLAN RS1) | `crates/engine/src/settings.rs` `Workflow` default / prefs migration | `cfg_attr` lines marked `// FORK-ONLY`, engine feature `fork-resolve-default` enabled by `apps/filmcraft` | — |

## Older fork-only code (before §1a.36, not yet isolated)

The pipeline's existing "remove fork-only parts" step still applies to these by hand: ffmpeg
fallbacks in `crates/platform/src/ffmpeg.rs`, `vaapi_pipe.rs`, `hevc_export.rs`,
`crates/engine/src/media_pool.rs`; FORK-PLAN / HANDOFF / `*-PLAN.md` / `docs/fork/`; the X-M5
benches in `crates/ui-egui/examples/bench/`.
