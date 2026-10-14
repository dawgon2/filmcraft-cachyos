//! Look and template packs (fork Y3.5): effect presets, LUTs and graphics templates in one
//! shareable `.fcpack` file.
//!
//! A pack is a zip archive (Deflate) holding:
//! - `pack.json`: `{"format": "filmcraft.pack", "version": 1, "name": …, "presets": […],
//!   "luts": […], "templates": […]}` (the names it carries, for a preview before importing);
//! - `presets.json`: an effect preset file (`presets.export`'s format), when it has presets;
//! - `luts/<name>.cube` or `.3dl`: LUT text as the project library keeps it;
//! - `templates/<name>.fcgt`: graphics templates as the user library keeps them.
//!
//! Packs come from other people, so every entry is treated as hostile: at most [`MAX_ENTRIES`]
//! entries, each at most [`MAX_ENTRY`] bytes unpacked and [`MAX_TOTAL`] in all; only the paths
//! above (one plain file name under `luts/` or `templates/`), no `..`, no absolute or backslash
//! paths; `.mogrt` and anything else refused by name. Importing never overwrites: a preset,
//! LUT or template whose name is taken gets " (2)", " (3)"… instead.

use std::io::{Read, Write};

use filmcraft_color::LutFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, str_p};
use crate::{EngineError, Result, Session};

/// `pack.json`'s format string.
pub const PACK_FORMAT: &str = "filmcraft.pack";
/// The file extension of a pack.
pub const PACK_EXTENSION: &str = "fcpack";
/// Most entries a pack may hold.
pub const MAX_ENTRIES: usize = 2048;
/// Largest single entry, unpacked (a 65³ .cube is about 9 MB of text).
pub const MAX_ENTRY: u64 = 64 << 20;
/// Largest pack, unpacked.
pub const MAX_TOTAL: u64 = 512 << 20;
/// Largest pack file read.
const MAX_FILE: u64 = 512 << 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    presets: Vec<String>,
    #[serde(default)]
    luts: Vec<String>,
    #[serde(default)]
    templates: Vec<String>,
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "packs.export",
            label: "Export Look Pack…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str (.fcpack),"name":str?,"presets":[name]? (default: all your presets),"luts":[id|name]? (default: every LUT in the project),"templates":[id|name]? (default: all your installed templates)}"#,
            enabled: always,
            run: export,
            journal: false,
        },
        CommandSpec {
            id: "packs.import",
            label: "Import Look Pack…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str (.fcpack)} (presets and templates go to your library, LUTs into the project: that part needs an open project)"#,
            enabled: always,
            run: import,
            journal: true,
        },
    ]
}

/// A file name safe to keep: no path parts, no control characters, not empty, at most 120 bytes.
fn safe_name(n: &str) -> Option<String> {
    let n: String = n.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    let n = n.trim().trim_matches('.').trim().to_string();
    if n.is_empty() {
        return None;
    }
    let mut end = n.len().min(120);
    while !n.is_char_boundary(end) {
        end -= 1;
    }
    n.get(..end).map(str::to_string)
}

/// `base`, or `base (2)`, `base (3)`… whichever `taken` doesn't hold.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..10_000).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap_or_else(|| format!("{base} (copy)"))
}

/// The pack's bytes for the chosen presets, LUTs (name, extension, text) and templates (name,
/// `.fcgt` bytes).
fn write_pack(name: &str, presets: &[crate::presets::EffectPreset], luts: &[(String, String, String)], templates: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let err = |e: zip::result::ZipError| EngineError::Other(format!("writing the pack: {e}"));
    let io = |e: std::io::Error| EngineError::Other(format!("writing the pack: {e}"));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut lut_names = Vec::new();
    let mut tpl_names = Vec::new();
    let manifest = |lut_names: &[String], tpl_names: &[String]| Manifest {
        format: PACK_FORMAT.into(),
        version: 1,
        name: name.into(),
        presets: presets.iter().map(|p| p.name.clone()).collect(),
        luts: lut_names.to_vec(),
        templates: tpl_names.to_vec(),
    };
    if !presets.is_empty() {
        z.start_file("presets.json", opts).map_err(err)?;
        z.write_all(&crate::presets::file_bytes(presets)).map_err(io)?;
    }
    for (n, ext, text) in luts {
        let file = unique(&safe_name(n).unwrap_or_else(|| "LUT".into()), |c| lut_names.iter().any(|x: &String| x.eq_ignore_ascii_case(c)));
        z.start_file(format!("luts/{file}.{ext}"), opts).map_err(err)?;
        z.write_all(text.as_bytes()).map_err(io)?;
        lut_names.push(file);
    }
    for (n, bytes) in templates {
        let file = unique(&safe_name(n).unwrap_or_else(|| "Template".into()), |c| tpl_names.iter().any(|x: &String| x.eq_ignore_ascii_case(c)));
        z.start_file(format!("templates/{file}.{}", filmcraft_project::gtemplate::TEMPLATE_EXTENSION), opts).map_err(err)?;
        z.write_all(bytes).map_err(io)?;
        tpl_names.push(file);
    }
    z.start_file("pack.json", opts).map_err(err)?;
    z.write_all(&serde_json::to_vec_pretty(&manifest(&lut_names, &tpl_names)).unwrap_or_default()).map_err(io)?;
    Ok(z.finish().map_err(err)?.into_inner())
}

