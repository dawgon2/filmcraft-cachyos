//! Compare the catalogue's models on a fixed set of cases (for choosing the default model).
//!
//! ```text
//! FILMCRAFT_LLM_DIR=~/models cargo run --release -p filmcraft-llm --features infer \
//!     --example llm_eval -- cases.jsonl [--model ID]... [--out out.jsonl] [--max-tokens 80]
//!     [--deadline 30] [--limit N] [--raw] [--tokens-only]
//! ```
//!
//! `cases.jsonl` holds one JSON object per line: `{"label": "...", "facts": {...} or "...",
//! "template": "..."}`. Models default to every catalogue entry installed under
//! `FILMCRAFT_LLM_DIR` (`<dir>/<model id>/<files>`). Decoding is greedy. Each answer is printed and,
//! with `--out`, written as a JSON line `{model, label, text, prompt_tokens, gen_tokens, prefill_ms,
//! decode_ms, stop}`; a per-model summary gives prefill and decode speed in tokens/s.
//!
//! `--local id=dir/model.gguf:llama|qwen2` adds a GGUF outside the catalogue (e.g. a fine-tuned or
//! freshly quantized file; `tokenizer.json` must sit next to it; with `--local` only the
//! `--model`s named are taken from the catalogue). `--repeat-penalty 1.0` turns the default 1.1
//! penalty off. `--prompts` prints each case's `{label, system, user}` and exits.
//!
//! `--raw` sends the facts JSON as given instead of [`filmcraft_llm::compact_facts`]'s short form
//! (the prompt before compaction); `--tokens-only` loads each model, prints the prompt token counts
//! of every case (compact and raw) and generates nothing.

use std::error::Error;
use std::io::Write;
use std::time::{Duration, Instant};

use filmcraft_llm::{Family, GenOptions, Model, ModelFile, ModelInfo, explain_prompt, models};

struct Case {
    label: String,
    facts: String,
    template: String,
}

struct Args {
    cases: String,
    models: Vec<String>,
    out: Option<String>,
    max_tokens: usize,
    deadline: Duration,
    limit: usize,
    raw: bool,
    tokens_only: bool,
    /// Local GGUF files outside the catalogue: (id, path to the .gguf, family).
    local: Vec<(String, std::path::PathBuf, Family)>,
    repeat_penalty: f32,
    prompts_only: bool,
}

/// `id=path/to/model.gguf:llama|qwen2` (the folder must also hold `tokenizer.json`).
fn parse_local(s: &str) -> Result<(String, std::path::PathBuf, Family), Box<dyn Error>> {
    let (id, rest) = s.split_once('=').ok_or("--local needs id=file.gguf:family")?;
    let (path, fam) = rest.rsplit_once(':').ok_or("--local needs id=file.gguf:family")?;
    let family = match fam {
        "llama" | "smollm2" => Family::SmolLm2,
        "qwen2" | "qwen" => Family::Qwen25,
        other => return Err(format!("unknown family {other} (llama or qwen2)").into()),
    };
    Ok((id.to_string(), path.into(), family))
}

