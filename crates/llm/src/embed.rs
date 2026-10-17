//! Sentence embeddings with a small BERT encoder on the CPU (AI-PLAN AI-Q45): text → WordPiece
//! tokens (the model's own `tokenizer.json`) → the encoder's published architecture (absolute
//! position and token-type embeddings, post-norm blocks with full self-attention and an erf GELU
//! feed-forward) → the last layer's `[CLS]` vector, L2-normalised. Written for
//! BAAI/bge-small-en-v1.5 (12 × 384, 12 heads) but reads any BERT of that shape from
//! `config.json` + `model.safetensors`; checked against candle's `BertModel` in `tests/embed.rs`.
//!
//! Every number in the three files is hostile: the config is capped before anything is
//! allocated, every tensor's shape is checked, and token ids are checked against the vocabulary.
//! Plain f32; the matmuls reuse the vision encoder's row-tiled, SIMD-dispatched linear layer.

use std::path::Path;

use rayon::prelude::*;
use tokenizers::{Tokenizer, TruncationParams};

use crate::LlmError;
use crate::safetensors::{self, Header};
use crate::vision::Linear;

/// Most layers accepted.
pub const MAX_LAYERS: u64 = 48;
/// Widest hidden size accepted.
pub const MAX_HIDDEN: u64 = 4096;
/// Widest feed-forward accepted.
pub const MAX_INTERMEDIATE: u64 = 16_384;
/// Largest vocabulary accepted.
pub const MAX_VOCAB: u64 = 1_000_000;
/// Most positions accepted.
pub const MAX_POSITIONS: u64 = 8192;
/// Most token types accepted.
pub const MAX_TOKEN_TYPES: u64 = 16;
/// Most weights (f32 elements) a model may have in all: 2.4 GB.
pub const MAX_PARAMS: u64 = 600_000_000;
const MAX_CONFIG_BYTES: u64 = 1 << 20;
const MAX_TOKENIZER_BYTES: u64 = 64 << 20;
const MAX_WEIGHTS_BYTES: u64 = 4 << 30;
/// Bytes of text kept per position before tokenizing (the rest would be truncated anyway).
const TEXT_BYTES_PER_POSITION: usize = 64;

fn bad(file: &str, reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: file.to_string(), reason: reason.into() }
}

#[derive(serde::Deserialize)]
struct RawConfig {
    vocab_size: u64,
    hidden_size: u64,
    num_hidden_layers: u64,
    num_attention_heads: u64,
    intermediate_size: u64,
    #[serde(default = "default_act")]
    hidden_act: String,
    max_position_embeddings: u64,
    #[serde(default = "default_types")]
    type_vocab_size: u64,
    #[serde(default = "default_eps")]
    layer_norm_eps: f64,
    #[serde(default)]
    position_embedding_type: Option<String>,
}

fn default_act() -> String {
    "gelu".into()
}
fn default_types() -> u64 {
    2
}
fn default_eps() -> f64 {
    1e-12
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    GeluErf,
    GeluTanh,
    Relu,
}

/// The checked model shape.
#[derive(Clone, Copy, Debug)]
struct Dims {
    vocab: usize,
    hidden: usize,
    layers: usize,
    heads: usize,
    inter: usize,
    positions: usize,
    types: usize,
    eps: f32,
    act: Act,
}

fn in_range(file: &str, what: &str, v: u64, lo: u64, hi: u64) -> Result<usize, LlmError> {
    if v < lo || v > hi {
        return Err(bad(file, format!("{what} = {v} is outside {lo}..={hi}")));
    }
    usize::try_from(v).map_err(|_| bad(file, format!("{what} = {v} is too large")))
}

