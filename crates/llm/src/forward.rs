//! Our own transformer forward pass for the catalogue's two families (AI-PLAN AI1.1d), on the
//! kernels in [`crate::quant`] / [`crate::kquant`] through [`Weight`].
//!
//! The model structure follows candle 0.9.2's `quantized_qwen2.rs` / `quantized_llama.rs`
//! (MIT / Apache-2.0, © the candle authors): token embedding → per layer RMSNorm, Q/K/V (Qwen2
//! adds biases), rotary embedding (Qwen2 rotates the two halves of a head, Llama GGUFs adjacent
//! pairs), grouped-query attention over a KV cache, output projection, residual, RMSNorm, SwiGLU
//! MLP, residual; final RMSNorm and the output projection (tied to the embedding when the file has
//! no `output.weight`).

use std::io::{Read, Seek};

use crate::LlmError;
use crate::gguf::GgufSummary;
use crate::quant::err;
use crate::weights::{Scratch, Weight};

/// Which pairs of a head the rotary embedding rotates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rope {
    /// `(i, i + d/2)`: Qwen2.
    Halves,
    /// `(2i, 2i + 1)`: Llama-architecture GGUFs (SmolLM2).
    Interleaved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub embd: usize,
    pub n_head: usize,
    pub n_kv_head: usize,
    pub head_dim: usize,
    pub n_layer: usize,
    pub vocab: usize,
    pub eps: f32,
    pub rope_base: f32,
    pub rope: Rope,
    /// Positions the rope table and KV cache allow.
    pub max_pos: usize,
}

/// Caps that keep a hostile header from sizing huge tables (the catalogue's largest model, Qwen2.5
/// 1.5B, has 28 layers, 12 heads and a 32 k context).
const MAX_LAYERS: usize = 512;
const MAX_POS: usize = 32_768;
const MAX_HEAD_DIM: usize = 512;
/// Prompt tokens per batched pass (bounds the activation buffers: 64 × the MLP width).
const PREFILL_CHUNK: usize = 64;

impl Config {
    /// Reads the hyper-parameters for architecture `arch` ("qwen2" or "llama") with checked ranges.
    pub fn from_gguf(s: &GgufSummary, arch: &str, max_pos: usize) -> Result<Config, LlmError> {
        let int = |k: &str| -> Result<usize, LlmError> {
            let key = format!("{arch}.{k}");
            let v = s.get(&key).ok_or_else(|| err(format!("{key} missing")))?;
            usize::try_from(v).map_err(|_| err(format!("{key} = {v}")))
        };
        let embd = int("embedding_length")?;
        let n_head = int("attention.head_count")?;
        let n_kv_head = int("attention.head_count_kv")?;
        let n_layer = int("block_count")?;
        if embd == 0 || n_head == 0 || n_kv_head == 0 || n_head % n_kv_head != 0 || embd % n_head != 0 || n_layer == 0 || n_layer > MAX_LAYERS {
            return Err(err(format!("bad shape: embd {embd}, heads {n_head}/{n_kv_head}, layers {n_layer}")));
        }
        let head_dim = embd / n_head;
        if head_dim % 2 != 0 || head_dim > MAX_HEAD_DIM {
            return Err(err(format!("head dimension {head_dim}")));
        }
        let rope = match arch {
            "qwen2" => Rope::Halves,
            "llama" => {
                if int("rope.dimension_count")? != head_dim {
                    return Err(err("rope.dimension_count differs from the head dimension"));
                }
                Rope::Interleaved
            }
            a => return Err(err(format!("unsupported architecture {a}"))),
        };
        let float = |k: &str| s.float(&format!("{arch}.{k}"));
        let eps = float("attention.layer_norm_rms_epsilon").ok_or_else(|| err("RMS epsilon missing"))? as f32;
        let rope_base = float("rope.freq_base").map_or(10_000.0, |v| v as f32);
        if !(eps > 0.0 && eps < 1.0) || !(rope_base > 1.0 && rope_base.is_finite()) {
            return Err(err(format!("RMS epsilon {eps} / rope base {rope_base}")));
        }
        let vocab = match s.token_embd.as_deref() {
            Some([e, v]) if usize::try_from(*e).ok() == Some(embd) => usize::try_from(*v).map_err(|_| err("vocabulary too large"))?,
            other => return Err(err(format!("token_embd.weight is {other:?}"))),
        };
        Ok(Config { embd, n_head, n_kv_head, head_dim, n_layer, vocab, eps, rope_base, rope, max_pos: max_pos.clamp(1, MAX_POS) })
    }
}

