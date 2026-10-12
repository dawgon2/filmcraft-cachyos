//! Convert a full-precision Hugging Face checkpoint (Llama / SmolLM2 or Qwen2 / Qwen2.5:
//! `model.safetensors` + `config.json`) to a GGUF file that candle's quantized `llama` / `qwen2`
//! loaders (and so [`filmcraft_llm::Model`]) read. Pure Rust (candle); GRADE-PLAN G2.11b.
//!
//! ```text
//! cargo run --release -p filmcraft-llm --features infer --example llm_quantize -- \
//!     <model dir> <out.gguf> [--type q8_0|q4k|q5k|q6k|q4_0|q5_0|f16|f32] [--name NAME]
//! cargo run --release -p filmcraft-llm --features infer --example llm_quantize -- --inspect <file.gguf>
//! cargo run --release -p filmcraft-llm --features infer --example llm_quantize -- --compare <a.gguf> <b.gguf>
//! ```
//!
//! Names follow the GGUF conventions (the GGUF specification published with ggml):
//! `model.embed_tokens` → `token_embd`, `model.norm` → `output_norm`, `lm_head` → `output`,
//! `model.layers.N.input_layernorm` → `blk.N.attn_norm`, `post_attention_layernorm` → `ffn_norm`,
//! `self_attn.{q,k,v,o}_proj` → `attn_{q,k,v,output}`, `mlp.{gate,up,down}_proj` → `ffn_{gate,up,down}`.
//! Metadata: `general.architecture` (`llama` / `qwen2`) and the `<arch>.*` hyper-parameters
//! candle reads (head counts, embedding length, block count, RMS epsilon, rope base, rope
//! dimension, context length).
//!
//! Llama only: candle's quantized Llama applies rotary embeddings to interleaved pairs
//! (`rope_i`), while Hugging Face Llama checkpoints rotate halves, so the rows of the q and k
//! projections are permuted per head (half i ↔ pair i), the standard GGUF Llama layout. Qwen2
//! rotates halves in both, so it is copied as is. Llama checkpoints with projection biases
//! (`attention_bias` / `mlp_bias`) are rejected: candle's quantized Llama has no bias terms (it
//! would drop them silently), and q/k biases would need the same per-head permutation.
//!
//! Types: 1-D tensors (norms, biases) stay F32. `q8_0` quantizes every matrix to Q8_0 (as the
//! published SmolLM2 Q8_0 file). `q4k` follows llama.cpp's Q4_K_M mix as the published Qwen2.5
//! Q4_K_M file shows it: Q4_K for the matrices, Q6_K for `attn_v` / `ffn_down` in the "more bits"
//! blocks (first and last eighth, every third in between) and for the output head (a separate
//! copy of tied embeddings for Qwen2); a k-quant needs rows that are a multiple of 256, so a
//! matrix whose rows aren't (Qwen2.5-0.5B's 896) falls back from Q4_K to Q5_0 and from Q6_K to Q8_0.

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use candle_core::quantized::{GgmlDType, QTensor, gguf_file};
use candle_core::{DType, Device, Tensor};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mix {
    Q8,
    Q4KM,
    Uniform(GgmlDType),
}

fn parse_type(s: &str) -> Result<Mix, Box<dyn Error>> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "q8_0" | "q8" => Mix::Q8,
        "q4k" | "q4_k_m" | "q4km" => Mix::Q4KM,
        "q4_k" => Mix::Uniform(GgmlDType::Q4K),
        "q5k" | "q5_k" => Mix::Uniform(GgmlDType::Q5K),
        "q6k" | "q6_k" => Mix::Uniform(GgmlDType::Q6K),
        "q4_0" => Mix::Uniform(GgmlDType::Q4_0),
        "q5_0" => Mix::Uniform(GgmlDType::Q5_0),
        "f16" => Mix::Uniform(GgmlDType::F16),
        "f32" => Mix::Uniform(GgmlDType::F32),
        other => return Err(format!("unknown type {other}").into()),
    })
}

