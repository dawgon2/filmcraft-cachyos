# AI4.2 design note: Qwen3-VL 4B's text model on FilmCraft's runtime

Status: draft, 2026-10-16 (scheduled run, MINIMAL mode). For AI-PLAN AI4 item 2. The vision side
is done on the CPU (`crates/llm/src/vision.rs`, parity with candle's encoder, see HANDOFF). This
note covers what the language side needs and, most importantly, **what to check it against**.

## What the text model is

From the downloaded model's `config.json` (`text_config`) and tensor names
(`model.language_model.*`, 398 tensors; nothing under `lm_head`, embeddings are tied):

| | |
|---|---|
| layers | 36, pre-norm (RMSNorm `input_layernorm`, `post_attention_layernorm`) |
| hidden / MLP | 2560 / 9728, SwiGLU (`gate_proj`, `up_proj`, `down_proj`) |
| attention | 32 query heads, 8 key / value heads (GQA 4:1), head 128, **per-head RMSNorm on q and k** (`q_norm`, `k_norm`, applied before the rotary embedding) |
| rotary | theta 5 000 000, **interleaved M-RoPE**, sections [24, 20, 20] |
| vocab | 151 936, tied output |
| weights | BF16 safetensors (no GGUF release we use) |

Compared with what `forward.rs` runs today (Qwen2.5, SmolLM2 / 3 from GGUF):

1. **q / k norm**: new, small (RMSNorm over each 128-wide head).
2. **M-RoPE**: new. Each token has three positions (t, h, w). The 64 rotary frequency pairs are
   split by section: with `mrope_interleaved`, pair j takes its angle from h when
   j mod 3 = 1 and j < 3 × 20, from w when j mod 3 = 2 and j < 3 × 20, and from t otherwise.
   Text tokens have t = h = w = their index, so **for text-only prompts M-RoPE equals ordinary
   RoPE**. Picture tokens share one t and take (h, w) from their merged-grid cell, offset by the
   position where the picture starts; the text after a picture continues from
   max(position used) + 1, not from the token count. This must be checked against the published
   reference implementation (Hugging Face `transformers` `Qwen3VLModel.get_rope_index`,
   Apache-2.0) before building; the interleave rule above is from the config's names and must be
   confirmed there.
3. **DeepStack**: the 3 tap outputs from the vision encoder (`VisionOutput::deepstack`) are
   **added to the hidden state at the picture-token positions after decoder layers 0, 1 and 2**
   (one tap per layer, in order). The final merger output replaces the picture placeholder
   tokens' embeddings before layer 0.
4. **Weights**: BF16 safetensors, 4 B parameters. f32 would need ~16 GB (too much). Options:
   (a) quantise at load to our Q8_0 / Q4_K kernels (`quant.rs`, `kquant.rs`; the `llm_quantize`
   example already writes GGUF from f32) and run the existing GPU / CPU matvec paths; (b) keep BF16
   and add BF16 kernels. **Recommendation: (a), one-time conversion to a cached GGUF next to the
   model** (reuses every kernel we have, GPU included; Q8_0 ≈ 4.3 GB, Q4_K_M ≈ 2.5 GB). The vision
   encoder stays f32 (0.4 B parameters, 1.6 GB) or goes to f16 on the GPU later.

## Oracles: candle's Qwen3-VL text side is NOT one

Reading `candle-transformers` 0.9.2 `models/qwen3_vl/{mod,text}.rs`:

- it has **no M-RoPE**: `RotaryEmbedding` is plain 1-D RoPE on sequence offsets, so positions of
  picture tokens and of text after a picture differ from the reference;
- the causal mask is built **only when the sequence length is ≤ 1**
  (`if seqlen <= 1 { Some(mask) } else { None }` in `Qwen3VLModel::forward`), so a multi-token
  prefill attends to future tokens.

Its logits therefore can't serve as the end-to-end oracle. Instead:

1. **Text-only parity** (where M-RoPE = RoPE): candle's dense `models::qwen3` (`qwen3.rs`, same
   crate, MIT / Apache-2.0) loaded with the VL model's `model.language_model.*` weights renamed to
   `model.*`, tied head, `text_config` values. Compare logits on a fixed prompt, f32 vs our
   quantised path within the tolerance our Qwen2.5 parity used.
2. **M-RoPE positions**: unit tests from the reference's documented examples (a picture of 2 × 3
   merged tokens between text; positions of the text after it), written from the reference
   implementation's behaviour, plus a hostile-grid test (zero / huge grids refused).
3. **DeepStack + picture tokens**: no faithful oracle on hand. Behavioural acceptance: on a few
   synthetic pictures with unambiguous content (a red square on white, the word "FILM" in large
   letters, a 3-bar chart), the answer names the content. The Python reference (owner question
   if Python packages must be installed) remains the only exact check; ask once, don't block.

## Steps (each small, in order)

1. `Family::Qwen3Vl` text config from `text_config`; tensor-name map for `model.language_model.*`.
2. BF16 → GGUF conversion tool (extend `llm_quantize`), cached by model commit; checked tensor
   shapes.
3. q / k norm in `forward.rs` (also unlocks plain Qwen3 models).
4. M-RoPE position builder (`(t, h, w)` per token, picture spans) + rotary application; unit
   tests (step "oracles" 2).
5. Text-only parity against candle `qwen3` (oracle 1).
6. Picture placeholders: the chat template's `<|vision_start|><|image_pad|>…<|vision_end|>`, one
   pad per merged token; merger output replaces them; DeepStack adds after layers 0–2.
7. Behavioural acceptance (oracle 3), then GPU (the existing matvec / attention kernels, plus the
   vision encoder's f32 matmul on the GPU).

GPU and the larger design calls (step 2's format, step 6's prompt) are STEADY+ work.