struct Layer {
    attn_norm: Vec<f32>,
    wq: Weight,
    wk: Weight,
    wv: Weight,
    bias: Option<[Vec<f32>; 3]>,
    wo: Weight,
    ffn_norm: Vec<f32>,
    gate: Weight,
    up: Weight,
    down: Weight,
    /// `pos × n_kv_head × head_dim`, appended per token.
    k_cache: Vec<f32>,
    v_cache: Vec<f32>,
}

pub struct Forward {
    pub cfg: Config,
    embd: Weight,
    layers: Vec<Layer>,
    norm: Vec<f32>,
    /// `None`: tied to `embd`.
    output: Option<Weight>,
    /// `max_pos × head_dim / 2`.
    cos: Vec<f32>,
    sin: Vec<f32>,
    /// Tokens in the KV cache.
    len: usize,
    scratch: Scratch,
}

fn check_dims(w: &Weight, rows: usize, cols: usize) -> Result<(), LlmError> {
    if (w.rows, w.cols) == (rows, cols) { Ok(()) } else { Err(err(format!("{} is {}×{}, expected {rows}×{cols}", w.name, w.rows, w.cols))) }
}

fn vector<R: Read + Seek>(r: &mut R, s: &GgufSummary, name: &str, len: usize) -> Result<Vec<f32>, LlmError> {
    let v = Weight::read(r, s, name)?.vector()?;
    if v.len() == len { Ok(v) } else { Err(err(format!("{name} has {} values, expected {len}", v.len()))) }
}

impl Forward {
    /// Loads every tensor of a checked GGUF (`s` from [`crate::gguf::check`] on `r`).
    pub fn load<R: Read + Seek>(r: &mut R, s: &GgufSummary, cfg: Config) -> Result<Forward, LlmError> {
        let kv_dim = cfg.n_kv_head * cfg.head_dim;
        let embd = Weight::read(r, s, "token_embd.weight")?;
        check_dims(&embd, cfg.vocab, cfg.embd)?;
        let norm = vector(r, s, "output_norm.weight", cfg.embd)?;
        let output = match s.tensor("output.weight") {
            Some(_) => {
                let w = Weight::read(r, s, "output.weight")?;
                check_dims(&w, cfg.vocab, cfg.embd)?;
                Some(w)
            }
            None => None,
        };
        let mut layers = Vec::with_capacity(cfg.n_layer);
        for i in 0..cfg.n_layer {
            let name = |t: &str| format!("blk.{i}.{t}");
            let read = |r: &mut R, t: &str, rows: usize, cols: usize| -> Result<Weight, LlmError> {
                let w = Weight::read(r, s, &name(t))?;
                check_dims(&w, rows, cols)?;
                Ok(w)
            };
            let wq = read(r, "attn_q.weight", cfg.embd, cfg.embd)?;
            let wk = read(r, "attn_k.weight", kv_dim, cfg.embd)?;
            let wv = read(r, "attn_v.weight", kv_dim, cfg.embd)?;
            let wo = read(r, "attn_output.weight", cfg.embd, cfg.embd)?;
            let gate = Weight::read(r, s, &name("ffn_gate.weight"))?;
            let ffn = gate.rows;
            check_dims(&gate, ffn, cfg.embd)?;
            let up = read(r, "ffn_up.weight", ffn, cfg.embd)?;
            let down = read(r, "ffn_down.weight", cfg.embd, ffn)?;
            let bias = match cfg.rope {
                Rope::Halves => Some([
                    vector(r, s, &name("attn_q.bias"), cfg.embd)?,
                    vector(r, s, &name("attn_k.bias"), kv_dim)?,
                    vector(r, s, &name("attn_v.bias"), kv_dim)?,
                ]),
                Rope::Interleaved => None,
            };
            layers.push(Layer {
                attn_norm: vector(r, s, &name("attn_norm.weight"), cfg.embd)?,
                ffn_norm: vector(r, s, &name("ffn_norm.weight"), cfg.embd)?,
                wq,
                wk,
                wv,
                bias,
                wo,
                gate,
                up,
                down,
                k_cache: Vec::new(),
                v_cache: Vec::new(),
            });
        }
        // The same table as candle's `precomput_freqs_cis`: θ_i = base^(−2i/d), angle = pos · θ_i.
        let half = cfg.head_dim / 2;
        let theta: Vec<f32> = (0..half).map(|i| 1.0 / cfg.rope_base.powf((2 * i) as f32 / cfg.head_dim as f32)).collect();
        let (mut cos, mut sin) = (Vec::with_capacity(cfg.max_pos * half), Vec::with_capacity(cfg.max_pos * half));
        for p in 0..cfg.max_pos {
            for t in &theta {
                let a = p as f32 * t;
                cos.push(a.cos());
                sin.push(a.sin());
            }
        }
        Ok(Forward { cfg, embd, layers, norm, output, cos, sin, len: 0, scratch: Scratch::default() })
    }

