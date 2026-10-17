#!/usr/bin/env python3
"""Choose AI-POOL's daily sample (AI-PLAN AI-POOL: ~150 held-out questions for a quick GPU run).

Usage: python3 -I tools/fork/ai-pool-daily-sample.py docs/fork/coach-eval [150]

Deterministic: every batch gets a share of the slots in proportion to its held-out `[[question]]`
count (at least one), and inside a batch the picks are evenly spaced over its questions sorted by
kind and style, so each batch's mix of kinds and styles is kept; the owner's verbatim questions (questions.toml, keyword-coach format) are always in,
listed separately because they are not in the advanced format. Writes pool/daily-sample.toml;
rerun after new batches.
"""
import glob, os, sys, tomllib

ev = sys.argv[1].rstrip('/') + '/'
target = int(sys.argv[2]) if len(sys.argv) > 2 else 150
owner = [q['id'] for q in tomllib.load(open(ev + 'questions.toml', 'rb')).get('question', []) if q.get('origin') == 'owner-verbatim']
batches = []
for f in sorted(glob.glob(ev + 'pool/*.toml')):
    if os.path.basename(f) == 'daily-sample.toml':
        continue
    qs = tomllib.load(open(f, 'rb')).get('question', [])
    if qs:
        batches.append((os.path.basename(f)[:-5], qs))
total = sum(len(qs) for _, qs in batches)
slots = max(len(batches), target - len(owner))
# largest-remainder shares, at least one per batch
raw = [(name, len(qs) * slots / total) for name, qs in batches]
share = {name: max(1, int(r)) for name, r in raw}
left = slots - sum(share.values())
for name, r in sorted(raw, key=lambda x: -(x[1] - int(x[1]))):
    if left <= 0:
        break
    share[name] += 1
    left -= 1
picked = []
for name, qs in batches:
    n = min(share[name], len(qs))
    # evenly spaced over the batch sorted by kind and style (file order inside), so the mix is kept
    ordered = [q['id'] for _, q in sorted(enumerate(qs), key=lambda iq: (iq[1]['kind'], iq[1].get('style', ''), iq[0]))]
    picked.append((name, [ordered[int((j + 0.5) * len(ordered) / n)] for j in range(n)]))
with open(ev + 'pool/daily-sample.toml', 'w') as out:
    out.write('# AI-POOL daily sample (AI-PLAN AI-POOL), chosen by tools/fork/ai-pool-daily-sample.py:\n')
    out.write('# held-out [[question]] ids from every pool batch, spread over kinds and styles, plus the\n')
    out.write("# owner's verbatim questions (questions.toml, keyword-coach format: their `facts` are the\n")
    out.write('# facts a right answer contains). Not a batch: no [[question]] tables here.\n\n')
    out.write('owner = [' + ', '.join(f'"{i}"' for i in owner) + ']\n\nids = [\n')
    for name, ids in picked:
        out.write(f'  # {name}\n')
        for i in ids:
            out.write(f'  "{i}",\n')
    out.write(']\n')
print(len(owner), 'owner +', sum(len(i) for _, i in picked), 'pool ids from', len(picked), 'batches')
