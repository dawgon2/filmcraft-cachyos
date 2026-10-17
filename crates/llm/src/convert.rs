//! Converting a downloaded Qwen3-VL release's language model (BF16 / F32 safetensors) into a
//! Q8_0 GGUF that [`crate::forward::Forward`] loads (AI-PLAN AI4 item 2), inside FilmCraft:
//! users can't run the `llm_quantize` example, which needs candle. One tensor at a time, so
//! memory stays near the largest tensor; matrices become Q8_0 (as ggml's `quantize_row_q8_0`,
//! [`crate::quant::quantize_q8_0`]), vectors stay F32. Names and metadata follow the GGUF
//! conventions `llm_quantize` uses (`blk.N.attn_q`, `qwen3.attention.key_length`, …).
//! [`Quant::Q4KMix`]: Q4_K matrices (candle's quantizer, byte for byte), with Q8_0 where
//! `llm_quantize`'s Q4_K_M uses Q6_K (`attn_v` / `ffn_down` in the "more bits" blocks, and an
//! output head copied from the tied embedding).

use std::collections::BTreeMap;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::LlmError;
use crate::quant::{QK8_0, quantize_q8_0};
use crate::safetensors::{self, Header, TensorInfo};

/// Largest single tensor converted (elements): Qwen3-VL 4B's embedding is ~3.9 × 10^8.
const MAX_ELEMENTS: u64 = 1 << 29;
const ALIGN: u64 = 32;
const GGML_F32: u32 = 0;
const GGML_Q8_0: u32 = 8;
const GGML_Q4K: u32 = 12;
const Q8_0_BYTES: u64 = 34;

/// How the matrices are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quant {
    /// Every matrix Q8_0 (4.3 GB for Qwen3-VL 4B's text model).
    Q8_0,
    /// Q4_K with Q8_0 for the sensitive tensors (about 2.9 GB).
    Q4KMix,
}

/// llama.cpp's Q4_K_M "more bits" blocks: the first and last eighth, and every third between.
fn more_bits(block: u64, blocks: u64) -> bool {
    let e = blocks / 8;
    block < e || block >= blocks.saturating_sub(e) || (block >= e && (block - e) % 3 == 2)
}

/// The GGML type and byte size of a `rows × cols` matrix named `name`.
fn matrix_type(quant: Quant, name: &str, rows: u64, cols: u64, layers: u64) -> (u32, u64) {
    let q8 = (GGML_Q8_0, rows * (cols / QK8_0 as u64) * Q8_0_BYTES);
    let k_fits = cols.is_multiple_of(crate::kquant::QK_K as u64);
    match quant {
        Quant::Q4KMix if k_fits => {
            let block = name.strip_prefix("blk.").and_then(|r| r.split_once('.')).and_then(|(n, _)| n.parse::<u64>().ok());
            let more = (name.ends_with("attn_v.weight") || name.ends_with("ffn_down.weight")) && block.is_some_and(|b| more_bits(b, layers));
            if name == "output.weight" || more { q8 } else { (GGML_Q4K, rows * (cols / crate::kquant::QK_K as u64) * crate::kquant::Q4K_BYTES as u64) }
        }
        _ => q8,
    }
}

fn bad(file: &str, reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: file.to_string(), reason: reason.into() }
}

/// The text model's shape from a Qwen3-VL `config.json` (`text_config`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextShape {
    pub hidden: u64,
    pub heads: u64,
    pub kv_heads: u64,
    pub head_dim: u64,
    pub layers: u64,
    pub ffn: u64,
    pub vocab: u64,
    pub context: u64,
    pub eps: f64,
    pub theta: f64,
}

