"""LoRA tune of Qwen3-VL's text model on picture descriptions (fork AI-PLAN AI4 item 3).

Offline tooling only. Inputs come from FilmCraft itself:
  1. `vision_traindata` (frames + labels) and
  2. `vision_traindump` (token ids, M-RoPE positions, picture rows + DeepStack taps from our own
     encoder, through the app's own prompt code),
so this script only runs the text model (`qwen3vl.py`). The encoder stays frozen.

  HSA_OVERRIDE_GFX_VERSION=10.3.0 ~/Apps/filmcraft-train/.venv/bin/python tools/explainer-train/train_vl.py \
      --release ~/.local/share/filmcraft-fork/models/qwen3-vl-4b-instruct \
      --data ~/Apps/filmcraft-train/runs/vision-data-v1 \
      --run-dir ~/Apps/filmcraft-train/runs/vision-v1 \
      --out ~/.local/share/filmcraft-fork/models/tuned/qwen3-vl-4b-look-v1

`--check 3` only prints greedy answers for 3 validation frames (base model) and stops: compare
them with the app's answers (`vision_traindump --ask 3`) before training.

Output folder (`--out`): the merged text weights (`model-tuned-text.safetensors`), a copy of the
release's index pointing those tensors there, symlinks to the release's other files (config,
tokenizer, the shards holding the picture encoder), `manifest.json`. Convert it for the app with
`llm_quantize <out> <out>/text-own-q8_0.gguf --type q8_0`.
"""

import argparse
import json
import math
import mmap
import os
import random
import time
from pathlib import Path

import torch
from safetensors.torch import save_file
from tokenizers import Tokenizer

import qwen3vl


class Inputs:
    """One split of `vision_traindump`'s output."""

    def __init__(self, data: Path, split: str, hidden: int):
        self.rows = [json.loads(l) for l in (data / f"inputs-{split}.jsonl").read_text().splitlines() if l.strip()]
        self.hidden = hidden
        f = open(data / f"inputs-{split}.bin", "rb")
        size = os.fstat(f.fileno()).st_size
        self.bin = torch.frombuffer(mmap.mmap(f.fileno(), size, access=mmap.ACCESS_READ), dtype=torch.bfloat16) if size else torch.empty(0, dtype=torch.bfloat16)

    def __len__(self):
        return len(self.rows)

    def get(self, i: int, device):
        r = self.rows[i]
        n, taps, off, hdim = r["count"], r["taps"], r["offset"], self.hidden
        block = self.bin[off : off + n * hdim * (1 + taps)].view(1 + taps, n, hdim).to(device)
        ids = torch.tensor(r["ids"], device=device)
        pos = torch.tensor(r["positions"], device=device)
        return ids, pos, block[0], [block[k] for k in range(1, 1 + taps)], r["answer_start"]


def loss_of(m, ids, pos, pic, taps, start):
    h = m.hidden(ids, pos, pic, taps)
    # the token at i predicts i + 1: the answer's tokens are predicted from start - 1 on
    logits = m.logits(h[start - 1 : -1])
    return torch.nn.functional.cross_entropy(logits, ids[start:])