fn parse_config(json: &[u8]) -> Result<Dims, LlmError> {
    const F: &str = "config.json";
    let c: RawConfig = serde_json::from_slice(json).map_err(|e| bad(F, e.to_string()))?;
    let hidden = in_range(F, "hidden_size", c.hidden_size, 1, MAX_HIDDEN)?;
    let heads = in_range(F, "num_attention_heads", c.num_attention_heads, 1, MAX_HIDDEN)?;
    if hidden % heads != 0 {
        return Err(bad(F, format!("{heads} heads don't divide hidden_size {hidden}")));
    }
    let act = match c.hidden_act.as_str() {
        "gelu" => Act::GeluErf,
        "gelu_new" | "gelu_pytorch_tanh" | "gelu_approximate" => Act::GeluTanh,
        "relu" => Act::Relu,
        other => return Err(bad(F, format!("hidden_act `{other}` is not supported"))),
    };
    if let Some(p) = &c.position_embedding_type
        && p != "absolute"
    {
        return Err(bad(F, format!("position_embedding_type `{p}` is not supported")));
    }
    if !(c.layer_norm_eps.is_finite() && c.layer_norm_eps > 0.0 && c.layer_norm_eps < 1.0) {
        return Err(bad(F, format!("layer_norm_eps {} is out of range", c.layer_norm_eps)));
    }
    let d = Dims {
        vocab: in_range(F, "vocab_size", c.vocab_size, 1, MAX_VOCAB)?,
        hidden,
        layers: in_range(F, "num_hidden_layers", c.num_hidden_layers, 1, MAX_LAYERS)?,
        heads,
        inter: in_range(F, "intermediate_size", c.intermediate_size, 1, MAX_INTERMEDIATE)?,
        positions: in_range(F, "max_position_embeddings", c.max_position_embeddings, 4, MAX_POSITIONS)?,
        types: in_range(F, "type_vocab_size", c.type_vocab_size, 1, MAX_TOKEN_TYPES)?,
        eps: c.layer_norm_eps as f32,
        act,
    };
    let params = d.params().ok_or_else(|| bad(F, "the model size overflows"))?;
    if params > MAX_PARAMS {
        return Err(bad(F, format!("the model has {params} weights, more than the {MAX_PARAMS} allowed")));
    }
    Ok(d)
}

impl Dims {
    /// Weights the config implies (embeddings + layers), with checked math.
    fn params(&self) -> Option<u64> {
        let (h, i) = (self.hidden as u64, self.inter as u64);
        let rows = (self.vocab as u64).checked_add(self.positions as u64)?.checked_add(self.types as u64)?;
        let emb = rows.checked_mul(h)?.checked_add(h.checked_mul(2)?)?;
        // q, k, v, out: 4 (h × h + h); up and down: 2 h i + i + h; two norms: 4 h
        let layer =
            h.checked_mul(h)?.checked_add(h)?.checked_mul(4)?.checked_add(h.checked_mul(i)?.checked_mul(2)?)?.checked_add(i)?.checked_add(h.checked_mul(5)?)?;
        emb.checked_add(layer.checked_mul(self.layers as u64)?)
    }
}

struct LayerNorm {
    w: Vec<f32>,
    b: Vec<f32>,
}

impl LayerNorm {
    /// Layer norm of each `w.len()`-wide row, in place.
    fn apply(&self, x: &mut [f32], eps: f32) {
        let dim = self.w.len();
        if dim == 0 {
            return;
        }
        for r in x.chunks_exact_mut(dim) {
            let mean = r.iter().sum::<f32>() / dim as f32;
            let var = r.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / dim as f32;
            let inv = 1.0 / (var + eps).sqrt();
            for ((v, w), b) in r.iter_mut().zip(&self.w).zip(&self.b) {
                *v = (*v - mean) * inv * w + b;
            }
        }
    }
}

struct Layer {
    /// Query, key and value fused: `3 hidden × hidden`.
    qkv: Linear,
    attn_out: Linear,
    norm1: LayerNorm,
    up: Linear,
    down: Linear,
    norm2: LayerNorm,
}

/// Reads and shape-checks tensors from one safetensors file.
struct Loader<'a> {
    path: &'a Path,
    header: Header,
}

