//! Picture-model tune inputs (fork AI-PLAN AI4 item 3): runs FilmCraft's own Qwen3-VL picture
//! encoder over the frames [`vision_traindata`](./vision_traindata.rs) made and writes exactly what
//! the app's text model would be given (via `VisionLanguage::training_inputs`, the code `ask` uses),
//! so the trainer never runs the encoder and can't drift from the app.
//!
//! ```text
//! FILMCRAFT_DATA_DIR=~/.local/share/filmcraft-fork cargo run --release -p filmcraft-engine --features llm \
//!     --example vision_traindump -- --run-dir ~/Apps/filmcraft-train/runs/vision-data-v1 [--limit n]
//! ```
//!
//! For each split (`train`, `val`) it writes `inputs-<split>.jsonl` (one line per frame: `id`, `ids`
//! with `-1` for picture rows, `positions` [t, h, w], `answer_start`, `count`, `taps`, `offset` in
//! values) and `inputs-<split>.bin`: per frame, the `count × 2560` picture rows then each DeepStack
//! tap, as little-endian bf16. Offline tooling: not part of the app.

use std::error::Error;
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;

use filmcraft_engine::coach_assist::LOOK_PIXELS;
use filmcraft_llm::vl::VisionLanguage;

/// The text model file the app converts on first use (`ai_explain`).
const GGUF: &str = "text-own-q8_0.gguf";

/// Round-to-nearest-even f32 → bf16 bits.
fn bf16(v: f32) -> u16 {
    let b = v.to_bits();
    if v.is_nan() {
        return 0x7fc0;
    }
    let round = 0x7fff + ((b >> 16) & 1);
    (b.wrapping_add(round) >> 16) as u16
}