    /// Forget the KV cache (a new conversation).
    pub fn reset(&mut self) {
        self.len = 0;
        for l in &mut self.layers {
            l.k_cache.clear();
            l.v_cache.clear();
        }
    }

    /// Keep only the first `len` cached tokens (no-op when fewer are cached), so a prompt that
    /// shares a prefix with the previous one continues from there instead of starting over.
    pub fn truncate(&mut self, len: usize) {
        let len = len.min(self.len);
        let per_token = self.cfg.n_kv_head.saturating_mul(self.cfg.head_dim);
        for l in &mut self.layers {
            l.k_cache.truncate(len.saturating_mul(per_token));
            l.v_cache.truncate(len.saturating_mul(per_token));
        }
        self.len = len;
    }

    /// Tokens currently in the KV cache.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Runs `tokens` at positions `pos..` and returns the logits after the last one. `pos` must
    /// equal [`Self::len`] (continue) or be 0 (start over).
    pub fn forward(&mut self, tokens: &[u32], pos: usize) -> Result<Vec<f32>, LlmError> {
        if pos == 0 {
            self.reset();
        }
        if pos != self.len {
            return Err(err(format!("position {pos}, but {} tokens are cached", self.len)));
        }
        let end = pos.checked_add(tokens.len()).filter(|e| *e <= self.cfg.max_pos);
        if tokens.is_empty() || end.is_none() {
            return Err(err(format!("{} tokens at {pos} exceed the context of {}", tokens.len(), self.cfg.max_pos)));
        }
        let embd = self.cfg.embd;
        let mut last = Vec::new();
        for (ci, chunk) in tokens.chunks(PREFILL_CHUNK).enumerate() {
            let mut xs = Vec::with_capacity(chunk.len() * embd);
            for &t in chunk {
                let t = usize::try_from(t).map_err(|_| err("token id"))?;
                xs.extend(self.embd.row(t)?);
            }
            self.chunk(&mut xs, pos + ci * PREFILL_CHUNK)?;
            last = xs.get(xs.len().saturating_sub(embd)..).map(<[f32]>::to_vec).unwrap_or_default();
        }
        let c = &self.cfg;
        let mut normed = vec![0.0; c.embd];
        rms_norm(&last, &self.norm, c.eps, &mut normed);
        let mut logits = vec![0.0; c.vocab];
        self.output.as_ref().unwrap_or(&self.embd).matvec(&normed, &mut logits, &mut self.scratch)?;
        Ok(logits)
    }