impl Loader<'_> {
    /// The first of `names` present (each also tried with a `bert.` prefix), checked to be `shape`.
    fn get(&self, names: &[&str], shape: &[usize]) -> Result<Vec<f32>, LlmError> {
        const F: &str = "model.safetensors";
        let found = names.iter().flat_map(|n| [n.to_string(), format!("bert.{n}")]).find_map(|n| self.header.tensors.get(&n).map(|t| (n, t)));
        let Some((name, info)) = found else {
            return Err(bad(F, format!("missing tensor {}", names.first().copied().unwrap_or(""))));
        };
        let want: Vec<u64> = shape.iter().map(|&d| d as u64).collect();
        if info.shape != want {
            return Err(bad(F, format!("{name} has shape {:?}, expected {want:?}", info.shape)));
        }
        let elements = want.iter().try_fold(1u64, |n, d| n.checked_mul(*d)).ok_or_else(|| bad(F, "tensor size overflows"))?;
        let v = safetensors::read_f32(self.path, info, elements)?;
        if v.len() as u64 != elements {
            return Err(bad(F, format!("{name} read back {} values, expected {elements}", v.len())));
        }
        Ok(v)
    }

    fn linear(&self, name: &str, inp: usize, out: usize) -> Result<Linear, LlmError> {
        Ok(Linear { w: self.get(&[&format!("{name}.weight")], &[out, inp])?, b: self.get(&[&format!("{name}.bias")], &[out])?, inp, out })
    }

    fn norm(&self, name: &str, dim: usize) -> Result<LayerNorm, LlmError> {
        Ok(LayerNorm {
            w: self.get(&[&format!("{name}.weight"), &format!("{name}.gamma")], &[dim])?,
            b: self.get(&[&format!("{name}.bias"), &format!("{name}.beta")], &[dim])?,
        })
    }

    fn layer(&self, i: usize, d: &Dims) -> Result<Layer, LlmError> {
        let p = format!("encoder.layer.{i}");
        let (h, n) = (d.hidden, d.inter);
        let (q, k, v) = (
            self.linear(&format!("{p}.attention.self.query"), h, h)?,
            self.linear(&format!("{p}.attention.self.key"), h, h)?,
            self.linear(&format!("{p}.attention.self.value"), h, h)?,
        );
        let qkv = Linear { w: [q.w, k.w, v.w].concat(), b: [q.b, k.b, v.b].concat(), inp: h, out: 3 * h };
        Ok(Layer {
            qkv,
            attn_out: self.linear(&format!("{p}.attention.output.dense"), h, h)?,
            norm1: self.norm(&format!("{p}.attention.output.LayerNorm"), h)?,
            up: self.linear(&format!("{p}.intermediate.dense"), h, n)?,
            down: self.linear(&format!("{p}.output.dense"), n, h)?,
            norm2: self.norm(&format!("{p}.output.LayerNorm"), h)?,
        })
    }
}

fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, LlmError> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let len = std::fs::metadata(path).map_err(|_| LlmError::NotInstalled(path.display().to_string()))?.len();
    if len > max {
        return Err(LlmError::TooLarge { file: name, size: len, max });
    }
    Ok(std::fs::read(path)?)
}

/// A loaded sentence embedder.
pub struct Embedder {
    d: Dims,
    tok: Tokenizer,
    /// `vocab × hidden`.
    word: Vec<f32>,
    /// `positions × hidden`.
    pos: Vec<f32>,
    /// Token type 0's row.
    type0: Vec<f32>,
    emb_norm: LayerNorm,
    layers: Vec<Layer>,
}

impl Embedder {
    /// Loads `config.json`, `model.safetensors` and `tokenizer.json` from `dir`.
    pub fn load(dir: &Path) -> Result<Embedder, LlmError> {
        let d = parse_config(&read_capped(&dir.join("config.json"), MAX_CONFIG_BYTES)?)?;

        let tok_bytes = read_capped(&dir.join("tokenizer.json"), MAX_TOKENIZER_BYTES)?;
        let mut tok = Tokenizer::from_bytes(&tok_bytes).map_err(|e| bad("tokenizer.json", e.to_string()))?;
        tok.with_truncation(Some(TruncationParams { max_length: d.positions, ..Default::default() })).map_err(|e| bad("tokenizer.json", e.to_string()))?;
        tok.with_padding(None);

        let path = dir.join("model.safetensors");
        let size = std::fs::metadata(&path).map_err(|_| LlmError::NotInstalled(path.display().to_string()))?.len();
        if size > MAX_WEIGHTS_BYTES {
            return Err(LlmError::TooLarge { file: "model.safetensors".into(), size, max: MAX_WEIGHTS_BYTES });
        }
        let ld = Loader { path: &path, header: safetensors::read_header(&path)? };
        let h = d.hidden;
        let word = ld.get(&["embeddings.word_embeddings.weight"], &[d.vocab, h])?;
        let pos = ld.get(&["embeddings.position_embeddings.weight"], &[d.positions, h])?;
        let types = ld.get(&["embeddings.token_type_embeddings.weight"], &[d.types, h])?;
        let type0 = types.get(..h).ok_or_else(|| bad("model.safetensors", "token type table is empty"))?.to_vec();
        let emb_norm = ld.norm("embeddings.LayerNorm", h)?;
        let layers = (0..d.layers).map(|i| ld.layer(i, &d)).collect::<Result<Vec<_>, _>>()?;
        Ok(Embedder { d, tok, word, pos, type0, emb_norm, layers })
    }

