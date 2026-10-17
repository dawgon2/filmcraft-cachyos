//! Qwen3-VL 4B's text model on the GPU (AI4.2, `GpuForward::forward_multimodal`) against the
//! CPU `Forward` on the same converted GGUF: the text-only "Paris" prompt and the coloured
//! squares through FilmCraft's own (CPU) picture encoder, with prefill and per-token timings.
//! Runs only with `FILMCRAFT_QWEN3VL_DIR` set to the model folder (holding the GGUF,
//! `config.json`, `tokenizer.json` and the safetensors the encoder reads) and a GPU adapter;
//! skips otherwise. `FILMCRAFT_QWEN3VL_GGUF` picks the file (default `text-own-q4k.gguf`).
//! Build with `--release --features infer,gpu` and run with `--test-threads=1`: each test holds
//! ~10 GB of host memory. Pick the adapter with `MESA_VK_DEVICE_SELECT`.

#![cfg(feature = "gpu")]

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;

use filmcraft_llm::forward::{Config, Forward, MmPicture, MmToken};
use filmcraft_llm::gpu_forward::GpuForward;
use filmcraft_llm::mrope::{Span, positions};
use filmcraft_llm::{gguf, vision_config};

/// Context the text model is loaded with (as `vl.rs`).
const CONTEXT: usize = 8192;

fn argmax(v: &[f32]) -> u32 {
    v.iter().enumerate().fold((0usize, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0 as u32
}

/// Worst |a − b| over the largest |b|.
fn rel(a: &[f32], b: &[f32]) -> f32 {
    let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs())) / scale
}

/// The model folder and the GGUF inside it, when the test should run.
fn model() -> Option<(PathBuf, PathBuf)> {
    let dir = PathBuf::from(std::env::var_os("FILMCRAFT_QWEN3VL_DIR")?);
    let path = dir.join(std::env::var("FILMCRAFT_QWEN3VL_GGUF").unwrap_or_else(|_| "text-own-q4k.gguf".into()));
    if !path.exists() {
        eprintln!("skipping: {} missing (run llm_quantize)", path.display());
        return None;
    }
    Some((dir, path))
}

fn load(dir: &std::path::Path, path: &std::path::Path) -> Option<(Forward, GpuForward, tokenizers::Tokenizer, [usize; 3])> {
    let bytes = std::fs::read(path).unwrap();
    let mut c = Cursor::new(&bytes[..]);
    let s = gguf::check(&mut c, bytes.len() as u64).unwrap();
    let cfg = Config::from_gguf(&s, "qwen3", CONTEXT).unwrap();
    assert_eq!((cfg.head_dim, cfg.q_dim, cfg.embd), (128, 4096, 2560));
    let t = Instant::now();
    let cpu = Forward::load(&mut c, &s, cfg).unwrap();
    eprintln!("CPU model loaded in {:.1} s", t.elapsed().as_secs_f32());
    drop(bytes);
    let t = Instant::now();
    let gpu = match GpuForward::load(&cpu) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: no GPU forward ({e})");
            return None;
        }
    };
    eprintln!("{}: {} MB uploaded in {:.1} s, prompt batches of {}", gpu.adapter(), gpu.bytes() / 1_000_000, t.elapsed().as_secs_f32(), gpu.batch());
    let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
    let sections = filmcraft_llm::vl::mrope_sections(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
    Some((cpu, gpu, tok, sections))
}

/// Greedy decoding on `f` after `logits`, text tokens at positions `next..`; returns the tokens
/// and the seconds per generated token.
fn generate(f: &mut dyn FnMut(&[MmToken], &[[u32; 3]]) -> Vec<f32>, mut logits: Vec<f32>, mut next: u32, stops: &[u32], max_new: usize) -> (Vec<u32>, f32) {
    let mut out = Vec::new();
    let t = Instant::now();
    for _ in 0..max_new {
        let id = argmax(&logits);
        if stops.contains(&id) {
            break;
        }
        out.push(id);
        logits = f(&[MmToken::Text(id)], &[[next; 3]]);
        next += 1;
    }
    let per = t.elapsed().as_secs_f32() / out.len().max(1) as f32;
    (out, per)
}