    /// `t = xs.len() / embd` tokens at positions `pos..pos + t` through every layer at once (each
    /// weight row is read once per chunk); appends to the KV cache. Bit-identical to running the
    /// tokens one by one.
    fn chunk(&mut self, xs: &mut [f32], pos: usize) -> Result<(), LlmError> {
        use rayon::prelude::*;
        let c = self.cfg.clone();
        let t = xs.len() / c.embd;
        let kv_dim = c.n_kv_head * c.head_dim;
        let group = c.n_head / c.n_kv_head;
        let half = c.head_dim / 2;
        let sqrt_dim = (c.head_dim as f32).sqrt();
        let (mut h, mut q, mut k, mut v) = (vec![0.0; t * c.embd], vec![0.0; t * c.embd], vec![0.0; t * kv_dim], vec![0.0; t * kv_dim]);
        let mut att = vec![0.0; t * c.embd];
        let mut proj = vec![0.0; t * c.embd];
        for l in &mut self.layers {
            for (x, hh) in xs.chunks_exact(c.embd).zip(h.chunks_exact_mut(c.embd)) {
                rms_norm(x, &l.attn_norm, c.eps, hh);
            }
            l.wq.matmul(&h, &mut q, &mut self.scratch)?;
            l.wk.matmul(&h, &mut k, &mut self.scratch)?;
            l.wv.matmul(&h, &mut v, &mut self.scratch)?;
            for (i, ((qi, ki), vi)) in q.chunks_exact_mut(c.embd).zip(k.chunks_exact_mut(kv_dim)).zip(v.chunks_exact_mut(kv_dim)).enumerate() {
                if let Some([bq, bk, bv]) = &l.bias {
                    add(qi, bq);
                    add(ki, bk);
                    add(vi, bv);
                }
                let at = (pos + i).checked_mul(half).ok_or_else(|| err("position"))?;
                let cos = self.cos.get(at..at + half).ok_or_else(|| err("position past the rope table"))?;
                let sin = self.sin.get(at..at + half).ok_or_else(|| err("position past the rope table"))?;
                for head in qi.chunks_exact_mut(c.head_dim).chain(ki.chunks_exact_mut(c.head_dim)) {
                    rotate(head, cos, sin, c.rope);
                }
            }
            l.k_cache.extend_from_slice(&k);
            l.v_cache.extend_from_slice(&v);
            // Tokens are independent here (each reads the cache up to its own position), so the
            // split across threads never changes a result.
            let (kc, vc) = (&l.k_cache, &l.v_cache);
            att.par_chunks_mut(c.embd).zip(q.par_chunks(c.embd)).enumerate().try_for_each(|(i, (ai, qi))| -> Result<(), LlmError> {
                let n = pos + i + 1;
                let mut scores = Vec::with_capacity(n);
                for (hi, (qh, out)) in qi.chunks_exact(c.head_dim).zip(ai.chunks_exact_mut(c.head_dim)).enumerate() {
                    let kv_off = (hi / group) * c.head_dim;
                    scores.clear();
                    for p in 0..n {
                        let at = p * kv_dim + kv_off;
                        let kp = kc.get(at..at + c.head_dim).ok_or_else(|| err("KV cache"))?;
                        scores.push(dot(qh, kp) / sqrt_dim);
                    }
                    softmax(&mut scores);
                    out.fill(0.0);
                    for (p, &w) in scores.iter().enumerate() {
                        let at = p * kv_dim + kv_off;
                        let vp = vc.get(at..at + c.head_dim).ok_or_else(|| err("KV cache"))?;
                        for (o, &vv) in out.iter_mut().zip(vp) {
                            *o += w * vv;
                        }
                    }
                }
                Ok(())
            })?;
            l.wo.matmul(&att, &mut proj, &mut self.scratch)?;
            add(xs, &proj);
            for (x, hh) in xs.chunks_exact(c.embd).zip(h.chunks_exact_mut(c.embd)) {
                rms_norm(x, &l.ffn_norm, c.eps, hh);
            }
            let mut g = vec![0.0; t * l.gate.rows];
            let mut u = vec![0.0; t * l.up.rows];
            l.gate.matmul(&h, &mut g, &mut self.scratch)?;
            l.up.matmul(&h, &mut u, &mut self.scratch)?;
            for (gv, uv) in g.iter_mut().zip(&u) {
                *gv = silu(*gv) * uv;
            }
            l.down.matmul(&g, &mut proj, &mut self.scratch)?;
            add(xs, &proj);
        }
        self.len = pos + t;
        Ok(())
    }
}

fn add(x: &mut [f32], y: &[f32]) {
    for (a, b) in x.iter_mut().zip(y) {
        *a += b;
    }
}

/// f32 dot in eight explicit lanes, so it vectorizes (a single running sum cannot be reordered by
/// the compiler); attention spends most of its time here on long prompts.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let (a8, a_rest) = a.as_chunks::<8>();
    let (b8, b_rest) = b.as_chunks::<8>();
    let mut lanes = [0f32; 8];
    for (x, y) in a8.iter().zip(b8) {
        for l in 0..8 {
            lanes[l] += x[l] * y[l];
        }
    }
    let tail: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    lanes.iter().sum::<f32>() + tail
}

