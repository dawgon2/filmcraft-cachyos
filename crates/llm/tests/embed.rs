//! `embed::Embedder` against candle's `BertModel` (the oracle): a tiny random BERT built here
//! (config, safetensors and a WordPiece tokenizer), hostile files, and the real
//! bge-small-en-v1.5 when it is installed.
#![cfg(feature = "infer")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use filmcraft_llm::embed::Embedder;
use tokenizers::Tokenizer;
use tokenizers::models::wordpiece::WordPiece;
use tokenizers::normalizers::BertNormalizer;
use tokenizers::pre_tokenizers::bert::BertPreTokenizer;
use tokenizers::processors::bert::BertProcessing;

const HIDDEN: usize = 32;
const LAYERS: usize = 2;
const INTER: usize = 64;
const POSITIONS: usize = 64;

fn vocab() -> Vec<String> {
    let mut v: Vec<String> = ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]", ".", ",", "?"].iter().map(|s| s.to_string()).collect();
    for w in ["the", "how", "do", "i", "warm", "up", "skin", "tone", "make", "face", "look", "export", "timeline", "as", "gif", "color", "grade"] {
        v.push(w.into());
    }
    for c in 'a'..='z' {
        v.push(c.to_string());
        v.push(format!("##{c}"));
    }
    for s in ["##s", "##er", "##ing"] {
        v.push(s.into());
    }
    v
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fc-embed-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn config_json(vocab: usize) -> String {
    format!(
        r#"{{"vocab_size":{vocab},"hidden_size":{HIDDEN},"num_hidden_layers":{LAYERS},"num_attention_heads":4,"intermediate_size":{INTER},
        "hidden_act":"gelu","hidden_dropout_prob":0.1,"max_position_embeddings":{POSITIONS},"type_vocab_size":2,"initializer_range":0.02,
        "layer_norm_eps":1e-12,"pad_token_id":0,"position_embedding_type":"absolute","model_type":"bert"}}"#
    )
}

/// Random weights (a fixed LCG), named as Hugging Face's BERT names them.
fn weights(vocab: usize, prefix: &str) -> Vec<(String, Vec<usize>, Vec<f32>)> {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = |n: usize, scale: f32, base: f32| -> Vec<f32> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                base + scale * (((seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0)
            })
            .collect()
    };
    let mut out = Vec::new();
    let mut add = |name: String, shape: Vec<usize>, scale: f32, base: f32| {
        let n = shape.iter().product();
        out.push((format!("{prefix}{name}"), shape, rnd(n, scale, base)));
    };
    add("embeddings.word_embeddings.weight".into(), vec![vocab, HIDDEN], 1.0, 0.0);
    add("embeddings.position_embeddings.weight".into(), vec![POSITIONS, HIDDEN], 0.5, 0.0);
    add("embeddings.token_type_embeddings.weight".into(), vec![2, HIDDEN], 0.5, 0.0);
    add("embeddings.LayerNorm.weight".into(), vec![HIDDEN], 0.2, 1.0);
    add("embeddings.LayerNorm.bias".into(), vec![HIDDEN], 0.1, 0.0);
    for l in 0..LAYERS {
        let p = format!("encoder.layer.{l}");
        for (name, o, i) in [
            ("attention.self.query", HIDDEN, HIDDEN),
            ("attention.self.key", HIDDEN, HIDDEN),
            ("attention.self.value", HIDDEN, HIDDEN),
            ("attention.output.dense", HIDDEN, HIDDEN),
            ("intermediate.dense", INTER, HIDDEN),
            ("output.dense", HIDDEN, INTER),
        ] {
            add(format!("{p}.{name}.weight"), vec![o, i], 0.4, 0.0);
            add(format!("{p}.{name}.bias"), vec![o], 0.1, 0.0);
        }
        for name in ["attention.output.LayerNorm", "output.LayerNorm"] {
            add(format!("{p}.{name}.weight"), vec![HIDDEN], 0.2, 1.0);
            add(format!("{p}.{name}.bias"), vec![HIDDEN], 0.1, 0.0);
        }
    }
    out
}

fn tensors(w: &[(String, Vec<usize>, Vec<f32>)], dtype: DType) -> HashMap<String, Tensor> {
    w.iter().map(|(n, s, v)| (n.clone(), Tensor::from_vec(v.clone(), s.as_slice(), &Device::Cpu).unwrap().to_dtype(dtype).unwrap())).collect()
}

