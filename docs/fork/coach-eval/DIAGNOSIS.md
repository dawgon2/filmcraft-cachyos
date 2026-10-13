# Ask the coach: why it misses, and what to change (C1 item 6 prep)

*2026-10-12, prepared ahead of COMPOSITOR-PLAN C1 item 6 (FORK-PLAN §1a.28 (a), §1a.40 (b)).
Nothing here was built or run with cargo: the predictions come from a line-by-line Python port of
`crates/engine/src/coach_ask.rs` (tokeniser, synonyms, section split, BM25, boosts, thresholds)
run over `docs/guide/` as of commit `5b27f3ab`. The port reproduces the owner's logged numbers
for miss 1 (coverage 0.667, confidence 0.161 vs the logged 0.163) and the state of the existing
30-question test (28/30 best-first, the test's bar). Treat every number as a prediction to
confirm with the real test on the C1 lane.*

Companion files: [`questions.toml`](questions.toml) (116 questions: 100 the guide should answer,
6 partial, 7 it must decline + the 8 existing off-topic ones), [`ASSISTANT-PLAN-DRAFT.md`](ASSISTANT-PLAN-DRAFT.md).

## 1. Predicted state today

| Set | Pass | Rule |
|---|---|---|
| `coach-eval/questions.toml`, 116 questions | **67 / 116 (58 %)** | best section is the expected one (or an `alt`) and `covered = true`; `uncovered` ones must fall back |
| …of which the owner's two verbatim misses | 0 / 2 | |
| …of which the 9 paraphrases of them | 4 / 9 | |
| `assets/coach/questions.toml`, existing 30 | 28 / 30 | matches the test's `hits >= 28` (the two misses are "why did my skin go grey" → recipe 8, added after the test, and "how do I save a still" → Stills gallery; both are better answers than the expected ones, so the expected sections should be updated, not the code) |
| existing 8 off-topic | 8 / 8 decline | |

Of the 49 failures: 27 are "the right section ranks first (or close) but the **coverage gate**
(`MIN_COVERAGE` 0.75) refuses it" because one or two of the question's words are not in that
section; 19 are a **wrong best section**; 3 are questions the guide doesn't answer that the
coach answers anyway. Per-question outcomes are in the appendix.

## 2. How the coach answers today (what matters for the misses)

1. **Tokenise**: lower case, `colour → color`, stop-word list, a small stemmer, words ≥ 2 chars.
   Stop words are removed **before** stemming only, so `whats` → `what` survives (`what` is a stop
   word), as do `no`, `vs`, `thats`, `dont`, `im`.
2. **Synonyms** (`SYNONYMS`, 74 entries) expand a question word into guide words at half weight.
   There are no entries for order words (first / last / order / before / after / tackle / start),
   for "what is it for" words (useful / purpose / good for), for Resolve's or Premiere's names
   beyond `shortcut`, nor for `gradient`, `pop`, `dark`, `reduce`, `difference`.
3. **Sections**: one document per `##` heading (`###` folded in). The text under a page's `#`
   title also becomes a document whose heading *is the page title* — so the intro of
   `secondaries.md` ("Secondaries: qualifier, windows, tracking") carries "qualifier" at heading
   weight **and** in the body line `page_title`, and beats "The qualifier: select by colour"
   for "what is a qualifier for". Same for `render-cache.md` and `noise.md`. Intros have no steps,
   so the answer degrades to a summary + lesson. Stub sections whose body is `*To write.*`
   (`getting-started.md › Install`, `› Import X-M5 clips`) are indexed too: "how do I import my
   X-M5 clips" answers from the stub with confidence 0.578 and summary "To write."
4. **BM25** (k1 1.2, b 0.5) with heading words at 5× body weight. Five is a lot: a generic verb
   in a heading dominates ("Steps: **fix** a flat F-Log2 clip" beats the WB section for "how do
   I **fix** a colour cast"; "6. **Give** one grade to a whole scene" wins "give me the simplest
   first grade"; "Highlight soft **clip**" wins "apply the same grade to all my **clips**").
   Eight pages have a `## Why` and three a `## Steps`, so those headings carry nothing.
   `glossary.md › At a glance` is a table of Resolve / Premiere names: it wins every question
   that uses one ("comparison view", "key output") and has no steps.
5. **Boosts**: ×1.15 for sections with steps on a "how" question; up to ×1.3 when the heading's
   words are all in the question (one-word headings only on "what" questions). Both are mild
   next to the 5× heading weight.
6. **Gates**: `confidence = best score / ceiling ≥ 0.15` and `coverage ≥ 0.75`, where coverage
   is the share of the question's *own* words found in the best section (plus its following
   "Steps" section). With four content words one miss gives 0.75 (passes), with three one miss
   gives 0.667 (fails). Coverage looks at the section only, never the page, and counts every
   non-stop word the same: `chain`, `point`, `instead`, `again`, `switch`, `no` each refused an
   otherwise correct answer in this set. The confidence ceiling counts out-of-vocabulary words at
   the maximum idf, so a single word the guide never uses (`tackle`, `useful`, `pop`) also drags
   confidence towards the floor.
7. The model (G2.11) is never used; the lesson index only picks the "closest lesson" line.

## 3. The failures, by cause

**A. Coverage refuses a correct first section (27 of 49).** Examples: `owner-1-order` (best,
tackle; the starter tree is second), `owner-2-qualifier` (useful, cli), `order-lut` (chain),
`order-wb` (point), `order-secondaries` (before, stemmed to `befor`), `why-no-parallel` (no),
`how-nr-shadows` (`reduce` vs the guide's `reduction`: the stemmer has no -tion rule),
`what-tnr-vs-nr` (difference, between), `owner-2-para-b` (`qualifer` → `qualif`, not `qualifi`).
The gate was built to refuse off-topic questions ("add music") and does; it also refuses any
on-topic question phrased with one word the section lacks.

**B. A page intro, a stub or the glossary table outranks the real section (8).**
`owner-2-para-a`, `what-render-cache`, `cutoff-order` (intro sections); `uncovered-import`
(stub); `premiere-comparison-view`, `resolve-key-output` (At a glance); `resolve-smart-cache`,
`why-grain-missing` (one-word glossary entries beating the page with the steps on a "how/why"
question; acceptable as an `alt` only when the glossary entry has the fact).

**C. A generic heading word dominates (6).** `how-cast`, `order-six-steps`,
`how-same-grade-all`, `how-insert-before` ("first" in "First grade in six steps"),
`how-reset-node`, `how-side-by-side` ("compare" in "Compare shots…").

**D. Synonym drift (4).** `how-washed-out` (`washed → saturation contrast`, `gray → qualifier
matte hue` → recipe 8 instead of Interpret Footage), `how-lut-file` (`download` → AI
explanations), `how-hsl-secondary` (`select → qualifier` → glossary Key), `how-vignette`
(`vignette → window`; acceptable).

**E. Vocabulary the guide doesn't use (3 that stay covered).** `how-gradient-sky` (gradient /
graduated), `slang-pop`, `typo-stil`.

**F. The guide answers in prose the coach can't surface (the 6 `partial` ones).** `grade.moveNode`,
`grade.clear`, keyframes (`grade.toggleAnimation`), the node-tree-as-still trick, "grain only at
100 %", and the starter tree without noise reduction. Steps come only from the first numbered
list, so a fact in a trailing paragraph is read by nobody. These are guide gaps (§6), not
retrieval bugs.

**G. Questions the guide doesn't answer but the coach does (3).** `uncovered-edit-then-grade`
→ the Workflow section (cut, edit, grade all occur there), `uncovered-import` (stub),
`uncovered-album` (borderline). No word-overlap gate can tell "do I grade before or after I
cut" from a section about the Resolve workflow; only a reader (or a model) can.

## 4. The fix, in layers, with measured effect

Each layer was added to the port and run against the 116, the existing 30 and the 8 off-topic
questions. Only layers 1–2 are free of regressions; 3–4 are a trade.

| Layer | Change | 116-set | existing 30 | off-topic (must decline) |
|---|---|---|---|---|
| 0 | today | 67 (58 %) | 28/30 | 8/8 |
| 1 | **hygiene**: stop words checked after stemming + `no not vs whats thats hows wheres dont cant im ive isnt wont ok okay yes`; fold a page's `#` intro into its first `##` section (ranking keeps the words, the answer is the real section); skip sections whose body is only `*To write.*`; don't answer from `glossary.md › At a glance` (use its rows as synonyms instead) | 73 (63 %) | 28/30 | 8/8 |
| 2 | **synonyms**, curated one by one: order words (`order first last tackle begin` → `order starter tree`; *not* `start`, it pulls "Start a lesson"), `useful purpose` → `use why`, `gradient` → `linear graduated`, `pop punch punchy` → `contrast saturation`, `dark` → `gamma gain lift`, `reduce` ↔ `reduction`, `soften` → `soft`, `layout` → `workflow page`, `instead` → `qualifier window`, Resolve / Premiere names from the glossary table (`power` → window, `corrector` → node, `optimized` → proxy, `smart` → render cache, `comparison` → wipe split compare), and the owner's typos / cut-offs (`qualifer`, `stil`, `cli` → clip, `fa` → face). *Not* `difference / between / vs` → `compare`: that drags "X vs Y" questions to any section that says "compare" (measured: it breaks `what-disable-vs-bypass`, `what-lumetri-vs-nodes`). *Not* `album` → gallery (makes the album question answer). | 81 (70 %) | 28/30 | 8/8 |
| 3 | coverage counted over the **whole page** of the best section, not the section alone | 85 (73 %) | 28/30 | 7/8 ("add a title with my name" → coach.md, where title / name / add all occur) |
| 4 | **specific-term gate** instead of the 0.75 rule: covered when confidence ≥ 0.15 and (coverage ≥ 0.75, or at least one matched term has idf ≥ 1.3 — a word in fewer than ~a quarter of sections — and half the in-vocabulary words match) | 88 (76 %) | 28/30 | 6/8 (+ "cut a clip in half" → proxies, `half`) |

So the **recommended C1 fix is layers 1 + 2 plus the two structural items below**, which the
port could not measure but which address causes A, C and G directly:

- **Question-type routing** (a small data file, `assets/coach/routes.toml`, same spirit as
  `hints.toml`): a route is a set of trigger words and a list of sections. When a question hits
  a route, those sections get a fixed boost (×2 on their BM25 score) and the coverage gate is
  computed over them *and* the words of the route count as covered. Routes to write first:
  `order` (first, last, order, before, after, tackle, then, sequence-of-steps words →
  `nodes.md › A starter tree for F-Log2`, `recipes.md › 1.`, `noise.md › Why`, `finishing.md ›
  Highlight soft clip`), `what-for` ("what is X for", "useful for", "good for", "when do I use"
  → the section whose heading contains X, else the glossary entry X), `x-vs-y` ("difference
  between", "vs", "or"): require **both** X and Y in the section (conjunctive match) instead of
  synonyms, and `not-yet` ("is there", "do you have", "where is" + a word in no section → decline
  with "The guide doesn't mention X"). Routes don't invent anything: they only choose among guide
  sections, and the id check stays. They need the regression set to keep honest: each route is
  a line in `questions.toml`.
- **Heading weight by idf**: a heading word counts 5× only when it is specific (idf ≥ 1.0);
  generic verbs (fix, give, make, add, clip, compare, steps, why) count 1×. Measured by eye on
  the port: fixes `how-cast`, `order-six-steps`, `how-same-grade-all`, `how-insert-before`,
  `how-reset-node` without touching the existing 30 (their headings are specific).
- **Answer from the whole section when the steps aren't the answer**: for `what-for` and `why`
  questions return the section's "Why" paragraph (or the bullet that mentions the question's
  nouns) as the summary ahead of the steps. This is presentation, not retrieval, but it is what
  makes `owner-2-qualifier` a good answer without clip facts (COACH-QUESTIONS.md entry 2).

Expected after all of it: the two owner misses pass; roughly 95–100 of the 116 (82–86 %) with
the guide as it is, the rest being the `partial` and `uncovered` ones that need the guide gaps in
§6 written. The remaining "G" cases (a confident wrong answer to an off-guide question) are a
job for the assistant (§5), not for word matching.

### Options considered and not recommended for C1

- **Section-level BM25 with field weights / BM25F**: that *is* the current design (heading as a
  field at 5×); the bug is the weight and the extra documents, not the model.
- **Embeddings** (a small sentence-embedding model): would catch paraphrases (`tackle`, `useful
  for`) but adds a model download to every install of the coach, a 50–100 ms per-question cost on
  CPU, and non-determinism in tests; the guide is 19 pages, a route table covers the question
  types a grading guide gets. Keep for the assistant lane if the route table stops scaling.
- **Lowering `MIN_COVERAGE` to 0.5 or `MIN_CONFIDENCE`**: measured: every off-topic question
  starts answering. The gate is right; the counting is wrong.
- **Using G2.11's local model to rerank the top 5 now**: it would fix cause G (and a few of A)
  because a model reads "do I grade before or after I cut" and sees no section answers it. But
  the model is optional and absent on most installs, answers in 2.5–15 s (AI explanations'
  measured range), and a rerank must stay deterministic for tests. So: in C1 the coach stays
  keyword-only and deterministic; the model enters with §1a.28 (b) as a *second* answerer with
  its own checks (ASSISTANT-PLAN-DRAFT.md), and the keyword coach remains the fallback that
  every install has.

## 5. What this set can't measure

- Whether the *steps* are the right ones: the set names the section and the key facts; a reviewer
  (or the C1 test, by substring) checks the facts. Today's extraction takes the section's first
  numbered list, so for `what-for` questions it returns steps when the "Why" paragraph was the
  answer.
- Clip-dependent answers (`needs_clip = true`, 12 questions): the right answer changes with the
  picture on screen (the owner's qualifier question, "why is my LUT wrong", "why did the picker
  refuse"). The keyword coach can only give the general section; the assistant plan takes these.

## 6. Guide gaps worth writing (in the order they unblock questions)

1. **`nodes.md › A starter tree for F-Log2`**: one complete order, including where **noise
   reduction** (node 0 / first), **secondaries** (after saturation), **LUT**, **soft clip** and
   **grain** go, and *why* each sits there ("each node sees the one before"). Today the tree
   stops at "Look" and NR is only in noise.md. Unblocks `cutoff-order`, `owner-1-*` answers that
   should mention NR.
2. **A workflow overview** (getting-started.md, or `workflow.md`): edit → lock → grade →
   deliver; where proxies, the render cache and the Resolve pages fit. The coach has nothing to
   say to "do I grade before or after I cut" and answers wrongly. Also the only place to say
   "use the original (Auto proxies) when judging a grade".
3. **`nodes.md`**: a `## Move a node` subsection with numbered steps for `grade.moveNode` (and the
   drag when it exists); a `## Remove the whole grade` line for `grade.clear` / Bypass.
4. **`primaries.md`**: a `## Keyframes` subsection (`grade.toggleAnimation`, no button yet).
5. **`recipes.md`**: "9. Reuse a node tree" (still → `stills.apply`, Copy / Paste / Append; no
   preset yet), so `how-node-preset` has steps.
6. **`finishing.md › Film grain`**: say grain is judged at 100 % and not visible at 1/2 or 1/4
   playback resolution, and that a disabled node or a bypassed grade shows none.
7. **`finishing.md › Scopes`** (listed "to write"): waveform / parade / vectorscope and where skin
   and white sit on the DaVinci Intermediate scale.
8. **`matching.md › Stills gallery`**: say there are no albums / memories / versions (the "If
   you know Resolve" line says it at the page end; the gallery section should).
9. **`getting-started.md`**: write `Install` and `Import X-M5 clips` or remove the stubs; until
   then the retrieval fix skips them.
10. **Glossary**: entries for **secondary**, **primary**, **qualifier** (the glossary has Key and
    Matte but not the tool's name), **pivot**, **soft clip**, **temporal / spatial**.

## Appendix: per-question prediction (today, and after layers 1 + 2)

"fail" rows name the mechanism; `conf / cov` are today's values. `alt` sections count as a pass.

| id | status | today | conf / cov | best section today | after L1+L2 |
|---|---|---|---|---|---|
| `owner-1-order` | covered | **fail**: under threshold, missing best, tackle | 0.161 / 0.667 | color/README.md › The idea in one minute | pass |
| `owner-1-para-a` | covered | **fail**: wrong section | 0.466 / 1.000 | glossary.md › Serial node | pass |
| `owner-1-para-b` | covered | **fail**: wrong section | 0.253 / 1.000 | color/primaries.md › White balance picker and Auto balance | pass |
| `owner-1-para-c` | covered | pass | 0.446 / 1.000 | color/nodes.md › A starter tree for F-Log2 | pass |
| `owner-1-para-d` | covered | pass | 0.283 / 1.000 | color/recipes.md › 1. Turn a flat F-Log2 clip into a normal picture | fail (a `start` synonym pulls "Start a lesson": leave `start` out) |
| `owner-1-para-e` | covered | **fail**: under threshold, missing tackle, grade | 0.144 / 0.500 | color/recipes.md › If you know Resolve | pass |
| `owner-2-qualifier` | covered | **fail**: under threshold, missing useful, cli | 0.121 / 0.333 | color/secondaries.md › (page intro) | fail (needs the what-for route: `useful` is OOV) |
| `owner-2-para-a` | covered | **fail**: page intro outranks the section | 0.636 / 1.000 | color/secondaries.md › (page intro) | pass |
| `owner-2-para-b` | covered | **fail**: under threshold, missing what, qualif | 0.142 / 0.333 | render-cache.md › Good to know | fail (typo + `whats`; needs route or prefix match) |
| `owner-2-para-c` | covered | pass | 0.431 / 1.000 | color/secondaries.md › The qualifier: select by colour | pass |
| `owner-2-para-d` | covered | **fail**: under threshold, missing instead | 0.470 / 0.667 | color/secondaries.md › (page intro) | pass |
| `order-nr` | covered | pass | 0.410 / 1.000 | color/noise.md › Steps | pass |
| `order-grain` | covered | pass | 0.443 / 0.750 | color/recipes.md › 7. Finish with film grain | pass |
| `order-lut` | covered | **fail**: under threshold, missing chain | 0.277 / 0.500 | color/finishing.md › A LUT on a node | fail (order route) |
| `order-cm-first` | covered | pass | 0.424 / 1.000 | color/README.md › First grade in six steps | pass |
| `order-lumetri` | covered | pass | 0.231 / 0.750 | color/lumetri.md › Where Lumetri sits in the picture | pass |
| `order-secondaries` | covered | **fail**: under threshold, missing before | 0.257 / 0.667 | color/secondaries.md › (page intro) | pass |
| `order-still` | covered | **fail**: wrong section (Stills gallery, no `alt`; arguably fine) | 0.630 / 1.000 | color/matching.md › Stills gallery | fail |
| `order-scene` | covered | pass | 0.390 / 0.750 | color/recipes.md › 6. Give one grade to a whole scene | pass |
| `order-softclip` | covered | pass | 0.291 / 0.800 | color/finishing.md › Highlight soft clip | pass |
| `order-wb` | covered | **fail**: under threshold, missing point | 0.432 / 0.667 | color/primaries.md › White balance picker and Auto balance | fail (order route) |
| `order-tnr-node` | covered | pass | 0.748 / 1.000 | color/noise.md › Temporal noise reduction | pass |
| `order-proxies-grading` | covered | **fail**: under threshold, missing proxy | 0.265 / 0.667 | color/README.md › First grade in six steps | fail (order route; guide gap 2) |
| `order-six-steps` | covered | **fail**: "Give" in a heading | 0.240 / 0.750 | color/recipes.md › 6. Give one grade to a whole scene | fail (idf heading weight) |
| `what-node` | covered | pass | 0.809 / 1.000 | glossary.md › Node | pass |
| `what-offset` | covered | pass | 0.669 / 1.000 | glossary.md › Wheel | pass |
| `what-pivot` | covered | pass | 0.401 / 1.000 | color/primaries.md › The panel | pass |
| `what-window` | covered | pass | 0.807 / 1.000 | glossary.md › Window | pass |
| `what-matte` | covered | pass | 0.793 / 1.000 | glossary.md › Matte | pass |
| `what-working-space` | covered | pass | 0.591 / 1.000 | glossary.md › Working space | pass |
| `what-bypass` | covered | pass | 0.529 / 1.000 | color/nodes.md › The Nodes panel | pass |
| `what-disable-vs-bypass` | covered | **fail**: under threshold, missing difference, between | 0.245 / 0.500 | color/nodes.md › The Nodes panel | fail (x-vs-y route) |
| `what-shot-match` | covered | pass | 0.694 / 1.000 | color/matching.md › Shot Match | pass |
| `what-explainer` | covered | pass | 0.759 / 1.000 | color/matching.md › The node explainer | pass |
| `what-render-cache` | covered | **fail**: page intro outranks | 0.782 / 1.000 | render-cache.md › (page intro) | pass |
| `what-proxy` | covered | pass | 0.811 / 1.000 | proxies.md › What a proxy is, and why | pass |
| `what-temp-tint` | covered | pass | 0.456 / 1.000 | color/primaries.md › White balance picker and Auto balance | pass |
| `what-still` | covered | pass | 0.798 / 1.000 | glossary.md › Still | pass |
| `what-lumetri-vs-nodes` | covered | **fail**: wrong section | 0.296 / 0.750 | color/lumetri.md › If you know Premiere | fail (x-vs-y route) |
| `what-lut` | covered | pass | 0.809 / 1.000 | glossary.md › LUT | pass |
| `what-matte-blur` | covered | pass | 0.465 / 1.000 | color/secondaries.md › The qualifier: select by colour | pass |
| `what-hue-width` | covered | pass | 0.365 / 1.000 | color/secondaries.md › The qualifier: select by colour | pass |
| `what-serial` | covered | pass | 0.779 / 1.000 | glossary.md › Serial node | pass |
| `what-flog2` | covered | **fail**: "F-Log2" in a heading at 5× | 0.601 / 1.000 | color/nodes.md › A starter tree for F-Log2 | fail (what-for route → glossary Log; guide gap 10) |
| `what-tnr-vs-nr` | covered | **fail**: under threshold, missing difference, between | 0.473 / 0.667 | color/noise.md › Temporal noise reduction | fail (x-vs-y route) |
| `how-rename` | covered | pass | 0.637 / 1.000 | color/nodes.md › Reorder and name nodes | pass |
| `how-move-node` | partial | **fail**: under threshold, missing earlier, chain | 0.200 / 0.500 | color/secondaries.md › Tracking a window | fail (guide gap 3) |
| `how-insert-before` | covered | **fail**: "First" in a heading | 0.465 / 1.000 | color/README.md › First grade in six steps | fail (idf heading weight) |
| `how-cast` | covered | **fail**: "fix" in a heading | 0.463 / 1.000 | color/primaries.md › Steps: fix a flat F-Log2 clip | fail (idf heading weight) |
| `how-grey-card` | covered | pass | 0.416 / 1.000 | color/primaries.md › White balance picker and Auto balance | pass |
| `how-warmer` | covered | pass | 0.599 / 1.000 | color/recipes.md › 2. Warm a shot up | pass |
| `how-gradient-sky` | covered | **fail**: under threshold, missing gradient | 0.185 / 0.667 | color/nodes.md › A starter tree for F-Log2 | pass |
| `how-invert` | covered | pass | 0.293 / 0.833 | color/secondaries.md › The qualifier: select by colour | pass |
| `how-track` | covered | pass | 0.932 / 1.000 | color/secondaries.md › Tracking a window | pass |
| `how-same-grade-all` | covered | **fail**: "clip" in "Highlight soft clip" | 0.519 / 1.000 | color/finishing.md › Highlight soft clip | fail (idf heading weight) |
| `how-remove-grade` | partial | **fail**: under threshold, missing completely | 0.206 / 0.667 | color/nodes.md › Delete a node | fail (guide gap 3) |
| `how-nr-shadows` | covered | **fail**: `reduce` ≠ `reduction` | 0.374 / 0.667 | color/noise.md › Why | pass |
| `how-tnr-on` | covered | pass | 0.682 / 0.750 | color/noise.md › Temporal noise reduction | pass |
| `how-vignette` | covered | pass | 0.399 / 1.000 | color/secondaries.md › Windows: select by shape | pass |
| `how-lut-file` | covered | **fail**: `download` → AI explanations | 0.393 / 0.500 | color/matching.md › AI explanations | fail (drop `download` from the synonym pull or weight `lut` higher) |
| `how-softclip` | covered | **fail**: under threshold, missing soften | 0.366 / 0.667 | color/finishing.md › Highlight soft clip | pass |
| `how-side-by-side` | covered | **fail**: "Compare" in a heading | 0.450 / 0.750 | color/matching.md › Compare shots and check a scene | fail (idf heading weight) |
| `how-split-screen` | covered | pass | 0.426 / 1.000 | color/README.md › The Color page layout | pass |
| `how-resolve-layout` | covered | **fail**: "layout" in a heading, `switch` missing | 0.421 / 0.667 | color/README.md › The Color page layout | fail (idf heading weight) |
| `how-check-scene` | covered | pass | 0.355 / 0.750 | color/matching.md › Compare shots and check a scene | pass |
| `how-node-preset` | partial | **fail**: under threshold, missing tree, preset | 0.270 / 0.600 | color/recipes.md › 6. Give one grade to a whole scene | fail (guide gap 5) |
| `how-lift-shadows-no-noise` | covered | pass | 0.504 / 1.000 | color/noise.md › Why | pass |
| `how-washed-out` | covered | **fail**: synonym drift to recipe 8 | 0.221 / 0.800 | color/recipes.md › 8. Fix grey or lifeless skin | fail (acceptable as an `alt`? no: it doesn't mention Interpret Footage) |
| `how-dwg` | covered | pass | 0.516 / 0.800 | color/color-management.md › Choose the working space (sequence) | pass |
| `how-youtube` | covered | pass | 0.395 / 1.000 | export.md › Choose a preset | pass |
| `how-pause-cache` | covered | pass | 0.569 / 1.000 | render-cache.md › Settings and commands | pass |
| `how-proxies-off` | covered | pass | 0.348 / 1.000 | proxies.md › Choosing when proxies are used | pass |
| `how-ai-explain` | covered | pass | 0.504 / 1.000 | color/matching.md › AI explanations | pass |
| `how-hints-off` | covered | pass | 0.494 / 1.000 | color/coach.md › Context hints | pass |
| `how-curves-contrast` | covered | pass | 0.496 / 1.000 | color/curves.md › Steps | pass |
| `how-hsl-secondary` | covered | **fail**: `select` → qualifier → glossary Key | 0.413 / 1.000 | glossary.md › Key | fail (`lumetri` should outweigh; idf heading weight helps) |
| `how-keyframe-node` | partial | pass (prose only: lesson fallback) | 0.456 / 1.000 | color/primaries.md › Commands | pass |
| `how-reset-node` | covered | **fail**: wrong section | 0.365 / 0.750 | color/primaries.md › White balance picker and Auto balance | fail (`neutral` → balance synonym; add `reset`) |
| `why-skin-after-sky` | covered | pass | 0.319 / 0.800 | color/recipes.md › 3. Make the sky bluer without touching skin | pass |
| `why-flicker` | covered | pass | 0.326 / 1.000 | color/secondaries.md › The qualifier: select by colour | pass |
| `why-matte-in-export` | covered | pass | 0.386 / 0.800 | glossary.md › Matte | pass |
| `why-nr-outside-qualifier` | covered | pass | 0.382 / 1.000 | color/noise.md › Steps | pass |
| `why-tnr-not-playing` | covered | pass | 0.523 / 1.000 | color/noise.md › Temporal noise reduction | pass |
| `why-lut-wrong` | covered | pass | 0.401 / 1.000 | color/finishing.md › A LUT on a node | pass |
| `why-lumetri-changes` | covered | **fail**: under threshold, missing look, different | 0.299 / 0.600 | color/lumetri.md › Where Lumetri sits in the picture | fail (page-level coverage, layer 3) |
| `why-wb-refuses` | covered | pass | 0.553 / 1.000 | color/primaries.md › White balance picker and Auto balance | pass |
| `why-pivot-0336` | covered | pass | 0.326 / 1.000 | color/color-management.md › Why | pass |
| `why-422-slow` | covered | **fail**: under threshold, missing play | 0.323 / 0.667 | hardware.md › When something is slow | fail (page-level coverage) |
| `why-shotmatch-lumetri` | covered | pass | 0.308 / 0.800 | color/lumetri.md › What doesn't carry over | pass |
| `why-render-bar-red` | covered | **fail**: under threshold, missing again, edit | 0.383 / 0.600 | render-cache.md › What the render bar shows | fail (page-level coverage) |
| `why-rec709-different` | covered | pass | 0.191 / 0.800 | color/color-management.md › Why | pass |
| `why-no-parallel` | covered | **fail**: `no` counted as a content word | 0.514 / 0.667 | color/nodes.md › Later: parallel and layer nodes (C2) | pass |
| `why-paste-windows` | covered | pass | 0.314 / 0.800 | color/secondaries.md › Tracking a window | pass |
| `why-grain-missing` | partial | **fail**: glossary Grain outranks | 0.430 / 1.000 | glossary.md › Grain | fail (guide gap 6) |
| `resolve-power-window` | covered | pass | 0.429 / 1.000 | glossary.md › Window | pass |
| `resolve-corrector` | covered | pass | 0.367 / 1.000 | glossary.md › Node | pass |
| `resolve-timeline-cs` | covered | pass | 0.462 / 0.750 | glossary.md › Working space | pass |
| `premiere-wheels` | covered | pass | 0.454 / 1.000 | glossary.md › Wheel | pass |
| `premiere-comparison-view` | covered | **fail**: At a glance table | 0.378 / 1.000 | glossary.md › At a glance | pass |
| `resolve-optimized-media` | covered | pass | 0.426 / 1.000 | glossary.md › Proxy | pass |
| `resolve-smart-cache` | covered | **fail**: glossary entry outranks the page | 0.511 / 1.000 | glossary.md › Render cache | fail (acceptable: add as `alt`) |
| `typo-stil` | covered | **fail**: under threshold, missing stil | 0.171 / 0.500 | glossary.md › Still | pass |
| `cutoff-qualifier-face` | covered | **fail**: under threshold (typo + cut-off) | 0.134 / 0.600 | color/secondaries.md › The qualifier: select by colour | fail (confidence 0.141 even with coverage 1.0: the OOV ceiling; needs the route or prefix match) |
| `slang-gain-gamma` | covered | **fail**: under threshold, missing vs, what, diff | 0.113 / 0.400 | glossary.md › Wheel | fail (x-vs-y route) |
| `resolve-alt-s` | covered | pass | 0.251 / 1.000 | getting-started.md › Choose your workflow: Premiere or Resolve | pass |
| `slang-pop` | partial | **fail**: under threshold, missing pop | 0.114 / 0.500 | color/lumetri.md › Where Lumetri sits in the picture | pass |
| `typo-to-dark` | covered | pass | 0.259 / 1.000 | color/recipes.md › 1. Turn a flat F-Log2 clip into a normal picture | pass |
| `cutoff-order` | partial | **fail**: noise.md page intro outranks | 0.509 / 0.750 | color/noise.md › (page intro) | fail (guide gap 1 + order route) |
| `resolve-key-output` | covered | **fail**: At a glance table | 0.408 / 1.000 | glossary.md › At a glance | pass |
| `uncovered-edit-then-grade` | uncovered | **fail**: answers from the Workflow section | 0.243 / 0.750 | getting-started.md › Choose your workflow: Premiere or Resolve | fail (guide gap 2; or the assistant) |
| `uncovered-scopes` | uncovered | pass (declines) | 0.206 / 0.667 | — | pass |
| `uncovered-stabilise` | uncovered | pass (declines) | 0.239 / 0.500 | — | pass |
| `uncovered-import` | uncovered | **fail**: answers from the "To write" stub | 0.578 / 1.000 | getting-started.md › Import X-M5 clips | fail (stub skipped, then proxies.md › Automatic proxies answers: `import` is there; needs the not-yet route or guide gap 9) |
| `uncovered-music` | uncovered | pass (declines) | 0.122 / 0.500 | — | pass |
| `uncovered-album` | uncovered | pass (declines) | 0.146 / 0.667 | — | pass (fails if `album` → gallery is added: don't) |
| `uncovered-6k-timeline` | uncovered | pass (declines) | 0.185 / 0.600 | — | pass |
