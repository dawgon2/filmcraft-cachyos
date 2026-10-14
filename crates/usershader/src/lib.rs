//! User shader effects (fork Y3.4, [ADR 0006](../../../docs/adr/0006-user-shader-effects.md)).
//!
//! A user effect is one `.wgsl` file: `// @…` annotation lines describe the effect and its
//! parameters, and the code defines `fn effect(px: vec2<f32>) -> vec4<f32>`, which returns the
//! output pixel at layer pixel `px` as straight, display-referred (sRGB-encoded) colour. This crate
//! parses the annotations, refuses what user code may not declare, composes the code with
//! FilmCraft's prelude (bindings, `P`, `size`, `time`, `load`, `sample`) and entry point, and
//! validates the result with naga. Errors point at the user's own lines. No GPU here: the GPU stage
//! (`filmcraft-gpu`) compiles [`compose`]'s output.
//!
//! Annotation grammar (one per line, anywhere in the file):
//!
//! ```text
//! // @effect "Name"
//! // @category "Folder"
//! // @param <name>: float = <default> [<min>, <max>] "Label"
//! // @param <name>: bool = true|false "Label"
//! // @param <name>: color = #rrggbb "Label"
//! // @param <name>: point = auto | <x>, <y> "Label"
//! ```

use std::fmt;

/// Largest effect file accepted.
pub const MAX_SOURCE: usize = 256 * 1024;
/// Most parameters an effect may declare (one `vec4` uniform slot each).
pub const MAX_PARAMS: usize = 16;
/// Longest effect, category, label or parameter name.
const MAX_TEXT: usize = 80;

/// A parameter's kind and default.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Float {
        default: f32,
        min: f32,
        max: f32,
    },
    Bool(bool),
    /// Straight sRGB-encoded 0..1.
    Color([f32; 3]),
    /// Layer pixels; None: the layer's centre.
    Point(Option<[f32; 2]>),
}

/// One `// @param` line.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
}

/// A parsed, validated effect.
#[derive(Clone, Debug, PartialEq)]
pub struct UserEffect {
    pub name: String,
    pub category: Option<String>,
    pub params: Vec<Param>,
    /// The user's code (annotations included), as given.
    pub source: String,
    /// FNV-1a of `source`: a changed file is a different effect version.
    pub hash: u64,
}

/// An error in the user's file. `line` is 1-based in that file; None: not tied to a line.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderError {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

fn err(line: Option<usize>, message: impl Into<String>) -> ShaderError {
    ShaderError { line, message: message.into() }
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A quoted string at the start of `s` (no escapes): (text, rest).
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start().strip_prefix('"')?;
    let end = s.find('"')?;
    Some((s.get(..end)?, s.get(end + 1..)?))
}

fn clean_text(t: &str, line: usize, what: &str) -> Result<String, ShaderError> {
    if t.chars().any(char::is_control) || t.trim().is_empty() || t.chars().count() > MAX_TEXT {
        return Err(err(Some(line), format!("{what} must be 1–{MAX_TEXT} printable characters")));
    }
    Ok(t.trim().to_string())
}

fn number(s: &str, line: usize) -> Result<f32, ShaderError> {
    let v: f32 = s.trim().parse().map_err(|_| err(Some(line), format!("`{}` is not a number", s.trim())))?;
    if v.is_finite() { Ok(v) } else { Err(err(Some(line), "numbers must be finite")) }
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase() || f == '_')
        && n.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && n.len() <= 32
        && !n.starts_with("fx_")
}