struct Hyper {
    arch: &'static str,
    heads: usize,
    kv_heads: usize,
    hidden: usize,
    inter: usize,
    layers: usize,
    eps: f64,
    theta: f64,
    context: usize,
    vocab: usize,
}

fn read_config(dir: &std::path::Path) -> Result<Hyper, Box<dyn Error>> {
    let text = std::fs::read_to_string(dir.join("config.json"))?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let u = |k: &str| -> Result<usize, Box<dyn Error>> {
        v.get(k)
            .and_then(serde_json::Value::as_u64)
            .and_then(|x| usize::try_from(x).ok())
            .filter(|x| (1..=1 << 24).contains(x))
            .ok_or_else(|| format!("config.json: bad or missing {k}").into())
    };
    let f = |k: &str, d: f64| v.get(k).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite() && *x > 0.0).unwrap_or(d);
    let arch = match v.get("model_type").and_then(|m| m.as_str()) {
        Some("llama") => "llama",
        Some("qwen2") => "qwen2",
        other => return Err(format!("unsupported model_type {other:?} (llama or qwen2)").into()),
    };
    // (candle's quantized Llama has no projection biases: refuse rather than drop them)
    let flag = |k: &str| v.get(k).and_then(serde_json::Value::as_bool).unwrap_or(false);
    if arch == "llama" && (flag("attention_bias") || flag("mlp_bias")) {
        return Err("config.json: Llama with attention_bias / mlp_bias is not supported (candle's quantized Llama has no projection biases)".into());
    }
    if v.get("rope_scaling").is_some_and(|r| !r.is_null()) {
        return Err("rope_scaling is not supported".into());
    }
    let heads = u("num_attention_heads")?;
    let hidden = u("hidden_size")?;
    let kv_heads = v.get("num_key_value_heads").map_or(Ok(heads), |_| u("num_key_value_heads"))?;
    if hidden % heads != 0 || heads % kv_heads != 0 || (hidden / heads) % 2 != 0 {
        return Err("config.json: bad head layout".into());
    }
    Ok(Hyper {
        arch,
        heads,
        kv_heads,
        hidden,
        inter: u("intermediate_size")?,
        layers: u("num_hidden_layers")?,
        eps: f("rms_norm_eps", 1e-6),
        theta: f("rope_theta", 10_000.0),
        context: u("max_position_embeddings")?.min(131_072),
        vocab: u("vocab_size")?,
    })
}

/// The GGUF name of a Hugging Face tensor name, or None for tensors the format doesn't keep.
fn gguf_name(hf: &str) -> Option<String> {
    match hf {
        "model.embed_tokens.weight" => return Some("token_embd.weight".into()),
        "model.norm.weight" => return Some("output_norm.weight".into()),
        "lm_head.weight" => return Some("output.weight".into()),
        _ => {}
    }
    let rest = hf.strip_prefix("model.layers.")?;
    let (n, tail) = rest.split_once('.')?;
    let n: usize = n.parse().ok()?;
    let (module, kind) = tail.rsplit_once('.')?;
    let short = match module {
        "input_layernorm" => "attn_norm",
        "post_attention_layernorm" => "ffn_norm",
        "self_attn.q_proj" => "attn_q",
        "self_attn.k_proj" => "attn_k",
        "self_attn.v_proj" => "attn_v",
        "self_attn.o_proj" => "attn_output",
        "mlp.gate_proj" => "ffn_gate",
        "mlp.up_proj" => "ffn_up",
        "mlp.down_proj" => "ffn_down",
        _ => return None,
    };
    if kind != "weight" && kind != "bias" {
        return None;
    }
    Some(format!("blk.{n}.{short}.{kind}"))
}

