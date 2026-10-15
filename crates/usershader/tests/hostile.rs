//! Hostile effect files (cloud USHADER-HOSTILE): a deterministic generator feeds `parse` 2 000
//! mutated files and checks it never panics, every error's line is inside the file (or absent),
//! and every effect it accepts is one `validate` accepts too, within the documented limits.

use filmcraft_usershader::{MAX_PARAMS, ParamKind, ShaderError, parse, validate};

/// xorshift64*: no dependency, the same sequence on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const NUMBERS: &[&str] = &[
    "0",
    "0.5",
    "1",
    "-1",
    "4.0",
    "1e3",
    "1e38",
    "3.4e38",
    "3.5e38",
    "1e39",
    "-1e39",
    "1e-45",
    "NaN",
    "nan",
    "inf",
    "-inf",
    "infinity",
    "+1",
    "0x10",
    "1_000",
    "",
    " ",
    "1.",
    ".5",
    "--1",
    "١",
    "1e",
    "99999999999999999999999999999999999999999",
];

const NAMES: &[&str] = &[
    "amount",
    "a",
    "_",
    "_x",
    "radius2",
    "fx_gain",
    "fx_",
    "Amount",
    "ñame",
    "名前",
    "a-b",
    "a b",
    "",
    "9lives",
    "size",
    "time",
    "P",
    "load",
    "sample",
    "effect",
    "abcdefghijklmnopqrstuvwxyzabcdefg",
    "abcdefghijklmnopqrstuvwxyzabcdef",
];

const LABELS: &[&str] = &[
    "\"Amount\"",
    "\"\"",
    "\" \"",
    "\"Ünïcödé 名前\"",
    "\"unterminated",
    "\"a\" trailing",
    "\"tab\there\"",
    "\"\u{7}bell\"",
    "\"0123456789012345678901234567890123456789012345678901234567890123456789012345678901\"",
    "",
];

const KINDS: &[&str] = &["float", "bool", "color", "point", "vec9", "", "Float", "float float"];

fn float_value(r: &mut Rng) -> String {
    match r.below(4) {
        0 => r.pick(NUMBERS).to_string(),
        1 => format!("{} [{}, {}]", r.pick(NUMBERS), r.pick(NUMBERS), r.pick(NUMBERS)),
        2 => format!("{} [{}, {}", r.pick(NUMBERS), r.pick(NUMBERS), r.pick(NUMBERS)),
        _ => format!("{} [{}]", r.pick(NUMBERS), r.pick(NUMBERS)),
    }
}

fn value(r: &mut Rng, kind: &str) -> String {
    match kind {
        "float" => float_value(r),
        "bool" => r.pick(&["true", "false", "yes", "1", ""]).to_string(),
        "color" => r.pick(&["#ff8000", "#FFFFFF", "#fff", "#gggggg", "ff8000", "#ff80001", "#ÿÿÿ", "#"]).to_string(),
        "point" => match r.below(3) {
            0 => "auto".to_string(),
            1 => format!("{}, {}", r.pick(NUMBERS), r.pick(NUMBERS)),
            _ => r.pick(NUMBERS).to_string(),
        },
        _ => r.pick(NUMBERS).to_string(),
    }
}

fn param_line(r: &mut Rng) -> String {
    let kind = r.pick(KINDS);
    let mut l = format!("// @param {}: {} = {} {}", r.pick(NAMES), kind, value(r, kind), r.pick(LABELS));
    if r.chance(10) {
        // a truncated line (on a char boundary)
        let cut = r.below(l.len() + 1);
        let cut = (0..=cut).rev().find(|&i| l.is_char_boundary(i)).unwrap_or(0);
        l.truncate(cut);
    }
    l
}

fn annotation(r: &mut Rng) -> String {
    match r.below(12) {
        0 => format!("// @effect {}", r.pick(LABELS)),
        1 => format!("// @category {}", r.pick(LABELS)),
        2 => "// @param".to_string(),
        3 => "//@".to_string(),
        4 => format!("// @{} \"x\"", r.pick(&["effects", "Effect", "", "param\u{0}", "名前"])),
        5 => format!("    //   @param {}: float = 0.5", r.pick(NAMES)),
        _ => param_line(r),
    }
}