    /// Width of the vectors [`Embedder::embed`] returns.
    pub fn dim(&self) -> usize {
        self.d.hidden
    }

    /// Most tokens one text uses (including `[CLS]` and `[SEP]`); longer texts are truncated.
    pub fn max_tokens(&self) -> usize {
        self.d.positions
    }

    /// The token ids `text` is embedded from: the tokenizer's special tokens added, truncated
    /// to [`Embedder::max_tokens`].
    pub fn tokens(&self, text: &str) -> Result<Vec<u32>, LlmError> {
        let text = clip(text, self.d.positions.saturating_mul(TEXT_BYTES_PER_POSITION));
        let enc = self.tok.encode(text, true).map_err(|e| LlmError::Inference(format!("tokenizing: {e}")))?;
        let mut ids = enc.get_ids().to_vec();
        ids.truncate(self.d.positions);
        Ok(ids)
    }

    /// The L2-normalised `[CLS]` vector of the last layer for `text` (an empty text still has
    /// its special tokens).
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let mut v = self.embed_raw(text)?;
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm.is_finite() && norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        Ok(v)
    }

    /// [`Embedder::embed`] before normalising: the last layer's `[CLS]` hidden state.
    pub fn embed_raw(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ids = self.tokens(text)?;
            self.forward(&ids)
        }))
        .map_err(|p| LlmError::Panicked(crate::panic_message(&*p)))?
    }

    /// [`Embedder::embed`] for each of `texts`, in parallel.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        texts.par_iter().map(|t| self.embed(t)).collect()
    }

    /// The encoder on `ids` → the first token's last hidden state.
    pub fn forward(&self, ids: &[u32]) -> Result<Vec<f32>, LlmError> {
        let (h, n) = (self.d.hidden, ids.len());
        if n == 0 {
            return Err(LlmError::Inference("no tokens".into()));
        }
        if n > self.d.positions {
            return Err(LlmError::PromptTooLong { tokens: n, max: self.d.positions });
        }
        let mut x = Vec::with_capacity(n * h);
        for (t, &id) in ids.iter().enumerate() {
            let i = id as usize;
            // checked: a hostile id times the width can overflow a 32-bit usize (wasm32)
            let span = |k: usize| k.checked_mul(h).and_then(|a| Some(a..a.checked_add(h)?));
            let (Some(w), Some(p)) = (span(i).and_then(|r| self.word.get(r)), span(t).and_then(|r| self.pos.get(r))) else {
                return Err(LlmError::Inference(format!("token id {id} is outside the vocabulary of {}", self.d.vocab)));
            };
            x.extend(w.iter().zip(p).zip(&self.type0).map(|((a, b), c)| a + b + c));
        }
        self.emb_norm.apply(&mut x, self.d.eps);
        let last = self.layers.len().saturating_sub(1);
        let mut rows = n;
        for (li, layer) in self.layers.iter().enumerate() {
            // only [CLS] is read out, so the last layer's other rows are skipped after attention
            let keep = if li == last { 1 } else { rows };
            x = self.layer(layer, &x, rows, keep);
            rows = keep;
        }
        x.truncate(h);
        Ok(x)
    }

    /// One post-norm block on `rows × hidden`, returning its first `keep` rows.
    fn layer(&self, l: &Layer, x: &[f32], rows: usize, keep: usize) -> Vec<f32> {
        let h = self.d.hidden;
        let qkv = l.qkv.forward(x, rows);
        let ctx = attention(&qkv, rows, keep, h, self.d.heads);
        let mut a = l.attn_out.forward(&ctx, keep);
        for (o, r) in a.iter_mut().zip(x) {
            *o += r;
        }
        l.norm1.apply(&mut a, self.d.eps);
        let mut up = l.up.forward(&a, keep);
        let act = self.d.act;
        up.par_chunks_mut(4096).for_each(|c| c.iter_mut().for_each(|v| *v = activate(*v, act)));
        let mut y = l.down.forward(&up, keep);
        for (o, r) in y.iter_mut().zip(&a) {
            *o += r;
        }
        l.norm2.apply(&mut y, self.d.eps);
        y
    }
}

