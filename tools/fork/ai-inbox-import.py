#!/usr/bin/env python3
"""FORK-ONLY (AI-PLAN AI5 item 8, FORK-PLAN §1a.92): import the owner's Assistant training inbox.

Usage:
  python3 -I tools/fork/ai-inbox-import.py [--inbox DIR] [--out DIR] [--dry-run]
  python3 -I tools/fork/ai-inbox-import.py --self-test

Reads the conversations FilmCraft's "Send to my training inbox" wrote (`<data dir>/assistant-inbox/
conversation-<ms>-<id>.json`, default ~/.local/share/filmcraft-fork/assistant-inbox), moves each one
(and its frame) into the inbox's `imported/` folder once read, and triages each conversation, whole,
into held-out OR training (never both; a stable hash of the file's name decides, 1 in 3 held out):
  - held-out: one `[[question]]` per turn in `<out>/heldout.toml` (kind "owner"; the owner's
    correction becomes a `must`; the answer's guide sections become `sources` when rated 👍);
  - training: one pair per turn in `<out>/training.jsonl`, answered by the owner's correction, or
    by the checked answer when rated 👍 (unrated / 👎 without a correction: skipped, no reference);
  - turns the Assistant couldn't answer (nothing found / said it doesn't know) go to `<out>/gaps.md`.
Every case carries `origin = "owner-inbox"`. The owner reads them before any tune (Q35).
Default out: `<inbox>/cases/`, OUTSIDE git (provisional, OWNER-QUESTIONS Q48): the inbox isn't
stripped, so the cases can hold client / project names, and the repo is snapshotted publicly.
Pass `--out docs/fork/coach-eval/owner-inbox` once the owner says they may be committed.
"""
import hashlib
import json
import os
import sys
import tempfile

DEFAULT_INBOX = os.path.expanduser("~/.local/share/filmcraft-fork/assistant-inbox")
DEFAULT_OUT = None  # <inbox>/cases
HELD_OUT_ONE_IN = 3
MAX_FILE_BYTES = 4 * 1024 * 1024


def toml_str(s):
    return json.dumps(s, ensure_ascii=False)


def answer_text(a):
    if not isinstance(a, dict):
        return ""
    lines = [l.get("text", "") for l in a.get("lines") or [] if isinstance(l, dict)]
    if not lines and isinstance(a.get("coach"), dict):
        lines = [s.get("text", "") for s in a["coach"].get("steps") or [] if isinstance(s, dict)]
    return " ".join(t.strip() for t in lines if t and t.strip())


def is_gap(a):
    if not isinstance(a, dict):
        return False
    fb = a.get("fallback")
    return bool(a.get("dontKnow")) or (isinstance(fb, dict) and fb.get("kind") == "nothingFound")


def held_out(key):
    return int(hashlib.sha256(key.encode()).hexdigest(), 16) % HELD_OUT_ONE_IN == 0


def load(inbox):
    """Conversation files not imported yet, keyed by file name (ids restart with every launch)."""
    convs = {}
    names = []
    for name in sorted(os.listdir(inbox)) if os.path.isdir(inbox) else []:
        path = os.path.join(inbox, name)
        if not name.endswith(".json") or not os.path.isfile(path):
            continue
        if os.path.getsize(path) > MAX_FILE_BYTES:
            print(f"skip {name}: too large", file=sys.stderr)
            continue
        try:
            conv = json.load(open(path, encoding="utf-8"))
        except (OSError, ValueError) as e:
            print(f"skip {name}: {e}", file=sys.stderr)
            continue
        if not isinstance(conv, dict) or conv.get("origin") != "owner-inbox" or not isinstance(conv.get("turns"), list):
            print(f"skip {name}: not an inbox conversation", file=sys.stderr)
            continue
        names.append(name)
        convs[name[: -len(".json")]] = conv
    return convs, names


def move_imported(inbox, names, convs):
    done = os.path.join(inbox, "imported")
    os.makedirs(done, exist_ok=True)
    for name in names:
        stem = name[: -len(".json")]
        files = [name]
        frame = convs.get(stem, {}).get("frame")
        if isinstance(frame, str) and os.path.basename(frame) == frame:
            files.append(frame)
        for f in files:
            src = os.path.join(inbox, f)
            if os.path.isfile(src):
                os.replace(src, os.path.join(done, f))