/// The q / k projection rows reordered from "rotate halves" to "rotate pairs", per head:
/// (heads, 2, head_dim / 2, in) → (heads, head_dim / 2, 2, in).
fn permute_rope(t: &Tensor, heads: usize) -> candle_core::Result<Tensor> {
    let dims = t.dims().to_vec();
    let rows = dims.first().copied().unwrap_or(0);
    if heads == 0 || rows % (heads * 2) != 0 {
        candle_core::bail!("cannot permute {dims:?} for {heads} heads");
    }
    let half = rows / heads / 2;
    let rest: Vec<usize> = dims.get(1..).map(<[usize]>::to_vec).unwrap_or_default();
    let mut shape = vec![heads, 2, half];
    shape.extend(&rest);
    t.reshape(shape)?.transpose(1, 2)?.contiguous()?.reshape(dims)
}

/// llama.cpp's "more bits" layers of its Q4_K_M mix: the first and last eighth of the blocks and
/// every third block in between.
fn more_bits(block: usize, blocks: usize) -> bool {
    let e = blocks / 8;
    block < e || block >= blocks.saturating_sub(e) || (block >= e && (block - e) % 3 == 2)
}

/// The type of a tensor whose preferred type doesn't fit its rows: k-quants need rows that are a
/// multiple of 256, so Q4_K falls back to Q5_0 and Q6_K to Q8_0 (as llama.cpp does).
fn fallback(d: GgmlDType) -> GgmlDType {
    match d {
        GgmlDType::Q4K | GgmlDType::Q5K | GgmlDType::Q2K | GgmlDType::Q3K => GgmlDType::Q5_0,
        GgmlDType::Q6K | GgmlDType::Q8K => GgmlDType::Q8_0,
        other => other,
    }
}

fn dtype_for(mix: Mix, gguf: &str, t: &Tensor, blocks: usize) -> GgmlDType {
    if t.rank() < 2 {
        return GgmlDType::F32;
    }
    let row = t.dims().last().copied().unwrap_or(0);
    let fits = |d: GgmlDType| row % d.block_size() == 0;
    let want = match mix {
        Mix::Q8 => GgmlDType::Q8_0,
        Mix::Uniform(d) => d,
        Mix::Q4KM => {
            let block = gguf.strip_prefix("blk.").and_then(|r| r.split_once('.')).and_then(|(n, _)| n.parse::<usize>().ok());
            let more = (gguf.ends_with("attn_v.weight") || gguf.ends_with("ffn_down.weight")) && block.is_some_and(|b| more_bits(b, blocks));
            if gguf == "output.weight" || more { GgmlDType::Q6K } else { GgmlDType::Q4K }
        }
    };
    for d in [want, fallback(want), GgmlDType::Q8_0] {
        if fits(d) {
            return d;
        }
    }
    GgmlDType::F16
}