/// Parse one `@param` line's body (after `@param`).
fn param(body: &str, line: usize) -> Result<Param, ShaderError> {
    let (name, rest) = body.split_once(':').ok_or_else(|| err(Some(line), "expected `@param name: kind = default`"))?;
    let name = name.trim();
    if !valid_name(name) {
        return Err(err(Some(line), format!("`{name}`: a parameter name is lowercase letters, digits and _, starts with a letter or _, at most 32, not fx_…")));
    }
    let (kind, rest) = rest.split_once('=').ok_or_else(|| err(Some(line), format!("`{name}` needs a default (`= …`)")))?;
    // the label, if any, is the trailing quoted string
    let (value, label) = match rest.find('"') {
        Some(q) => {
            let (l, tail) = quoted(rest.get(q..).unwrap_or_default()).ok_or_else(|| err(Some(line), "unterminated label"))?;
            if !tail.trim().is_empty() {
                return Err(err(Some(line), "nothing may follow the label"));
            }
            (rest.get(..q).unwrap_or_default().trim(), clean_text(l, line, "a label")?)
        }
        None => (rest.trim(), name.to_string()),
    };
    let kind = match kind.trim() {
        "float" => {
            let (d, range) = match value.split_once('[') {
                Some((d, r)) => (d, Some(r.strip_suffix(']').ok_or_else(|| err(Some(line), "the range must end with `]`"))?)),
                None => (value, None),
            };
            let default = number(d, line)?;
            let (min, max) = match range {
                Some(r) => {
                    let (a, b) = r.split_once(',').ok_or_else(|| err(Some(line), "a range is `[min, max]`"))?;
                    (number(a, line)?, number(b, line)?)
                }
                None => (0.0, 1.0),
            };
            if min >= max {
                return Err(err(Some(line), "the range's min must be below its max"));
            }
            if default < min || default > max {
                return Err(err(Some(line), format!("the default {default} is outside [{min}, {max}]")));
            }
            ParamKind::Float { default, min, max }
        }
        "bool" => match value {
            "true" => ParamKind::Bool(true),
            "false" => ParamKind::Bool(false),
            v => return Err(err(Some(line), format!("a bool default is true or false, not `{v}`"))),
        },
        "color" => {
            let hex = value.strip_prefix('#').filter(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()));
            let hex = hex.ok_or_else(|| err(Some(line), "a color default is #rrggbb"))?;
            let ch = |i: usize| hex.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map_or(0.0, |v| f32::from(v) / 255.0);
            ParamKind::Color([ch(0), ch(2), ch(4)])
        }
        "point" => {
            if value == "auto" {
                ParamKind::Point(None)
            } else {
                let (x, y) = value.split_once(',').ok_or_else(|| err(Some(line), "a point default is `auto` or `x, y`"))?;
                ParamKind::Point(Some([number(x, line)?, number(y, line)?]))
            }
        }
        k => return Err(err(Some(line), format!("unknown kind `{k}` (float, bool, color, point)"))),
    };
    Ok(Param { name: name.to_string(), label, kind })
}

/// What user code may not contain (the prelude owns bindings and the entry point).
const FORBIDDEN: &[(&str, &str)] = &[
    ("@group", "bindings"),
    ("@binding", "bindings"),
    ("@compute", "entry points"),
    ("@vertex", "entry points"),
    ("@fragment", "entry points"),
    ("@workgroup_size", "entry points"),
    ("var<storage", "storage buffers"),
    ("var<workgroup", "workgroup memory"),
    ("var<uniform", "uniforms"),
    ("override ", "pipeline overrides"),
    ("enable ", "`enable` directives"),
    ("requires ", "`requires` directives"),
    ("fx_", "names starting with fx_ (FilmCraft's own)"),
];