fn silu(x: f32) -> f32 {
    x / (1.0 + (-x).exp())
}

/// `out = x / sqrt(mean(x²) + eps) · w`, rounded exactly as candle's `rms_norm` (divide, not
/// multiply by the reciprocal: the difference grows through the layers on 4-bit models).
fn rms_norm(x: &[f32], w: &[f32], eps: f32, out: &mut [f32]) {
    let n = x.len().max(1) as f32;
    let m = (x.iter().map(|v| v * v).sum::<f32>() / n + eps).sqrt();
    for ((o, &v), &g) in out.iter_mut().zip(x).zip(w) {
        *o = v / m * g;
    }
}

fn softmax(v: &mut [f32]) {
    let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for x in v.iter_mut() {
        *x = (*x - max).exp();
        sum += *x;
    }
    if sum > 0.0 {
        for x in v.iter_mut() {
            *x /= sum;
        }
    }
}

fn rotate(head: &mut [f32], cos: &[f32], sin: &[f32], style: Rope) {
    match style {
        Rope::Halves => {
            let (a, b) = head.split_at_mut(head.len() / 2);
            for (((x0, x1), &c), &s) in a.iter_mut().zip(b.iter_mut()).zip(cos).zip(sin) {
                let (u, v) = (*x0, *x1);
                *x0 = u * c - v * s;
                *x1 = u * s + v * c;
            }
        }
        Rope::Interleaved => {
            for ((pair, &c), &s) in head.as_chunks_mut::<2>().0.iter_mut().zip(cos).zip(sin) {
                let [u, v] = *pair;
                *pair = [u * c - v * s, u * s + v * c];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::tests::Builder;
    use std::io::Cursor;

    const F32: u32 = 0;
    const Q8_0: u32 = 8;

    fn values(n: usize, seed: u32, amp: f32) -> Vec<f32> {
        (0..n).map(|i| ((((i as u32).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40_503)) >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * amp).collect()
    }

    fn f32_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn q8_bytes(v: &[f32]) -> Vec<u8> {
        let mut q = Vec::new();
        crate::quant::quantize_q8_0(v, &mut q).unwrap();
        q.iter().flat_map(|b| half::f16::from_f32(b.d).to_le_bytes().into_iter().chain(b.qs.iter().map(|x| x.to_le_bytes()[0]))).collect()
    }

    /// A tiny random model of `arch` that candle loads too: embd 64, 4 heads (2 KV), 2 layers,
    /// MLP 128, vocabulary 96; matrices Q8_0, vectors F32; tied output.
    fn tiny(arch: &str) -> Vec<u8> {
        let (embd, kv, ffn, vocab, layers) = (64u64, 32u64, 128u64, 96u64, 2);
        let mut b = Builder::default()
            .str("general.architecture", arch)
            .u32(&format!("{arch}.embedding_length"), embd as u32)
            .u32(&format!("{arch}.attention.head_count"), 4)
            .u32(&format!("{arch}.attention.head_count_kv"), 2)
            .u32(&format!("{arch}.block_count"), layers)
            .u32(&format!("{arch}.context_length"), 256)
            .u32(&format!("{arch}.rope.dimension_count"), 16)
            .f32(&format!("{arch}.attention.layer_norm_rms_epsilon"), 1e-6)
            .f32(&format!("{arch}.rope.freq_base"), 10_000.0);
        let mut seed = 1;
        let mut mat = |b: Builder, name: &str, rows: u64, cols: u64, amp: f32| {
            seed += 1;
            b.tensor_data(name, &[cols, rows], Q8_0, &q8_bytes(&values((rows * cols) as usize, seed, amp)))
        };
        b = mat(b, "token_embd.weight", vocab, embd, 2.0);
        for i in 0..layers {
            b = mat(b, &format!("blk.{i}.attn_q.weight"), embd, embd, 0.5);
            b = mat(b, &format!("blk.{i}.attn_k.weight"), kv, embd, 0.5);
            b = mat(b, &format!("blk.{i}.attn_v.weight"), kv, embd, 0.5);
            b = mat(b, &format!("blk.{i}.attn_output.weight"), embd, embd, 0.5);
            b = mat(b, &format!("blk.{i}.ffn_gate.weight"), ffn, embd, 0.5);
            b = mat(b, &format!("blk.{i}.ffn_up.weight"), ffn, embd, 0.5);
            b = mat(b, &format!("blk.{i}.ffn_down.weight"), embd, ffn, 0.5);
        }
        let mut vec = |b: Builder, name: &str, n: u64, base: f32| {
            seed += 1;
            let v: Vec<f32> = values(n as usize, seed, 0.2).iter().map(|x| x + base).collect();
            b.tensor_data(name, &[n], F32, &f32_bytes(&v))
        };
        b = vec(b, "output_norm.weight", embd, 1.0);
        for i in 0..layers {
            b = vec(b, &format!("blk.{i}.attn_norm.weight"), embd, 1.0);
            b = vec(b, &format!("blk.{i}.ffn_norm.weight"), embd, 1.0);
            if arch == "qwen2" {
                b = vec(b, &format!("blk.{i}.attn_q.bias"), embd, 0.0);
                b = vec(b, &format!("blk.{i}.attn_k.bias"), kv, 0.0);
                b = vec(b, &format!("blk.{i}.attn_v.bias"), kv, 0.0);
            }
        }
        b.build()
    }

    fn ours(bytes: &[u8], arch: &str) -> Forward {
        let mut c = Cursor::new(bytes);
        let s = crate::gguf::check(&mut c, bytes.len() as u64).unwrap();
        let cfg = Config::from_gguf(&s, arch, 256).unwrap();
        Forward::load(&mut c, &s, cfg).unwrap()
    }

    enum Candle {
        Qwen(candle_transformers::models::quantized_qwen2::ModelWeights),
        Llama(candle_transformers::models::quantized_llama::ModelWeights),
    }

    fn candle_logits(bytes: &[u8], arch: &str, steps: &[(&[u32], usize)]) -> Vec<Vec<f32>> {
        use candle_core::quantized::gguf_file;
        use candle_core::{Device, Tensor};
        let dev = Device::Cpu;
        let mut c = Cursor::new(bytes);
        let content = gguf_file::Content::read(&mut c).unwrap();
        let mut m = match arch {
            "qwen2" => Candle::Qwen(candle_transformers::models::quantized_qwen2::ModelWeights::from_gguf(content, &mut c, &dev).unwrap()),
            _ => Candle::Llama(candle_transformers::models::quantized_llama::ModelWeights::from_gguf(content, &mut c, &dev).unwrap()),
        };
        steps
            .iter()
            .map(|(toks, pos)| {
                let x = Tensor::new(*toks, &dev).unwrap().unsqueeze(0).unwrap();
                let l = match &mut m {
                    Candle::Qwen(w) => w.forward(&x, *pos),
                    Candle::Llama(w) => w.forward(&x, *pos),
                }
                .unwrap();
                l.squeeze(0).unwrap().to_vec1::<f32>().unwrap()
            })
            .collect()
    }

    fn assert_close(a: &[f32], b: &[f32], what: &str) {
        assert_eq!(a.len(), b.len(), "{what}: lengths");
        let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-3);
        let worst = a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(worst <= 2e-2 * scale, "{what}: max diff {worst} vs scale {scale}");
        let arg = |v: &[f32]| v.iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0;
        assert_eq!(arg(a), arg(b), "{what}: argmax");
    }

    #[test]
    fn matches_candle_for_both_families_token_by_token() {
        for arch in ["qwen2", "llama"] {
            let bytes = tiny(arch);
            let mut f = ours(&bytes, arch);
            // Single-token steps only: candle 0.9's prefill mask is (t, t), which is wrong once the
            // cache is non-empty (infer.rs works around it the same way).
            let toks: Vec<u32> = vec![5, 17, 3, 88, 42, 9];
            let steps: Vec<(&[u32], usize)> = toks.iter().enumerate().map(|(i, t)| (std::slice::from_ref(t), i)).collect();
            let want = candle_logits(&bytes, arch, &steps);
            for (i, t) in toks.iter().enumerate() {
                let got = f.forward(std::slice::from_ref(t), i).unwrap();
                assert_close(&got, &want[i], &format!("{arch} step {i}"));
            }
        }
    }

    #[test]
    fn a_batch_equals_the_same_tokens_one_by_one() {
        let bytes = tiny("qwen2");
        let mut f = ours(&bytes, "qwen2");
        // 70 tokens: more than one prefill chunk.
        let toks: Vec<u32> = (0..70u32).map(|i| (i * 37 + 5) % 96).collect();
        let batch = f.forward(&toks, 0).unwrap();
        let mut g = ours(&bytes, "qwen2");
        let mut last = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            last = g.forward(std::slice::from_ref(t), i).unwrap();
        }
        assert_eq!(batch, last);
        // Continuing after a batch equals continuing after single steps; restarting forgets.
        assert_eq!(f.forward(&[7], 70).unwrap(), g.forward(&[7], 70).unwrap());
        assert_eq!(f.forward(&toks, 0).unwrap(), batch);
        // Truncating to a prefix and continuing equals running the whole sequence.
        let tail: Vec<u32> = vec![11, 12, 13];
        let whole: Vec<u32> = toks.iter().chain(&tail).copied().collect();
        let want = g.forward(&whole, 0).unwrap();
        f.forward(&[1, 2, 3, 4], 70).unwrap();
        f.truncate(70);
        assert_eq!(f.len(), 70);
        assert_eq!(f.forward(&tail, 70).unwrap(), want);
        f.truncate(1000);
        assert_eq!(f.len(), 73, "truncating past the cache keeps it");
        f.truncate(0);
        assert!(f.is_empty());
        assert!(f.forward(&[1], 1).is_err());
    }

    #[test]
    fn hostile_calls_and_headers_are_errors() {
        let bytes = tiny("llama");
        let mut f = ours(&bytes, "llama");
        assert!(f.forward(&[], 0).is_err());
        assert!(f.forward(&[96], 0).is_err(), "token id past the vocabulary");
        assert!(f.forward(&[1], 5).is_err(), "position that skips the cache");
        assert!(f.forward(&vec![1; 257], 0).is_err(), "past the context");
        f.forward(&[1, 2], 0).unwrap();
        assert!(f.forward(&[3], 2).is_ok());
        let mut c = Cursor::new(&bytes);
        let s = crate::gguf::check(&mut c, bytes.len() as u64).unwrap();
        assert!(Config::from_gguf(&s, "qwen2", 256).is_err(), "wrong architecture keys");
        assert!(Config::from_gguf(&s, "gpt2", 256).is_err());
        // A Qwen2 header over Llama tensors misses the biases.
        let qb = tiny("qwen2");
        let qs = crate::gguf::check(&mut Cursor::new(&qb), qb.len() as u64).unwrap();
        let cfg = Config::from_gguf(&qs, "qwen2", 256).unwrap();
        let mut l = Cursor::new(&bytes);
        assert!(Forward::load(&mut l, &s, cfg).is_err());
    }

    /// Real-model parity and timing: `FILMCRAFT_LLM_GGUF=<file> cargo test --release -p
    /// filmcraft-llm --features infer --lib real_model -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_model_matches_candle() {
        let Ok(path) = std::env::var("FILMCRAFT_LLM_GGUF") else {
            eprintln!("FILMCRAFT_LLM_GGUF not set; skipped");
            return;
        };
        let bytes = std::fs::read(&path).unwrap();
        let mut c = Cursor::new(&bytes);
        let s = crate::gguf::check(&mut c, bytes.len() as u64).unwrap();
        let arch = s.architecture.clone().unwrap();
        let mut types = std::collections::BTreeMap::<(u32, usize), usize>::new();
        for t in &s.tensor_infos {
            *types.entry((t.ty, t.dims.len())).or_default() += 1;
        }
        eprintln!("tensor (type, dims) counts: {types:?}");
        {
            use candle_core::{Device, Module, Tensor, quantized};
            let dev = Device::Cpu;
            let mut cc = Cursor::new(&bytes);
            let content = quantized::gguf_file::Content::read(&mut cc).unwrap();
            for name in ["blk.0.attn_q.weight", "blk.0.ffn_down.weight", "token_embd.weight"] {
                let ours_w = Weight::read(&mut Cursor::new(&bytes), &s, name).unwrap();
                let x: Vec<f32> = values(ours_w.cols, 77, 2.0);
                let mut got = vec![0f32; ours_w.rows];
                ours_w.matvec(&x, &mut got, &mut Scratch::default()).unwrap();
                let qt = content.tensor(&mut cc, name, &dev).unwrap();
                let ty = qt.dtype();
                let mm = quantized::QMatMul::from_qtensor(qt).unwrap();
                let want = mm.forward(&Tensor::from_vec(x, (1, ours_w.cols), &dev).unwrap()).unwrap().squeeze(0).unwrap().to_vec1::<f32>().unwrap();
                let worst = got.iter().zip(&want).fold(0f32, |m, (g, w)| m.max((g - w).abs()));
                eprintln!("{name} ({ty:?}): max |ours − candle| = {worst:e}");
            }
        }
        let t = std::time::Instant::now();
        let mut f = Forward::load(&mut c, &s, Config::from_gguf(&s, &arch, 1024).unwrap()).unwrap();
        eprintln!("{arch}: loaded in {:.2} s", t.elapsed().as_secs_f64());
        let toks: Vec<u32> = vec![1, 504, 1730, 314, 260, 1953, 30, 16, 30, 216];
        let steps: Vec<(&[u32], usize)> = toks.iter().enumerate().map(|(i, t)| (std::slice::from_ref(t), i)).collect();
        let t = std::time::Instant::now();
        let want = candle_logits(&bytes, &arch, &steps);
        eprintln!("candle: {:.1} ms per token (incl. load)", t.elapsed().as_secs_f64() * 1e3 / toks.len() as f64);
        let t = std::time::Instant::now();
        let mut got = Vec::new();
        for (i, tok) in toks.iter().enumerate() {
            got.push(f.forward(std::slice::from_ref(tok), i).unwrap());
        }
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let scale = w.iter().fold(0f32, |m, v| m.max(v.abs()));
            let worst = g.iter().zip(w).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
            let arg = |v: &[f32]| v.iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0;
            eprintln!("step {i}: max diff {worst:.4} / {scale:.2}, argmax {} vs {}", arg(g), arg(w));
        }
        // Step 0 (one attention position) is bit-identical to candle: same kernels, same rounding.
        // From two positions on, candle's gemm sums attention in another order; the ~1e-7
        // difference flips 8-bit activation roundings and grows through the layers (≤ 4 % on the
        // Q8_0 models, up to ~16 % on SmolLM2-1.7B Q4_K_M, whose activations have large outliers;
        // its top token still agrees on 10 of 10 steps).
        // AI-PLAN's real gate is the fact checker; here: step 0 exact to 1e-4, the same top token on
        // at least 8 of 10 steps, every step within 20 % of the logit range.
        let arg = |v: &[f32]| v.iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (i, &x)| if x > m.1 { (i, x) } else { m }).0;
        let mut agree = 0;
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let scale = w.iter().fold(0f32, |m, v| m.max(v.abs()));
            let worst = g.iter().zip(w).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
            // candle's AVX2 kernels (flagged builds) sum in their own order: no exact step 0 there.
            // Our explicit AVX2 dots (AI1.1g) also sum in their own order: exact only at Baseline.
            let exact = !cfg!(target_feature = "avx2") && crate::quant::simd_level() == crate::quant::SimdLevel::Baseline;
            let limit = if i == 0 && exact { 1e-4 } else { 0.20 };
            assert!(worst <= limit * scale, "{arch} step {i}: max diff {worst} / {scale}");
            agree += usize::from(arg(g) == arg(w));
        }
        assert!(agree >= 8, "{arch}: top token agrees on {agree} of 10 steps");
        eprintln!("ours: {:.1} ms per token", t.elapsed().as_secs_f64() * 1e3 / toks.len() as f64);
        let n: u32 = std::env::var("FILMCRAFT_LLM_PREFILL").ok().and_then(|v| v.parse().ok()).unwrap_or(300).clamp(1, 1000);
        let prompt: Vec<u32> = (0..n).map(|i| (i * 7919 + 13) % 5000).collect();
        let t = std::time::Instant::now();
        f.forward(&prompt, 0).unwrap();
        eprintln!("ours: {n}-token prefill {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        let steps: Vec<(&[u32], usize)> = vec![(&prompt, 0)];
        let t = std::time::Instant::now();
        candle_logits(&bytes, &arch, &steps);
        eprintln!("candle: {n}-token prefill {:.0} ms (incl. load)", t.elapsed().as_secs_f64() * 1e3);
    }
}
