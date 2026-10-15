//! User shader effects (fork Y3.4, ADR 0006): load `<data dir>/Effects/*.wgsl` (and the bundled
//! examples), validate each with `filmcraft-usershader` and register the good ones as effects
//! (`filmcraft_project::effect::register_user_effects`). Files that fail keep their message for
//! the UI (`effects.listUser`).

use filmcraft_project::ParamValue;
use filmcraft_project::effect::{ParamKind, UserEffectSpec, UserParamSpec};
use filmcraft_usershader::{ParamKind as K, UserEffect};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{Result, Session};

/// Most effect files read from the folder.
pub const MAX_FILES: usize = 256;

/// The bundled examples (original work, ADR 0006): (file stem, source).
pub const EXAMPLES: &[(&str, &str)] = &[
    ("chromatic-aberration", include_str!("../../../assets/effects/chromatic-aberration.wgsl")),
    ("glitch", include_str!("../../../assets/effects/glitch.wgsl")),
    ("vhs", include_str!("../../../assets/effects/vhs.wgsl")),
    ("light-leak", include_str!("../../../assets/effects/light-leak.wgsl")),
];

/// One effect file's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct UserEffectStatus {
    pub id: String,
    pub name: String,
    /// The file (None: a bundled example).
    pub path: Option<String>,
    /// None: loaded; Some: why not (with the line when there is one).
    pub error: Option<String>,
    /// The parsed effect (for the GPU stage), when it loaded.
    pub effect: Option<UserEffect>,
}

/// `<data dir>/Effects` (None without a data directory).
pub fn folder(s: &Session) -> Option<std::path::PathBuf> {
    s.prefs_path.as_ref().and_then(|p| p.parent()).map(|d| d.join("Effects"))
}

/// Effects turned off because the GPU stopped while they ran: `<Effects>/quarantine.json`, a list of
/// `{"id", "hash"}` (ADR 0006). A changed file has a new hash and loads again.
pub const QUARANTINE_FILE: &str = "quarantine.json";
/// Most quarantine entries kept (the oldest are dropped).
const MAX_QUARANTINE: usize = 256;
/// Largest quarantine file read.
const MAX_QUARANTINE_BYTES: u64 = 256 * 1024;

/// The quarantined (id, hash) pairs in `dir` (empty when the file is missing, too large or not
/// a valid list).
pub fn read_quarantine(dir: &std::path::Path) -> Vec<(String, u64)> {
    let path = dir.join(QUARANTINE_FILE);
    if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_QUARANTINE_BYTES) {
        return Vec::new();
    }
    let Ok(v) = std::fs::read_to_string(&path).map_err(|_| ()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ())) else {
        return Vec::new();
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("hash")?.as_str()?.parse::<u64>().ok()?)))
                .filter(|(id, _)| filmcraft_project::effect::is_user_effect(id))
                .take(MAX_QUARANTINE)
                .collect()
        })
        .unwrap_or_default()
}

/// Write `keys` as the quarantine list of `dir` (hashes as decimal strings: JSON readers that
/// use doubles keep them exact).
fn write_quarantine_list(dir: &std::path::Path, keys: &[(String, u64)]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let v: Vec<Value> = keys.iter().map(|(id, h)| json!({"id": id, "hash": h.to_string()})).collect();
    let text = serde_json::to_string_pretty(&v).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{QUARANTINE_FILE}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(QUARANTINE_FILE))
}

/// Add the user effects that were on the GPU when it stopped (`filmcraft_gpu`'s device-lost
/// hook) to the quarantine of the Effects folder `dir`.
pub fn quarantine(dir: &std::path::Path, keys: &[(String, u64)]) -> std::io::Result<()> {
    let mut all = read_quarantine(dir);
    for k in keys.iter().filter(|(id, _)| filmcraft_project::effect::is_user_effect(id)) {
        if !all.contains(k) {
            all.push(k.clone());
        }
    }
    let drop = all.len().saturating_sub(MAX_QUARANTINE);
    all.drain(..drop);
    write_quarantine_list(dir, &all)
}

