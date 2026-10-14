# AI-PLAN.md: local AI that runs well on every CPU

Written 2026-10-12 (live session, with the owner). Order: FORK-PLAN §1a.6 (AI1 right after C2,
before AUDIO-PLAN A2 and GRADE-PLAN G3, which build on it). Every FORK-PLAN rule applies (never
crash, everything is a command, layering, `cargo xtask ci` per phase, a build and a `cachyos`
publish per phase). All inference stays local: models are optional, downloaded once on demand,
checksummed, never bundled or committed; without a model the app falls back cleanly.

## Phase AI1: portable fast inference (≈ 8–15 h)

Today candle's quantized kernels only use SIMD when AVX2 is enabled at compile time, so the fork
builds for `x86-64-v3` (OWNER-QUESTIONS Q8, provisional (a)); without the flag an explanation
takes ~17–30 s instead of ~4 s. AI1 implements Q8 option (c) and retires the flag.
1. **Own quantized forward pass** in `crates/llm` (from candle's MIT / Apache-2.0 code, ~860
   lines, attribution kept), with the heavy matrix maths through `gemm`, which picks AVX-512 /
   AVX2 / SSE / NEON at run time. No `unsafe` in our code; one binary for every CPU.
2. ✓ (2026-10-13) **Delete the `x86-64-v3` block in `.cargo/config.toml`**, resolve Q8 in OWNER-QUESTIONS.md, rebuild.
   Done: candle's f32 ops checked without the flag (`crates/speech/examples/candle_ops_bench.rs`):
   gemm-backed ones unchanged; Kokoro's transposed conv and last-dim mean now go through gemm.
3. ✓ (2026-10-13, AI1.3a–c) **Feels instant everywhere:** stream tokens to the UI as they're generated; pre-generate the
   node explanation in the background (cancellable, `catch_unwind`) while the user adjusts, so it
   is usually ready before it's asked for; keep the template fallback for slow machines or no model.
4. ✓ (2026-10-14, `smol135-v1`, 34 / 36 + 36 / 40 + 13 / 13 at ~0.6 s; catalogued as the "fast" option, TRAINING.md) **Smaller tune (try, keep only if it passes):** SmolLM2-135M through TRAINING.md's pipeline
   (SmolLM2 is the base, §1a.27); ship it as a "fast" option only if it matches the 360M tune's
   fact-check pass rate within a few cases.
5. ✓ (2026-10-13, `crates/ml`) **Shared runtime for A2 / G3:** the run-time CPU dispatch and background-job plumbing are a small
   reusable layer that AUDIO-PLAN A2 (voice isolation) and GRADE-PLAN G3 (Magic Mask) use.