/// Full (unmasked) multi-head self-attention: `qkv` is `n × 3h` (queries, keys, values); the
/// context of the first `keep` queries, `keep × h`.
fn attention(qkv: &[f32], n: usize, keep: usize, h: usize, heads: usize) -> Vec<f32> {
    let hd = h / heads.max(1);
    let scale = 1.0 / (hd.max(1) as f32).sqrt();
    let stride = 3 * h;
    let mut out = vec![0.0f32; keep * h];
    out.par_chunks_mut(h).enumerate().for_each(|(i, o)| {
        let mut scores = vec![0.0f32; n];
        for hi in 0..heads {
            let Some(q) = qkv.get(i * stride + hi * hd..i * stride + (hi + 1) * hd) else { continue };
            for (j, s) in scores.iter_mut().enumerate() {
                let k0 = j * stride + h + hi * hd;
                *s = qkv.get(k0..k0 + hd).map_or(f32::NEG_INFINITY, |k| q.iter().zip(k).map(|(a, b)| a * b).sum::<f32>() * scale);
            }
            let max = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let mut sum = 0.0f32;
            for s in scores.iter_mut() {
                *s = (*s - max).exp();
                sum += *s;
            }
            let inv = if sum > 0.0 { 1.0 / sum } else { 0.0 };
            let Some(oh) = o.get_mut(hi * hd..(hi + 1) * hd) else { continue };
            for (j, s) in scores.iter().enumerate() {
                let v0 = j * stride + 2 * h + hi * hd;
                if let Some(v) = qkv.get(v0..v0 + hd) {
                    let p = s * inv;
                    for (a, b) in oh.iter_mut().zip(v) {
                        *a += p * b;
                    }
                }
            }
        }
    });
    out
}

fn activate(x: f32, act: Act) -> f32 {
    match act {
        Act::GeluErf => {
            let x = f64::from(x);
            (0.5 * x * (1.0 + erf(x * std::f64::consts::FRAC_1_SQRT_2))) as f32
        }
        Act::GeluTanh => 0.5 * x * (1.0 + ((2.0 / std::f32::consts::PI).sqrt() * (x + 0.044_715 * x * x * x)).tanh()),
        Act::Relu => x.max(0.0),
    }
}

/// The error function (Numerical Recipes' Chebyshev fit of erfc, |error| < 1.2e-7).
fn erf(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06 + t * (0.278_868_07 + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let erfc = t * poly.exp();
    if x >= 0.0 { 1.0 - erfc } else { erfc - 1.0 }
}

/// `s` cut to at most `max` bytes, at a char boundary.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erf_matches_known_values() {
        for (x, want) in [(0.0, 0.0), (0.5, 0.520_499_877_8), (1.0, 0.842_700_792_9), (-2.0, -0.995_322_265_0), (4.0, 0.999_999_984_6)] {
            assert!((erf(x) - want).abs() < 2e-7, "erf({x}) = {}", erf(x));
        }
    }

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("héllo", 2), "h");
        assert_eq!(clip("abc", 10), "abc");
        assert_eq!(clip("", 0), "");
    }

    #[test]
    fn hostile_configs_are_errors() {
        let ok = r#"{"vocab_size":30,"hidden_size":8,"num_hidden_layers":1,"num_attention_heads":2,"intermediate_size":16,"max_position_embeddings":16}"#;
        assert!(parse_config(ok.as_bytes()).is_ok());
        for (from, to) in [
            ("\"hidden_size\":8", "\"hidden_size\":7"),
            ("\"hidden_size\":8", "\"hidden_size\":0"),
            ("\"hidden_size\":8", "\"hidden_size\":-8"),
            ("\"num_hidden_layers\":1", "\"num_hidden_layers\":100000"),
            ("\"num_attention_heads\":2", "\"num_attention_heads\":0"),
            ("\"vocab_size\":30", "\"vocab_size\":18446744073709551615"),
            ("\"max_position_embeddings\":16", "\"max_position_embeddings\":1000000000"),
            ("\"intermediate_size\":16", "\"intermediate_size\":1e9"),
            ("{", "{\"hidden_act\":\"swish\","),
            ("{", "{\"layer_norm_eps\":-1,"),
            ("{", "{\"position_embedding_type\":\"relative_key\","),
        ] {
            let s = ok.replace(from, to);
            assert!(parse_config(s.as_bytes()).is_err(), "{s}");
        }
        // every number at its cap: too many weights in all
        let big = r#"{"vocab_size":1000000,"hidden_size":4096,"num_hidden_layers":48,"num_attention_heads":32,"intermediate_size":16384,"max_position_embeddings":8192}"#;
        assert!(parse_config(big.as_bytes()).is_err());
        assert!(parse_config(b"not json").is_err());
    }
}