/// Parse and validate an effect file. `name_hint` names it when it has no `@effect` line (the
/// file stem). Every problem found is reported, user lines first.
pub fn parse(source: &str, name_hint: &str) -> Result<UserEffect, Vec<ShaderError>> {
    if source.len() > MAX_SOURCE {
        return Err(vec![err(None, format!("the file is larger than {} KB", MAX_SOURCE / 1024))]);
    }
    let mut errors = Vec::new();
    let (mut name, mut category, mut params) = (None, None, Vec::<Param>::new());
    for (i, l) in source.lines().enumerate() {
        let n = i + 1;
        let t = l.trim_start();
        if let Some(a) = t.strip_prefix("//").map(str::trim_start).and_then(|c| c.strip_prefix('@')) {
            let (key, body) = a.split_once(char::is_whitespace).unwrap_or((a, ""));
            let r = match key {
                "effect" => quoted(body)
                    .ok_or_else(|| err(Some(n), "expected `@effect \"Name\"`"))
                    .and_then(|(q, _)| clean_text(q, n, "the name"))
                    .map(|q| name = Some(q)),
                "category" => quoted(body)
                    .ok_or_else(|| err(Some(n), "expected `@category \"Folder\"`"))
                    .and_then(|(q, _)| clean_text(q, n, "the category"))
                    .map(|q| category = Some(q)),
                "param" => param(body, n).and_then(|p| {
                    if params.iter().any(|q| q.name == p.name) {
                        Err(err(Some(n), format!("`{}` is declared twice", p.name)))
                    } else if params.len() >= MAX_PARAMS {
                        Err(err(Some(n), format!("at most {MAX_PARAMS} parameters")))
                    } else {
                        params.push(p);
                        Ok(())
                    }
                }),
                k => Err(err(Some(n), format!("unknown annotation `@{k}` (effect, category, param)"))),
            };
            if let Err(e) = r {
                errors.push(e);
            }
            continue;
        }
        // code (a trailing comment is not code)
        let code = l.split("//").next().unwrap_or_default();
        for (tok, what) in FORBIDDEN {
            if code.contains(tok) {
                errors.push(err(Some(n), format!("user effects may not declare {what} (`{}`)", tok.trim())));
            }
        }
    }
    if !source.lines().any(|l| l.split("//").next().unwrap_or_default().contains("fn effect(")) {
        errors.push(err(None, "the file must define `fn effect(px: vec2<f32>) -> vec4<f32>`"));
    }
    let name = match name {
        Some(n) => n,
        None => clean_text(name_hint, 0, "the name").map_err(|_| vec![err(None, "the effect needs a name (`// @effect \"Name\"`)")])?,
    };
    let fx = UserEffect { name, category, params, source: source.to_string(), hash: fnv(source) };
    if errors.is_empty() {
        validate(&fx).map_err(|e| vec![e])?;
        Ok(fx)
    } else {
        Err(errors)
    }
}

/// The WGSL type of a parameter inside `P`.
fn wgsl_type(k: &ParamKind) -> &'static str {
    match k {
        ParamKind::Float { .. } => "f32",
        ParamKind::Bool(_) => "bool",
        ParamKind::Color(_) => "vec3<f32>",
        ParamKind::Point(_) => "vec2<f32>",
    }
}

/// How slot `i` of the uniform array becomes the parameter's value.
fn unpack(k: &ParamKind, i: usize) -> String {
    match k {
        ParamKind::Float { .. } => format!("fx_u.pv[{i}].x"),
        ParamKind::Bool(_) => format!("fx_u.pv[{i}].x != 0.0"),
        ParamKind::Color(_) => format!("fx_u.pv[{i}].xyz"),
        ParamKind::Point(_) => format!("fx_u.pv[{i}].xy"),
    }
}

/// Lines of [`compose`]'s prelude before the user's first line (for mapping naga's line numbers).
fn prelude(fx: &UserEffect, storage_format: &str) -> String {
    let mut p = String::from(
        "// FilmCraft user effect prelude (ADR 0006)\n\
         struct FxUniform { size: vec4<f32>, pv: array<vec4<f32>, 16>, };\n\
         @group(0) @binding(0) var<uniform> fx_u: FxUniform;\n\
         @group(0) @binding(1) var fx_src: texture_2d<f32>;\n",
    );
    p.push_str(&format!("@group(0) @binding(2) var fx_dst: texture_storage_2d<{storage_format}, write>;\n"));
    p.push_str(
        "var<private> size: vec2<f32>;\n\
         var<private> time: f32;\n\
         fn fx_enc1(v: f32) -> f32 { if v <= 0.0031308 { return v * 12.92; } return 1.055 * pow(v, 1.0 / 2.4) - 0.055; }\n\
         fn fx_dec1(v: f32) -> f32 { if v <= 0.04045 { return v / 12.92; } return pow((v + 0.055) / 1.055, 2.4); }\n\
         fn load(p: vec2<i32>) -> vec4<f32> {\n\
             let s = vec2<i32>(textureDimensions(fx_src));\n\
             let o = textureLoad(fx_src, clamp(p, vec2(0), s - 1), 0);\n\
             if o.a <= 1e-6 { return vec4(0.0); }\n\
             let c = max(o.rgb / o.a, vec3(0.0));\n\
             return vec4(fx_enc1(c.x), fx_enc1(c.y), fx_enc1(c.z), o.a);\n\
         }\n\
         fn sample(p: vec2<f32>) -> vec4<f32> {\n\
             let f = p - 0.5;\n\
             let i = vec2<i32>(floor(f));\n\
             let t = f - floor(f);\n\
             let a = load(i); let b = load(i + vec2(1, 0)); let c = load(i + vec2(0, 1)); let d = load(i + vec2(1, 1));\n\
             return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);\n\
         }\n",
    );
    if !fx.params.is_empty() {
        p.push_str("struct FxParams {\n");
        for q in &fx.params {
            p.push_str(&format!("    {}: {},\n", q.name, wgsl_type(&q.kind)));
        }
        p.push_str("};\nvar<private> P: FxParams;\n");
    }
    p.push_str("// ---- user code\n");
    p
}