6. **Retrain at the end of AI1 (owner, 2026-10-12, FORK-PLAN §1a.49):** a new SmolLM2 tune,
   **`smol360-v2`**, on everything the explainer should know by then: the existing serial-grade
   data plus what shipped since v1 (G2c white balance / Auto balance, 6.7b clip-level Temporal NR,
   Y3's keys / effects where the explainer covers them), TRAINING.md's v2 ideas (more epochs,
   more near-threshold colour / saturation cases), and the owner's logged misses in
   `docs/fork/COACH-QUESTIONS.md` where they're explainer-shaped. Trained on the RX 6700 XT
   (~1 h per run), run through the new runtime (item 1), evaluated on both held-out sets plus a
   small new set for the features added since v1. Ships first in `TUNED_PREFERENCE` and is
   published as `explainer-smol360-v2` (§1a.33) only if it beats v1 (see Accept).

**Accept:** with the flag removed, `smol360` (Q8_0) scores the same on both held-out sets (75
cases) and answers in **≤ 2.5 s** mean on this PC (target ~2 s); a forced-SSE run (dispatch
override for tests) still works and its time is recorded; first streamed words appear in
< 0.5 s; results are identical to the old path within the fact checker; no model or a corrupt
model never crashes; **`smol360-v2` scores ≥ v1 on both old held-out sets and passes the
new-features set** (otherwise v1 stays the default and the reason is logged). Tag `ai-1`.

## Phase AI2: teach the explainer the compositor and branch nodes (≈ 6–12 h; owner, 2026-10-12)

Right after C2 (FORK-PLAN §1a.48). C1 adds comp graphs and C2 adds parallel / layer / splitter-
combiner / key-mixer / outside nodes, all with template explanations (C1 item 7, C2's Coach
lessons); this phase trains the local model on them so its explanations are as good there as on
serial grades.
1. **Data:** extend the training-data generator (`tools/explainer-train/`) with comp-graph and
   branch-node cases (sampled graphs → the template explainer's facts → target sentences), keeping
   the existing serial-grade data; near-threshold cases for blend modes, key strength, opacity.
2. **Fact checker:** extend it to comp nodes and branch nodes (only real controls and real values;
   rejects invented ones), with tests.
3. **Held-out sets:** a third held-out set for comp graphs and a fourth for branch-node grades
   (never seen in training), plus the two existing sets (no regression allowed).
4. **Train** the SmolLM2 tune (`smol360-v3`, starting from AI1's v2 data and recipe; also the 135M "fast" tune if AI1 kept it) on the RX
   6700 XT per TRAINING.md (~1 h per run); Qwen stays frozen (§1a.27).
5. **Ship:** new version first in `TUNED_PREFERENCE`, catalogue entry pointing at its release
   asset, published as `explainer-smol360-v3` (§1a.33; never `manifest.private.json`).
6. **Coach:** add the C1 / C2 questions to `docs/fork/coach-eval/questions.toml` if C1 / C2 didn't,
   and re-measure.

**Accept:** on all held-out sets the new tune's fact-check pass rate ≥ v2's on the old sets
and ≥ the template explainer's coverage on the new ones; mean answer time within AI1's ≤ 2.5 s;
no crash on hostile graphs. Tag `ai-2`.

## Phase AI3: the grading assistant, a model-backed "Ask the coach" (owner, 2026-10-14; ≈ 70–120 h)

Owner: "I want to be able to ask it advanced questions about how to grade. What nodes do what,
what to do if I want to grade in a specific way, how to tweak something just right, what a
certain tool in the app is and how to use it, etc. We can use a bigger model if needed."
Today's Ask the coach (G2.12 part 2) is a keyword search that returns one guide section's steps;
the local model only rewrites node explanations. AI3 makes the coach a real assistant, grounded so
it can't invent buttons or steps.

**Scope (owner, 2026-10-14 ~12:55, FORK-PLAN §1a.75): all of editing, not only grading.** The
corpus, feature pages, mapping, training pairs and eval set cover every feature built when the
tune runs, the Compositor (C1) included. Features not built yet aren't written ahead; each later
phase adds its pages, pairs and eval questions, and the tune is refreshed as they land.

0. **Owner additions (2026-10-14): real professional practice, every feature, mapped, token-lean.**
   "We should feed it in-depth detail from real data of people doing professional work in resolve
   and premier. We should also add an in-depth explanation of every feature in our app and how it
   works (especially ones that are unique). Then we should train it on how what those people did in
   resolve and premier work in our app. We should make the guide have sections that is easily
   parsable to save on tokens." So:
   a. **Professional practice corpus (sources rule):** collect how working colourists and editors
      actually do things in Resolve and Premiere (node trees, grading orders, look recipes,
      matching, skin, log / HDR delivery, editing habits). Openly licensed material (CC-BY,
      CC-BY-SA, CC0 / public domain, permissively licensed docs) may be quoted and used verbatim in
      training data, with a `.attribution` entry like assets. Everything else (tutorial videos,
      courses, forum posts, blogs, Blackmagic / Adobe manuals) is a **reading source only**: the
      techniques and facts are written up in our own words as "practice notes" (one technique per
      note: goal, when, steps in Resolve / Premiere terms, why, pitfalls), never transcripts,
      quotes or copied tables; the source list stays maintainer-local (`plan/`), not in the repo
      or the published model. Clean-room (CLAUDE.md, GRADE-PLAN G2.12): never copy a manual. No
      scraping against a site's terms; video transcripts aren't stored. ≈ 10–20 h of research and
      writing, ≥ 150 notes.
   b. **Every FilmCraft feature explained in depth** (fork guide): what it is, how it works
      inside (the model behind it, e.g. DWG / DaVinci Intermediate working space, the HSL
      qualifier's soft edges, Temporal NR, the node explainer's measurement), when to use it, its
      limits; unique fork features first (Grading Coach, node explainer, two-GPU export, proxies /
      render cache, Linux integration…). Every control named by `{ui:}` id / command. ≈ 15–25 h.
   c. **Mapping: Resolve / Premiere practice → FilmCraft.** For each practice note, the FilmCraft
      way (same tool, closest equivalent, or "not yet", logged as a gap). Then **training pairs**
      generated from notes + mapping + feature pages (question → grounded answer with the
      FilmCraft steps), human-reviewed samples, used to tune the chosen model (TRAINING.md
      pipeline on the RX 6700 XT) and as eval cases. Gaps feed `docs/gaps.md`.
   d. **Token-lean, parsable sections:** every guide section (and practice note) starts with a
      fixed compact header the retriever sends instead of prose when that is enough, e.g.
      `> id: color.qualifier.hsl | does: isolate a colour range | ui: grade.qualifier.* |
      cmd: grade.qualifier.* | resolve: Qualifier (HSL) | premiere: Lumetri HSL Secondary |
      when: skin, sky, single colours | see: color.windows`, then short numbered steps, then the
      long explanation. A test checks every section has a valid header; the retriever builds an
      index from headers and sends header + steps first, the full text only if the model asks
      (≈ 3–5× fewer prompt tokens). Applies to existing pages too (one pass over the guide).
1. **Knowledge first: the guide grows an advanced grading part** (clean-room, written for this
   fork, tagged as today): what each node type and tool does and when to use it; grading
   approaches (look development, matching, skin, log workflows, F-Log2 → DWG / DaVinci
   Intermediate, contrast shaping, highlight roll-off, film-style looks, day-for-night…) written as
   options with trade-offs (§1a.47 b, never "the one right way"); "how to tweak X just right"
   notes per control (ranges, what overdoing looks like, pairs that interact). Every app-specific
   step names real `{ui:}` ids / commands. ≈ 12–20 h. This is most of the quality.
2. **Retrieval that understands questions:** keep BM25, add a small local embedding model (or the
   chat model's own embeddings) so "node system", "why is my skin grey", "make it look like
   film" find the right sections; top-k sections (not one) go to the model, plus the current
   grade's facts (G2.10 explainer) and what's on screen (page, selected node / tool). ≈ 4–6 h.
3. **A bigger chat model, still local and offline:** pick by a measured eval (item 6), candidates
   with open licences that allow shipping: Phi-4-mini (3.8B, MIT), SmolLM3-3B (Apache-2.0),
   Qwen2.5-7B-Instruct (Apache-2.0), plus today's small tunes as the fast fallback. Run on the
   AI1 runtime (CPU, any machine; streaming; Q4/Q5); measure tokens/s on this PC and the laptop
   (Core Ultra 7 155H). GPU inference (wgpu) only if CPU is too slow for a 3–4B model: a separate,
   bigger step, decide on the numbers. Model downloads need the owner's yes per model (size,
   source). ≈ 6–10 h.
4. **Grounded answers, checked:** the model answers from the retrieved sections and the grade's
   facts; app steps must come from the guide (every named control / command checked against the
   live app as today, unknown ones removed); general colour-science / grading advice the guide
   doesn't state is allowed only when marked as general advice, never as a FilmCraft step; "I
   don't know" beats a guess. Sources listed under the answer. ≈ 6–10 h.
5. **A chat, not a box:** follow-up questions keep context ("and if it's backlit?"), steps stay
   clickable (outline the control), "Show me" starts the matching lesson, answers stream. The
   current model-free coach stays as the fallback without a model. ≈ 4–6 h.
6. **Eval:** an advanced question set (≥ 60: node roles, approaches, tweaks, tools, multi-step
   "how do I get this look") with expected sources and must / must-not facts; score each
   candidate model + retrieval; the owner's misses go to `docs/fork/COACH-QUESTIONS.md` and the
   set. Optional tune on the guide (TRAINING.md pipeline) if a base model misses the floor. ≈ 4–8 h.

**Accept:** every guide section has a valid compact header; ≥ 150 practice notes with mapping
(sources rule followed: no verbatim non-open text in the repo, the training data or the model);
every feature in the command list has a feature page section; on the advanced set (now including
"how would a Resolve / Premiere pro do X here" questions) the chosen setup answers ≥ 85 % correctly with correct sources
and **no invented controls, commands or steps** (any invented one fails the case); first words in
< 1 s and a typical answer in < 10 s on this PC (laptop time recorded); works offline; no model,
a corrupt model or a slow machine falls back to today's coach and never crashes. Tag `ai-3`.
