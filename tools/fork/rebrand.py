#!/usr/bin/env python3
"""Fork rebrand (FORK-PLAN §1a.103): every user-visible "FilmCraft" becomes "Revolution".

Idempotent: run it again after each upstream sync (UPSTREAM-PLAN) to rebrand new strings, then
`cargo xtask ci`. Internals keep their names: crate names, identifiers, comments, file formats,
interchange keys, on-disk folder names (so existing projects keep their previews and stills),
copyright / licence lines and test fixtures that stand for files written by earlier builds.

What it rewrites:
  * Rust string literals in crates/ and apps/ (not comments, not identifiers);
  * the in-app guide (docs/guide), its eval / training sets (docs/fork/coach-eval, assets/coach),
    and the UI translations (crates/ui-egui/src/i18n/*.tsv).

Usage: tools/fork/rebrand.py [--check]   (--check: print what would change, exit 1 if anything)
"""
import pathlib
import re
import subprocess
import sys

OLD, NEW = "FilmCraft", "Revolution"

# Substrings that keep the old name wherever they appear.
KEEP = [
    "FilmCraft contributors",          # copyright lines
    "FilmCraft Frame Size",            # AAF / OMF / OTIO keys read back from exported files
    "FilmCraft Audio Sample Rate",
    "FilmCraft Effect",
    "FilmCraft:SCLP",
    "FilmCraft:EFFE",
    "FilmCraftGenerator",
    "FilmCraft Previews",              # folders on disk next to existing projects
    "FilmCraft Stills",
    "FilmCraft Projects",
    "Application Support/FilmCraft",
    "based on FilmCraft",              # the credit (§1a.103)
    "Based on FilmCraft",
    "FilmCraft by the ArtCraft",
    "upstream FilmCraft",
    "FilmCraft (upstream)",
    "FilmCraft fork: ",                # titles of releases already published (recorded replies)
    "FilmCraft Default",               # pre-rebrand names read back from saved settings / projects
    "\"FilmCraft \"",
    "FilmCraft Software Only",
    "FilmCraft or another build",
]

# Files left alone entirely.
SKIP_FILES = {
    "crates/engine/src/autosave.rs",           # data folders: edited by hand (old folder fallback)
    "crates/testkit/src/golden.rs",            # writes ATTRIBUTION.md rows (copyright)
    "crates/engine/src/update_tests.rs",       # recorded release titles
    "crates/ui-egui/tests/ui/updates_ui.rs",
}
SKIP_DIRS = ("crates/ui-egui/examples/bench/", "xtask/", "crates/format/tests/fixtures/",
             "crates/matroska/tests/", "crates/isobmff/tests/")  # cached ffmpeg fixtures carry the old title

ROOT = pathlib.Path(__file__).resolve().parents[2]


def protect(s: str) -> str:
    keep = {}
    for i, k in enumerate(KEEP):
        tok = f"\x00{i}\x00"
        if k in s:
            s = s.replace(k, tok)
            keep[tok] = k
    s = s.replace(OLD, NEW)
    for tok, k in keep.items():
        s = s.replace(tok, k)
    return s


def rust_literals(src: str) -> str:
    """Rewrites OLD inside string literals only (normal, byte and raw strings); comments,
    char literals and code are copied unchanged."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(src[i:j]); i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j): depth += 1; j += 2
                elif src.startswith("*/", j): depth -= 1; j += 2
                else: j += 1
            out.append(src[i:j]); i = j
        elif (m := re.match(r'(b?r)(#*)"', src[i:])) and (i == 0 or not (src[i-1].isalnum() or src[i-1] == "_")):
            hashes = m.group(2)
            end = src.find('"' + hashes, i + len(m.group(0)))
            end = n if end < 0 else end + 1 + len(hashes)
            out.append(protect(src[i:end])); i = end
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(protect(src[i:j])); i = j
        elif c == "'":
            # char literal ('x', '\n', '\u{..}', '"') or a lifetime ('a): copy as-is
            m = re.match(r"'(\\u\{[0-9a-fA-F]+\}|\\.|[^\\'])'", src[i:])
            j = i + (len(m.group(0)) if m else 1)
            out.append(src[i:j]); i = j
        else:
            j = i + 1
            while j < n and src[j] not in "/\"'br":
                j += 1
            if j == i + 1 and c in "br":
                pass
            out.append(src[i:j]); i = j
    return "".join(out)


def files():
    tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split("\n")
    for f in tracked:
        if not f or f in SKIP_FILES or f.startswith(SKIP_DIRS):
            continue
        if f.endswith(".rs") and f.startswith(("crates/", "apps/")):
            yield f, rust_literals
        elif f.startswith("docs/guide/") and f.endswith(".md"):
            yield f, protect
        elif f.startswith(("docs/fork/coach-eval/", "assets/coach/")) and f.endswith((".toml", ".jsonl", ".md")):
            yield f, protect
        elif f.startswith("crates/ui-egui/src/i18n/") and f.endswith(".tsv"):
            yield f, protect


def main():
    check = "--check" in sys.argv
    changed = []
    for f, fn in files():
        p = ROOT / f
        try:
            src = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError):
            continue
        if OLD not in src:
            continue
        new = fn(src)
        if new != src:
            changed.append(f)
            if not check:
                p.write_text(new, encoding="utf-8")
    for f in changed:
        print(f)
    print(f"{len(changed)} files {'would change' if check else 'changed'}", file=sys.stderr)
    sys.exit(1 if check and changed else 0)


if __name__ == "__main__":
    main()
