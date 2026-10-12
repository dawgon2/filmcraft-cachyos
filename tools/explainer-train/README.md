# explainer-train (offline tooling, not part of FilmCraft)

Fine-tunes a small instruct model so it explains colour-grade nodes in the controlled grammar
FilmCraft's fact checker accepts (GRADE-PLAN G2.11b). **This folder is offline tooling for the
maintainer.** Nothing here is built into, linked by or bundled with FilmCraft; the product stays
pure Rust and only ever *runs* a finished GGUF file through `filmcraft-llm` (candle, CPU). Tuned
weights are never committed; they live under `~/.local/share/filmcraft-fork/models/tuned/`.

| file | what |
|------|------|
| `run.sh <base> <name>` | one whole session: generate → train → quantize → eval → score |
| `train.py` | LoRA fine-tune on the full-precision base, early stopping on validation loss, merge, save safetensors + config + tokenizer + `manifest.json` |
| `LICENSE-Apache-2.0.txt` | the Apache License 2.0 text `train.py` writes as the tuned model's `LICENSE` when the base folder has none (it also writes `NOTICE`) |
| `tinylm.py` | a small PyTorch implementation of the Llama (SmolLM2) and Qwen2 (Qwen2.5) decoders with LoRA, reading and writing the checkpoints' own tensor names; needs only `torch`, `safetensors`, `tokenizers` |

The Rust stages are cargo examples: `filmcraft-render`'s `explain_traindata` (training data) and
`explain_score` (scoring), `filmcraft-llm`'s `llm_quantize` (safetensors → GGUF) and `llm_eval`
(runs the 36 held-out cases). The full session, timings, where files go and how to publish:
[`docs/fork/TRAINING.md`](../../docs/fork/TRAINING.md).

Requirements: a Python 3.12 venv with `torch` (ROCm or CUDA build; CPU works, slowly),
`safetensors` and `tokenizers` (the owner's venv is `~/Apps/filmcraft-train/.venv`). `train.py`
deliberately does not use `transformers` / `peft`, so it keeps working whatever versions of those
the venv holds.