/// The text-only prompt: the GPU's logits agree with the CPU's (top token, and within the
/// CPU-vs-GPU activation-quantization gap seen on the other models), both answer "Paris";
/// prompt and per-token times on both.
#[test]
fn text_prompt_matches_the_cpu_on_the_gpu() {
    let Some((dir, path)) = model() else { return };
    let Some((mut cpu, mut gpu, tok, sections)) = load(&dir, &path) else { return };
    let prompt = "<|im_start|>user\nWhat is the capital of France? Answer in one word.<|im_end|>\n<|im_start|>assistant\n";
    let ids = tok.encode(prompt, false).unwrap().get_ids().to_vec();
    let toks: Vec<MmToken> = ids.iter().map(|&t| MmToken::Text(t)).collect();
    let (pos, next) = positions(&[Span::Text(ids.len())]).unwrap();
    let stops: Vec<u32> = ["<|im_end|>", "<|endoftext|>"].iter().filter_map(|t| tok.token_to_id(t)).collect();
    // warm up (pipelines, first submission), then time
    gpu.forward_multimodal(&toks[..2], &pos[..2], sections, None).unwrap();
    // the prompt three times (its first run also zero-fills the KV caches on first use)
    let mut lg = Vec::new();
    let mut runs = Vec::new();
    for _ in 0..3 {
        gpu.reset();
        let t = Instant::now();
        lg = gpu.forward_multimodal(&toks, &pos, sections, None).unwrap();
        runs.push(t.elapsed().as_secs_f32());
    }
    eprintln!("{}: the {}-token prompt three times: {runs:.2?} s", gpu.adapter(), ids.len());
    let gpu_prompt = runs.last().copied().unwrap_or(0.0);
    let t = Instant::now();
    let lc = cpu.forward_multimodal(&toks, &pos, sections, None).unwrap();
    let cpu_prompt = t.elapsed().as_secs_f32();
    let diff = rel(&lg, &lc);
    let (ans_g, per_g) = generate(&mut |t, p| gpu.forward_multimodal(t, p, sections, None).unwrap(), lg.clone(), next, &stops, 8);
    let (ans_c, per_c) = generate(&mut |t, p| cpu.forward_multimodal(t, p, sections, None).unwrap(), lc.clone(), next, &stops, 8);
    let (ans_g, ans_c) = (tok.decode(&ans_g, true).unwrap(), tok.decode(&ans_c, true).unwrap());
    eprintln!(
        "{}: {} prompt tokens: GPU {gpu_prompt:.2} s ({:.0} tok/s), CPU {cpu_prompt:.2} s; generation GPU {:.0} ms/token, CPU {:.0} ms/token; logit difference {diff:.3} of the largest; GPU {ans_g:?}, CPU {ans_c:?}",
        gpu.adapter(),
        ids.len(),
        ids.len() as f32 / gpu_prompt,
        per_g * 1e3,
        per_c * 1e3
    );
    assert_eq!(argmax(&lg), argmax(&lc), "top token");
    assert!(diff <= 0.25, "logit difference {diff}");
    assert!(ans_g.contains("Paris"), "{ans_g:?}");
}

