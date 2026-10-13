# Owner questions

Decisions the runs could not settle on their own (FORK-PLAN §1a.20). Each has the provisional,
reversible choice the run made so the work could continue. **Answer under the question** (or in
the planning chat); the next run applies the answer and marks it resolved. Never delete answers.

## Q1 (2026-10-11, U0 step 2): project schema numbers collided with upstream

Upstream and the fork both used project schema v13 and v14: upstream v13 = multi-stream audio,
v14 = narrations, v15 = project notes; the fork's v13 = colour grades, v14 = stills gallery.
- **Provisional choice:** upstream's numbers win. Grades + stills become **v16** (a no-op step).
  Fork files saved as v13 / v14 are recognised on load (old single-stream audio object, a
  `stills` list or a clip `grade`) and upgraded from v12 through upstream's chain, keeping the
  grades and stills. New saves carry an envelope marker `"lineage": "fork/hevc-vaapi"`; a v16 file
  *without* the marker (i.e. a future upstream v16 meaning something else) is refused with a clear
  error instead of being misread (`FormatError::OtherLineage`, `crates/format/src/lib.rs`).
- **Options:** (a) the above; (b) keep the fork's numbers and renumber upstream's (breaks loading
  projects saved by upstream builds); (c) refuse ambiguous v13 / v14 files.
- **To reverse:** change the detection in `crates/format/src/lib.rs` (`fork_shape`) and the
  refusal; tests in the same crate.

**Owner's answer (2026-10-11, via the planning chat): keep (a).** Resolved; nothing to change.

## Q2 (2026-10-11, U0 step 2): export CRF setting model

Both sides added CRF: the fork as `crf: Option<u8>` (set = constant quality), upstream as
`bitrate_mode: Crf` + `crf: f32`.
- **Provisional choice:** upstream's model (`bitrate_mode` decides; the fork's HEVC encoders take
  `crf` when the mode is CRF; old `"crf": null` reads as the default). Consequence: a user preset
  saved by the fork with `"crf": 22` but no `"bitrateMode": "crf"` loads as bitrate mode; re-pick
  CRF once.
- **To reverse:** map a numeric `crf` without `bitrateMode` to CRF mode in `deserialize_crf` /
  the preset loader (`crates/export`).

**Owner's answer (2026-10-11): keep upstream's model, plus the compatibility fix:** a numeric `crf` without `bitrateMode` loads as CRF mode (`deserialize_crf` / the preset loader in `crates/export`), with a test, so no fork preset silently changes. Apply it, then mark resolved.

**Applied** in U0 (`crates/export/src/lib.rs` `deserialize_crf` / legacy-CRF mapping, tests in `settings_tests.rs`). Resolved (session 13, 2026-10-11).

## Q3 (2026-10-11, U0 step 2): two VA-API decoders during the merge