fn write_tokenizer(dir: &Path) {
    let v = vocab();
    let map: HashMap<String, u32> = v.iter().enumerate().map(|(i, s)| (s.clone(), i as u32)).collect();
    let json = serde_json::json!({
        "type": "WordPiece", "unk_token": "[UNK]", "continuing_subword_prefix": "##", "max_input_chars_per_word": 100, "vocab": map
    })
    .to_string();
    let wp: WordPiece = serde_json::from_str(&json).unwrap();
    let mut tok = Tokenizer::new(wp);
    tok.with_normalizer(Some(BertNormalizer::default()));
    tok.with_pre_tokenizer(Some(BertPreTokenizer));
    tok.with_post_processor(Some(BertProcessing::new(("[SEP]".into(), 3), ("[CLS]".into(), 2))));
    tok.save(dir.join("tokenizer.json"), false).unwrap();
}

/// A tiny model in a fresh folder; returns the folder and the tensors candle should use.
fn tiny(tag: &str, prefix: &str, dtype: DType) -> (PathBuf, HashMap<String, Tensor>) {
    let dir = tmp(tag);
    let n = vocab().len();
    std::fs::write(dir.join("config.json"), config_json(n)).unwrap();
    write_tokenizer(&dir);
    let t = tensors(&weights(n, prefix), dtype);
    candle_core::safetensors::save(&t, dir.join("model.safetensors")).unwrap();
    let as_f32 = t.into_iter().map(|(k, v)| (k, v.to_dtype(DType::F32).unwrap())).collect();
    (dir, as_f32)
}