const CODE: &[&str] = &[
    "fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }",
    "fn effect(px: vec2<f32>) -> vec4<f32> {",
    "    return vec4(load(vec2<i32>(px)).rgb * 0.5, 1.0);",
    "}",
    "/* a /* nested */ comment */",
    "/* unterminated",
    "*/",
    "fn effect(px: f32) -> f32 { return px; }",
    "fn effect(px: vec2<f32>) -> vec4<f32> { return vec4(P.amount); }",
    "fn helper(x: f32) -> f32 { return x * x; }",
    "fn fx_mine() {}",
    "@group(0) @binding(3) var<uniform> u: f32;",
    "@compute @workgroup_size(1) fn main() {}",
    "enable f16;",
    "override k: f32 = 1.0;",
    "fn load(p: vec2<i32>) -> vec4<f32> { return vec4(0.0); }",
    "let x = ;",
    "fn effect(px: vec2<f32>) -> vec4<f32> { loop { } }",
    "fn effect(px: vec2<f32>) -> vec4<f32> { return effect(px); }",
    "const big: f32 = 1e39;",
    "// fn effect( in a comment only",
    "\u{feff}fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }",
    "fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); } // trailing @group comment",
    "",
    "\t",
    "名前",
];

/// One mutated file.
fn file(r: &mut Rng) -> String {
    if r.chance(2) {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    // a valid core most of the time, so mutations land on files that would otherwise parse
    if r.chance(60) {
        lines.push("// @effect \"Hostile\"".into());
        lines.push("// @param amount: float = 0.5 [0.0, 1.0] \"Amount\"".into());
        lines.push(CODE[0].into());
    }
    let extra = r.below(8);
    for _ in 0..extra {
        let l = if r.chance(55) { annotation(r) } else { r.pick(CODE).to_string() };
        let at = r.below(lines.len() + 1);
        lines.insert(at, l);
    }
    // 16+ params: many distinct ones, or one line repeated
    if r.chance(8) {
        let n = MAX_PARAMS - 2 + r.below(6);
        let repeat = r.chance(30);
        for i in 0..n {
            let name = if repeat { "dup".to_string() } else { format!("p{i}") };
            lines.insert(0, format!("// @param {name}: float = 0.5"));
        }
    }
    let sep = if r.chance(10) { "\r\n" } else { "\n" };
    let mut s = lines.join(sep);
    if r.chance(70) {
        s.push_str(sep);
    }
    if r.chance(8) {
        // truncate the whole file on a char boundary
        let cut = r.below(s.len() + 1);
        let cut = (0..=cut).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        s.truncate(cut);
    }
    s
}

fn check_errors(src: &str, errors: &[ShaderError]) {
    assert!(!errors.is_empty(), "a rejection carries at least one error\n{src}");
    let lines = src.lines().count();
    for e in errors {
        if let Some(l) = e.line {
            assert!((1..=lines).contains(&l), "error line {l} outside the file's {lines} lines ({})\n---\n{src}\n---", e.message);
        }
        assert!(!e.message.trim().is_empty(), "an error says what is wrong\n{src}");
    }
}

#[test]
fn mutated_effect_files_never_panic_and_errors_point_inside_the_file() {
    let start = std::time::Instant::now();
    let mut r = Rng(0x5eed_f11e_c0de_0001);
    let (mut accepted, mut rejected) = (0usize, 0usize);
    for i in 0..2_000 {
        let src = file(&mut r);
        let hint = r.pick(&["Hostile", "", "x", "名前", "fx_name", "\u{7}"]);
        let res = std::panic::catch_unwind(|| parse(&src, hint));
        let Ok(res) = res else { panic!("parse panicked on case {i}:\n---\n{src}\n---") };
        match res {
            Ok(fx) => {
                accepted += 1;
                assert!(fx.params.len() <= MAX_PARAMS, "case {i}: {} params", fx.params.len());
                assert!(!fx.name.trim().is_empty(), "case {i}: an accepted effect has a name");
                for p in &fx.params {
                    assert!(!p.name.starts_with("fx_"), "case {i}: {}", p.name);
                    match p.kind {
                        ParamKind::Float { default, min, max } => {
                            assert!(default.is_finite() && min.is_finite() && max.is_finite(), "case {i}: {:?}", p.kind);
                            assert!(min < max && (min..=max).contains(&default), "case {i}: {:?}", p.kind);
                        }
                        ParamKind::Point(Some([x, y])) => assert!(x.is_finite() && y.is_finite(), "case {i}: {:?}", p.kind),
                        ParamKind::Color(c) => assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "case {i}: {:?}", p.kind),
                        _ => {}
                    }
                }
                assert_eq!(validate(&fx), Ok(()), "case {i}: an accepted effect validates\n{src}");
            }
            Err(errors) => {
                rejected += 1;
                check_errors(&src, &errors);
            }
        }
    }
    // the generator must exercise both paths, or the test proves little
    assert!(accepted >= 50, "only {accepted} accepted");
    assert!(rejected >= 500, "only {rejected} rejected");
    // no time assertion (it would flake under the parallel ci run); printed for the log
    println!("{accepted} accepted, {rejected} rejected in {:?}", start.elapsed());
}

#[test]
fn an_empty_file_is_refused_without_a_line() {
    let errors = parse("", "Empty").expect_err("no effect fn");
    check_errors("", &errors);
    assert!(errors.iter().all(|e| e.line.is_none()));
}
