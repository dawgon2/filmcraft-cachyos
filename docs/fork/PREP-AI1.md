# PREP-AI1: audit for AI1.1 (own quantized forward pass, run-time CPU dispatch)

Written 2026-10-13 (scheduled run, MINIMAL mode, docs only; CLOUD-QUEUE item AI1-PREP). Read with
[`AI-PLAN.md`](../../AI-PLAN.md) Phase AI1 and OWNER-QUESTIONS Q8.

## What exists

- **`crates/llm`** (1 888 lines): `models.rs` catalogue (pinned, checksummed), `gguf.rs` (607 lines,
  our own checked GGUF header walk that runs *before* candle sees a file), `prompt.rs` (chat
  templates), `infer.rs` (350 lines, feature `infer`), `stub.rs` (no-`infer` builds / wasm).
- **`infer.rs` is a thin wrapper over candle 0.9.2:** `gguf_file::Content::read`, then
  `quantized_qwen2::ModelWeights` (Qwen2.5) or `quantized_llama::ModelWeights` (SmolLM2) on
  `Device::Cpu`; per step `forward(&Tensor, pos)`, `LogitsProcessor` for sampling,
  `utils::apply_repeat_penalty`. The prefill works around candle's (t, t) causal-mask limitation
  (comment at `infer.rs:197`).
- **Callers:** only `crates/engine/src/ai_explain.rs` (`Model::load`, `Model::generate` behind a
  backend trait, already under `catch_unwind` at line 618, one job at a time) and its tests.
  `crates/render` depends on `filmcraft-llm` without the `infer` feature (catalogue only).
  Engine features: `llm = ["filmcraft-llm/infer"]`, `llm-download`.
- **Quant formats in the catalogue:** Q8_0 (`smollm2-360m-instruct-q8`, both fork tunes
  `*-v1-q8`, the tuned default through `TUNED_PREFERENCE`) and Q4_K_M (stock Qwen2.5 0.5B / 1.5B,
  SmolLM2 1.7B; Q4_K_M files mix **Q4_K and Q6_K** tensors, plus F32 norms). AI1's acceptance runs
  on Q8_0; Q4_K / Q6_K must keep working (stock default `qwen2.5-0.5b-instruct-q4km`).
- **The flag:** `.cargo/config.toml` sets `target-cpu=x86-64-v3` for x86_64 Linux (Q8 provisional (a)).

## Key finding: `gemm` alone doesn't fix this

candle's quantized matmul (`QMatMul` → `k_quants.rs`, 2 505 lines) **does not use `gemm`**: it
quantizes the activation row to the weight's vec-dot type (Q8_0 for Q8_0/Q4_0, Q8_K for the
k-quants) and calls `vec_dot` per weight row. Only `vec_dot` has SIMD, in `avx.rs` (669 lines, 17
`unsafe`), `neon.rs`, `simd128.rs`, picked with **compile-time** `#[cfg(target_feature = "avx2")]`
(`k_quants.rs:241, 650, …`). That is exactly why the flag gives 4–5×.