/// What a pack holds, read and checked (see the module docs for the rules).
#[derive(Debug, Default)]
pub struct PackContents {
    pub name: String,
    pub presets: Vec<crate::presets::EffectPreset>,
    /// (name, format, text)
    pub luts: Vec<(String, LutFormat, String)>,
    /// (name, `.fcgt` bytes)
    pub templates: Vec<(String, Vec<u8>)>,
}

/// Read a pack from its bytes. Errors name the entry that broke a rule.
pub fn read_pack(bytes: &[u8]) -> std::result::Result<PackContents, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a FilmCraft pack: {e}"))?;
    if z.len() > MAX_ENTRIES {
        return Err(format!("the pack has {} entries (at most {MAX_ENTRIES})", z.len()));
    }
    let mut out = PackContents::default();
    let mut manifest: Option<Manifest> = None;
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
        let name = f.name().to_string();
        if f.is_dir() {
            continue;
        }
        if name.contains("..") || name.starts_with('/') || name.contains('\\') || name.contains(':') {
            return Err(format!("`{name}`: not a plain file name"));
        }
        if name.to_ascii_lowercase().ends_with(".mogrt") {
            return Err(format!("`{name}`: FilmCraft does not open .mogrt files"));
        }
        let declared = f.size();
        if declared > MAX_ENTRY {
            return Err(format!("`{name}` is too large ({declared} bytes)"));
        }
        let mut data = Vec::new();
        // (the declared size can lie: read at most one byte past the cap)
        (&mut f).take(MAX_ENTRY + 1).read_to_end(&mut data).map_err(|e| format!("`{name}`: {e}"))?;
        if data.len() as u64 > MAX_ENTRY {
            return Err(format!("`{name}` is too large"));
        }
        total += data.len() as u64;
        if total > MAX_TOTAL {
            return Err("the pack is too large unpacked".into());
        }
        let file = |dir: &str| name.strip_prefix(dir).filter(|r| !r.is_empty() && !r.contains('/'));
        if name == "pack.json" {
            manifest = Some(serde_json::from_slice(&data).map_err(|e| format!("pack.json: {e}"))?);
        } else if name == "presets.json" {
            out.presets = crate::presets::parse_file(&data).map_err(|e| format!("presets.json: {e}"))?;
        } else if let Some(r) = file("luts/") {
            let fmt = LutFormat::from_path(r).ok_or_else(|| format!("`{name}`: a LUT must be .cube or .3dl"))?;
            let text = String::from_utf8(data).map_err(|_| format!("`{name}`: not text"))?;
            let stem = r.rsplit_once('.').map_or(r, |(a, _)| a);
            out.luts.push((safe_name(stem).ok_or_else(|| format!("`{name}`: no name"))?, fmt, text));
        } else if let Some(r) = file("templates/") {
            let stem = r
                .strip_suffix(&format!(".{}", filmcraft_project::gtemplate::TEMPLATE_EXTENSION))
                .ok_or_else(|| format!("`{name}`: a template must be .fcgt"))?;
            out.templates.push((safe_name(stem).ok_or_else(|| format!("`{name}`: no name"))?, data));
        } else {
            return Err(format!("`{name}`: not something a pack holds"));
        }
    }
    let m = manifest.ok_or("not a FilmCraft pack (no pack.json)")?;
    if m.format != PACK_FORMAT {
        return Err(format!("not a FilmCraft pack (format `{}`)", m.format));
    }
    if m.version > 1 {
        return Err(format!("this pack needs a newer FilmCraft (version {})", m.version));
    }
    out.name = m.name;
    Ok(out)
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "packs.export";
    let path = str_p(p, "path").ok_or_else(|| bad(ID, "need `path`"))?;
    let names = |k: &str| p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
    let all = s.presets.all();
    let presets: Vec<crate::presets::EffectPreset> = match names("presets") {
        Some(n) => {
            let v: Vec<_> = all.into_iter().filter(|x| n.contains(&x.name)).collect();
            if v.len() < n.len() {
                return Err(bad(ID, "unknown preset name"));
            }
            v
        }
        None => s.presets.user.clone(),
    };
    let luts: Vec<(String, String, String)> = {
        let pick = names("luts");
        let v: Vec<_> = s
            .project
            .luts
            .iter()
            .filter(|l| pick.as_ref().is_none_or(|n| n.iter().any(|k| *k == l.id || k.eq_ignore_ascii_case(&l.name))))
            .map(|l| (l.name.clone(), l.format.clone(), l.text.to_string()))
            .collect();
        if pick.as_ref().is_some_and(|n| v.len() < n.len()) {
            return Err(bad(ID, "unknown LUT"));
        }
        v
    };
    let templates: Vec<(String, Vec<u8>)> = match names("templates") {
        Some(n) => n
            .iter()
            .map(|k| crate::graphic_templates::find_template(s, k).map(|e| (e.template.name.clone(), e.template.to_json().into_bytes())))
            .collect::<Result<_>>()?,
        None => crate::graphic_templates::library(s)
            .into_iter()
            .filter(|e| e.path.is_some())
            .map(|e| (e.template.name.clone(), e.template.to_json().into_bytes()))
            .collect(),
    };
    if presets.is_empty() && luts.is_empty() && templates.is_empty() {
        return Err(bad(ID, "nothing to put in the pack (no presets, LUTs or templates)"));
    }
    let stem = std::path::Path::new(path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "Look Pack".into());
    let name = str_p(p, "name").map(str::to_string).unwrap_or(stem);
    let bytes = write_pack(&name, &presets, &luts, &templates)?;
    s.services.write_file(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "presets": presets.len(), "luts": luts.len(), "templates": templates.len()}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "packs.import";
    let path = str_p(p, "path").ok_or_else(|| bad(ID, "need `path`"))?;
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_FILE) {
        return Err(bad(ID, format!("{path}: the file is too large for a pack")));
    }
    let bytes = s.services.read_file(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let pack = read_pack(&bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    // presets: renamed when the name is taken
    let mut preset_names = Vec::new();
    for mut pr in pack.presets {
        let all = s.presets.all();
        pr.name = unique(&pr.name, |n| all.iter().any(|x| x.name == n));
        preset_names.push(pr.name.clone());
        s.presets.user.push(pr);
    }
    if !preset_names.is_empty() {
        s.presets.persist().map_err(EngineError::Other)?;
    }
    // LUTs: identical text reuses the project's entry; a different LUT under a taken name is renamed
    let mut lut_refs = Vec::new();
    for (name, fmt, text) in pack.luts {
        let name = if s.project.luts.iter().any(|l| *l.text == *text) { name } else { unique(&name, |n| s.project.luts.iter().any(|l| l.name == n)) };
        let (_, v) = crate::color::add_lut(s, &text, fmt, name, None)?;
        lut_refs.push(v);
    }
    // templates: renamed when a user template of that name exists
    let mut tpl = Vec::new();
    for (name, bytes) in pack.templates {
        let mut t =
            filmcraft_project::gtemplate::GraphicsTemplate::from_bytes(&bytes).map_err(|e| EngineError::Other(format!("{path}: template `{name}`: {e}")))?;
        let lib = crate::graphic_templates::library(s);
        let new_name = unique(&t.name, |n| lib.iter().any(|e| e.path.is_some() && e.template.name.eq_ignore_ascii_case(n)));
        if new_name != t.name {
            t.name = new_name;
            t.id = String::new();
        }
        tpl.push(crate::graphic_templates::install_template_bytes(s, t.to_json().as_bytes(), &format!("{path}: {name}"))?);
    }
    s.toast(format!("Imported look pack \"{}\"", pack.name));
    Ok(json!({"name": pack.name, "presets": preset_names, "luts": lut_refs, "templates": tpl}))
}