Upstream has its own VA-API H.264 / HEVC decoder (`crates/platform/src/vaapi/`); the fork has its
own HEVC decoder with the zero-copy display path (`va.rs`, `va_hevc.rs`). The fork's ffmpeg pipe
module was renamed `vaapi_pipe` to end the name clash.
- **Provisional choice (stage 1):** both compiled in; the decoder chain on Linux is the fork's
  HEVC decoder → upstream's VA-API decoder (H.264 and anything the fork's refuses) → ffmpeg pipe;
  `hw_backend` reports plain "VA-API" (upstream's convention). UPSTREAM-PLAN asks for one stack
  (upstream's decoder + the fork's zero-copy path on top): that consolidation is U0 stage 2 and is
  only done if it keeps the fork's 6.2K numbers and conformance; otherwise the fork's HEVC decoder
  stays and this question stays open.
- **To reverse:** the order of the chain in `vaapi_factory` (`crates/platform/src/lib.rs`).

**Owner's answer (2026-10-11): keep the staged approach, and go ahead with stage 2** (one VA-API stack: upstream's decoder + the fork's zero-copy path) as UPSTREAM-PLAN plans, with the same 6.2K and conformance gate. **New owner direction: phase out ffmpeg from the product** (FORK-PLAN §1a.24): don't add new ffmpeg dependencies, and the ffmpeg pipe leaves the decoder chain in Phase 7.7.

**Owner update (2026-10-11, live in the runner chat): stage 2 moves to right before Phase 8** (U0.2,
FORK-PLAN §1a.6). U0 merges with both stacks (stage 1) once its gates are green. Resolved for U0.

## Q4 (2026-10-11, U0 step 2): smaller merge calls

Recorded so they can be undone; all keep both sides working.
- **Lumetri:** upstream's single GPU `OP_LUMETRI` op was removed; Lumetri runs as the fork's
  ColorOps chain (ADR 0003) with upstream's vignette roundness fix (#719) ported and upstream's
  Lumetri tests run against the chain.
- **Render plans:** new `PlanStep::Output` carries the output transform in upstream's step plans;
  DWG / DI stays on the GPU for monitor and SDR export; HDR export frames render on the CPU inside
  a pool renderer (correct, slightly slower).
- **`media.setInterpretation` `pixelAspect`:** accepts `[n,d]`, `"N:D"`, an id, `"file"` or
  `"auto"`; the response is upstream's `[n,d]` plus `pixelAspectOverride`.
- **Audio:** the fork's cpal 0.18 + PulseAudio (fixes the fork's issue #23) kept, with upstream's
  device fallback / input formats ported onto it. Shuttle audio keeps the fork's varispeed
  (reverse and forward up to 2×) instead of upstream's forward-only mix rate.

**Owner's answer (2026-10-11): fine, all accepted.** Resolved.

## Q5 (2026-10-11, between G2 and G2b): make `cargo xtask ci` ~20 minutes

Owner asked (live): can ci take ~20 min instead of 40–65 without weakening it? Where the time
goes: two full release passes (`clippy --release`, `test --release`) plus the wasm build, all
non-incremental, ~135 test binaries run one by one, `CARGO_BUILD_JOBS=6` and `nice 19`.
- **Plan (proposed to the owner, not started):** a measured step at the start of the next run:
  (1) per-stage timing in `xtask ci`; (2) a `[profile.ci]` inheriting release (same opt-level, so
  the realtime-mixer test keeps its meaning) with `incremental = true` and more codegen units;
  (3) clippy without `--release`; (4) `cargo-nextest` if installed (user-level, optional, same
  tests in parallel). Keep 6 jobs (desktop rule). Verify the same test count passes; record
  before / after.
- **Trade-off:** `Cargo.toml` / `xtask` are upstream files → a small merge conflict per sync;
  an env-only variant (no profile) gains less.
- **Owner's answer (2026-10-11, live): "make all the appropriate changes … apply to this CI".**
  Done in session 13: per-stage timing, clippy in the check profile, `[profile.ci]` (release
  opt-level, incremental, 64 codegen units), `cargo-nextest` 0.9.148 installed (user-level) with
  6 test threads and the `perf_*` / `realtime` tests run alone, doctests via `cargo test --doc`,
  and the 68 `crates/ui-egui/tests/*.rs` binaries merged into one (`tests/ui/main.rs`). First run
  on the new setup: **21.7 min** (tests 21 min incl. ~19 min compiling, 3,095 + 7 passed; test
  execution 2 min 21 s) vs 40–65 min before. Resolved.

## Q6 (2026-10-11, G2b step G2.11): may the agent download the candidate AI models?

To pick the explainer model (GRADE-PLAN G2.11: quality on the 30-case set, ≤ ~5 s on this CPU)
the run needs the candidate weights locally. Candidates (all Apache-2.0, official publisher repos
on Hugging Face, sizes from HEAD requests): `qwen2.5-0.5b-instruct-q4_k_m.gguf` 491 MB,
`qwen2.5-1.5b-instruct-q4_k_m.gguf` 1.12 GB, `smollm2-360m-instruct-q8_0.gguf` 386 MB,
`smollm2-1.7b-instruct-q4_k_m.gguf` 1.06 GB, plus two `tokenizer.json` (7 MB + 2 MB): ≈ 3.1 GB,
into FilmCraft's per-user models folder (never in the repo, never bundled). The losers can be
deleted after the evaluation.
- (a) The agent downloads them (owner says yes, here or in chat). Fast; the evaluation runs in G2b.
- (b) The owner downloads them by hand (or via Settings ▸ Color ▸ AI explanations once wired).
**Provisional choice:** build and test everything against the template fallback (no model needed);
Qwen2.5-0.5B is the provisional default in the catalogue until the evaluation. Asked live in the
session on 2026-10-11 16:45; waiting for the answer.

**Follow-up (owner, live 2026-10-11): "pick a model we can alter to best fit our needs".** Answer
given: the inference code (`crates/llm`) is ours; behaviour is changed by fine-tuning the weights
(LoRA on facts → explanation pairs we generate and filter with the G2.11 fact checker). All four
candidates are Apache-2.0 (modification and redistribution allowed). SmolLM2 is the most open
(training data and recipes published), Qwen2.5 is weights-only. Recommendation (provisional):
evaluate stock models first; prefer SmolLM2 as the base; fine-tune only if the 30-case pass rate
is low, as a step after G2b's core. Fine-tuning would need training tooling (offline, not in the
product; e.g. PyTorch + ROCm on the RX 6700 XT: more downloads, owner OK) and a place to host the
tuned weights for the opt-in download (owner's HF account or a fork release). Waiting for the
owner on: download yes/no, SmolLM2 vs Qwen base, fine-tune yes/no.

**Owner's answer (2026-10-11, live): "yes I'd like to train them for use on our filmcraft fork."**
Applied: the four candidate GGUF files + tokenizers were downloaded by the agent into
`~/.local/share/filmcraft-fork/models/llm/` (SHA-256 checked against the publishers' LFS hashes);
fine-tuning is approved and planned as GRADE-PLAN G2.11b (after G2b's core). Base model: still
open (provisional lean SmolLM2, fully open data); decided by the stock evaluation. Open
follow-ups in Q7. Resolved for the download and the fine-tune decision.

## Q7 (2026-10-11, G2.11b): how to train, and where the tuned weights live

Fine-tuning needs the full-precision base weights (GGUF 4-bit files can't be trained), a trainer,
and a host for the tuned file that the app's opt-in download fetches.
1. **Trainer.** (a) Pure Rust with candle (LoRA, CPU; fits the project, no new toolchain; slower,
   hours per run for a ~0.4B model, and we write more of it ourselves). (b) Python + PyTorch
   with ROCm on the RX 6700 XT (standard tooling, fast; several GB of downloads and a Python
   environment outside the repo; offline tooling only, never in the product).
   **Provisional:** (a) candle, because it needs no new toolchain and keeps the project pure Rust.
2. **Base weights** (needed for either trainer): the safetensors of the chosen base, e.g.
   SmolLM2-360M-Instruct ≈ 0.72 GB or Qwen2.5-0.5B-Instruct ≈ 1 GB. Needs an owner OK to download.
3. **Hosting the tuned model** (Apache-2.0 allows redistribution with the licence + a change
   note): (a) the owner's Hugging Face account, (b) a GitHub release on dawgon2/filmcraft-fork.
   Only the owner can create either. **Provisional:** keep the tuned file local and point the
   catalogue at a local path until the owner sets up a host.

**Owner's answers (2026-10-11, live):**
1. Trainer: **"a mix of both"**. Applied as: Rust (candle) for the training-set generator, the
   evaluation, GGUF quantization and a CPU LoRA fallback; Python + PyTorch on the RX 6700 XT
   (ROCm, a venv outside the repo, offline tooling only) for the fast LoRA runs.
2. Base weights: **yes**, and **quantize them afterwards** (to GGUF Q4_K_M / Q8_0, in Rust).
3. Hosting: **a release on the GitHub fork.** Caveat recorded by the agent: dawgon2/filmcraft-fork
   is private, so release assets need a login and the app's anonymous opt-in download can't fetch
   them. Provisional: the catalogue entry for the tuned model reads a local file until there is
   a public URL (the repo or a release made public, or a separate public models repo, owner's
   call whenever convenient; no run waits for it).
4. Scope now: **test-train the candidates, pick the better one, and leave a repeatable training
   session the owner can start any time.** Settings ▸ Color ▸ AI explanations lets the user
   **choose which model to download** (every catalogue entry, not one fixed default).
5. **Don't ask the owner to decide things to finish work** (even live): decide, record here,
   continue (§2b). Resolved.

**Owner (2026-10-11, live): host the tuned model on https://github.com/dawgon2/filmcraft-cachyos
(public).** Applied: the tuned explainer is published as a GitHub *release asset* there (tag
e.g. `explainer-<base>-v1`, GGUF + tokenizer + LICENSE (Apache-2.0) + a NOTICE saying it is a
fine-tune of the named base, what changed, and the training-set provenance); the catalogue entry
gets that URL pinned by SHA-256. The standing rule "never push to the `cachyos` remote" still
holds for git branches and code; a release (and the tag `gh release create` makes for it) is the
only thing created there. Publishing is public content, so the agent prepares the release and
publishes it when the owner says "publish" in chat; nothing waits for that (the app reads the
local file until then).

## Q8 (2026-10-11, G2.11): build the fork for x86-64-v3 (AVX2)?

candle (the AI explainer's inference library) only uses SIMD for 4-bit models when AVX2 is
enabled at compile time; the default x86-64 build runs it scalar (~17–30 s per explanation vs
~4 s with AVX2, measured on this PC). Options: (a) `.cargo/config.toml` builds x86-64 Linux for
`x86-64-v3` (any CPU since 2013; one-time full rebuild; float results unchanged because Rust
doesn't fuse or reorder float maths by itself); (b) flag only for the phase builds (cache
thrash between flagged and unflagged release builds); (c) our own copy of candle's quantized
forward pass (MIT/Apache, ~860 lines) with gemm-based prefill (≈ 2 s per explanation even
without the flag; more code to own). **Provisional: (a)**, committed with a comment block
in `.cargo/config.toml` (delete it to undo); (c) stays a possible later speed-up.
**Owner, 2026-10-12: do (c)** as AI-PLAN Phase AI1 (after C2), then remove the flag (FORK-PLAN §1a.37).

## Q9 (2026-10-12, Phase 6 item 7): temporal noise reduction on which node?

Temporal NR needs the neighbouring frames *as they look at that node*. On the first node that
is just the decoded clip; on a later node every earlier node would have to be re-run on each
neighbour (2–4 extra full grades per frame at 6.2K). Options: (a) **first active node only**
in v1 (cheap; a note in the panel says so; later nodes ignore it); (b) any node, re-grading the
neighbours (correct anywhere, roughly 2–5× slower export with TNR on); (c) a separate clip-level
"Temporal NR" before the grade (like Resolve's per-clip NR setting, not a node). **Provisional:
(a)**, marked "provisional (owner question Q9)" in the compile step; switching to (b) or (c)
later keeps the same parameters. Design: `docs/fork/PHASE6-DESIGN.md` §6.7.

**Owner's answer (2026-10-12, live): (c)**, a separate clip-level "Temporal NR" before the grade, like Resolve's per-clip setting. Scheduled as step **6.7b** right after RS3, before Y1 (FORK-PLAN §1a.40); same parameters, existing node-level TNR settings migrate to the clip. Resolved once 6.7b lands.

**Resolved 2026-10-12 (6.7b landed):** clip-level Temporal NR (schema v17, `tnr.*` commands, Effect Controls / Inspector and Color ▸ Noise rows); the provisional first-node rule is gone; old projects migrate on open.

## Q10 (2026-10-12, L1 prep): drag and drop from Dolphin on Wayland

The Linux audit (`docs/fork/LINUX-AUDIT.md`) found that winit 0.30 (the window library eframe 0.36
uses) has **no drag-and-drop of files and no pinch gesture on Wayland**. Options: (a) drop both
from L1 and ship fallbacks: "Open With FilmCraft" + hand-off to the running window, Paste of
copied files, the Import dialog (no extra cost); (b) wait for upstream to move to winit ≥ 0.31
/ a newer egui, then add DnD (cost ~2–4 h at that point); (c) patch winit ourselves (~12 h,
a fork of a core dependency to maintain). **Provisional: (a) now, (b) when upstream upgrades**
(the lane re-checks at each upstream sync). Reversible: nothing is built that blocks DnD later.

**Owner's answer (2026-10-12, live): "ok sounds good"** — (a) now, (b) when upstream upgrades. Resolved.
