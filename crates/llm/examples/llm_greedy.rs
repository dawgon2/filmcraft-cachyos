//! Greedy continuations of given token ids through FilmCraft's own forward pass (the parity check
//! for the training tool's PyTorch model, `tools/explainer-train/parity_lm.py`; fork §1a.97 item 1).
//!
//! ```text
//! cargo run --release -p filmcraft-llm --features infer --example llm_greedy -- \
//!     <model.gguf> <arch: llama|smollm3|qwen2|qwen3> <cases.jsonl> [--max-new 32]
//! ```
//!
//! `cases.jsonl`: one `{"ids": [u32, …], "stop": [u32, …]}` per line. Prints one
//! `{"ids": [generated …]}` per case (stops before a stop id, like tinylm's `greedy`, or after `--max-new`).
//! A developer tool: errors are printed and end the run.

use std::error::Error;
use std::io::{BufRead, BufReader};

use filmcraft_llm::forward::{Config, Forward};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [gguf, arch, cases, rest @ ..] = args.as_slice() else {
        return Err("usage: llm_greedy <model.gguf> <arch> <cases.jsonl> [--max-new N]".into());
    };
    let max_new = match rest {
        [flag, n] if flag == "--max-new" => n.parse::<usize>()?.min(4096),
        [] => 32,
        _ => return Err("unknown arguments".into()),
    };
    let len = std::fs::metadata(gguf)?.len();
    let mut file = BufReader::new(std::fs::File::open(gguf)?);
    let summary = filmcraft_llm::gguf::check(&mut file, len)?;
    for line in BufReader::new(std::fs::File::open(cases)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let case: serde_json::Value = serde_json::from_str(&line)?;
        let ids = |k: &str| -> Vec<u32> {
            case.get(k)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64().and_then(|x| u32::try_from(x).ok())).collect())
                .unwrap_or_default()
        };
        let (prompt, stop) = (ids("ids"), ids("stop"));
        if prompt.is_empty() {
            return Err("a case without ids".into());
        }
        let cfg = Config::from_gguf(&summary, arch, prompt.len().saturating_add(max_new).saturating_add(1))?;
        let mut f = Forward::load(&mut file, &summary, cfg)?;
        let mut logits = f.forward(&prompt, 0)?;
        let mut out: Vec<u32> = Vec::new();
        for step in 0..max_new {
            let next = logits.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as u32).ok_or("no logits")?;
            if stop.contains(&next) {
                break;
            }
            out.push(next);
            if step + 1 == max_new {
                break;
            }
            logits = f.forward(&[next], prompt.len() + out.len() - 1)?;
        }
        println!("{}", serde_json::json!({ "ids": out }));
    }
    Ok(())
}