/// Coloured squares through FilmCraft's own picture encoder (CPU) and the GPU text model
/// (`vl.rs`'s prompt layout): red / blue / green named on the GPU as on the CPU, with the
/// picture prompt's prefill time and the per-token generation time.
#[test]
fn describes_the_synthetic_pictures_on_the_gpu() {
    use filmcraft_llm::vision::VisionEncoder;
    let Some((dir, path)) = model() else { return };
    let config = std::fs::read(dir.join("config.json")).unwrap();
    let vcfg = vision_config::parse(&config, "config.json").unwrap();
    let encoder = VisionEncoder::load(&dir, &vcfg).unwrap();
    let Some((mut cpu, mut gpu, tok, sections)) = load(&dir, &path) else { return };
    let enc = |s: &str| tok.encode(s, false).unwrap().get_ids().to_vec();
    let (vision_start, vision_end) = (tok.token_to_id("<|vision_start|>").unwrap(), tok.token_to_id("<|vision_end|>").unwrap());
    let stops: Vec<u32> = ["<|im_end|>", "<|endoftext|>"].iter().filter_map(|t| tok.token_to_id(t)).collect();
    let question = "What colour is the square? Answer in one word.";
    for (i, ((name, colour), (w, h))) in [("red", [0.9f32, 0.1, 0.1]), ("blue", [0.1, 0.2, 0.9]), ("green", [0.1, 0.8, 0.2])]
        .into_iter()
        .zip([(256usize, 256usize), (448, 256), (256, 256)])
        .enumerate()
    {
        let rgb: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                if (w / 4..w * 3 / 4).contains(&x) && (h / 4..h * 3 / 4).contains(&y) { colour } else { [1.0, 1.0, 1.0] }
            })
            .collect();
        let t = Instant::now();
        let pic = encoder.encode_image_within(&rgb, w, h, 1 << 18).unwrap();
        let encode = t.elapsed().as_secs_f32();
        let mut before = enc("<|im_start|>user\n");
        before.push(vision_start);
        let mut after = vec![vision_end];
        after.extend(enc(&format!("{question}<|im_end|>\n<|im_start|>assistant\n")));
        let mut toks: Vec<MmToken> = before.iter().map(|&t| MmToken::Text(t)).collect();
        toks.extend((0..pic.count).map(MmToken::Picture));
        toks.extend(after.iter().map(|&t| MmToken::Text(t)));
        let (pos, next) = positions(&[Span::Text(before.len()), Span::Picture { rows: pic.rows, cols: pic.cols }, Span::Text(after.len())]).unwrap();
        let picture = MmPicture { tokens: &pic.tokens, deepstack: &pic.deepstack };
        gpu.reset();
        let t = Instant::now();
        let lg = gpu.forward_multimodal(&toks, &pos, sections, Some(picture)).unwrap();
        let gpu_prompt = t.elapsed().as_secs_f32();
        let (ans, per) = generate(&mut |t, p| gpu.forward_multimodal(t, p, sections, None).unwrap(), lg.clone(), next, &stops, 8);
        let answer = tok.decode(&ans, true).unwrap();
        let mut line = format!(
            "{}: {name} ({w} × {h}, {} picture tokens of {}): encode {encode:.1} s (CPU), GPU prompt {gpu_prompt:.2} s, {:.0} ms/token → {answer:?}",
            gpu.adapter(),
            pic.count,
            toks.len(),
            per * 1e3
        );
        if i == 0 {
            // the CPU on the first picture: the same top token, and its times
            cpu.reset();
            let t = Instant::now();
            let lc = cpu.forward_multimodal(&toks, &pos, sections, Some(picture)).unwrap();
            let cpu_prompt = t.elapsed().as_secs_f32();
            let (ans_c, per_c) = generate(&mut |t, p| cpu.forward_multimodal(t, p, sections, None).unwrap(), lc.clone(), next, &stops, 8);
            line.push_str(&format!(
                "; CPU prompt {cpu_prompt:.2} s, {:.0} ms/token → {:?}, logit difference {:.3}",
                per_c * 1e3,
                tok.decode(&ans_c, true).unwrap(),
                rel(&lg, &lc)
            ));
            assert_eq!(argmax(&lg), argmax(&lc), "top token after the picture prompt");
        }
        eprintln!("{line}");
        assert!(answer.to_lowercase().contains(name), "{name}: {answer:?}");
    }
}
