//! Qwen3-VL 4B's text model on our runtime (AI-PLAN AI4 item 2, DESIGN-AI4-TEXT step 5): the
//! cached GGUF (`llm_quantize <model dir> <dir>/text-q8_0.gguf --type q8_0`) against candle's
//! dense `quantized_qwen3` on the same file, for a text-only prompt (where M-RoPE is the plain
//! rotary embedding). Runs only with `FILMCRAFT_QWEN3VL_DIR` set to the model folder holding
//! that file and `tokenizer.json`; skips otherwise. Build with `--features infer` and run with
//! `--test-threads=1`: each test holds ~10 GB, two at once overflow the build slice's 18 GB.
//! `FILMCRAFT_QWEN3VL_GGUF` picks another converted file in that folder (e.g. `text-q4k.gguf`).

#![cfg(feature = "infer")]

use std::io::Cursor;

use filmcraft_llm::forward::{Config, Forward};
use filmcraft_llm::gguf;

#[test]
fn text_model_matches_candle_qwen3() {
    let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
    let path = dir.join(std::env::var("FILMCRAFT_QWEN3VL_GGUF").unwrap_or_else(|_| "text-q8_0.gguf".into()));
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping: {} missing (run llm_quantize)", path.display());
        return;
    };
    let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
    let prompt = "<|im_start|>user\nWhat is the capital of France? Answer in one word.<|im_end|>\n<|im_start|>assistant\n";
    let ids = tok.encode(prompt, false).unwrap().get_ids().to_vec();

    let mut c = Cursor::new(&bytes[..]);
    let s = gguf::check(&mut c, bytes.len() as u64).unwrap();
    let cfg = Config::from_gguf(&s, "qwen3", 512).unwrap();
    assert_eq!((cfg.head_dim, cfg.q_dim, cfg.embd), (128, 4096, 2560));
    let mut ours = Forward::load(&mut c, &s, cfg).unwrap();

    use candle_core::quantized::gguf_file;
    use candle_core::{Device, Tensor};
    let dev = Device::Cpu;
    let mut c = Cursor::new(&bytes[..]);
    let content = gguf_file::Content::read(&mut c).unwrap();
    let mut theirs = candle_transformers::models::quantized_qwen3::ModelWeights::from_gguf(content, &mut c, &dev).unwrap();

    let argmax = |v: &[f32]| v.iter().enumerate().fold((0usize, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0;
    // whole prompt at once on ours; token by token on candle (its prefill mask is only right with
    // an empty cache, so single steps keep it honest)
    let got = ours.forward(&ids, 0).unwrap();
    let mut want = Vec::new();
    for (i, t) in ids.iter().enumerate() {
        let x = Tensor::new(&[*t], &dev).unwrap().unsqueeze(0).unwrap();
        want = theirs.forward(&x, i).unwrap().squeeze(0).unwrap().to_vec1::<f32>().unwrap();
    }
    let scale = want.iter().fold(0f32, |m, v| m.max(v.abs()));
    let worst = got.iter().zip(&want).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    let next = tok.decode(&[argmax(&got) as u32], false).unwrap();
    eprintln!(
        "{} prompt tokens; max |diff| {worst} on logits up to {scale}; next token ours {:?} candle {:?}",
        ids.len(),
        next,
        tok.decode(&[argmax(&want) as u32], false).unwrap()
    );
    assert_eq!(argmax(&got), argmax(&want));
    // Q8_0: 0.39 / 40.5 (1 %); Q4_K_M: 0.85 / 39.0 (2.2 %, 2026-10-16): 4-bit weights leave more
    // room for the two runtimes' different summation orders, so K-quant files get 3 %
    let bound = if path.to_string_lossy().contains("q8_0") { 2e-2 } else { 3e-2 };
    assert!(worst <= bound * scale, "{worst} vs {scale}");
    assert!(next.contains("Paris"), "{next:?}");
}

/// DESIGN-AI4-TEXT step 7: a picture through FilmCraft's own encoder and text model (no candle),
/// via [`filmcraft_llm::vl::VisionLanguage`]: a coloured square on white must be named by its
/// colour, on a square and a wide picture. Synthetic; CPU.
#[test]
fn describes_a_synthetic_picture() {
    use filmcraft_llm::vl::VisionLanguage;

    let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
    let gguf = std::env::var("FILMCRAFT_QWEN3VL_GGUF").unwrap_or_else(|_| "text-q8_0.gguf".into());
    if !dir.join(&gguf).exists() {
        return;
    }
    let mut vl = VisionLanguage::load(&dir, &gguf).unwrap();
    for ((name, colour), (w, h)) in
        [("red", [0.9f32, 0.1, 0.1]), ("blue", [0.1, 0.2, 0.9]), ("green", [0.1, 0.8, 0.2])].into_iter().zip([(256usize, 256usize), (448, 256), (256, 256)])
    {
        let rgb: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                if (w / 4..w * 3 / 4).contains(&x) && (h / 4..h * 3 / 4).contains(&y) { colour } else { [1.0, 1.0, 1.0] }
            })
            .collect();
        let t = std::time::Instant::now();
        let answer = vl.ask(&rgb, w, h, 1 << 18, "What colour is the square? Answer in one word.", 8, None).unwrap();
        eprintln!("{name} ({w} × {h}): {answer:?} in {:.1} s", t.elapsed().as_secs_f32());
        assert!(answer.to_lowercase().contains(name), "{name}: {answer:?}");
    }
    // a stop request is honoured
    let stop = std::sync::atomic::AtomicBool::new(true);
    let rgb = vec![1.0f32; 256 * 256 * 3];
    assert!(matches!(vl.ask(&rgb, 256, 256, 1 << 18, "Describe it.", 8, Some(&stop)), Err(filmcraft_llm::LlmError::Cancelled)));
}

/// FilmCraft's own converter (`convert::qwen3vl_text_to_gguf`) on the real release, when
/// `FILMCRAFT_QWEN3VL_CONVERT=1` (writes `text-own-q8_0.gguf` into the model folder, ~30 s); then
/// run the tests above with `FILMCRAFT_QWEN3VL_GGUF=text-own-q8_0.gguf`.
#[test]
fn own_converter_writes_the_text_model() {
    let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
    if std::env::var_os("FILMCRAFT_QWEN3VL_CONVERT").is_none() {
        return;
    }
    use filmcraft_llm::convert::Quant;
    // `FILMCRAFT_QWEN3VL_CONVERT=q4k` writes the Q4_K mix (`text-own-q4k.gguf`) instead
    let (quant, name) = if std::env::var("FILMCRAFT_QWEN3VL_CONVERT").is_ok_and(|v| v == "q4k") {
        (Quant::Q4KMix, "text-own-q4k.gguf")
    } else {
        (Quant::Q8_0, "text-own-q8_0.gguf")
    };
    let t = std::time::Instant::now();
    let size = filmcraft_llm::convert::qwen3vl_text_to_gguf(&dir, &dir.join(name), quant, None).unwrap();
    eprintln!("{name}: {size} bytes in {:.1} s", t.elapsed().as_secs_f32());
    let bytes = std::fs::read(dir.join(name)).unwrap();
    let mut c = Cursor::new(&bytes[..]);
    let s = gguf::check(&mut c, bytes.len() as u64).unwrap();
    assert!(Forward::load(&mut c, &s, Config::from_gguf(&s, "qwen3", 256).unwrap()).is_ok());
}
