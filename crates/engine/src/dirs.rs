//! Where FilmCraft keeps its files: the data, config, cache and state directories.
//!
//! On Linux and the other freedesktop systems the per-user install follows the XDG Base
//! Directory specification: data in `$XDG_DATA_HOME/filmcraft` (`~/.local/share/filmcraft`), the
//! media cache in `$XDG_CACHE_HOME/filmcraft` (`~/.cache/filmcraft`), logs and crash logs in
//! `$XDG_STATE_HOME/filmcraft` (`~/.local/state/filmcraft`), and `$XDG_CONFIG_HOME/filmcraft`
//! reserved for settings. An explicit data directory (`--data-dir`, `FILMCRAFT_DATA_DIR`, a
//! portable install) keeps **everything** under that one folder, as before; so do macOS, Windows
//! and the web, where there is one per-user folder.
//!
//! Callers that only know the data directory (most of the engine: it is the parent of
//! `preferences.json`) get the others with [`AppDirs::for_data_dir`]. An older version's
//! `<data>/Logs` and `<data>/Media Cache` are renamed into place once at start-up
//! ([`AppDirs::migrate`]); one that can't be moved keeps being used. Nothing is deleted.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The folder name under each XDG base directory.
pub const APP_DIR: &str = "filmcraft";
/// Log files and crash logs, inside [`AppDirs::state`].
pub const LOGS: &str = "Logs";
/// The media cache's default folder, inside [`AppDirs::cache`].
pub const MEDIA_CACHE: &str = "Media Cache";

/// The four per-user directories. All four are the same folder unless the XDG split applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    /// Auto-save, crash recovery, presets, keyboard shortcuts, templates, downloaded models.
    pub data: PathBuf,
    /// Settings (reserved: `preferences.json` still lives in `data`).
    pub config: PathBuf,
    /// Files FilmCraft can rebuild: the media cache (conformed audio, peaks, render previews).
    pub cache: PathBuf,
    /// Logs, crash logs and the window size (`window_state`).
    pub state: PathBuf,
}

impl AppDirs {
    /// Everything in one folder (explicit data directory, portable install, macOS, Windows).
    pub fn single(dir: &Path) -> Self {
        Self { data: dir.to_path_buf(), config: dir.to_path_buf(), cache: dir.to_path_buf(), state: dir.to_path_buf() }
    }

    /// The directories that go with the data directory `data`, from this process's environment.
    pub fn for_data_dir(data: &Path) -> Self {
        Self::for_data_dir_with(data, xdg_platform(), |k| std::env::var_os(k))
    }

    /// [`AppDirs::for_data_dir`] with the platform and environment passed in (tests).
    ///
    /// The XDG split applies only on an XDG platform, when `FILMCRAFT_DATA_DIR` is unset and `data`
    /// is exactly the per-user default (`$XDG_DATA_HOME/filmcraft` or `~/.local/share/filmcraft`);
    /// any other data directory was chosen on purpose and keeps everything. Relative or empty XDG
    /// variables are ignored, as the specification says.
    pub fn for_data_dir_with(data: &Path, xdg: bool, env: impl Fn(&str) -> Option<OsString>) -> Self {
        if !xdg || env("FILMCRAFT_DATA_DIR").is_some_and(|v| !v.is_empty()) {
            return Self::single(data);
        }
        let Some(home) = env("HOME").filter(|v| !v.is_empty()).map(PathBuf::from) else { return Self::single(data) };
        let base = |var: &str, fallback: &str| xdg_var(&env, var).unwrap_or_else(|| home.join(fallback)).join(APP_DIR);
        if base("XDG_DATA_HOME", ".local/share") != data {
            return Self::single(data);
        }
        Self {
            data: data.to_path_buf(),
            config: base("XDG_CONFIG_HOME", ".config"),
            cache: base("XDG_CACHE_HOME", ".cache"),
            state: base("XDG_STATE_HOME", ".local/state"),
        }
    }

    /// Where log files and crash logs go: `<state>/Logs`, unless an older version's
    /// `<data>/Logs` is still there and could not be moved ([`AppDirs::migrate`]).
    pub fn logs(&self) -> PathBuf {
        self.existing_or_new(LOGS, &self.state)
    }

    /// The media cache's default folder (Settings ▸ Media Cache ▸ Location empty):
    /// `<cache>/Media Cache`, unless an older version's `<data>/Media Cache` is still there and
    /// could not be moved ([`AppDirs::migrate`]), so its files stay visible to Media Cache Info
    /// and Delete.
    pub fn media_cache(&self) -> PathBuf {
        self.existing_or_new(MEDIA_CACHE, &self.cache)
    }

    fn existing_or_new(&self, name: &str, base: &Path) -> PathBuf {
        let new = base.join(name);
        if base != self.data {
            let old = self.data.join(name);
            if old.is_dir() && !new.exists() {
                return old;
            }
        }
        new
    }

    /// Move an older version's `<data>/Logs` and `<data>/Media Cache` to the XDG state and cache
    /// directories (a rename; nothing is copied or deleted). A folder that can't be renamed (another
    /// file system, permissions) stays where it is and keeps being used. Returns a line per folder
    /// for the log. Does nothing when everything lives in one folder.
    pub fn migrate(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, base) in [(LOGS, &self.state), (MEDIA_CACHE, &self.cache)] {
            if *base == self.data {
                continue;
            }
            let (old, new) = (self.data.join(name), base.join(name));
            if !old.is_dir() || new.exists() {
                continue;
            }
            let moved = std::fs::create_dir_all(base).and_then(|()| std::fs::rename(&old, &new));
            out.push(match moved {
                Ok(()) => format!("moved {} to {}", old.display(), new.display()),
                Err(e) => format!("kept {} (could not move it to {}: {e})", old.display(), new.display()),
            });
        }
        out
    }

    /// Paths for `app.paths` (agents, Help ▸ system report).
    pub fn to_json(&self) -> serde_json::Value {
        let s = |p: &Path| p.to_string_lossy().into_owned();
        serde_json::json!({
            "data": s(&self.data),
            "config": s(&self.config),
            "cache": s(&self.cache),
            "state": s(&self.state),
            "logs": s(&self.logs()),
        })
    }
}

/// Does this build follow the XDG Base Directory specification (Linux, the BSDs, …)?
pub fn xdg_platform() -> bool {
    cfg!(all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"), not(target_arch = "wasm32")))
}

/// An XDG base directory variable, if it is set to an absolute path.
fn xdg_var(env: &impl Fn(&str) -> Option<OsString>, var: &str) -> Option<PathBuf> {
    env(var).map(PathBuf::from).filter(|p| p.is_absolute())
}