/// Whether `answer` says what the frame's label is about (by topic, from the known camera and
/// scene), and whether it invents something no picture shows (a log curve's name).
fn score(row: &serde_json::Value, answer: &str) -> (bool, bool) {
    let a = format!(" {} ", answer.to_lowercase().replace(|c: char| !c.is_alphanumeric() && c != '-', " "));
    let has = |ws: &[&str]| ws.iter().any(|w| a.contains(&format!(" {w} ")));
    let num = |k: &str| row.get("camera").and_then(|c| c.get(k)).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let invented = has(&["s-log3", "slog3", "logc", "logc3", "logc4", "f-log", "f-log2", "v-log", "c-log", "d-log", "n-log"]) || a.contains(" apple log ");
    let ok = match s("topic").as_str() {
        "exposure" => {
            let (st, dark, bright) = (num("stops"), ["dark", "underexposed", "dim", "dimly"], ["bright", "overexposed", "clipped", "blown"]);
            if st <= -1.2 {
                has(&dark) && !has(&["overexposed"])
            } else if st >= 1.2 {
                has(&bright) && !has(&["underexposed"])
            } else {
                !has(&["underexposed", "overexposed"])
            }
        }
        "white-balance" => {
            let (w, t) = (num("warm"), num("tint"));
            let mut need = true;
            if w >= 0.25 {
                need &= has(&["warm", "orange", "amber", "yellow", "yellowish"]);
            } else if w <= -0.25 {
                need &= has(&["cool", "blue", "bluish"]);
            }
            if t >= 0.3 {
                need &= has(&["magenta", "pink", "purple"]);
            } else if t <= -0.3 {
                need &= has(&["green", "greenish"]);
            }
            need
        }
        "log" => has(&["flat", "washed", "log", "low-contrast", "desaturated"]) || a.contains(" low contrast "),
        "sky" => match s("scene").as_str() {
            "sunset" => has(&["orange", "purple", "sunset", "pink"]),
            "overcast" => has(&["grey", "gray", "overcast"]),
            _ => has(&["blue"]),
        },
        "skin" => has(&["skin", "face"]),
        _ => match s("scene").as_str() {
            "interior-lamp" => has(&["lamp", "warm", "orange"]),
            "daylight" => has(&["sun", "sunlight", "daylight", "blue"]),
            "sunset" => has(&["sunset", "sun", "orange"]),
            "overcast" => has(&["overcast", "grey", "gray", "soft", "diffuse", "diffused"]),
            "night-street" => has(&["street", "lamp", "lamps", "night", "neon"]),
            "interview" => has(&["backdrop", "key", "grey", "gray"]),
            _ => has(&["beach", "sand", "bright", "sun", "sunlight"]),
        },
    };
    (ok, invented)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut run_dir = PathBuf::new();
    let mut limit = usize::MAX;
    let mut ask = 0usize;
    let mut model_dir_arg: Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k} needs a value"))?;
        match k.as_str() {
            "--run-dir" => run_dir = PathBuf::from(v),
            "--limit" => limit = v.parse()?,
            "--ask" => ask = v.parse()?,
            "--model-dir" => model_dir_arg = Some(PathBuf::from(v)),
            _ => return Err(format!("unknown option {k}").into()),
        }
    }
    if run_dir.as_os_str().is_empty() {
        return Err("--run-dir is required".into());
    }
    let data = std::env::var_os("FILMCRAFT_DATA_DIR").ok_or("set FILMCRAFT_DATA_DIR (the folder with models/)")?;
    let model_dir = model_dir_arg.unwrap_or_else(|| PathBuf::from(data).join("models").join("qwen3-vl-4b-instruct"));
    if !model_dir.join(GGUF).exists() {
        eprintln!("converting the text model in {} …", model_dir.display());
        filmcraft_llm::convert::qwen3vl_text_to_gguf(&model_dir, &model_dir.join(GGUF), filmcraft_llm::convert::Quant::Q8_0, None)?;
    }
    let mut vl = VisionLanguage::load(&model_dir, GGUF)?;
    eprintln!("encoder on the GPU: {}", vl.encoder_on_gpu());
    if ask > 0 {
        // the app's own answers for the first validation frames (to compare with the trainer's
        // `--check`), scored against the labels (base vs tuned: AI4 item 3)
        let src = std::io::BufReader::new(std::fs::File::open(run_dir.join("val.jsonl"))?);
        let (mut right, mut invented, mut n) = (0usize, 0usize, 0usize);
        let mut by_topic: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for line in src.lines().take(ask) {
            let row: serde_json::Value = serde_json::from_str(&line?)?;
            let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let img = image::open(run_dir.join(s("image")))?.to_rgb8();
            let rgb: Vec<f32> = img.as_raw().iter().map(|&b| f32::from(b) / 255.0).collect();
            let a = vl.ask(&rgb, img.width() as usize, img.height() as usize, LOOK_PIXELS, &s("prompt"), 80, None)?;
            let (ok, inv) = score(&row, &a);
            n += 1;
            right += usize::from(ok && !inv);
            invented += usize::from(inv);
            let t = by_topic.entry(s("topic")).or_default();
            t.0 += usize::from(ok && !inv);
            t.1 += 1;
            println!("{} [{}] ok {ok} invented {inv} -> {a}", s("id"), s("topic"));
        }
        println!("score: {right} / {n} right, {invented} with an invented log name; by topic {by_topic:?}");
        return Ok(());
    }
    for split in ["train", "val"] {
        let src = std::io::BufReader::new(std::fs::File::open(run_dir.join(format!("{split}.jsonl")))?);
        let mut index = std::io::BufWriter::new(std::fs::File::create(run_dir.join(format!("inputs-{split}.jsonl")))?);
        let mut bin = std::io::BufWriter::new(std::fs::File::create(run_dir.join(format!("inputs-{split}.bin")))?);
        let (mut offset, mut n, mut failed) = (0usize, 0usize, 0usize);
        let started = std::time::Instant::now();
        for line in src.lines().take(limit) {
            let row: serde_json::Value = serde_json::from_str(&line?)?;
            let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let img = image::open(run_dir.join(s("image")))?.to_rgb8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let rgb: Vec<f32> = img.as_raw().iter().map(|&b| f32::from(b) / 255.0).collect();
            let sample = match vl.training_inputs(&rgb, w, h, LOOK_PIXELS, &s("prompt"), &s("answer")) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{}: {e}", s("id"));
                    failed += 1;
                    continue;
                }
            };
            let p = &sample.picture;
            for v in p.tokens.iter().chain(p.deepstack.iter().flatten()) {
                bin.write_all(&bf16(*v).to_le_bytes())?;
            }
            let values = p.tokens.len().saturating_add(p.deepstack.iter().map(Vec::len).sum::<usize>());
            let entry = serde_json::json!({
                "id": s("id"), "ids": sample.ids, "positions": sample.positions, "answer_start": sample.answer_start,
                "count": p.count, "taps": p.deepstack.len(), "offset": offset,
            });
            writeln!(index, "{}", serde_json::to_string(&entry)?)?;
            offset = offset.saturating_add(values);
            n += 1;
        }
        index.flush()?;
        bin.flush()?;
        eprintln!("{split}: {n} frames ({failed} failed) in {:.0} s, {:.1} GB", started.elapsed().as_secs_f64(), offset as f64 * 2.0 / 1e9);
    }
    Ok(())
}
