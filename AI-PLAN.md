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
4. **Smaller tune (try, keep only if it passes):** SmolLM2-135M through TRAINING.md's pipeline
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