fn candle_cls(model: &BertModel, ids: &[u32]) -> Vec<f32> {
    let input = Tensor::new(ids, &Device::Cpu).unwrap().unsqueeze(0).unwrap();
    let types = input.zeros_like().unwrap();
    let out = model.forward(&input, &types, None).unwrap();
    out.get(0).unwrap().get(0).unwrap().to_vec1::<f32>().unwrap()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn long_text() -> String {
    "how do i warm up the skin tones, make faces look warmer. ".repeat(20)
}

fn check_parity(dir: &Path, t: HashMap<String, Tensor>, config: &str) {
    let ours = Embedder::load(dir).unwrap();
    assert_eq!(ours.dim(), HIDDEN);
    let cfg: Config = serde_json::from_str(config).unwrap();
    let theirs = BertModel::load(VarBuilder::from_tensors(t, DType::F32, &Device::Cpu), &cfg).unwrap();
    let long = long_text();
    for text in ["How do I warm up the skin tones?", "export the timeline as a GIF", "", "zzqx", long.as_str()] {
        let ids = ours.tokens(text).unwrap();
        assert_eq!(ids.first(), Some(&2), "[CLS] first: {text}");
        assert_eq!(ids.last(), Some(&3), "[SEP] last: {text}");
        let raw = ours.embed_raw(text).unwrap();
        let want = candle_cls(&theirs, &ids);
        let d = max_diff(&raw, &want);
        assert!(d < 1e-4, "{text:?}: max diff {d}");
        let unit = ours.embed(text).unwrap();
        assert!((cos(&unit, &unit) - 1.0).abs() < 1e-5);
    }
    // truncation: the long text fills every position, ending in [SEP]
    assert_eq!(ours.tokens(&long).unwrap().len(), POSITIONS);
    let batch = ours.embed_batch(&["export the timeline as a GIF", ""]).unwrap();
    assert_eq!(batch.len(), 2);
    assert!(max_diff(&batch[0], &ours.embed("export the timeline as a GIF").unwrap()) < 1e-6);
}

#[test]
fn tiny_bert_matches_candle() {
    let (dir, t) = tiny("plain", "", DType::F32);
    check_parity(&dir, t, &config_json(vocab().len()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tiny_bert_with_prefix_and_half_weights_matches_candle() {
    for (tag, dtype) in [("f16", DType::F16), ("bf16", DType::BF16)] {
        let (dir, t) = tiny(tag, "bert.", dtype);
        check_parity(&dir, t, &config_json(vocab().len()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn hostile_files_are_errors_not_panics() {
    let n = vocab().len();
    let fresh = |tag: &str, w: Vec<(String, Vec<usize>, Vec<f32>)>, config: String| {
        let dir = tmp(tag);
        std::fs::write(dir.join("config.json"), config).unwrap();
        write_tokenizer(&dir);
        candle_core::safetensors::save(&tensors(&w, DType::F32), dir.join("model.safetensors")).unwrap();
        dir
    };
    let mut cases = Vec::new();
    // a missing tensor
    let mut w = weights(n, "");
    w.retain(|(name, _, _)| name != "encoder.layer.1.output.dense.weight");
    cases.push(fresh("missing", w, config_json(n)));
    // a wrong shape
    let mut w = weights(n, "");
    if let Some(e) = w.iter_mut().find(|(name, _, _)| name == "encoder.layer.0.attention.self.key.weight") {
        e.1 = vec![HIDDEN / 2, HIDDEN * 2];
    }
    cases.push(fresh("shape", w, config_json(n)));
    // absurd config numbers
    for (i, (from, to)) in [
        ("\"num_hidden_layers\":2", "\"num_hidden_layers\":4000000000"),
        ("\"num_attention_heads\":4", "\"num_attention_heads\":5"),
        ("\"max_position_embeddings\":64", "\"max_position_embeddings\":99999999"),
        ("\"hidden_size\":32", "\"hidden_size\":-1"),
    ]
    .into_iter()
    .enumerate()
    {
        cases.push(fresh(&format!("config{i}"), weights(n, ""), config_json(n).replace(from, to)));
    }
    for dir in &cases {
        assert!(Embedder::load(dir).is_err(), "{}", dir.display());
    }
    // a truncated safetensors file, at several cuts
    let dir = fresh("truncated", weights(n, ""), config_json(n));
    let bytes = std::fs::read(dir.join("model.safetensors")).unwrap();
    for cut in [0, 4, 8, 100, bytes.len() / 2, bytes.len() - 1] {
        std::fs::write(dir.join("model.safetensors"), &bytes[..cut]).unwrap();
        assert!(Embedder::load(&dir).is_err(), "cut at {cut}");
    }
    // garbage tokenizer, missing files
    std::fs::write(dir.join("model.safetensors"), &bytes).unwrap();
    assert!(Embedder::load(&dir).is_ok());
    std::fs::write(dir.join("tokenizer.json"), b"{not json").unwrap();
    assert!(Embedder::load(&dir).is_err());
    std::fs::remove_file(dir.join("tokenizer.json")).unwrap();
    assert!(Embedder::load(&dir).is_err());
    assert!(Embedder::load(&dir.join("nowhere")).is_err());
    cases.push(dir);
    // a tokenizer whose ids run past the model's vocabulary: loads, then each text is an error
    let small = 12;
    let dir = fresh("vocab", weights(small, ""), config_json(small));
    let e = Embedder::load(&dir).unwrap();
    assert!(e.embed("zzz").is_err());
    assert!(e.forward(&[]).is_err());
    assert!(e.forward(&[0; POSITIONS + 1]).is_err());
    cases.push(dir);
    for d in cases {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// The real BAAI/bge-small-en-v1.5 (skips when it isn't installed).
#[test]
fn bge_small_matches_candle_and_ranks_sensibly() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = PathBuf::from(home).join(".local/share/filmcraft-fork/models/embed/bge-small-en-v1.5");
    if !dir.join("model.safetensors").exists() {
        eprintln!("skipping: bge-small-en-v1.5 is not installed at {}", dir.display());
        return;
    }
    let ours = Embedder::load(&dir).unwrap();
    assert_eq!(ours.dim(), 384);
    let cfg: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
    let st = candle_core::safetensors::load(dir.join("model.safetensors"), &Device::Cpu).unwrap();
    let st = st.into_iter().map(|(k, v)| (k, v.to_dtype(DType::F32).unwrap())).collect();
    let theirs = BertModel::load(VarBuilder::from_tensors(st, DType::F32, &Device::Cpu), &cfg).unwrap();
    let q = "how do I warm up the skin tones";
    let (near, far) = ("make faces look warmer", "export the timeline as a GIF");
    for text in [q, near, far] {
        let ids = ours.tokens(text).unwrap();
        let d = max_diff(&ours.embed_raw(text).unwrap(), &candle_cls(&theirs, &ids));
        assert!(d < 1e-4, "{text:?}: max diff {d}");
    }
    let t = std::time::Instant::now();
    let e = ours.embed_batch(&[q, near, far]).unwrap();
    eprintln!("3 texts in {:?}", t.elapsed());
    let (a, b) = (cos(&e[0], &e[1]), cos(&e[0], &e[2]));
    eprintln!("cos(near) {a:.4}, cos(far) {b:.4}");
    assert!(a > b, "near {a} should beat far {b}");
}