Batch-1 decode (the explainer's whole answer after the prompt) is a matrix-*vector* product that is
memory-bound on the quantized weights. Routing it through `gemm` would mean dequantizing weights to
f32 (≈ 4× the memory traffic, 1.4 GB for a 360M model), which is slower, not faster. So:

- **Quantized dots (the hot path):** our own `vec_dot_q8_0_q8_0`, `vec_dot_q4k_q8k`,
  `vec_dot_q6k_q8k` (ported from candle's scalar reference + the AVX2 algorithm, attribution kept)
  written in safe Rust with **`pulp`** (0.22.3, already in `Cargo.lock` as a dependency of `gemm`):
  `pulp::Arch::new()` detects AVX-512 / AVX2 (V3) / SSE / NEON at run time and dispatches with
  `arch.dispatch(...)`, no `unsafe` in our code. Fallback: a scalar loop over fixed-size block
  arrays (`[i8; 32]`) that LLVM vectorizes for SSE2 at least.
- **`gemm` where it does fit:** the f32 parts of prefill (Q·Kᵀ and softmax·V over the prompt, and
  optionally prefill projections by dequantizing one row block at a time into a reused buffer).
  That keeps AI-PLAN's "heavy maths through gemm" true for the f32 maths.
- **Forced-SSE override for tests:** pulp has no public "pretend this CPU lacks AVX2" switch; wrap
  dispatch in our own `enum Simd { Auto, Scalar, ... }` chosen once (env var
  `FILMCRAFT_LLM_SIMD=scalar|sse|auto` read through a validated parser, plus a `pub(crate)` setter
  for tests). With pulp, `Arch::Scalar` / the `x86::V2`-level path can be called explicitly.

## The forward pass to own (per family)

From `quantized_qwen2.rs` (338 lines) and `quantized_llama.rs` (520 lines), both MIT / Apache-2.0:
token embedding (dequantize one row), per layer RMSNorm → Q/K/V projections (Qwen2 has **biases**
on Q/K/V, Llama none) → RoPE (Qwen: θ from GGUF, neox layout; check `rope.freq_base`) → GQA
attention with KV cache (`n_head_kv` < `n_head`) → output projection → residual → RMSNorm →
SwiGLU MLP (w1/w3 → silu·mul → w2) → residual; final norm, `output` (or tied `token_embd`) →
logits. Sampling (`LogitsProcessor`: temperature / top-p / seed) and the repeat penalty are small
and pure; port them too so candle-transformers can go (candle-core can then go as well if our
GGUF reader covers tensor data: `gguf.rs` already walks the header and offsets).

Suggested files: `crates/llm/src/quant/{mod,q8_0,q4k,q6k}.rs` (block types, dequantize, dots),
`crates/llm/src/simd.rs` (dispatch + override), `crates/llm/src/forward.rs` (model, KV cache),
`crates/llm/src/sample.rs`; keep `infer.rs`'s public API (`Model::load / generate /
generate_detailed / verify / prompt_tokens`) unchanged so `ai_explain.rs` needs no edits.

## Testing parity

1. **Kernel level (no model needed):** random blocks (seeded) → our dot vs a straightforward f64
   dequantize-then-dot reference, for every dispatch level (scalar / SSE / AVX2 / AVX-512 where the
   CPU has it); bound the difference (integer block sums are exact; only the f32 scale products
   differ). Hostile blocks: NaN / Inf f16 scales, `n` not a multiple of the block size → `Err`.
2. **Layer level:** a tiny synthetic GGUF (2 layers, d=64, written by the test) run through old
   (candle, kept as a dev-dependency behind a test feature during AI1) and new paths: logits
   within 1e-3 relative, same argmax for 32 greedy tokens.
3. **Model level (local files, skip when absent):** the 75 held-out cases through both paths;
   accept = same fact-check verdicts (AI-PLAN: "identical within the fact checker"), times per
   dispatch level recorded (forced-SSE included).
4. **Never crash:** missing / truncated / foreign GGUF, tensor shapes that disagree with the
   header, out-of-range token ids, `pos` past `MAX_CONTEXT` → `LlmError`, regression tests exist
   for the loader (`infer.rs` tests) and must keep passing on the new path.

## Risks

- **Speed without the flag:** pulp's V3 path must actually inline (the dispatch closure must hold
  the whole row loop, not one block, or call overhead eats the gain). Measure early: one Q8_0 row
  loop vs candle-with-flag before porting the rest.
- **Numerics drift** between candle and ours (summation order in the k-quant dots) can flip a
  near-tie token; acceptance is the fact checker, not byte-identical text.
- **Threads:** candle parallelises the per-row loop with rayon; ours must too (rows split across
  a bounded pool), or 360M decode is ~4× slower on 12 threads.
- **Q4_K / Q6_K scale unpacking** (6-bit packed scales) is the classic bug spot: test against the
  reference dequantize on random blocks.
- Removing the flag also affects every other crate's codegen (AI1.2): rerun the decode / render
  benches once afterwards to confirm nothing else silently relied on AVX2.

GPU owed: none (CPU-only phase until AI1.6 retraining).