fn inspect(path: &str) -> Result<(), Box<dyn Error>> {
    let mut f = std::fs::File::open(path)?;
    let c = gguf_file::Content::read(&mut f)?;
    let mut keys: Vec<_> = c.metadata.keys().collect();
    keys.sort();
    for k in keys {
        if k.starts_with("tokenizer.ggml.") && !k.ends_with(".model") && !k.ends_with("_id") {
            println!("{k} = …");
            continue;
        }
        println!("{k} = {:?}", c.metadata.get(k));
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut names: Vec<_> = c.tensor_infos.iter().collect();
    names.sort_by(|a, b| a.0.cmp(b.0));
    for (n, i) in names {
        if !n.starts_with("blk.") || n.starts_with("blk.0.") || n.starts_with("blk.1.") {
            println!("{n}: {:?} {:?}", i.ggml_dtype, i.shape);
        }
        *counts.entry(format!("{:?}", i.ggml_dtype)).or_default() += 1;
    }
    println!("tensor types: {counts:?}");
    Ok(())
}

/// Compare two GGUF files tensor by tensor: type, shape, identical bytes, and for differing data
/// the relative RMS error of the dequantized values (b against a).
fn compare(a: &str, b: &str) -> Result<(), Box<dyn Error>> {
    let mut fa = std::fs::File::open(a)?;
    let mut fb = std::fs::File::open(b)?;
    let ca = gguf_file::Content::read(&mut fa)?;
    let cb = gguf_file::Content::read(&mut fb)?;
    let mut names: Vec<&String> = ca.tensor_infos.keys().collect();
    names.sort();
    let (mut same_bytes, mut same_type, mut total, mut worst) = (0usize, 0usize, 0usize, (0.0f64, String::new()));
    let mut sum_err = 0.0f64;
    for n in names {
        let (Some(ia), Some(ib)) = (ca.tensor_infos.get(n), cb.tensor_infos.get(n)) else {
            println!("{n}: only in {a}");
            continue;
        };
        total += 1;
        if ia.shape.dims() != ib.shape.dims() {
            println!("{n}: shape {:?} vs {:?}", ia.shape, ib.shape);
            continue;
        }
        let ta = ca.tensor(&mut fa, n, &Device::Cpu)?;
        let tb = cb.tensor(&mut fb, n, &Device::Cpu)?;
        if ia.ggml_dtype == ib.ggml_dtype {
            same_type += 1;
            if ta.data()? == tb.data()? {
                same_bytes += 1;
                continue;
            }
        }
        let da = ta.dequantize(&Device::Cpu)?.flatten_all()?;
        let db = tb.dequantize(&Device::Cpu)?.flatten_all()?;
        let num = (&db - &da)?.sqr()?.mean_all()?.to_scalar::<f32>()? as f64;
        let den = da.sqr()?.mean_all()?.to_scalar::<f32>()? as f64;
        let rel = if den > 0.0 { (num / den).sqrt() } else { num.sqrt() };
        sum_err += rel;
        if rel > worst.0 {
            worst = (rel, n.clone());
        }
        if n.starts_with("blk.0.") || !n.starts_with("blk.") {
            println!("{n}: {:?} vs {:?}, relative RMS difference {rel:.4}", ia.ggml_dtype, ib.ggml_dtype);
        }
    }
    for n in cb.tensor_infos.keys() {
        if !ca.tensor_infos.contains_key(n) {
            println!("{n}: only in {b}");
        }
    }
    let differing = total - same_bytes;
    println!(
        "{total} tensors: {same_type} of the same type, {same_bytes} byte-identical; {differing} differ, mean relative RMS difference {:.4}, worst {:.4} ({})",
        if differing > 0 { sum_err / differing as f64 } else { 0.0 },
        worst.0,
        worst.1
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut positional = Vec::new();
    let mut mix = Mix::Q8;
    let mut name: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--inspect" => return inspect(&args.next().ok_or("--inspect needs a file")?),
            "--compare" => {
                let a = args.next().ok_or("--compare needs two files")?;
                let b = args.next().ok_or("--compare needs two files")?;
                return compare(&a, &b);
            }
            "--type" => mix = parse_type(&args.next().ok_or("--type needs a value")?)?,
            "--name" => name = Some(args.next().ok_or("--name needs a value")?),
            _ => positional.push(a),
        }
    }
    let [dir, out] = positional.as_slice() else {
        return Err("usage: llm_quantize <model dir> <out.gguf> [--type q8_0|q4k|...] [--name NAME] | --inspect <file.gguf>".into());
    };
    let dir = PathBuf::from(dir);
    let h = read_config(&dir)?;
    let t0 = std::time::Instant::now();
    let tensors = candle_core::safetensors::load(dir.join("model.safetensors"), &Device::Cpu)?;
    let mut names: Vec<&String> = tensors.keys().collect();
    names.sort();
    let mut q: Vec<(String, QTensor)> = Vec::with_capacity(names.len());
    let mut counts: HashMap<String, usize> = HashMap::new();
    for hf in names {
        let Some(t) = tensors.get(hf) else { continue };
        let Some(g) = gguf_name(hf) else {
            eprintln!("skipping {hf} (not part of the GGUF layout)");
            continue;
        };
        if h.arch == "llama" && g.ends_with(".bias") {
            return Err(format!("{hf}: Llama projection biases are not supported (candle's quantized Llama has none)").into());
        }
        let mut t = t.to_dtype(DType::F32)?;
        if h.arch == "llama" {
            if g.ends_with("attn_q.weight") {
                t = permute_rope(&t, h.heads)?;
            } else if g.ends_with("attn_k.weight") {
                t = permute_rope(&t, h.kv_heads)?;
            }
        }
        let dt = dtype_for(mix, &g, &t, h.layers);
        *counts.entry(format!("{dt:?}")).or_default() += 1;
        q.push((g, QTensor::quantize(&t, dt)?));
    }
    if !q.iter().any(|(n, _)| n == "token_embd.weight") {
        return Err("no token embeddings (model.embed_tokens.weight)".into());
    }
    // Qwen2 with tied embeddings: the published Q4_K_M files carry a separate, higher-precision
    // output head (a copy of the embeddings); candle uses it when present.
    if mix == Mix::Q4KM
        && h.arch == "qwen2"
        && !q.iter().any(|(n, _)| n == "output.weight")
        && let Some(e) = tensors.get("model.embed_tokens.weight")
    {
        let e = e.to_dtype(DType::F32)?;
        let dt = dtype_for(mix, "output.weight", &e, h.layers);
        *counts.entry(format!("{dt:?}")).or_default() += 1;
        q.push(("output.weight".into(), QTensor::quantize(&e, dt)?));
    }
    let arch = h.arch;
    let head_dim = h.hidden / h.heads;
    let file_type: u32 = match mix {
        Mix::Q8 => 7,
        Mix::Q4KM => 15,
        Mix::Uniform(GgmlDType::F32) => 0,
        Mix::Uniform(GgmlDType::F16) => 1,
        Mix::Uniform(GgmlDType::Q4_0) => 2,
        Mix::Uniform(GgmlDType::Q5_0) => 8,
        Mix::Uniform(GgmlDType::Q4K) => 14,
        Mix::Uniform(GgmlDType::Q5K) => 17,
        Mix::Uniform(GgmlDType::Q6K) => 18,
        Mix::Uniform(_) => 7,
    };
    use gguf_file::Value as V;
    let u32v = |x: usize| V::U32(u32::try_from(x).unwrap_or(u32::MAX));
    let meta: Vec<(String, V)> = vec![
        ("general.architecture".into(), V::String(arch.into())),
        ("general.name".into(), V::String(name.unwrap_or_else(|| dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))),
        ("general.file_type".into(), V::U32(file_type)),
        ("general.quantized_by".into(), V::String("filmcraft-llm llm_quantize (candle)".into())),
        (format!("{arch}.context_length"), u32v(h.context)),
        (format!("{arch}.embedding_length"), u32v(h.hidden)),
        (format!("{arch}.feed_forward_length"), u32v(h.inter)),
        (format!("{arch}.block_count"), u32v(h.layers)),
        (format!("{arch}.attention.head_count"), u32v(h.heads)),
        (format!("{arch}.attention.head_count_kv"), u32v(h.kv_heads)),
        (format!("{arch}.attention.layer_norm_rms_epsilon"), V::F32(h.eps as f32)),
        (format!("{arch}.rope.freq_base"), V::F32(h.theta as f32)),
        (format!("{arch}.rope.dimension_count"), u32v(head_dim)),
        (format!("{arch}.vocab_size"), u32v(h.vocab)),
    ];
    let meta_refs: Vec<(&str, &V)> = meta.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let tensor_refs: Vec<(&str, &QTensor)> = q.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let tmp = PathBuf::from(format!("{out}.part"));
    {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        gguf_file::write(&mut w, &meta_refs, &tensor_refs)?;
        std::io::Write::flush(&mut w)?;
    }
    std::fs::rename(&tmp, out)?;
    let size = std::fs::metadata(out)?.len();
    println!("wrote {out}: {} tensors {counts:?}, {:.1} MB in {:.1?}", q.len(), size as f64 / 1e6, t0.elapsed());
    // for a catalogue entry (`models.rs`)
    println!("size {size}\nsha256 {}", sha256_file(std::path::Path::new(out))?);
    Ok(())
}

fn sha256_file(path: &std::path::Path) -> Result<String, Box<dyn Error>> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
        h.update(chunk);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