/// The full WGSL module for `fx`: prelude, the user's code, entry point `fx_user_main` (16×16).
/// `storage_format` is the output texture's WGSL format (`rgba32float`, `rgba16float`).
pub fn compose(fx: &UserEffect, storage_format: &str) -> String {
    let mut m = prelude(fx, storage_format);
    m.push_str(&fx.source);
    if !fx.source.ends_with('\n') {
        m.push('\n');
    }
    m.push_str(
        "// ---- FilmCraft entry point\n\
         @compute @workgroup_size(16, 16)\n\
         fn fx_user_main(@builtin(global_invocation_id) fx_id: vec3<u32>) {\n\
             let fx_s = vec2<u32>(textureDimensions(fx_src));\n\
             if fx_id.x >= fx_s.x || fx_id.y >= fx_s.y { return; }\n\
             size = vec2<f32>(fx_s);\n\
             time = fx_u.size.z;\n",
    );
    if !fx.params.is_empty() {
        let args: Vec<String> = fx.params.iter().enumerate().map(|(i, q)| unpack(&q.kind, i)).collect();
        m.push_str(&format!("    P = FxParams({});\n", args.join(", ")));
    }
    m.push_str(
        "    let fx_c = effect(vec2<f32>(fx_id.xy) + 0.5);\n\
             let fx_a = clamp(fx_c.a, 0.0, 1.0);\n\
             let fx_e = clamp(fx_c.rgb, vec3(0.0), vec3(1.0));\n\
             textureStore(fx_dst, vec2<i32>(fx_id.xy), vec4(vec3(fx_dec1(fx_e.x), fx_dec1(fx_e.y), fx_dec1(fx_e.z)) * fx_a, fx_a));\n\
         }\n",
    );
    m
}

/// Validate [`compose`]'s module with naga; a line number in the user's code when the error is
/// there.
pub fn validate(fx: &UserEffect) -> Result<(), ShaderError> {
    let module_src = compose(fx, "rgba32float");
    let offset = prelude(fx, "rgba32float").lines().count();
    let user_lines = fx.source.lines().count();
    let to_user = |line: u32| -> Option<usize> {
        let l = line as usize;
        (l > offset && l <= offset + user_lines).then(|| l - offset)
    };
    const HINT: &str = " (check that `effect` takes a vec2<f32> and returns a vec4<f32>)";
    let module = naga::front::wgsl::parse_str(&module_src).map_err(|e| {
        let line = e.location(&module_src).and_then(|l| to_user(l.line_number));
        let msg = e.message().to_string();
        err(line, if line.is_none() { format!("{msg}{HINT}") } else { msg })
    })?;
    let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty());
    v.validate(&module).map_err(|e| {
        let line = e.location(&module_src).and_then(|l| to_user(l.line_number));
        let msg = e.as_inner().to_string();
        err(line, if line.is_none() { format!("{msg}{HINT}") } else { msg })
    })?;
    Ok(())
}

#[cfg(test)]
mod tests;
