//! FORK-ONLY (RESOLVE-UI-PLAN RS1, FORK-PLAN §1a.36): Resolve is the fork's default workflow.
//!
//! Compiled only with the engine feature `fork-resolve-default` (enabled by `apps/filmcraft`).
//! Preferences written before RS1 (file `version` < 3), and a missing or unreadable preferences
//! file (a fresh install), ask for a one-time switch to the Resolve workflow. Loading cannot do
//! it alone (the switch also swaps the keyboard preset and remembers the previous one), so
//! [`after_load`] / [`fresh`] load the preferences as Premiere and set
//! `pending_resolve_migration`; [`apply_pending`] then runs `Session::set_workflow` once. That
//! saves version 3, so a later explicit switch back to Premiere is never migrated again.

use crate::Session;
use crate::autosave::Preferences;
use crate::settings::{self, Workflow};

/// A preferences file read as `file_version`: older than RS1 asks for the migration.
pub fn after_load(p: &mut Preferences, file_version: u64, file_was_resolve: bool) {
    if file_version < 3 {
        ask(p, file_was_resolve);
    }
}

/// Preferences for a fresh install (no readable file).
pub fn fresh() -> Preferences {
    let mut p = Preferences::default();
    ask(&mut p, false);
    p
}

fn ask(p: &mut Preferences, was_resolve: bool) {
    if was_resolve {
        // already in Resolve (chosen by hand before RS1): the keyboard stays as the file left it;
        // only the Resolve look is added, remembering the dark theme in use
        if p.appearance.dark_theme != settings::RESOLVE_THEME {
            p.workflow_return_theme = std::mem::replace(&mut p.appearance.dark_theme, settings::RESOLVE_THEME.into());
            p.workflow_auto_theme = settings::RESOLVE_THEME.into();
            p.appearance.sync_color_theme();
        }
        return;
    }
    // start from Premiere so `set_workflow(Resolve)` really switches (keymap, theme)
    p.workflow = Workflow::Premiere;
    p.pending_resolve_migration = true;
}

/// A session that runs without a data directory (headless CLI and MCP runs; the desktop app when
/// it has none): it reports the default workflow, Resolve, so it takes the Resolve keyboard preset
/// and look too, as `Session::set_workflow` would leave them (they used to stay FilmCraft
/// Default). The previous keyboard and dark theme are remembered for a switch to Premiere. No-op
/// once a data directory is in use (its preferences decide, [`apply_pending`]), in another
/// workflow, or when the keyboard is already Resolve's. Nothing is written.
pub fn without_data_dir(s: &mut Session) {
    use crate::shortcuts::RESOLVE_PRESET;
    if s.prefs_path.is_some() || s.prefs.workflow != Workflow::Resolve {
        return;
    }
    // (an edited keyboard is left alone: without a data directory it couldn't be kept as a preset)
    if s.shortcuts.preset != RESOLVE_PRESET
        && !s.shortcuts.modified
        && let Some(b) = s.shortcuts.builtin(RESOLVE_PRESET)
    {
        s.prefs.workflow_return_preset = std::mem::replace(&mut s.shortcuts.preset, RESOLVE_PRESET.into());
        s.shortcuts.bindings = b;
        s.shortcuts.revision = s.shortcuts.revision.wrapping_add(1);
    }
    let p = &mut s.prefs;
    if p.appearance.dark_theme != settings::RESOLVE_THEME && p.workflow_return_theme.is_empty() {
        p.workflow_return_theme = std::mem::replace(&mut p.appearance.dark_theme, settings::RESOLVE_THEME.into());
        p.workflow_auto_theme = settings::RESOLVE_THEME.into();
        p.appearance.sync_color_theme();
    }
}

/// The marker left in the data directory once the switch has run. Older builds rewrite the
/// preferences' `version` (back to 2) but never touch this file, so a Premiere choice made after
/// the switch survives running an older build in between.
pub const MARKER: &str = "fork-resolve-migrated";

/// Run the pending one-time switch (after the keyboard presets are loaded), unless the marker
/// says it already ran for an existing preferences file.
pub fn apply_pending(s: &mut Session, data_dir: &std::path::Path) {
    if !std::mem::take(&mut s.prefs.pending_resolve_migration) {
        return;
    }
    let marker = data_dir.join(MARKER);
    let prefs_exist = s.prefs_path.as_ref().is_some_and(|p| p.exists());
    if prefs_exist && marker.exists() {
        // (an older build rewrote the version: the user's workflow choice stands)
        return;
    }
    match s.set_workflow(Workflow::Resolve) {
        Ok(_) => {
            if let Err(e) = std::fs::write(&marker, b"RS1: switched to the Resolve workflow once\n") {
                log::warn!("writing {}: {e}", marker.display());
            }
        }
        Err(e) => {
            // keep the preferences pre-RS1 so the next launch tries again
            s.prefs.version = 2;
            log::warn!("switching to the Resolve workflow: {e}");
        }
    }
}
