//! Runs the real models when they are present: set `FILMCRAFT_LLM_DIR` to a folder holding
//! `<model id>/<files>` (the layout `models::model_dir` uses) and build with `--features infer`.
//! Skips otherwise; weights are never downloaded by tests.

#![cfg(feature = "infer")]

use std::time::Duration;

use filmcraft_llm::{GenOptions, Model, StopReason, explain_prompt, models};

/// A real `tokenizer.json` parses (pure-Rust regex) even when the weights are absent: loading
/// then fails on the weights, not the tokenizer.
#[test]
fn real_tokenizers_parse() {
    let Some(dir) = std::env::var_os("FILMCRAFT_LLM_DIR") else {
        eprintln!("skipping: FILMCRAFT_LLM_DIR is not set");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for m in models::catalogue() {
        let d = models::model_dir(&dir, m);
        let tok_ok = std::fs::metadata(d.join(m.tokenizer)).is_ok_and(|md| Some(md.len()) == m.file(m.tokenizer).map(|f| f.size));
        if !tok_ok || d.join(m.weights).exists() {
            continue;
        }
        match Model::load(&d, m) {
            Err(filmcraft_llm::LlmError::NotInstalled(p)) => assert!(p.ends_with(m.weights), "{p}"),
            other => panic!("{}: {other:?}", m.id),
        }
        eprintln!("{}: tokenizer parses", m.id);
    }
}

#[test]
fn installed_models_explain_a_grade() {
    let Some(dir) = std::env::var_os("FILMCRAFT_LLM_DIR") else {
        eprintln!("skipping: FILMCRAFT_LLM_DIR is not set");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let facts = r#"{"node":"Skin","changes":[{"what":"warmth","direction":"warmer","amount":"a little"},{"what":"red saturation","direction":"less","amount":"10%"}],"untouched":["sky"]}"#;
    let (system, user) = explain_prompt(facts, "This node makes the picture a little warmer and the reds 10% less saturated.");
    let mut ran = 0;
    for m in models::catalogue() {
        if !models::installed(&dir, m) {
            eprintln!("skipping {}: not installed in {}", m.id, dir.display());
            continue;
        }
        let md = models::model_dir(&dir, m);
        let t = std::time::Instant::now();
        Model::verify(&md, m).unwrap();
        let verify_time = t.elapsed();
        let t = std::time::Instant::now();
        let mut model = Model::load(&md, m).unwrap();
        eprintln!("{}: verify {verify_time:.2?}, load {:.2?}", m.id, t.elapsed());
        let opts = GenOptions { max_tokens: 80, deadline: Duration::from_secs(60), ..Default::default() };
        let g = model.generate_detailed(&system, &user, opts).unwrap();
        eprintln!("{} ({:?}, {} tokens, {:?}): {}", m.id, g.stop, g.new_tokens, g.elapsed, g.text);
        assert!(!g.text.is_empty());
        assert!(!g.text.contains("<|im_"));
        // Greedy decoding is deterministic.
        let again = model.generate(&system, &user, opts).unwrap();
        assert_eq!(again, g.text);
        // Streaming reports a growing answer that ends as the final text; the first words come
        // after the prefill, long before the answer is done.
        // (Run another grade first so only the shared system prompt is cached: the realistic case.)
        let other = r#"{"node":"Sky","changes":[{"what":"blue saturation","direction":"more","amount":"20%"}],"untouched":["skin"]}"#;
        let (os, ou) = explain_prompt(other, "This node makes the blues 20% more saturated.");
        model.generate_detailed(&os, &ou, GenOptions { max_tokens: 1, ..opts }).unwrap();
        let mut seen: Vec<String> = Vec::new();
        let t = std::time::Instant::now();
        let mut first = None;
        let gs = model
            .generate_streaming(&system, &user, opts, &mut |so_far| {
                first.get_or_insert(t.elapsed());
                seen.push(so_far.to_string());
                true
            })
            .unwrap();
        eprintln!("{}: first words after {:.2?}, {} updates", m.id, first.unwrap_or_default(), seen.len());
        assert_eq!(gs.text, g.text);
        assert!(seen.windows(2).all(|w| w[1].len() > w[0].len()), "{seen:?}");
        assert_eq!(seen.last().map(|s| s.trim_end()), Some(g.text.as_str()));
        // Returning false cancels and keeps the text so far.
        let gc = model.generate_streaming(&system, &user, opts, &mut |_| false).unwrap();
        assert_eq!(gc.stop, StopReason::Cancelled);
        assert!(gc.new_tokens >= 1 && gc.new_tokens < g.new_tokens.max(2), "{gc:?}");
        assert!(g.text.starts_with(&gc.text), "{gc:?}");
        // A panicking callback is an error, not a crash.
        let gp = model.generate_streaming(&system, &user, opts, &mut |_| panic!("callback"));
        assert!(matches!(gp, Err(filmcraft_llm::LlmError::Panicked(_))), "{gp:?}");
        // A deadline of zero stops at once.
        let g0 = model.generate_detailed(&system, &user, GenOptions { deadline: Duration::ZERO, ..opts }).unwrap();
        assert_eq!(g0.stop, StopReason::Deadline);
        assert_eq!(g0.new_tokens, 0);
        // A prompt longer than the batched prefill continues token by token.
        let long = format!("{{\"notes\":\"{}\"}}", "warm skin, cool sky. ".repeat(140));
        let (ls, lu) = explain_prompt(&long, "This node warms the skin.");
        let g = model.generate_detailed(&ls, &lu, GenOptions { max_tokens: 4, ..opts }).unwrap();
        eprintln!("{}: long prompt {} tokens in {:.2?}", m.id, g.prompt_tokens, g.elapsed);
        assert!(g.prompt_tokens > 512);
        // A prompt that cannot fit is an error, not a crash.
        let huge = "colour ".repeat(4000);
        assert!(model.generate(&huge, &huge, opts).is_err());
        ran += 1;
    }
    eprintln!("{ran} model(s) checked");
}
