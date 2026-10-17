#!/usr/bin/env python3
"""Draft AI3 training pairs from the fork guide (AI-PLAN AI3 0c; AI3-PREP part 4).

Usage: python3 -I tools/fork/ai3-training-pairs.py docs/guide docs/fork/coach-eval/training-pairs.jsonl

Every pair comes from the guide only (our own words): practice notes give "how do I" and
"In Resolve / Premiere I would use X" questions answered with the FilmCraft way, why and pitfalls;
feature sections give "what does X do" and "what should I use for <when>" questions answered with
the header's does-line and the first steps. Drafts for human review before tuning (TRAINING.md).
"""
import re, json, glob, sys
root = sys.argv[1]; out = sys.argv[2]
def strip_md(t):
    t = re.sub(r'\s*\(\s*`?\{ui:[^}]*\}`?\s*\)', '', t)
    t = re.sub(r'`?\{ui:[^}]*\}`?', '', t)
    t = re.sub(r'\s*\(\s*``\s*\)', '', t).replace('``', '')
    t = re.sub(r'\(\s*\)', '', t)
    t = re.sub(r'\[([^\]]+)\]\([^)]*\)', r'\1', t)
    t = t.replace('**', '')
    t = re.sub(r'\s+', ' ', t).strip()
    return t
def sections(path):
    page = path[len(root)+1:]
    src = open(path).read()
    fence = False; cur = None; out = []
    for line in src.split('\n'):
        if line.lstrip().startswith('```'): fence = not fence
        if not fence and line.startswith('## '):
            if cur: out.append(cur)
            cur = {'page': page, 'heading': line[3:].strip(), 'header': None, 'body': []}
            continue
        if cur is None: continue
        if cur['header'] is None and line.startswith('> id:'):
            cur['header'] = line; continue
        cur['body'].append(line)
    if cur: out.append(cur)
    return out
def parse_header(h):
    d = {}
    for f in h[2:].split(' | '):
        k, v = f.split(':', 1); d[k.strip()] = v.strip()
    return d
VERBS = set('normalise balance set shape check qualify deepen put finish match keep cut lay leave use expose grade deliver upload tone name copy organise label save archive hand write make turn fit record review read pick protect bring add remove move place trim select edit export import show change animate burn transform'.split())
pairs = []
for path in sorted(glob.glob(root + '/**/*.md', recursive=True)):
    if path.endswith('README.md') and '/practice/' not in path and path.count('/') == root.count('/') + 1: continue
    for s in sections(path):
        if not s['header']: continue
        h = parse_header(s['header'])
        sid = h['id']; body = '\n'.join(s['body'])
        practice = '/practice/' in path
        if practice:
            fields = {}
            for m in re.finditer(r'^- \*\*([^:*]+):\*\* (.*?)(?=^- \*\*|\Z)', body, re.S | re.M):
                fields[m.group(1).strip().lower()] = strip_md(m.group(2))
            fc = fields.get('filmcraft')
            if not fc: continue
            parts = [f"In FilmCraft: {fc}"]
            if 'why' in fields: parts.append(f"Why: {fields['why']}")
            if 'pitfalls' in fields: parts.append(f"Watch out: {fields['pitfalls']}")
            answer = ' '.join(parts)
            head = s['heading']
            first = head.split()[0].lower().rstrip(',')
            q1 = f"How do I {head[0].lower() + head[1:]}?" if first in VERBS else f"How do people handle \"{head.lower()}\" in FilmCraft?"
            pairs.append({'q': q1, 'a': answer, 'sources': [sid], 'kind': 'practice'})
            for app in ('resolve', 'premiere'):
                v = h.get(app, '')
                if v and not v.startswith('—'):
                    name = 'Resolve' if app == 'resolve' else 'Premiere'
                    pairs.append({'q': f"In {name} I'd use {v}. How do I do that in FilmCraft?", 'a': answer, 'sources': [sid], 'kind': 'mapping'})
        else:
            steps = re.findall(r'^\d+\. (.*?)(?=^\d+\. |^\S|\Z)', body, re.S | re.M)
            text = strip_md(' '.join(steps[:4])) if steps else ''
            if not text:
                paras = [p for p in body.split('\n\n') if p.strip() and not p.strip().startswith(('**[', '|', '>', '#', '-'))]
                text = strip_md(paras[0]) if paras else ''
            if not text: continue
            answer = f"{h['does'][0].upper() + h['does'][1:]}. {text}"
            pairs.append({'q': f"What does {s['heading']} do, and how do I use it?", 'a': answer, 'sources': [sid], 'kind': 'feature'})
            if h.get('when'):
                pairs.append({'q': f"What should I use for {h['when'].split(';')[0].strip().rstrip('.')}?", 'a': answer, 'sources': [sid], 'kind': 'feature'})
# Held out (AI-POOL): drop a pair whose question is a test question or a near-copy of one, by the
# same rule as crates/engine/tests/ai_pool.rs (content words, Jaccard >= 0.7).
import tomllib, os
STOP = set('a an and are can d do does how i if in is it my of on or s should that the this to what when where which why with'.split())
def words(q):
    w = re.findall(r'[a-z0-9]+', q.lower())
    return set(x for x in w if x not in STOP) or set(w)
ev = os.path.join(os.path.dirname(out), '')
tests = []
for tf in [ev + 'advanced.toml', ev + 'questions.toml'] + sorted(glob.glob(ev + 'pool/*.toml')):
    d = tomllib.load(open(tf, 'rb'))
    tests += [words(q['q']) for k in ('question', 'coach') for q in d.get(k, [])]
def near(q):
    w = words(q)
    return any(len(w & t) / max(1, len(w | t)) >= 0.7 for t in tests)
dropped = [p['q'] for p in pairs if near(p['q'])]
pairs = [p for p in pairs if not near(p['q'])]
if dropped: print('held out (near a test question):', dropped)
with open(out, 'w') as f:
    for p in pairs: f.write(json.dumps(p, ensure_ascii=False) + '\n')
import collections
print(len(pairs), collections.Counter(p['kind'] for p in pairs))