def triage(best):
    held, train, gaps = [], [], []
    for key, conv in sorted(best.items()):
        to_held = held_out(key)
        for t in conv["turns"]:
            if not isinstance(t, dict) or not str(t.get("question", "")).strip():
                continue
            q = str(t["question"]).strip()
            a = t.get("answer")
            correction = (t.get("correction") or "").strip()
            sources = a.get("sources", []) if isinstance(a, dict) else []
            case_id = f"inbox-{key}-{t.get('id', 0)}"
            if is_gap(a):
                gaps.append(f"- {case_id}: {q}" + (f" (owner: {correction})" if correction else ""))
            if to_held:
                held.append({
                    "id": case_id, "kind": "owner", "q": q,
                    "sources": sources if t.get("rating") == "up" else [],
                    "must": [correction] if correction else [], "must_not": [], "origin": "owner-inbox",
                })
            else:
                ref = correction or (answer_text(a) if t.get("rating") == "up" else "")
                if ref:
                    train.append({"q": q, "a": ref, "sources": sources, "kind": "owner", "origin": "owner-inbox", "id": case_id})
    return held, train, gaps


def write(out, held, train, gaps):
    os.makedirs(out, exist_ok=True)
    if held:
        with open(os.path.join(out, "heldout.toml"), "a", encoding="utf-8") as f:
            for c in held:
                f.write("\n[[question]]\n")
                for k in ("id", "kind", "q"):
                    f.write(f"{k} = {toml_str(c[k])}\n")
                for k in ("sources", "must", "must_not"):
                    f.write(f"{k} = [{', '.join(toml_str(x) for x in c[k])}]\n")
                f.write(f"origin = {toml_str(c['origin'])}\n")
    if train:
        with open(os.path.join(out, "training.jsonl"), "a", encoding="utf-8") as f:
            for p in train:
                f.write(json.dumps(p, ensure_ascii=False) + "\n")
    if gaps:
        with open(os.path.join(out, "gaps.md"), "a", encoding="utf-8") as f:
            f.write("\n".join(gaps) + "\n")


def run(inbox, out, dry):
    convs, names = load(inbox)
    held, train, gaps = triage(convs)
    print(f"{len(names)} conversation(s): {len(held)} held-out, {len(train)} training, {len(gaps)} gap(s)")
    if not dry:
        write(out or os.path.join(inbox, "cases"), held, train, gaps)
        move_imported(inbox, names, convs)
    return held, train, gaps


def self_test():
    with tempfile.TemporaryDirectory() as tmp:
        inbox, out = os.path.join(tmp, "inbox"), os.path.join(tmp, "out")
        os.makedirs(inbox)
        # two conversations: whichever way the hash sends them, no turn lands in both sets
        for conv_id in (3, 4):
            conv = {"format": "filmcraft-assistant-inbox/1", "origin": "owner-inbox", "conversation": conv_id, "turns": [
                {"id": conv_id, "question": "why is it flat?", "rating": "up", "correction": None,
                 "answer": {"lines": [{"text": "Add contrast with Gain."}], "sources": ["color.primaries"]}},
                {"id": conv_id + 10, "question": "and the skin?", "rating": "down", "correction": "Use the skin tone line.",
                 "answer": {"dontKnow": True, "lines": [], "sources": []}},
            ]}
            json.dump(conv, open(os.path.join(inbox, f"conversation-1-{conv_id}.json"), "w"))
        open(os.path.join(inbox, "junk.json"), "w").write("{not json")
        held, train, gaps = run(inbox, out, False)
        assert len(gaps) == 2, gaps
        assert os.path.exists(os.path.join(inbox, "imported", "conversation-1-3.json")), "moved aside"
        held_ids = {c["id"] for c in held}
        train_ids = {p["id"] for p in train}
        assert held_ids or train_ids
        assert not held_ids & train_ids, "a case in both sets"
        for c in held:
            assert c["origin"] == "owner-inbox"
            if c["q"] == "and the skin?":
                assert c["must"] == ["Use the skin tone line."] and c["sources"] == []
            else:
                assert c["sources"] == ["color.primaries"]
        for p in train:
            assert p["a"] in ("Add contrast with Gain.", "Use the skin tone line."), p
        # a second run imports nothing new
        held2, train2, _ = run(inbox, out, False)
        assert not held2 and not train2
        if held:
            text = open(os.path.join(out, "heldout.toml"), encoding="utf-8").read()
            assert 'origin = "owner-inbox"' in text
    print("self-test ok")


def main(argv):
    if "--self-test" in argv:
        self_test()
        return
    inbox, out = DEFAULT_INBOX, DEFAULT_OUT
    if "--inbox" in argv:
        inbox = argv[argv.index("--inbox") + 1]
    if "--out" in argv:
        out = argv[argv.index("--out") + 1]
    run(inbox, out, "--dry-run" in argv)


if __name__ == "__main__":
    main(sys.argv[1:])