/// Parses `text_config` of a Qwen3-VL `config.json`; every number checked.
pub fn text_shape(config: &[u8]) -> Result<TextShape, LlmError> {
    let v: serde_json::Value = serde_json::from_slice(config).map_err(|e| bad("config.json", e.to_string()))?;
    let t = v.get("text_config").ok_or_else(|| bad("config.json", "no text_config"))?;
    let u = |k: &str, max: u64| {
        t.get(k).and_then(serde_json::Value::as_u64).filter(|x| (1..=max).contains(x)).ok_or_else(|| bad("config.json", format!("text_config.{k}")))
    };
    let f = |k: &str| {
        t.get(k).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite() && *x > 0.0).ok_or_else(|| bad("config.json", format!("text_config.{k}")))
    };
    let s = TextShape {
        hidden: u("hidden_size", 1 << 16)?,
        heads: u("num_attention_heads", 1024)?,
        kv_heads: u("num_key_value_heads", 1024)?,
        head_dim: u("head_dim", 512)?,
        layers: u("num_hidden_layers", 512)?,
        ffn: u("intermediate_size", 1 << 18)?,
        vocab: u("vocab_size", 1 << 20)?,
        context: u("max_position_embeddings", 1 << 24)?.min(131_072),
        eps: f("rms_norm_eps")?,
        theta: f("rope_theta")?,
    };
    if !s.heads.is_multiple_of(s.kv_heads) || !s.head_dim.is_multiple_of(2) || s.eps >= 1.0 {
        return Err(bad("config.json", "bad head layout"));
    }
    Ok(s)
}

/// The GGUF name of a language-model tensor (`model.language_model.` already stripped).
fn gguf_name(rest: &str) -> Option<String> {
    match rest {
        "embed_tokens.weight" => return Some("token_embd.weight".into()),
        "norm.weight" => return Some("output_norm.weight".into()),
        _ => {}
    }
    let (n, tail) = rest.strip_prefix("layers.")?.split_once('.')?;
    let n: u32 = n.parse().ok()?;
    let (module, kind) = tail.rsplit_once('.')?;
    let short = match module {
        "input_layernorm" => "attn_norm",
        "post_attention_layernorm" => "ffn_norm",
        "self_attn.q_proj" => "attn_q",
        "self_attn.k_proj" => "attn_k",
        "self_attn.v_proj" => "attn_v",
        "self_attn.o_proj" => "attn_output",
        "self_attn.q_norm" => "attn_q_norm",
        "self_attn.k_norm" => "attn_k_norm",
        "mlp.gate_proj" => "ffn_gate",
        "mlp.up_proj" => "ffn_up",
        "mlp.down_proj" => "ffn_down",
        _ => return None,
    };
    (kind == "weight").then(|| format!("blk.{n}.{short}.weight"))
}

struct Planned {
    name: String,
    file: String,
    info: TensorInfo,
    /// GGUF dims (innermost first).
    dims: Vec<u64>,
    ty: u32,
    bytes: u64,
}

fn put_str(w: &mut Vec<u8>, s: &str) {
    w.extend_from_slice(&(s.len() as u64).to_le_bytes());
    w.extend_from_slice(s.as_bytes());
}

fn kv_u32(w: &mut Vec<u8>, k: &str, v: u64) -> Result<(), LlmError> {
    put_str(w, k);
    w.extend_from_slice(&4u32.to_le_bytes());
    w.extend_from_slice(&u32::try_from(v).map_err(|_| bad("config.json", format!("{k} too large")))?.to_le_bytes());
    Ok(())
}

fn kv_f32(w: &mut Vec<u8>, k: &str, v: f64) {
    put_str(w, k);
    w.extend_from_slice(&6u32.to_le_bytes());
    w.extend_from_slice(&(v as f32).to_le_bytes());
}

fn kv_str(w: &mut Vec<u8>, k: &str, v: &str) {
    put_str(w, k);
    w.extend_from_slice(&8u32.to_le_bytes());
    put_str(w, v);
}

