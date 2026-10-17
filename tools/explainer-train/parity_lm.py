"""Parity check: tinylm.py's PyTorch model against FilmCraft's own forward pass (fork FORK-PLAN
§1a.97 item 1, before tuning SmolLM3-3B).

    VENV=~/Apps/filmcraft-train/.venv
    $VENV/bin/python -I tools/explainer-train/parity_lm.py --base <HF model dir> --cases out/cases.jsonl
    cargo run --release -p filmcraft-llm --features infer --example llm_greedy -- \
        <model.gguf> smollm3 out/cases.jsonl > out/app.jsonl
    $VENV/bin/python -I tools/explainer-train/parity_lm.py --base <HF model dir> --cases out/cases.jsonl \
        --app out/app.jsonl

The first call writes a few chat prompts as token ids (the model's own tokenizer); the second runs
greedy decoding in tinylm (bf16 on the GPU, or float32 on the CPU) and compares each case's
generated ids with the app's, printing how many tokens agree. The GGUF is quantized (Q8_0), so
a late divergence on a near-tie is expected; the first tokens must agree.
"""

import argparse
import json
from pathlib import Path

import torch
from tokenizers import Tokenizer

import tinylm

QUESTIONS = [
    "How do I add film grain to a clip?",
    "What does the Lift wheel change?",
    "My footage looks too blue. What should I adjust first?",
    "Explain what a proxy file is in one sentence.",
    "How do I export the timeline as a GIF?",
]


def prompt_ids(tok: Tokenizer, q: str) -> list:
    text = f"<|im_start|>user\n{q}<|im_end|>\n<|im_start|>assistant\n"
    return tok.encode(text, add_special_tokens=False).ids


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", type=Path, required=True)
    ap.add_argument("--cases", type=Path, required=True)
    ap.add_argument("--app", type=Path, help="the app's greedy ids (llm_greedy output); compare when given")
    ap.add_argument("--max-new", type=int, default=32)
    ap.add_argument("--device", default="cuda")
    a = ap.parse_args()
    tok = Tokenizer.from_file(str(a.base / "tokenizer.json"))
    stop = [i for i in (tok.token_to_id("<|im_end|>"), tok.token_to_id("<|end_of_text|>")) if i is not None]
    if a.app is None:
        a.cases.parent.mkdir(parents=True, exist_ok=True)
        with a.cases.open("w") as f:
            for q in QUESTIONS:
                f.write(json.dumps({"ids": prompt_ids(tok, q), "stop": stop}) + "\n")
        print(f"wrote {len(QUESTIONS)} cases to {a.cases}")
        return
    dtype = torch.bfloat16 if a.device != "cpu" else torch.float32
    m = tinylm.load(a.base, a.device, dtype)
    m.eval()
    cases = [json.loads(l) for l in a.cases.read_text().splitlines() if l.strip()]
    apps = [json.loads(l)["ids"] for l in a.app.read_text().splitlines() if l.strip()]
    if len(cases) != len(apps):
        raise SystemExit(f"{len(cases)} cases but {len(apps)} app answers")
    first_ok = 0
    for c, app in zip(cases, apps):
        ours = tinylm.greedy(m, c["ids"], set(c["stop"]), max_new=a.max_new, device=a.device)
        same = 0
        for x, y in zip(ours, app):
            if x != y:
                break
            same += 1
        first_ok += int(same >= min(8, len(app)))
        print(f"agree {same:3d} / {max(len(ours), len(app)):3d}  ours: {tok.decode(ours)!r}")
        if same < len(app):
            print(f"{'':20s}app:  {tok.decode(app)!r}")
    print(f"PARITY: {first_ok} / {len(cases)} cases agree on their first 8 tokens")


if __name__ == "__main__":
    main()