@torch.no_grad()
def greedy(m, ids, pos, pic, taps, start, stop, max_new=80):
    ids, pos = ids[:start].clone(), pos[:start].clone()
    out = []
    nxt_pos = int(pos[:, :].max().item()) + 1
    for _ in range(max_new):
        h = m.hidden(ids, pos, pic, taps)
        t = int(m.logits(h[-1:]).argmax(-1).item())
        if t in stop:
            break
        out.append(t)
        ids = torch.cat([ids, torch.tensor([t], device=ids.device)])
        pos = torch.cat([pos, torch.tensor([[nxt_pos] * 3], device=pos.device)])
        nxt_pos += 1
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--release", type=Path, required=True)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--run-dir", type=Path, required=True)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--epochs", type=float, default=1.0)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--r", type=int, default=16)
    ap.add_argument("--alpha", type=float, default=32.0)
    ap.add_argument("--dropout", type=float, default=0.05)
    ap.add_argument("--accum", type=int, default=8)
    ap.add_argument("--val-every", type=int, default=50, help="optimizer steps between validation runs")
    ap.add_argument("--val-max", type=int, default=120)
    ap.add_argument("--patience", type=int, default=3)
    ap.add_argument("--check", type=int, default=0)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    a.run_dir.mkdir(parents=True, exist_ok=True)
    log = open(a.run_dir / "train.log", "a")

    def say(*x):
        line = " ".join(str(v) for v in x)
        print(line, flush=True)
        log.write(line + "\n")
        log.flush()

    torch.manual_seed(a.seed)
    random.seed(a.seed)
    dev = "cuda" if torch.cuda.is_available() else "cpu"
    t0 = time.time()
    m = qwen3vl.load(a.release, dev, torch.bfloat16)
    say(f"loaded text model on {dev} in {time.time() - t0:.0f} s")
    train, val = Inputs(a.data, "train", m.c.hidden), Inputs(a.data, "val", m.c.hidden)
    tok = Tokenizer.from_file(str(a.release / "tokenizer.json"))
    stop = {tok.token_to_id("<|im_end|>"), tok.token_to_id("<|endoftext|>")}

    if a.check:
        m.eval()
        for i in range(min(a.check, len(val))):
            ids, pos, pic, taps, start = val.get(i, dev)
            say(val.rows[i]["id"], "->", tok.decode(greedy(m, ids, pos, pic, taps, start, stop)))
            say("   label:", tok.decode([int(x) for x in ids[start:].tolist()]))
        return

    params = qwen3vl.add_lora(m, a.r, a.alpha, a.dropout)
    m.checkpoint = True
    opt = torch.optim.AdamW(params, lr=a.lr, weight_decay=0.0)
    total = max(1, int(len(train) * a.epochs / a.accum))
    warm = min(30, total // 10)

    def lr_at(s):
        if s < warm:
            return a.lr * (s + 1) / warm
        p = (s - warm) / max(1, total - warm)
        return a.lr * (0.1 + 0.9 * 0.5 * (1 + math.cos(math.pi * p)))

    @torch.no_grad()
    def val_loss():
        m.eval()
        ls = [float(loss_of(m, *val.get(i, dev))) for i in range(min(a.val_max, len(val)))]
        m.train()
        return sum(ls) / max(1, len(ls))

    best, best_state, bad, step = val_loss(), None, 0, 0
    say(f"base val loss {best:.4f}; {len(train)} train / {len(val)} val frames, {total} steps")
    m.train()
    order = []
    while step < total:
        if not order:
            order = list(range(len(train)))
            random.shuffle(order)
        opt.zero_grad(set_to_none=True)
        acc = 0.0
        for _ in range(a.accum):
            if not order:
                break
            l = loss_of(m, *train.get(order.pop(), dev)) / a.accum
            l.backward()
            acc += float(l)
        for g in opt.param_groups:
            g["lr"] = lr_at(step)
        torch.nn.utils.clip_grad_norm_(params, 1.0)
        opt.step()
        step += 1
        if step % 10 == 0:
            say(f"step {step}/{total} loss {acc:.4f} lr {lr_at(step):.2e} {time.time() - t0:.0f} s")
        if step % a.val_every == 0 or step == total:
            v = val_loss()
            say(f"step {step} val loss {v:.4f} (best {best:.4f})")
            if v < best:
                best, bad = v, 0
                best_state = {k: t.detach().clone() for k, t in m.state_dict().items() if ".lora_" in k}
                # on disk too: a run cut off later keeps its best adapter
                torch.save(best_state, a.run_dir / "best-lora.pt")
            else:
                bad += 1
                if bad >= a.patience:
                    say("early stop")
                    break
    if best_state is None:
        say("no step beat the base model's validation loss: nothing written")
        return
    m.load_state_dict(best_state, strict=False)
    if a.out:
        write(a, m, best, step, say)


def write(a, m, best, step, say):
    out = a.out
    out.mkdir(parents=True, exist_ok=True)
    merged = qwen3vl.merged_text_state(m)
    save_file(merged, str(out / "model-tuned-text.safetensors"))
    index = json.loads((a.release / "model.safetensors.index.json").read_text())
    for k in merged:
        index["weight_map"][k] = "model-tuned-text.safetensors"
    (out / "model.safetensors.index.json").write_text(json.dumps(index, indent=1))
    for f in a.release.iterdir():
        if f.name in ("model.safetensors.index.json",) or f.suffix == ".gguf" or f.name.startswith("model-tuned"):
            continue
        link = out / f.name
        if not link.exists():
            link.symlink_to(f)
    (out / "manifest.json").write_text(json.dumps({
        "base": str(a.release.name), "kind": "picture-look LoRA (text model only)", "r": a.r, "alpha": a.alpha,
        "lr": a.lr, "epochs": a.epochs, "steps": step, "best_val_loss": best, "data": str(a.data.name),
    }, indent=1))
    say(f"wrote {len(merged)} merged tensors to {out}")


if __name__ == "__main__":
    main()
