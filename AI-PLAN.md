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
2. **Delete the `x86-64-v3` block in `.cargo/config.toml`**, resolve Q8 in OWNER-QUESTIONS.md, rebuild.
3. **Feels instant everywhere:** stream tokens to the UI as they're generated; pre-generate the
   node explanation in the background (cancellable, `catch_unwind`) while the user adjusts, so it
   is usually ready before it's asked for; keep the template fallback for slow machines or no model.
4. **Smaller tune (try, keep only if it passes):** SmolLM2-135M through TRAINING.md's pipeline
   (SmolLM2 is the base, §1a.27); ship it as a "fast" option only if it matches the 360M tune's
   fact-check pass rate within a few cases.
5. **Shared runtime for A2 / G3:** the run-time CPU dispatch and background-job plumbing are a small
   reusable layer that AUDIO-PLAN A2 (voice isolation) and GRADE-PLAN G3 (Magic Mask) use.

**Accept:** with the flag removed, `smol360` (Q8_0) scores the same on both held-out sets (75
cases) and answers in **≤ 2.5 s** mean on this PC (target ~2 s); a forced-SSE run (dispatch
override for tests) still works and its time is recorded; first streamed words appear in
< 0.5 s; results are identical to the old path within the fact checker; no model or a corrupt
model never crashes. Tag `ai-1`.
