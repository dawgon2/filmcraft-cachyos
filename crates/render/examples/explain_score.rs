//! Score AI-written node explanations against the fact checker (GRADE-PLAN G2.11b).
//!
//! ```text
//! cargo run --release -p filmcraft-render --example explain_score -- outputs.jsonl [more.jsonl ...]
//!     [--summary summary.md] [--json summary.json] [--fails]
//! cargo run --release -p filmcraft-render --example explain_score -- --dump-cases cases.jsonl
//! ```
//!
//! `--dump-cases` writes the 36 held-out cases as JSON lines `{label, facts, template}` (the input
//! of `filmcraft-llm`'s `llm_eval`) and stops.
//!
//! Each input line is one answer as written by `filmcraft-llm`'s `llm_eval --out`:
//! `{"model", "label", "text", "prefill_ms", "decode_ms", "stop", ...}`. The label names one of the
//! 36 held-out cases of [`filmcraft_render::grade_explain::cases::eval_cases`]; the text is checked
//! with `check_explanation_labeled(facts, None, Some(1), text)` (the eval prompt labels the node
//! "Node 1"). Per model: pass rate, the histogram of rejection reasons (the `FactMismatch`
//! variant), mean time per answer (prefill + decode) and mean answer length. Unknown labels and
//! unreadable lines are counted, never fatal.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;

use filmcraft_render::grade_explain::cases::{EvalCase, eval_cases};
use filmcraft_render::grade_explain::{FactMismatch, check_explanation_labeled};

#[derive(Default)]
struct ModelScore {
    answers: usize,
    passed: usize,
    reasons: BTreeMap<String, usize>,
    total_ms: f64,
    timed: usize,
    gen_tokens: usize,
    unknown_labels: usize,
    passed_labels: Vec<String>,
    fails: Vec<(String, String, String)>,
}

/// The variant name of a rejection (`Ungrammatical`, `Direction`, ...).
fn reason_name(e: &FactMismatch) -> String {
    let dbg = format!("{e:?}");
    dbg.split(['(', ' ', '{']).next().unwrap_or("?").to_string()
}

fn num(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite() && *x >= 0.0)
}

struct Args {
    inputs: Vec<String>,
    summary: Option<String>,
    json: Option<String>,
    fails: bool,
    dump: Option<String>,
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut a = Args { inputs: Vec::new(), summary: None, json: None, fails: false, dump: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--summary" => a.summary = Some(it.next().ok_or("--summary needs a path")?),
            "--json" => a.json = Some(it.next().ok_or("--json needs a path")?),
            "--fails" => a.fails = true,
            "--dump-cases" => a.dump = Some(it.next().ok_or("--dump-cases needs a path")?),
            _ => a.inputs.push(arg),
        }
    }
    if a.inputs.is_empty() && a.dump.is_none() {
        return Err("usage: explain_score <outputs.jsonl>... [--summary out.md] [--json out.json] [--fails]".into());
    }
    Ok(a)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    if let Some(p) = &args.dump {
        let lines: Vec<String> = eval_cases().iter().filter_map(|c| serde_json::to_string(c).ok()).collect();
        std::fs::write(p, lines.join("\n") + "\n").map_err(|e| format!("{p}: {e}"))?;
        println!("wrote {} cases to {p}", lines.len());
        return Ok(());
    }
    let cases: BTreeMap<String, EvalCase> = eval_cases().into_iter().map(|c| (c.label.clone(), c)).collect();
    let mut models: BTreeMap<String, ModelScore> = BTreeMap::new();
    let mut bad_lines = 0usize;
    for path in &args.inputs {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                bad_lines += 1;
                continue;
            };
            let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("?").to_string();
            let label = v.get("label").and_then(|m| m.as_str()).unwrap_or("?").to_string();
            let answer = v.get("text").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let s = models.entry(model).or_default();
            let Some(case) = cases.get(&label) else {
                s.unknown_labels += 1;
                continue;
            };
            s.answers += 1;
            if let (Some(p), Some(d)) = (num(&v, "prefill_ms"), num(&v, "decode_ms")) {
                s.total_ms += p + d;
                s.timed += 1;
            }
            s.gen_tokens += num(&v, "gen_tokens").map_or(0, |t| t as usize);
            match check_explanation_labeled(&case.facts, None, Some(1), &answer) {
                Ok(()) => {
                    s.passed += 1;
                    s.passed_labels.push(label);
                }
                Err(e) => {
                    *s.reasons.entry(reason_name(&e)).or_default() += 1;
                    s.fails.push((label, e.to_string(), answer));
                }
            }
        }
    }

    let mut md = String::new();
    let _ = writeln!(md, "| model | pass | rate | mean s | mean tokens | rejection reasons |");
    let _ = writeln!(md, "|---|---|---|---|---|---|");
    let mut json = serde_json::Map::new();
    for (name, s) in &models {
        let rate = if s.answers > 0 { s.passed as f64 / s.answers as f64 } else { 0.0 };
        let mean_s = if s.timed > 0 { s.total_ms / s.timed as f64 / 1000.0 } else { f64::NAN };
        let mean_tok = if s.answers > 0 { s.gen_tokens as f64 / s.answers as f64 } else { 0.0 };
        let reasons: Vec<String> = {
            let mut r: Vec<(&String, &usize)> = s.reasons.iter().collect();
            r.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            r.iter().map(|(k, v)| format!("{k} {v}")).collect()
        };
        let _ = writeln!(
            md,
            "| {name} | {}/{} | {:.0} % | {mean_s:.2} | {mean_tok:.0} | {} |",
            s.passed,
            s.answers,
            rate * 100.0,
            if reasons.is_empty() { "-".to_string() } else { reasons.join(", ") }
        );
        json.insert(
            name.clone(),
            serde_json::json!({
                "answers": s.answers,
                "passed": s.passed,
                "pass_rate": rate,
                "mean_s": if mean_s.is_finite() { serde_json::json!(mean_s) } else { serde_json::Value::Null },
                "mean_gen_tokens": mean_tok,
                "reasons": s.reasons,
                "passed_labels": s.passed_labels,
                "unknown_labels": s.unknown_labels,
            }),
        );
    }
    if bad_lines > 0 {
        let _ = writeln!(md, "\n{bad_lines} unreadable line(s) skipped.");
    }
    if args.fails {
        for (name, s) in &models {
            let _ = writeln!(md, "\n## {name}: rejected answers\n");
            for (label, why, text) in &s.fails {
                let _ = writeln!(md, "- `{label}`: {why}\n  > {}", text.replace('\n', " "));
            }
        }
    }
    print!("{md}");
    if let Some(p) = &args.summary {
        std::fs::write(p, &md).map_err(|e| format!("{p}: {e}"))?;
    }
    if let Some(p) = &args.json {
        std::fs::write(p, serde_json::to_string_pretty(&serde_json::Value::Object(json))?).map_err(|e| format!("{p}: {e}"))?;
    }
    Ok(())
}
