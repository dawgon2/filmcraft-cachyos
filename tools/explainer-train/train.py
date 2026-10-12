#!/usr/bin/env python3
"""LoRA fine-tune of a small instruct model on FilmCraft's node-explanation data (GRADE-PLAN G2.11b).

Offline tooling only: never bundled with FilmCraft, never imported by the product. See
docs/fork/TRAINING.md for the whole session (generate -> train -> quantize -> eval -> score).

    HSA_OVERRIDE_GFX_VERSION=10.3.0 python train.py \
        --base ~/.local/share/filmcraft-fork/models/llm-base/SmolLM2-360M-Instruct \
        --data ~/Apps/filmcraft-train/runs/data-v1 \
        --run-dir ~/Apps/filmcraft-train/runs/smol360-v1 \
        --out ~/.local/share/filmcraft-fork/models/tuned/smol360-v1

Data: `train.jsonl` / `val.jsonl` with `{"messages": [system, user, assistant]}` per line (from
`cargo run -p filmcraft-render --example explain_traindata`). Each example is rendered with the
same ChatML template `filmcraft-llm` uses at inference (`chat_prompt`), the prompt and the answer
tokenized separately exactly as inference sees them, and the loss is taken on the answer and its
closing `<|im_end|>` only.

Trains LoRA adapters (all attention and MLP projections) on the frozen full-precision base in
bf16 (GPU) or float32 (`--device cpu`, the slow fallback), evaluates the validation loss every
`--eval-every` steps, keeps the best adapter (early stopping with `--patience`), merges it into
the base and writes `model.safetensors` (the base's tensor names) + `config.json` + the tokenizer
files + `LICENSE` + `NOTICE` to `--out`, and `manifest.json` (hyper-parameters, base revision,
data stats, loss curve, timings) to both `--run-dir` and `--out`. `LICENSE` is the base's licence
file, or the Apache-2.0 text kept next to this script (`LICENSE-Apache-2.0.txt`) when the base
folder has none (every catalogue base is Apache-2.0).
"""

import argparse
import json
import math
import random
import shutil
import sys
import time
from pathlib import Path

import torch
from safetensors import safe_open
from safetensors.torch import save_file
from tokenizers import Tokenizer

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tinylm  # noqa: E402

IM_START, IM_END = "<|im_start|>", "<|im_end|>"


def sanitize(text: str) -> str:
    """crates/llm prompt.rs `sanitize`: drop `<|...|>` markers (none are expected in the data)."""
    out, rest = [], text
    while "<|" in rest:
        i = rest.index("<|")
        out.append(rest[:i])
        tail = rest[i:]
        j = tail.find("|>")
        rest = tail[j + 2 :] if j >= 0 else tail[2:]
    out.append(rest)
    return "".join(out)


def chat_prompt(system: str, user: str) -> str:
    """crates/llm prompt.rs `chat_prompt` (ChatML, both families)."""
    return f"{IM_START}system\n{sanitize(system)}{IM_END}\n{IM_START}user\n{sanitize(user)}{IM_END}\n{IM_START}assistant\n"


def load_examples(path: Path, tok: Tokenizer, max_len: int):
    out, skipped = [], 0
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        ex = json.loads(line)
        msgs = {m["role"]: m["content"] for m in ex["messages"]}
        p = tok.encode(chat_prompt(msgs["system"], msgs["user"]), add_special_tokens=False).ids
        a = tok.encode(msgs["assistant"] + IM_END, add_special_tokens=False).ids
        if len(p) + len(a) > max_len:
            skipped += 1
            continue
        out.append((p, a))
    return out, skipped


def batches(examples, bs, shuffle, rng):
    idx = list(range(len(examples)))
    if shuffle:
        rng.shuffle(idx)
    for i in range(0, len(idx), bs):
        yield [examples[j] for j in idx[i : i + bs]]


def collate(batch, pad_id, device):
    t = max(len(p) + len(a) for p, a in batch)
    ids = torch.full((len(batch), t), pad_id, dtype=torch.long)
    labels = torch.full((len(batch), t), -100, dtype=torch.long)
    for i, (p, a) in enumerate(batch):
        seq = p + a
        ids[i, : len(seq)] = torch.tensor(seq)
        labels[i, len(p) : len(seq)] = torch.tensor(a)
    return ids.to(device), labels.to(device)


def loss_of(m, ids, labels):
    h = m.hidden(ids)
    # predict token t+1 from position t; logits only where there is a label (the answer), which
    # keeps the large vocabulary head cheap
    h, tgt = h[:, :-1], labels[:, 1:]
    sel = tgt != -100
    logits = m.logits_at(h[sel])
    return torch.nn.functional.cross_entropy(logits, tgt[sel], reduction="sum"), int(sel.sum())


@torch.no_grad()
def evaluate(m, examples, bs, pad_id, device):
    m.eval()
    tot, n = 0.0, 0
    for b in batches(examples, bs, False, None):
        ids, labels = collate(b, pad_id, device)
        l, k = loss_of(m, ids, labels)
        tot += float(l)
        n += k
    m.train()
    return tot / max(n, 1)


