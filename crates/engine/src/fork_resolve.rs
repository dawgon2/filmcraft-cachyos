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
            p.appearance.sync_color_theme();
        }
        return;
    }
    // start from Premiere so `set_workflow(Resolve)` really switches (keymap, theme)
    p.workflow = Workflow::Premiere;
    p.pending_resolve_migration = true;
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