/// Converts the language model in `dir` (a Qwen3-VL release with `config.json` and
/// `model.safetensors.index.json`) to a Q8_0 GGUF at `out` (written as `out.part`, renamed when
/// complete). `cancel` stops between tensors. Returns the file size.
pub fn qwen3vl_text_to_gguf(dir: &Path, out: &Path, quant: Quant, cancel: Option<&AtomicBool>) -> Result<u64, LlmError> {
    let shape = text_shape(&std::fs::read(dir.join("config.json")).map_err(|_| LlmError::NotInstalled("config.json".into()))?)?;
    let index = std::fs::read(dir.join("model.safetensors.index.json")).map_err(|_| LlmError::NotInstalled("model.safetensors.index.json".into()))?;
    let index = safetensors::parse_index(&index, "model.safetensors.index.json")?;
    let mut headers: BTreeMap<String, Header> = BTreeMap::new();
    // an untied output head would be dropped (the GGUF ties it to the embedding): refuse such a
    // release rather than convert it into a model that writes garbage
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("config.json")).map_err(|_| LlmError::NotInstalled("config.json".into()))?)
        .map_err(|e| bad("config.json", e.to_string()))?;
    let tied =
        config.get("tie_word_embeddings").or_else(|| config.get("text_config").and_then(|t| t.get("tie_word_embeddings"))).and_then(serde_json::Value::as_bool);
    if tied == Some(false) || index.keys().any(|k| k == "lm_head.weight" || k.ends_with(".lm_head.weight")) {
        return Err(bad("config.json", "an untied output head (lm_head) is not supported"));
    }
    let mut plan: Vec<Planned> = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for (hf, file) in &index {
        let Some(rest) = hf.strip_prefix("model.language_model.") else { continue };
        let Some(name) = gguf_name(rest) else { continue };
        if let Some(n) = name.strip_prefix("blk.").and_then(|r| r.split('.').next()).and_then(|n| n.parse::<u64>().ok())
            && n >= shape.layers
        {
            return Err(bad(file, format!("{hf}: layer {n} of {}", shape.layers)));
        }
        if !names.insert(name.clone()) {
            return Err(bad(file, format!("{hf}: two tensors would be named {name}")));
        }
        if !headers.contains_key(file) {
            headers.insert(file.clone(), safetensors::read_header(&dir.join(file))?);
        }
        let info = headers.get(file).and_then(|h| h.tensors.get(hf)).ok_or_else(|| bad(file, format!("{hf} missing")))?.clone();
        let elements = info.elements();
        if !matches!(info.dtype, safetensors::Dtype::F32 | safetensors::Dtype::Bf16) {
            return Err(bad(file, format!("{hf}: dtype {:?} (F32 or BF16 only)", info.dtype)));
        }
        if elements == 0 || elements > MAX_ELEMENTS {
            return Err(bad(file, format!("{hf}: {elements} elements")));
        }
        let (dims, ty, bytes) = match info.shape.as_slice() {
            [n] => (vec![*n], GGML_F32, n * 4),
            [rows, cols] if cols % QK8_0 as u64 == 0 => {
                let (ty, bytes) = matrix_type(quant, &name, *rows, *cols, shape.layers);
                (vec![*cols, *rows], ty, bytes)
            }
            [rows, cols] => (vec![*cols, *rows], GGML_F32, rows * cols * 4),
            other => return Err(bad(file, format!("{hf}: shape {other:?}"))),
        };
        plan.push(Planned { name, file: file.clone(), info, dims, ty, bytes });
    }
    // Q4_K mix: a separate, higher-precision output head (a copy of the tied embedding)
    if quant == Quant::Q4KMix
        && let Some(e) = plan.iter().find(|p| p.name == "token_embd.weight")
        && let [rows, cols] = e.info.shape.as_slice()
        && cols % QK8_0 as u64 == 0
    {
        let (ty, bytes) = matrix_type(quant, "output.weight", *rows, *cols, shape.layers);
        let head = Planned { name: "output.weight".into(), file: e.file.clone(), info: e.info.clone(), dims: e.dims.clone(), ty, bytes };
        plan.push(head);
    }
    let have = |n: &str| plan.iter().any(|p| p.name == n);
    if !have("token_embd.weight") || !have("output_norm.weight") || !have(&format!("blk.{}.ffn_down.weight", shape.layers - 1)) {
        return Err(bad("model.safetensors.index.json", "the language model's tensors are incomplete"));
    }

    // header: metadata, tensor infos, then aligned data
    let mut kv = Vec::new();
    kv_str(&mut kv, "general.architecture", "qwen3");
    kv_str(&mut kv, "general.name", "qwen3-vl-text");
    kv_u32(&mut kv, "general.file_type", if quant == Quant::Q8_0 { 7 } else { 15 })?;
    kv_str(&mut kv, "general.quantized_by", "filmcraft-llm convert");
    let a = "qwen3";
    for (k, v) in [
        ("context_length", shape.context),
        ("embedding_length", shape.hidden),
        ("feed_forward_length", shape.ffn),
        ("block_count", shape.layers),
        ("attention.head_count", shape.heads),
        ("attention.head_count_kv", shape.kv_heads),
        ("rope.dimension_count", shape.head_dim),
        ("attention.key_length", shape.head_dim),
        ("attention.value_length", shape.head_dim),
        ("vocab_size", shape.vocab),
    ] {
        kv_u32(&mut kv, &format!("{a}.{k}"), v)?;
    }
    kv_f32(&mut kv, &format!("{a}.attention.layer_norm_rms_epsilon"), shape.eps);
    kv_f32(&mut kv, &format!("{a}.rope.freq_base"), shape.theta);
    let n_kv = 4 + 10 + 2u64;
    let mut ti = Vec::new();
    let mut offset = 0u64;
    for p in &plan {
        put_str(&mut ti, &p.name);
        ti.extend_from_slice(&(p.dims.len() as u32).to_le_bytes());
        for d in &p.dims {
            ti.extend_from_slice(&d.to_le_bytes());
        }
        ti.extend_from_slice(&p.ty.to_le_bytes());
        ti.extend_from_slice(&offset.to_le_bytes());
        offset = (offset + p.bytes).div_ceil(ALIGN) * ALIGN;
    }
    let mut head = Vec::new();
    head.extend_from_slice(b"GGUF");
    head.extend_from_slice(&3u32.to_le_bytes());
    head.extend_from_slice(&(plan.len() as u64).to_le_bytes());
    head.extend_from_slice(&n_kv.to_le_bytes());
    head.extend_from_slice(&kv);
    head.extend_from_slice(&ti);
    head.resize(((head.len() as u64).div_ceil(ALIGN) * ALIGN) as usize, 0);

    let part = out.with_extension("gguf.part");
    let result = (|| -> Result<u64, LlmError> {
        let mut w = BufWriter::new(std::fs::File::create(&part)?);
        w.write_all(&head)?;
        let mut written = 0u64;
        let mut blocks = Vec::new();
        for p in &plan {
            let start = w.stream_position()?;
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(LlmError::Cancelled);
            }
            let values = safetensors::read_f32(&dir.join(&p.file), &p.info, MAX_ELEMENTS)?;
            if p.ty == GGML_Q4K {
                let mut bytes = Vec::new();
                crate::kquant::quantize_q4k_bytes(&values, &mut bytes)?;
                w.write_all(&bytes)?;
            } else if p.ty == GGML_Q8_0 {
                quantize_q8_0(&values, &mut blocks)?;
                for b in &blocks {
                    w.write_all(&half::f16::from_f32(b.d).to_le_bytes())?;
                    w.write_all(&b.qs.map(|q| q.to_le_bytes()[0]))?;
                }
            } else {
                for v in &values {
                    w.write_all(&v.to_le_bytes())?;
                }
            }
            // the offsets in the header were planned from `p.bytes`: what was written must match
            let wrote = w.stream_position()?.saturating_sub(start);
            if wrote != p.bytes {
                return Err(bad(&p.file, format!("{}: wrote {wrote} bytes, planned {}", p.name, p.bytes)));
            }
            let padded = p.bytes.div_ceil(ALIGN) * ALIGN;
            w.write_all(&vec![0u8; (padded - p.bytes) as usize])?;
            written += padded;
        }
        w.flush()?;
        w.get_ref().sync_all()?;
        Ok(head.len() as u64 + written)
    })();
    match result {
        Ok(size) => {
            if let Err(e) = std::fs::rename(&part, out) {
                let _ = std::fs::remove_file(&part);
                return Err(e.into());
            }
            Ok(size)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_gguf_conventions() {
        assert_eq!(gguf_name("embed_tokens.weight").as_deref(), Some("token_embd.weight"));
        assert_eq!(gguf_name("layers.3.self_attn.k_norm.weight").as_deref(), Some("blk.3.attn_k_norm.weight"));
        assert_eq!(gguf_name("layers.0.mlp.down_proj.weight").as_deref(), Some("blk.0.ffn_down.weight"));
        for skip in ["layers.x.mlp.down_proj.weight", "layers.0.mlp.down_proj.bias", "layers.0.other.weight", "lm_head.weight"] {
            assert_eq!(gguf_name(skip), None, "{skip}");
        }
    }

    #[test]
    fn text_shape_is_checked() {
        let good = br#"{"text_config":{"hidden_size":64,"num_attention_heads":4,"num_key_value_heads":2,"head_dim":32,"num_hidden_layers":2,"intermediate_size":128,"vocab_size":96,"max_position_embeddings":256,"rms_norm_eps":1e-6,"rope_theta":10000.0}}"#;
        assert_eq!(text_shape(good).unwrap().head_dim, 32);
        let s = String::from_utf8_lossy(good).into_owned();
        for bad in [
            s.replace("\"num_key_value_heads\":2", "\"num_key_value_heads\":3"),
            s.replace("\"head_dim\":32", "\"head_dim\":33"),
            s.replace("\"rms_norm_eps\":1e-6", "\"rms_norm_eps\":-1"),
            s.replace("\"hidden_size\":64", "\"hidden_size\":0"),
            "{}".into(),
        ] {
            assert!(text_shape(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    /// A tiny Qwen3-VL-shaped checkpoint (F32 safetensors + index + config) converted by us must
    /// run exactly like the same weights written as a GGUF directly.
    #[test]
    fn a_converted_checkpoint_runs_like_the_same_weights_as_gguf() {
        use crate::forward::{Config, Forward};
        use crate::gguf::tests::Builder;
        use std::io::Cursor;
        let dir = std::env::temp_dir().join(format!("filmcraft-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (embd, head, heads, kv_heads, ffn, vocab, layers) = (64usize, 32usize, 4usize, 2usize, 128usize, 96usize, 2usize);
        let mut seed = 7u32;
        let mut values = |n: usize, amp: f32, base: f32| -> Vec<f32> {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (0..n).map(|i| ((((i as u32).wrapping_mul(2_654_435_761) ^ seed) >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * amp + base).collect()
        };
        // (HF name, shape, values)
        let mut tensors: Vec<(String, Vec<usize>, Vec<f32>)> = vec![
            ("model.language_model.embed_tokens.weight".into(), vec![vocab, embd], values(vocab * embd, 2.0, 0.0)),
            ("model.language_model.norm.weight".into(), vec![embd], values(embd, 0.2, 1.0)),
            ("model.visual.patch_embed.proj.bias".into(), vec![8], values(8, 1.0, 0.0)),
        ];
        for i in 0..layers {
            let p = format!("model.language_model.layers.{i}");
            for (n, r, c) in [
                ("self_attn.q_proj", heads * head, embd),
                ("self_attn.k_proj", kv_heads * head, embd),
                ("self_attn.v_proj", kv_heads * head, embd),
                ("self_attn.o_proj", embd, heads * head),
                ("mlp.gate_proj", ffn, embd),
                ("mlp.up_proj", ffn, embd),
                ("mlp.down_proj", embd, ffn),
            ] {
                tensors.push((format!("{p}.{n}.weight"), vec![r, c], values(r * c, 0.5, 0.0)));
            }
            for (n, len) in [("input_layernorm", embd), ("post_attention_layernorm", embd), ("self_attn.q_norm", head), ("self_attn.k_norm", head)] {
                tensors.push((format!("{p}.{n}.weight"), vec![len], values(len, 0.2, 1.0)));
            }
        }
        // safetensors file + index + config
        let mut header = serde_json::Map::new();
        let mut data = Vec::new();
        for (name, shape, v) in &tensors {
            let start = data.len();
            data.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            header.insert(name.clone(), serde_json::json!({"dtype": "F32", "shape": shape, "data_offsets": [start, data.len()]}));
        }
        let h = serde_json::to_vec(&serde_json::Value::Object(header)).unwrap();
        let mut file = (h.len() as u64).to_le_bytes().to_vec();
        file.extend(&h);
        file.extend(&data);
        std::fs::write(dir.join("model-00001-of-00001.safetensors"), file).unwrap();
        let map: serde_json::Map<String, serde_json::Value> =
            tensors.iter().map(|(n, _, _)| (n.clone(), serde_json::json!("model-00001-of-00001.safetensors"))).collect();
        std::fs::write(dir.join("model.safetensors.index.json"), serde_json::to_vec(&serde_json::json!({"weight_map": map})).unwrap()).unwrap();
        std::fs::write(dir.join("config.json"), format!(r#"{{"text_config":{{"hidden_size":{embd},"num_attention_heads":{heads},"num_key_value_heads":{kv_heads},"head_dim":{head},"num_hidden_layers":{layers},"intermediate_size":{ffn},"vocab_size":{vocab},"max_position_embeddings":256,"rms_norm_eps":1e-6,"rope_theta":10000.0}}}}"#)).unwrap();

        let out = dir.join("text.gguf");
        let size = qwen3vl_text_to_gguf(&dir, &out, Quant::Q8_0, None).unwrap();
        let ours = std::fs::read(&out).unwrap();
        assert_eq!(ours.len() as u64, size);
        assert!(!dir.join("text.gguf.part").exists());

        // the same weights straight into a GGUF (Q8_0 matrices, F32 vectors)
        let q8 = |v: &[f32]| -> Vec<u8> {
            let mut q = Vec::new();
            crate::quant::quantize_q8_0(v, &mut q).unwrap();
            q.iter().flat_map(|b| half::f16::from_f32(b.d).to_le_bytes().into_iter().chain(b.qs.iter().map(|x| x.to_le_bytes()[0]))).collect()
        };
        let mut b = Builder::default()
            .str("general.architecture", "qwen3")
            .u32("qwen3.embedding_length", embd as u32)
            .u32("qwen3.attention.head_count", heads as u32)
            .u32("qwen3.attention.head_count_kv", kv_heads as u32)
            .u32("qwen3.attention.key_length", head as u32)
            .u32("qwen3.block_count", layers as u32)
            .u32("qwen3.context_length", 256)
            .f32("qwen3.attention.layer_norm_rms_epsilon", 1e-6)
            .f32("qwen3.rope.freq_base", 10_000.0);
        for (name, shape, v) in &tensors {
            let Some(g) = name.strip_prefix("model.language_model.").and_then(gguf_name) else { continue };
            b = match shape.as_slice() {
                [n] => b.tensor_data(&g, &[*n as u64], GGML_F32, &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()),
                [r, c] => b.tensor_data(&g, &[*c as u64, *r as u64], GGML_Q8_0, &q8(v)),
                _ => unreachable!(),
            };
        }
        let direct = b.build();
        let run = |bytes: &[u8]| {
            let mut c = Cursor::new(bytes);
            let s = crate::gguf::check(&mut c, bytes.len() as u64).unwrap();
            let mut f = Forward::load(&mut c, &s, Config::from_gguf(&s, "qwen3", 64).unwrap()).unwrap();
            f.forward(&[5, 17, 3], 0).unwrap()
        };
        assert_eq!(run(&ours), run(&direct));

        // a cancelled conversion leaves nothing behind
        let stop = AtomicBool::new(true);
        let out2 = dir.join("cancelled.gguf");
        assert!(matches!(qwen3vl_text_to_gguf(&dir, &out2, Quant::Q8_0, Some(&stop)), Err(LlmError::Cancelled)));
        assert!(!out2.exists() && !dir.join("cancelled.gguf.part").exists());
        // review fixes (2026-10-16): an untied head, a duplicate or out-of-range layer and a failed
        // rename are errors, and the failed rename leaves no `.part` behind
        let config = std::fs::read_to_string(dir.join("config.json")).unwrap();
        std::fs::write(dir.join("config.json"), config.replacen('{', r#"{"tie_word_embeddings":false,"#, 1)).unwrap();
        assert!(matches!(qwen3vl_text_to_gguf(&dir, &dir.join("x.gguf"), Quant::Q8_0, None), Err(LlmError::BadModel { .. })));
        std::fs::write(dir.join("config.json"), &config).unwrap();
        let index = std::fs::read_to_string(dir.join("model.safetensors.index.json")).unwrap();
        for alias in ["model.language_model.layers.01.mlp.up_proj.weight", "model.language_model.layers.7.mlp.up_proj.weight", "lm_head.weight"] {
            let bad_index = index.replacen("{\"weight_map\":{", &format!("{{\"weight_map\":{{\"{alias}\":\"model-00001-of-00001.safetensors\","), 1);
            assert_ne!(bad_index, index);
            std::fs::write(dir.join("model.safetensors.index.json"), bad_index).unwrap();
            assert!(matches!(qwen3vl_text_to_gguf(&dir, &dir.join("x.gguf"), Quant::Q8_0, None), Err(LlmError::BadModel { .. })), "{alias}");
        }
        std::fs::write(dir.join("model.safetensors.index.json"), &index).unwrap();
        let blocked = dir.join("a-folder.gguf");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(qwen3vl_text_to_gguf(&dir, &blocked, Quant::Q8_0, None).is_err());
        assert!(!dir.join("a-folder.gguf.part").exists(), "no partial file left after a failed rename");
        // an incomplete checkpoint is refused
        std::fs::write(dir.join("model.safetensors.index.json"), br#"{"weight_map":{"model.language_model.norm.weight":"model-00001-of-00001.safetensors"}}"#)
            .unwrap();
        assert!(matches!(qwen3vl_text_to_gguf(&dir, &dir.join("x.gguf"), Quant::Q8_0, None), Err(LlmError::BadModel { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Q4_K mix on a 256-wide checkpoint: Q4_K matrices, Q8_0 for the "more bits" ones and an
    /// output head; it runs and agrees with the Q8_0 file on the next token.
    #[test]
    fn the_q4k_mix_runs_like_the_q8_0_file() {
        use crate::forward::{Config, Forward};
        use std::io::Cursor;
        let dir = std::env::temp_dir().join(format!("filmcraft-convert-q4k-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (embd, head, heads, kv_heads, ffn, vocab, layers) = (256usize, 64usize, 4usize, 2usize, 512usize, 64usize, 2usize);
        let mut seed = 3u32;
        let mut values = |n: usize, amp: f32, base: f32| -> Vec<f32> {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (0..n).map(|i| ((((i as u32).wrapping_mul(2_654_435_761) ^ seed) >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * amp + base).collect()
        };
        let mut tensors: Vec<(String, Vec<usize>, Vec<f32>)> = vec![
            ("model.language_model.embed_tokens.weight".into(), vec![vocab, embd], values(vocab * embd, 2.0, 0.0)),
            ("model.language_model.norm.weight".into(), vec![embd], values(embd, 0.2, 1.0)),
        ];
        for i in 0..layers {
            let p = format!("model.language_model.layers.{i}");
            for (n, r, c) in [
                ("self_attn.q_proj", heads * head, embd),
                ("self_attn.k_proj", kv_heads * head, embd),
                ("self_attn.v_proj", kv_heads * head, embd),
                ("self_attn.o_proj", embd, heads * head),
                ("mlp.gate_proj", ffn, embd),
                ("mlp.up_proj", ffn, embd),
                ("mlp.down_proj", embd, ffn),
            ] {
                tensors.push((format!("{p}.{n}.weight"), vec![r, c], values(r * c, 0.2, 0.0)));
            }
            for (n, len) in [("input_layernorm", embd), ("post_attention_layernorm", embd), ("self_attn.q_norm", head), ("self_attn.k_norm", head)] {
                tensors.push((format!("{p}.{n}.weight"), vec![len], values(len, 0.2, 1.0)));
            }
        }
        let mut header = serde_json::Map::new();
        let mut data = Vec::new();
        for (name, shape, v) in &tensors {
            let start = data.len();
            data.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            header.insert(name.clone(), serde_json::json!({"dtype": "F32", "shape": shape, "data_offsets": [start, data.len()]}));
        }
        let h = serde_json::to_vec(&serde_json::Value::Object(header)).unwrap();
        let mut file = (h.len() as u64).to_le_bytes().to_vec();
        file.extend(&h);
        file.extend(&data);
        std::fs::write(dir.join("m.safetensors"), file).unwrap();
        let map: serde_json::Map<String, serde_json::Value> = tensors.iter().map(|(n, _, _)| (n.clone(), serde_json::json!("m.safetensors"))).collect();
        std::fs::write(dir.join("model.safetensors.index.json"), serde_json::to_vec(&serde_json::json!({"weight_map": map})).unwrap()).unwrap();
        std::fs::write(dir.join("config.json"), format!(r#"{{"text_config":{{"hidden_size":{embd},"num_attention_heads":{heads},"num_key_value_heads":{kv_heads},"head_dim":{head},"num_hidden_layers":{layers},"intermediate_size":{ffn},"vocab_size":{vocab},"max_position_embeddings":256,"rms_norm_eps":1e-6,"rope_theta":10000.0}}}}"#)).unwrap();

        let logits = |quant: Quant, name: &str| {
            let out = dir.join(name);
            qwen3vl_text_to_gguf(&dir, &out, quant, None).unwrap();
            let bytes = std::fs::read(&out).unwrap();
            let mut c = Cursor::new(&bytes[..]);
            let s = crate::gguf::check(&mut c, bytes.len() as u64).unwrap();
            let types: Vec<u32> =
                ["blk.0.attn_q.weight", "blk.0.attn_v.weight", "output.weight"].iter().map(|n| s.tensor(n).map_or(u32::MAX, |t| t.ty)).collect();
            let mut f = Forward::load(&mut c, &s, Config::from_gguf(&s, "qwen3", 64).unwrap()).unwrap();
            (f.forward(&[5, 17, 3], 0).unwrap(), types, bytes.len())
        };
        let (q8, t8, n8) = logits(Quant::Q8_0, "q8.gguf");
        let (q4, t4, n4) = logits(Quant::Q4KMix, "q4.gguf");
        assert_eq!(t8, vec![GGML_Q8_0, GGML_Q8_0, u32::MAX], "Q8_0: tied output");
        // layer 0 of 2 is a "more bits" block (first eighth rounds to 0, so: (0 − 0) % 3 ≠ 2 → no)
        assert_eq!(t4, vec![GGML_Q4K, if more_bits(0, 2) { GGML_Q8_0 } else { GGML_Q4K }, GGML_Q8_0], "Q4_K mix");
        assert!(n4 < n8, "{n4} vs {n8}");
        let arg = |v: &[f32]| v.iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0;
        let scale = q8.iter().fold(0f32, |m, v| m.max(v.abs()));
        let worst = q4.iter().zip(&q8).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst <= 0.1 * scale, "{worst} vs {scale}");
        assert_eq!(arg(&q4), arg(&q8));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