/// A catalogue-shaped entry for a local file (sizes from disk; the checksum is not checked here).
fn local_info(id: &str, path: &std::path::Path, family: Family) -> Result<(ModelInfo, std::path::PathBuf), Box<dyn Error>> {
    let dir = path.parent().ok_or("no folder")?.to_path_buf();
    let weights = path.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?.to_string();
    let wsize = std::fs::metadata(path)?.len();
    let tsize = std::fs::metadata(dir.join("tokenizer.json"))?.len();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let weights: &'static str = leak(weights);
    let files: &'static [ModelFile] = Box::leak(
        vec![ModelFile { name: weights, url: "", sha256: "", size: wsize }, ModelFile { name: "tokenizer.json", url: "", sha256: "", size: tsize }]
            .into_boxed_slice(),
    );
    let base = models::catalogue().first().copied().ok_or("empty catalogue")?;
    Ok((ModelInfo { id: leak(id.to_string()), name: leak(id.to_string()), family, weights, tokenizer: "tokenizer.json", files, ..base }, dir))
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut a = Args {
        cases: String::new(),
        models: Vec::new(),
        out: None,
        max_tokens: 80,
        deadline: Duration::from_secs(30),
        limit: usize::MAX,
        raw: false,
        tokens_only: false,
        local: Vec::new(),
        repeat_penalty: GenOptions::default().repeat_penalty,
        prompts_only: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--model" => a.models.push(value("--model")?),
            "--out" => a.out = Some(value("--out")?),
            "--max-tokens" => a.max_tokens = value("--max-tokens")?.parse()?,
            "--deadline" => a.deadline = Duration::from_secs_f64(value("--deadline")?.parse::<f64>()?.clamp(0.0, 3600.0)),
            "--limit" => a.limit = value("--limit")?.parse()?,
            "--raw" => a.raw = true,
            "--tokens-only" => a.tokens_only = true,
            "--prompts" => a.prompts_only = true,
            "--local" => a.local.push(parse_local(&value("--local")?)?),
            "--repeat-penalty" => a.repeat_penalty = value("--repeat-penalty")?.parse::<f32>()?.clamp(1.0, 2.0),
            s if s.starts_with("--") => return Err(format!("unknown option {s}").into()),
            s if a.cases.is_empty() => a.cases = s.to_string(),
            // Old form: extra positional arguments are model ids.
            s => a.models.push(s.to_string()),
        }
    }
    if a.cases.is_empty() {
        return Err(
            "usage: llm_eval <cases.jsonl> [--model ID]... [--out out.jsonl] [--max-tokens N] [--deadline S] [--limit N] [--raw] [--tokens-only]".into()
        );
    }
    Ok(a)
}

fn read_cases(path: &str) -> Result<Vec<Case>, Box<dyn Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut cases = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("{path}:{}: {e}", n + 1))?;
        let label = v.get("label").and_then(|l| l.as_str()).unwrap_or("?").to_string();
        let facts = match v.get("facts") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => return Err(format!("{path}:{}: no `facts`", n + 1).into()),
        };
        let template = v.get("template").and_then(|t| t.as_str()).unwrap_or("").to_string();
        cases.push(Case { label, facts, template });
    }
    Ok(cases)
}