/// A file stem as an id part: lowercase letters, digits and `-`.
fn slug(stem: &str) -> String {
    let mut out = String::new();
    for c in stem.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').chars().take(48).collect::<String>();
    if out.is_empty() { "effect".into() } else { out }
}

fn spec_of(id: &str, fx: &UserEffect) -> UserEffectSpec {
    let params = fx
        .params
        .iter()
        .map(|p| {
            let (kind, default) = match p.kind {
                K::Float { default, min, max } => (
                    ParamKind::Float { min: f64::from(min), max: f64::from(max), soft_min: f64::from(min), soft_max: f64::from(max), unit: "", decimals: 2 },
                    ParamValue::Float(f64::from(default)),
                ),
                K::Bool(b) => (ParamKind::Bool, ParamValue::Bool(b)),
                K::Color(c) => (ParamKind::Color, ParamValue::Color([c[0], c[1], c[2], 1.0])),
                K::Point(pt) => (
                    ParamKind::Point,
                    ParamValue::Vec2(pt.map_or(filmcraft_geom::Vec2::new(f64::NAN, f64::NAN), |q| filmcraft_geom::Vec2::new(f64::from(q[0]), f64::from(q[1])))),
                ),
            };
            UserParamSpec { id: p.name.clone(), label: p.label.clone(), kind, default }
        })
        .collect();
    UserEffectSpec {
        id: id.to_string(),
        name: fx.name.clone(),
        category: fx.category.clone(),
        params,
        hash: fx.hash,
        module: filmcraft_usershader::compose(fx, "rgba32float").into(),
    }
}

/// One file found: (id, file, name hint, source or why it couldn't be read).
pub(crate) type Found = (String, Option<String>, String, std::result::Result<String, String>);

/// The `.wgsl` files of the Effects folder `dir` (any case of the extension; directories, even
/// ones named `x.wgsl`, are not files and are skipped; an unreadable file is kept with its reason),
/// sorted, at most [`MAX_FILES`].
pub(crate) fn scan(dir: &std::path::Path) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<std::path::PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wgsl")) && !p.is_dir())
        .take(MAX_FILES * 4)
        .collect();
    files.sort();
    let more = files.len().saturating_sub(MAX_FILES);
    if more > 0 {
        log::warn!("{}: {more} effect file(s) past the first {MAX_FILES} ignored", dir.display());
    }
    files
        .into_iter()
        .take(MAX_FILES)
        .map(|f| {
            let stem = f.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            (format!("user.{}", slug(&stem)), Some(f.to_string_lossy().to_string()), stem, read_source(&f))
        })
        .collect()
}

