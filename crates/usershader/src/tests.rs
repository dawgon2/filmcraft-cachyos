use super::*;

const GOOD: &str = r#"// @effect "Chromatic Aberration"
// @category "Stylize"
// @param amount: float = 4.0 [0.0, 50.0] "Amount"
// @param center: point = auto "Center"
// @param tint: color = #ff8000 "Tint"
// @param radial: bool = true

fn effect(px: vec2<f32>) -> vec4<f32> {
    var d = vec2(P.amount, 0.0);
    if P.radial {
        d = normalize(px - P.center + vec2(1e-3)) * P.amount;
    }
    let r = sample(px + d).r;
    let g = sample(px).g;
    let b = sample(px - d).b;
    return vec4(vec3(r, g, b) * mix(vec3(1.0), P.tint, 0.1 * sin(time)), sample(px).a);
}
"#;

#[test]
fn a_good_effect_parses_and_validates() {
    let fx = parse(GOOD, "ca").unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(fx.name, "Chromatic Aberration");
    assert_eq!(fx.category.as_deref(), Some("Stylize"));
    assert_eq!(fx.params.len(), 4);
    assert_eq!(fx.params[0], Param { name: "amount".into(), label: "Amount".into(), kind: ParamKind::Float { default: 4.0, min: 0.0, max: 50.0 } });
    assert_eq!(fx.params[1].kind, ParamKind::Point(None));
    assert_eq!(fx.params[2].kind, ParamKind::Color([1.0, 128.0 / 255.0, 0.0]));
    assert_eq!(fx.params[3], Param { name: "radial".into(), label: "radial".into(), kind: ParamKind::Bool(true) });
    // the composed module carries the output format the GPU stage asks for
    assert!(compose(&fx, "rgba16float").contains("texture_storage_2d<rgba16float, write>"));
    assert_ne!(fx.hash, parse(&GOOD.replace("0.1 *", "0.2 *"), "ca").unwrap().hash, "an edit is a new version");
}

#[test]
fn a_minimal_effect_without_annotations_takes_its_file_name() {
    let fx = parse("fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n", "Pass Through").unwrap();
    assert_eq!(fx.name, "Pass Through");
    assert!(fx.params.is_empty());
}

fn first_error(src: &str) -> ShaderError {
    parse(src, "x").expect_err(src).into_iter().next().expect("an error")
}

#[test]
fn annotation_errors_name_their_line() {
    let body = "fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n";
    let cases = [
        ("// @param a: float = 2.0 [0.0, 1.0]\n", "outside"),
        ("// @param a: float = x\n", "not a number"),
        ("// @param a: float = 0.5 [1.0, 0.0]\n", "min"),
        ("// @param a: float = 0.5 [0.0, 1.0\n", "]"),
        ("// @param a: float\n", "default"),
        ("// @param Bad: float = 0.5\n", "parameter name"),
        ("// @param fx_a: float = 0.5\n", "parameter name"),
        ("// @param a: vec9 = 1\n", "unknown kind"),
        ("// @param a: bool = yes\n", "true or false"),
        ("// @param a: color = red\n", "#rrggbb"),
        ("// @param a: point = 1\n", "auto"),
        ("// @param a: float = NaN\n", "finite"),
        ("// @param a: float = 0.5 \"Label\" trailing\n", "follow"),
        ("// @param a: float = 0.5 \"unterminated\n", "unterminated"),
        ("// @effect Name\n", "@effect"),
        ("// @effect \"\"\n", "printable"),
        ("// @wobble 3\n", "unknown annotation"),
    ];
    for (line, want) in cases {
        let e = first_error(&format!("{line}{body}"));
        assert_eq!(e.line, Some(1), "{line}: {e}");
        assert!(e.message.contains(want), "{line}: {e}");
    }
    let dup = first_error(&format!("// @param a: float = 0.5\n// @param a: bool = true\n{body}"));
    assert_eq!(dup.line, Some(2), "{dup}");
    let many: String = (0..=MAX_PARAMS).map(|i| format!("// @param p{i}: float = 0.5\n")).collect();
    let e = first_error(&format!("{many}{body}"));
    assert_eq!(e.line, Some(MAX_PARAMS + 1), "{e}");
}

#[test]
fn what_user_code_may_not_declare_is_refused() {
    for bad in [
        "@group(0) @binding(9) var t: texture_2d<f32>;",
        "@compute @workgroup_size(1) fn other() {}",
        "var<storage, read_write> b: array<f32>;",
        "var<workgroup> w: array<f32, 4>;",
        "override k: f32 = 1.0;",
        "enable f16;",
        "fn fx_helper() {}",
    ] {
        let src = format!("fn effect(px: vec2<f32>) -> vec4<f32> {{ return sample(px); }}\n{bad}\n");
        let e = first_error(&src);
        assert_eq!(e.line, Some(2), "{bad}: {e}");
        assert!(e.message.contains("may not declare"), "{bad}: {e}");
    }
    // in a comment it's fine
    assert!(parse("// a @group in a comment\nfn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); } // @binding\n", "x").is_ok());
}

#[test]
fn wgsl_errors_point_at_the_users_line() {
    // a syntax error on line 3
    let e = first_error("// @effect \"Broken\"\nfn effect(px: vec2<f32>) -> vec4<f32> {\n    let a = ;\n    return sample(px);\n}\n");
    assert_eq!(e.line, Some(3), "{e}");
    // a type error on line 2
    let e = first_error("fn effect(px: vec2<f32>) -> vec4<f32> {\n    let a: f32 = vec2(1.0, 2.0);\n    return sample(px);\n}\n");
    assert_eq!(e.line, Some(2), "{e}");
    // an undefined parameter
    let e = first_error("fn effect(px: vec2<f32>) -> vec4<f32> {\n    return sample(px) * P.nothing;\n}\n");
    assert_eq!(e.line, Some(2), "{e}");
    // the wrong signature: the error is in FilmCraft's wrapper, so the message says what to check
    let e = first_error("fn effect(px: vec2<f32>) -> vec3<f32> { return vec3(1.0); }\n");
    assert!(e.line.is_none() && e.message.contains("returns a vec4"), "{e}");
    // no effect function at all
    let e = first_error("fn other(px: vec2<f32>) -> vec4<f32> { return vec4(1.0); }\n");
    assert_eq!(e.line, None);
    assert!(e.message.contains("fn effect("), "{e}");
}

#[test]
fn hostile_files_are_refused_without_panicking() {
    assert!(parse(&"a".repeat(MAX_SOURCE + 1), "x").is_err());
    for s in ["", "\0\0\0", "// @param", "// @param :", "// @param a: float = 1 [", "// @effect \"", "fn effect(", "}}}}{{{{"] {
        assert!(parse(s, "x").is_err(), "{s:?}");
    }
    // a name hint that can't be a name
    assert!(parse("fn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n", "").is_err());
    // non-ASCII text is fine in labels, not in parameter names
    assert!(parse("// @param a: float = 0.5 \"Stärke ✓\"\nfn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n", "x").is_ok());
    assert!(parse("// @param ä: float = 0.5\nfn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n", "x").is_err());
}