/// The prompt before compaction (G2.11 lane A): the facts JSON as given, the longer wording.
fn raw_prompt(facts: &str, template: &str) -> (String, String) {
    let system = "You explain colour corrections to a beginner video editor. \
Write one or two short sentences in plain, everyday language. \
Use only the facts you are given; never guess or add anything else. \
Do not write any number that does not appear in the facts. \
Do not name tools, controls, sliders, nodes, panels or menus. \
Do not give advice or instructions. \
If you are unsure, reword the reference sentence."
        .to_string();
    let user = format!(
        "Facts (JSON):\n{}\n\nReference sentence:\n{}\n\n\
         Explain what this change does to the picture, for a beginner, in one or two sentences.",
        facts.trim(),
        template.trim()
    );
    (system, user)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn per_s(tokens: usize, d: Duration) -> f64 {
    let s = d.as_secs_f64();
    if s > 0.0 { tokens as f64 / s } else { 0.0 }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let dir = std::env::var_os("FILMCRAFT_LLM_DIR").map(std::path::PathBuf::from).ok_or("set FILMCRAFT_LLM_DIR")?;
    let mut cases = read_cases(&args.cases)?;
    cases.truncate(args.limit);
    if args.prompts_only {
        // the exact (system, user) pair every model gets, for reference implementations
        for c in &cases {
            let (system, user) = explain_prompt(&c.facts, &c.template);
            println!("{}", serde_json::json!({"label": c.label, "system": system, "user": user}));
        }
        return Ok(());
    }
    let opts = GenOptions { max_tokens: args.max_tokens, deadline: args.deadline, temperature: 0.0, repeat_penalty: args.repeat_penalty, ..Default::default() };
    let mut out = match &args.out {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => None,
    };
    eprintln!(
        "# threads: candle {} (RAYON_NUM_THREADS={:?}), avx2 compiled in: {}, avx512f: {}, fma: {}, f16c: {}",
        candle_core::utils::get_num_threads(),
        std::env::var("RAYON_NUM_THREADS").ok(),
        cfg!(target_feature = "avx2"),
        cfg!(target_feature = "avx512f"),
        cfg!(target_feature = "fma"),
        cfg!(target_feature = "f16c"),
    );

    let mut run: Vec<(ModelInfo, std::path::PathBuf)> = Vec::new();
    for m in models::catalogue() {
        if !args.models.is_empty() && !args.models.iter().any(|w| w == m.id) {
            continue;
        }
        if !args.local.is_empty() && args.models.is_empty() {
            continue;
        }
        if !models::installed(&dir, m) {
            eprintln!("## {}: not installed, skipped", m.id);
            continue;
        }
        run.push((*m, models::model_dir(&dir, m)));
    }
    for (id, path, family) in &args.local {
        run.push(local_info(id, path, *family)?);
    }
    for (info, mdir) in &run {
        let m = info;
        let t = Instant::now();
        let mut model = match Model::load(mdir, m) {
            Ok(model) => model,
            Err(e) => {
                eprintln!("## {}: load failed: {e}", m.id);
                continue;
            }
        };
        eprintln!("## {} (loaded in {:.2?})", m.id, t.elapsed());

        if args.tokens_only {
            let (mut sum_c, mut sum_r, mut max_c, mut max_r) = (0usize, 0usize, 0usize, 0usize);
            for c in &cases {
                let (s, u) = explain_prompt(&c.facts, &c.template);
                let compact = model.prompt_tokens(&s, &u)?;
                let (s, u) = raw_prompt(&c.facts, &c.template);
                let raw = model.prompt_tokens(&s, &u)?;
                println!("{}\t{}\traw {raw}\tcompact {compact}", m.id, c.label);
                sum_c += compact;
                sum_r += raw;
                max_c = max_c.max(compact);
                max_r = max_r.max(raw);
            }
            let n = cases.len().max(1);
            println!("{}\tSUMMARY\traw mean {} max {max_r}\tcompact mean {} max {max_c}", m.id, sum_r / n, sum_c / n);
            continue;
        }

        let (mut prompt_tok, mut gen_tok, mut prefill, mut decode, mut total) = (0usize, 0usize, Duration::ZERO, Duration::ZERO, Duration::ZERO);
        let mut done = 0u32;
        for c in &cases {
            let (system, user) = if args.raw { raw_prompt(&c.facts, &c.template) } else { explain_prompt(&c.facts, &c.template) };
            match model.generate_detailed(&system, &user, opts) {
                Ok(g) => {
                    let dec = g.elapsed.saturating_sub(g.prefill);
                    prompt_tok += g.prompt_tokens;
                    gen_tok += g.new_tokens;
                    prefill += g.prefill;
                    decode += dec;
                    total += g.elapsed;
                    done += 1;
                    eprintln!(
                        "- [{}] {:.2?} (prefill {} tok {:.2?}, decode {} tok {:.2?}) {:?}: {}",
                        c.label, g.elapsed, g.prompt_tokens, g.prefill, g.new_tokens, dec, g.stop, g.text
                    );
                    if let Some(w) = out.as_mut() {
                        let line = serde_json::json!({
                            "model": m.id,
                            "label": c.label,
                            "text": g.text,
                            "prompt_tokens": g.prompt_tokens,
                            "gen_tokens": g.new_tokens,
                            "prefill_ms": (ms(g.prefill) * 10.0).round() / 10.0,
                            "decode_ms": (ms(dec) * 10.0).round() / 10.0,
                            "stop": format!("{:?}", g.stop),
                        });
                        writeln!(w, "{line}")?;
                        w.flush()?;
                    }
                }
                Err(e) => eprintln!("- [{}] error: {e}", c.label),
            }
        }
        let n = done.max(1);
        println!(
            "SUMMARY\t{}\tcases {done}\tprefill {:.1} tok/s\tdecode {:.1} tok/s\tmean prompt {} tok\tmean answer {} tok\tmean total {:.2?}\tmean prefill {:.2?}",
            m.id,
            per_s(prompt_tok, prefill),
            per_s(gen_tok, decode),
            prompt_tok / n as usize,
            gen_tok / n as usize,
            total / n,
            prefill / n,
        );
    }
    Ok(())
}