# The catalogue bases by folder name (docs/fork/TRAINING.md "Bases").
KNOWN_REPOS = {
    "SmolLM2-360M-Instruct": "HuggingFaceTB/SmolLM2-360M-Instruct",
    "Qwen2.5-0.5B-Instruct": "Qwen/Qwen2.5-0.5B-Instruct",
}

# The NOTICE shipped with a published fine-tune (docs/fork/TRAINING.md "Publishing").
NOTICE = """FilmCraft node explainer {name}
A fine-tune of {repo} (revision {revision}), licensed under the
Apache License 2.0 (see LICENSE). Changes: LoRA fine-tuning (merged) on synthetic training data
generated by FilmCraft's explain_traindata from measured grade facts (no third-party text);
quantized to GGUF with FilmCraft's llm_quantize (the type is in the file name). Training data
provenance: generated by FilmCraft (fork of storytold/filmcraft), seed {seed}; every target passes
FilmCraft's fact checker. Provided "AS IS", without warranties of any kind.
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", type=Path, required=True)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--run-dir", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--device", default="cuda", help="cuda (ROCm/CUDA GPU) or cpu (slow fallback)")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--no-checkpoint", action="store_true", help="keep all activations (faster, needs far more memory)")
    ap.add_argument("--batch", type=int, default=16, help="sequences per micro-batch")
    ap.add_argument("--accum", type=int, default=1, help="micro-batches per optimizer step (effective batch = batch x accum)")
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--warmup", type=int, default=30)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--alpha", type=float, default=32.0)
    ap.add_argument("--dropout", type=float, default=0.05)
    ap.add_argument("--weight-decay", type=float, default=0.0)
    ap.add_argument("--max-len", type=int, default=640)
    ap.add_argument("--eval-every", type=int, default=50, help="steps between validation-loss checks (v1 used 50)")
    ap.add_argument("--patience", type=int, default=3)
    ap.add_argument("--max-minutes", type=float, default=90.0, help="stop training after this long (keeps the best adapter)")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--samples", type=int, default=6, help="greedy answers to print for spot checks")
    ap.add_argument("--base-repo", default=None, help="the base's Hugging Face repository for NOTICE (default: from the base folder's name)")
    a = ap.parse_args()

    torch.set_num_threads(a.threads)
    torch.manual_seed(a.seed)
    rng = random.Random(a.seed)
    device = a.device
    if device.startswith("cuda") and not torch.cuda.is_available():
        sys.exit("no GPU visible to torch (ROCm: set HSA_OVERRIDE_GFX_VERSION=10.3.0 for the RX 6700 XT); use --device cpu for the slow fallback")
    dtype = torch.bfloat16 if device.startswith("cuda") else torch.float32
    a.run_dir.mkdir(parents=True, exist_ok=True)
    t_start = time.time()

    tok = Tokenizer.from_file(str(a.base / "tokenizer.json"))
    end_id = tok.token_to_id(IM_END)
    eot_id = tok.token_to_id("<|endoftext|>")
    if end_id is None:
        sys.exit("the tokenizer has no <|im_end|> token")
    pad_id = end_id
    train, sk_t = load_examples(a.data / "train.jsonl", tok, a.max_len)
    val, sk_v = load_examples(a.data / "val.jsonl", tok, a.max_len)
    print(f"examples: train {len(train)} (skipped {sk_t}), val {len(val)} (skipped {sk_v})", flush=True)
    mean_len = sum(len(p) + len(q) for p, q in train) / max(len(train), 1)

    m = tinylm.load(a.base, device, dtype)
    original_keys = list(safe_open(str(a.base / "model.safetensors"), "pt").keys())
    params = tinylm.add_lora(m, a.rank, a.alpha, a.dropout)
    m.checkpoint = not a.no_checkpoint
    n_lora = sum(p.numel() for p in params)
    n_all = sum(p.numel() for p in m.parameters())
    print(f"model {m.c.family}: {n_all / 1e6:.0f} M parameters, LoRA {n_lora / 1e6:.2f} M trainable", flush=True)
    opt = torch.optim.AdamW(params, lr=a.lr, weight_decay=a.weight_decay, betas=(0.9, 0.999))
    steps_per_epoch = math.ceil(len(train) / (a.batch * a.accum))
    total = max(1, int(steps_per_epoch * a.epochs))

    def lr_at(s):
        if s < a.warmup:
            return a.lr * (s + 1) / a.warmup
        p = (s - a.warmup) / max(1, total - a.warmup)
        return a.lr * (0.1 + 0.9 * 0.5 * (1 + math.cos(math.pi * min(p, 1.0))))

    t_load = time.time() - t_start
    base_val = evaluate(m, val, a.batch, pad_id, device)
    print(f"val loss before training: {base_val:.4f}", flush=True)
    curve = [{"step": 0, "val": base_val}]
    best, best_step, best_state, bad = base_val, 0, tinylm.lora_state(m), 0
    step, stop_reason = 0, "epochs"
    m.train()
    t_train = time.time()
    run_loss, run_tok = 0.0, 0
    done = False
    micro = 0
    while not done:
        for b in batches(train, a.batch, True, rng):
            for g in opt.param_groups:
                g["lr"] = lr_at(step)
            ids, labels = collate(b, pad_id, device)
            loss_sum, k = loss_of(m, ids, labels)
            (loss_sum / max(k, 1) / a.accum).backward()
            run_loss += float(loss_sum)
            run_tok += k
            micro += 1
            if micro % a.accum:
                continue
            torch.nn.utils.clip_grad_norm_(params, 1.0)
            opt.step()
            opt.zero_grad(set_to_none=True)
            step += 1
            if step % a.eval_every == 0 or step == total:
                v = evaluate(m, val, a.batch, pad_id, device)
                tr = run_loss / max(run_tok, 1)
                run_loss, run_tok = 0.0, 0
                el = time.time() - t_train
                curve.append({"step": step, "epoch": round(step / steps_per_epoch, 3), "train": tr, "val": v, "minutes": round(el / 60, 2)})
                print(f"step {step}/{total} epoch {step / steps_per_epoch:.2f} train {tr:.4f} val {v:.4f} ({el / 60:.1f} min)", flush=True)
                if v < best - 1e-4:
                    best, best_step, best_state, bad = v, step, tinylm.lora_state(m), 0
                else:
                    bad += 1
                    if bad >= a.patience:
                        stop_reason = f"early stop (no val improvement in {a.patience} evals)"
                        done = True
                        break
                if el > a.max_minutes * 60:
                    stop_reason = "time limit"
                    done = True
                    break
            if step >= total:
                done = True
                break
    train_min = (time.time() - t_train) / 60
    print(f"training done: {stop_reason}; best val {best:.4f} at step {best_step} ({train_min:.1f} min)", flush=True)
    m.load_state_dict(best_state, strict=False)
    m.eval()

    # spot checks: greedy answers to a few validation prompts
    stop = {end_id} | ({eot_id} if eot_id is not None else set())
    samples = []
    for p, ans in val[: a.samples]:
        out = tinylm.greedy(m, p, stop, 96, device)
        samples.append({"answer": tok.decode(out), "target": tok.decode(ans[:-1])})
        print(f"- {samples[-1]['answer']}\n  target: {samples[-1]['target']}", flush=True)

    # merge and save
    a.out.mkdir(parents=True, exist_ok=True)
    merged = tinylm.merged_state(m, original_keys)
    save_file(merged, str(a.out / "model.safetensors"), metadata={"format": "pt"})
    for f in ("config.json", "tokenizer.json", "tokenizer_config.json", "generation_config.json", "LICENSE"):
        if (a.base / f).exists():
            shutil.copy2(a.base / f, a.out / f)
    if not (a.base / "LICENSE").exists():
        # (the base folder has no licence file: every catalogue base is Apache-2.0)
        shutil.copy2(Path(__file__).resolve().parent / "LICENSE-Apache-2.0.txt", a.out / "LICENSE")
    revision = (a.base / "REVISION").read_text().strip() if (a.base / "REVISION").exists() else "unknown"
    base_repo = a.base_repo or KNOWN_REPOS.get(a.base.resolve().name, a.base.resolve().name)
    data_seed = None
    if (a.data / "data-stats.json").exists():
        data_seed = json.loads((a.data / "data-stats.json").read_text()).get("seed")
    (a.out / "NOTICE").write_text(NOTICE.format(name=a.out.resolve().name, repo=base_repo, revision=revision, seed=data_seed if data_seed is not None else "unknown"))
    stats = {}
    if (a.data / "data-stats.json").exists():
        stats = json.loads((a.data / "data-stats.json").read_text())
    manifest = {
        "base": str(a.base),
        "base_revision": revision,
        "family": m.c.family,
        "data": str(a.data),
        "data_stats": {k: stats.get(k) for k in ("seed", "grades", "per_grade", "train_examples", "val_examples")},
        "examples": {"train": len(train), "val": len(val), "mean_tokens": round(mean_len, 1)},
        "hyper": {k: getattr(a, k) for k in ("epochs", "batch", "accum", "lr", "warmup", "rank", "alpha", "dropout", "weight_decay", "max_len", "eval_every", "patience", "seed")},
        "lora_targets": list(tinylm.LORA_TARGETS),
        "lora_params": n_lora,
        "device": device,
        "dtype": str(dtype),
        "torch": torch.__version__,
        "steps": step,
        "stop_reason": stop_reason,
        "best_step": best_step,
        "val_before": base_val,
        "val_best": best,
        "curve": curve,
        "minutes": {"load": round(t_load / 60, 2), "train": round(train_min, 2), "total": round((time.time() - t_start) / 60, 2)},
        "samples": samples,
    }
    for d in (a.run_dir, a.out):
        (d / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"wrote {a.out}", flush=True)


if __name__ == "__main__":
    main()