/// An effect file's text, or why it can't be used. Only regular files are read (a FIFO or device
/// named `x.wgsl` would block or never end), at most [`filmcraft_usershader::MAX_SOURCE`] bytes.
fn read_source(f: &std::path::Path) -> std::result::Result<String, String> {
    use std::io::Read;
    let too_large = || format!("the file is larger than {} KB", filmcraft_usershader::MAX_SOURCE / 1024);
    let m = std::fs::metadata(f).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("not a regular file".into());
    }
    if m.len() > filmcraft_usershader::MAX_SOURCE as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(f).and_then(|h| h.take(filmcraft_usershader::MAX_SOURCE as u64 + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() > filmcraft_usershader::MAX_SOURCE {
        // it grew after the size check
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text".to_string())
}

/// Parse each found file; a later file whose id is taken is refused, naming the id.
pub(crate) fn statuses(found: Vec<Found>) -> Vec<UserEffectStatus> {
    let mut out: Vec<UserEffectStatus> = Vec::new();
    for (id, path, hint, text) in found {
        let status = if out.iter().any(|o| o.id == id) {
            UserEffectStatus { error: Some(format!("another file already uses the id `{id}`")), id, name: hint, path, effect: None }
        } else {
            match text
                .and_then(|src| filmcraft_usershader::parse(&src, &hint).map_err(|errs| errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))
            {
                Ok(fx) => UserEffectStatus { id, name: fx.name.clone(), path, error: None, effect: Some(fx) },
                Err(e) => UserEffectStatus { id, name: hint, path, error: Some(e), effect: None },
            }
        };
        out.push(status);
    }
    out
}

/// Read, validate and register every user effect; the outcome per file.
pub fn reload(s: &mut Session) -> Vec<UserEffectStatus> {
    let mut found: Vec<Found> =
        EXAMPLES.iter().map(|(stem, src)| (format!("example.{}", slug(stem)), None, (*stem).to_string(), Ok((*src).to_string()))).collect();
    if let Some(dir) = folder(s) {
        found.extend(scan(&dir));
    }
    let mut out = statuses(found);
    let quarantined = folder(s).map(|d| read_quarantine(&d)).unwrap_or_default();
    for o in out.iter_mut() {
        if let Some(fx) = &o.effect
            && quarantined.contains(&(o.id.clone(), fx.hash))
        {
            o.error =
                Some("turned off: the GPU stopped while this effect was running. Fix the file (a changed file loads again) or run effects.unquarantine".into());
            o.effect = None;
        }
    }
    let specs: Vec<UserEffectSpec> = out.iter().filter_map(|o| o.effect.as_ref().map(|fx| spec_of(&o.id, fx))).collect();
    let ok = filmcraft_project::effect::register_user_effects(&specs);
    for o in out.iter_mut().filter(|o| o.effect.is_some() && !ok.contains(&o.id)) {
        o.error = Some("too many versions of this effect loaded in this session: restart FilmCraft to load it".into());
        o.effect = None;
    }
    for o in out.iter().filter(|o| o.error.is_some()) {
        log::warn!("user effect {}: {}", o.path.as_deref().unwrap_or(&o.id), o.error.as_deref().unwrap_or_default());
    }
    s.user_effects = out.clone();
    out
}

fn to_json(v: &[UserEffectStatus]) -> Value {
    Value::Array(
        v.iter()
            .map(|o| json!({"id": o.id, "name": o.name, "path": o.path, "ok": o.error.is_none(), "error": o.error, "params": o.effect.as_ref().map_or(0, |f| f.params.len())}))
            .collect(),
    )
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "effects.reloadUser",
            label: "Reload User Effects",
            menu: &[],
            shortcut: None,
            params: "{} (reads <data dir>/Effects/*.wgsl again; each file's outcome)",
            enabled: always,
            run: |s, _| -> Result<Value> { Ok(to_json(&reload(s))) },
            journal: false,
        },
        CommandSpec {
            id: "effects.listUser",
            label: "List User Effects",
            menu: &[],
            shortcut: None,
            params: "{} (each user effect file: id, name, path, ok, error)",
            enabled: always,
            run: |s, _| -> Result<Value> { Ok(json!({"folder": folder(s).map(|f| f.to_string_lossy().to_string()), "effects": to_json(&s.user_effects)})) },
            journal: false,
        },
        CommandSpec {
            id: "effects.unquarantine",
            label: "Turn User Effect Back On",
            menu: &[],
            shortcut: None,
            params: "{id?} (removes the effect, or every effect without id, from the GPU-stop quarantine and reloads)",
            enabled: always,
            run: |s, p| -> Result<Value> {
                let id = p.get("id").and_then(Value::as_str);
                if let Some(dir) = folder(s) {
                    let keep: Vec<(String, u64)> = read_quarantine(&dir).into_iter().filter(|(k, _)| id.is_some_and(|i| i != k)).collect();
                    if dir.join(QUARANTINE_FILE).exists() {
                        write_quarantine_list(&dir, &keep).map_err(|e| crate::EngineError::Other(format!("couldn't update {QUARANTINE_FILE}: {e}")))?;
                    }
                }
                Ok(to_json(&reload(s)))
            },
            journal: false,
        },
    ]
}
