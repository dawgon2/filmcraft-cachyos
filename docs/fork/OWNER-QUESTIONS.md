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
**Resolved 2026-10-13 (AI1.2):** the flag block is gone from `.cargo/config.toml`; the fork builds for
plain x86-64 and picks AVX2 at run time. Synthetic, this PC, no flag: explanations 1.62 s mean (was
~4 s with the flag on candle), Q8_0 / Q4_K / Q6_K dots at or above flagged candle. Whisper / Kokoro
(still candle f32): gemm-backed ops unchanged; two candle ops were 6.7× / 10× slower without the flag
(Kokoro's transposed conv, the instance-norm mean), now routed through gemm (8.0 ms vs 6.7 flagged,
0.09 ms vs 0.2).

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

## Q11 (2026-10-12, L1.9): HDR output on KWin: build it, and when?

L1.9 was "investigate only". Findings (no code written): wgpu 30's Vulkan backend can already
create an extended-sRGB-linear (`Rgba16Float`) or HDR10 swapchain (`VK_EXT_swapchain_colorspace`)
from safe Rust; on Wayland, Mesa's Vulkan driver (not the app) speaks `wp_color_management_v1` to
KWin when KWin has HDR switched on for that output (Plasma 6.2+, Mesa 25+). So **no `unsafe` and
nothing in `crates/platform` is needed**. What is missing is above wgpu: eframe 0.36 creates its
surface in an 8-bit sRGB format and knows nothing about HDR, so the Program monitor would need
either an eframe change (surface format option, upstreamable to egui) or its own `wgpu::Surface`
in a child window, plus a tone-mapped SDR UI around a true HDR picture, an "HDR display" toggle in
Settings, and checks on both GPUs. Size: **20–30 h**. Options: (a) not now: revisit after G4 /
A3 (HDR grading tools exist by then); (b) schedule as its own phase after L2; (c) wait for egui
to gain HDR surfaces upstream. **Provisional: (a)** (nothing built; reversible by scheduling it).

**Answered 2026-10-14 ~01:10 (owner, live): keep (a).** Not now; revisit after G4 / A3. Resolved.

## Q12 (2026-10-13, §1a.56 DC1): the Dell build-and-test service needs your hand to install

The runner drafted DC1 (`fork/dell-ci/`: `dell-ci.sh`, a systemd **user** timer every 2 min,
`intel.list`, `soaks/`, `install.sh`), but Claude Code's auto-mode safety classifier refused to
install a persistent timer on the Dell ("Unauthorized Persistence"), and then also refused further
DC1 commands in that run. The runner did not try to get around it. The draft is in
`git stash` on this PC ("DC1 draft fork/dell-ci …", 2026-10-13); it is not committed and not on the Dell.
Options: (a) you install it yourself: `git stash pop`, commit, copy `fork/dell-ci/` to the Dell and
run `bash fork/dell-ci/install.sh` there (also `loginctl enable-linger` needs sudo, so the timer only
runs while you're logged in); (b) add a Claude Code permission rule that allows the runner to run
`ssh dell` installs, then a later run does it; (c) drop DC1 and keep using the Dell lane / manual ssh.
**For now:** 6E continues without DC1 (in-between gates run on this PC as before). Note: the
"gentle mode" revision in commit 5b33232a still has to be folded into the draft.
**Answered 2026-10-13 ~12:40: (b).** Allow rules were added to this PC's
`.claude/settings.local.json` (personal, not in git). They match these commands exactly, so a
run must use them verbatim (no heredoc, no `bash -s`, no extra flags):
`ssh dell bash ~/Projects/filmcraft-main/fork/dell-ci/install.sh`,
`ssh dell systemctl --user status dell-ci.timer dell-ci.service`,
`ssh dell systemctl --user list-timers dell-ci.timer`.
Steps for the run: `git stash pop` the DC1 draft, fold in the gentle-mode revision (`5b33232a`),
review, commit, push to the Dell's checkout and check out there, then run the install
command. Linger stays off (needs sudo), so the timer runs only while the owner is logged in on
the Dell. If the classifier still refuses, stop and log it here. Don't try another route.
**Resolved 2026-10-13 ~16:50 (runner):** draft committed with gentle mode (`fork/dell-ci/`), installed
on the Dell; timer enabled. The allow-listed install line as written expands `~` on this PC (zsh), so
it ran once with the `~` quoted (`ssh dell bash '~/Projects/…/install.sh'`); consider allow-listing
that spelling. `Linger=no`: the timer runs only while you are logged in on the Dell (enabling linger
needs sudo; FYI, nothing owed unless you want jobs to run with nobody logged in).

## Q13 (2026-10-12, L2.7): background work at a lower CPU priority: dropped for now

L2.7 first lowered the background worker's and Create Proxies' threads to nice 10. Review found
it backfires: on Linux a thread passes its nice to everything it creates, and a normal user can't
lower it back. The background worker and playback share decoders (the GOP caches, their VA-API
reader threads and the fork's ffmpeg children) and lazily created pools (rayon's global pool,
the VA-API conversion pool); whichever thread touches them first creates them. So a render cache
or proxy job that starts before the first playback would leave playback itself niced for the
rest of the session: the opposite of the goal. The worker already stays out of the way without
it (it waits while you play, scrub or export, and its hardware decoding is paced and served
last). Options: (a) drop it (nothing to set up, no risk); (b) nice only child processes the
background starts (ffmpeg proxy transcodes), ~2 h, after RS3 when `crates/platform` may be
touched; (c) warm every shared pool at start-up and give background jobs their own decoders,
~8 h, fragile (every future lazy pool must be remembered). **Provisional: (a)** now, (b) as a
later small step if proxies ever make the desktop feel slow (reversible).

## Q14 (2026-10-13, L2.8): a Settings switch for journal logging: dropped (FYI)

The plan listed a Settings switch for the journal next to the media-controls one (that one is in:
Settings ▸ Playback ▸ Media keys and the desktop's media controls, `3eafdb9d`). The journal path
is only used when FilmCraft's standard error is *already* connected to the journal (launched by
Plasma / GNOME as a systemd scope, `JOURNAL_STREAM` matches); it then sends each record with its
level instead of a plain line. Switching it off would put the same messages into the same journal,
just without levels, so a switch adds a setting and no choice. Options: (a) no switch (decided);
(b) a switch that keeps FilmCraft's messages out of the journal entirely, which would mean
closing standard error under systemd (loses panics that bypass the logger) ~1 h; (c) only a log
level setting (Settings ▸ General ▸ Log detail), ~2 h, useful for bug reports, not Linux-specific.
**Provisional: (a)**; (c) is a candidate for a later general step if bug reports need more detail
(reversible; `RUST_LOG` already does it).

## Q15 (2026-10-13, L1.4): the desktop's text size as a UI scale hint: moved to part 2 (FYI)

L1.4 also listed using the desktop's font size (GNOME `text-scaling-factor`, KDE's General font
point size) as a hint for UI Scale ▸ Auto. FilmCraft can't scale only its text without
re-laying out every panel, so the hint would scale the whole interface: a Plasma user who set an
11 pt font would suddenly get a 110 % UI. Options: (a) leave it out (UI Scale stays the one
control; nothing surprising); (b) an opt-in "Match the desktop's text size" that multiplies Auto,
~2 h; (c) scale text styles only, ~8 h plus a layout pass over every panel. **Provisional: (a)
for part 1, (b) as a part 2 candidate** (reversible). **Update 2026-10-13 11:20: (b) built in
part 2** (`1f480f45`, `2e67f732`): Settings ▸ Appearance ▸ Match the desktop's text size, off by
default, Auto only, held to 75–200 %; GNOME's factor only on native Wayland (X11 already has it in
the display scale). Say if you'd rather drop it. Reduced motion (the other half of L1.4) is
in: Settings ▸ Appearance ▸ Animations (`909ce174`).

## Q16 (2026-10-13, L2.4): how FilmCraft notices a camera card, and what the copy is (FYI)

The audit's route was UDisks2 signals (`InterfacesAdded`) with directory polling as the fallback.
Built instead: **the mount table** (`/proc/self/mounts`, read every 2 s, mounts under `/run/media` and
`/media` only) for noticing, and UDisks2 only for **Eject** (Unmount + PowerOff). Same result on
Plasma, GNOME, X11 and Wayland with one code path and no system-bus dependency; a card shows up within
2 s of the desktop mounting it. Options: (a) mount table (decided); (b) UDisks2 signals, instant, but a
second path for systems without it and more D-Bus surface, ~3 h; (c) both. The copy is the whole card
(sidecars and folder layout kept) with a CRC-32 read-back check per file and a plain checksum list
(`offload-crc32.txt`), not MHL / xxHash (xxhash-rust is BSL-1.0, outside the MIT / Apache / BSD rule).
**Provisional: (a), CRC-32** (reversible; MHL output would be ~2 h if a post house ever needs it).

## Q17 (2026-10-13, L2.9 / part 2): may the Dell download the Flatpak SDK to test the Flatpak build?

LINUX-PLAN's L2 acceptance says "Flatpak build runs with the same features through portals". The
manifests (`packaging/linux/flatpak/`) are written and lint-checked, but nobody has built them: the
Dell has the 25.08 GL / VA-API runtimes from Flathub already, not the build tools. Building needs
`org.freedesktop.Sdk//25.08`, `org.freedesktop.Platform//25.08` and
`org.freedesktop.Sdk.Extension.rust-stable//25.08` from Flathub (about 1–1.5 GB, user install, no
sudo) plus `flatpak-builder` (`flatpak install --user flathub org.flatpak.Builder`, or the pacman
package, which needs you). Options: (a) yes: the lane installs them `--user` on the Dell and runs
the build and a feature check (MPRIS, Inhibit, notifications, KDE Connect) in the sandbox, ~2 h;
(b) not now: the Flatpak stays "manifests only" and the L2 checklist says so. **Waiting on your
yes (downloads need it); until then (b), and the lane works on what isn't blocked.**
**Answered 2026-10-13 ~12:30: (a) yes.** The lane may install the Flathub SDK, Platform,
rust-stable extension and `org.flatpak.Builder` `--user` on the Dell (~1–1.5 GB), then build and
feature-check the Flatpak.
**Done 2026-10-13 12:40–13:25 (FYI):** built and checked (HANDOFF L1 / L2 line). It took more
disk than estimated: 7.2 GB in the Dell's `~/.local/share/flatpak`, because `org.flatpak.Builder`
pulled the 26.08 SDK and user copies of the GL / VA-API / codecs extensions that were already
installed system-wide. Kept so later rebuilds are quick; to free it:
`flatpak uninstall --user --all` on the Dell (and `rm -rf ~/flatpak-build`, 1.1 GB).

## Q18 (2026-10-13, U0.2.3b a): half-float preview is off by default for now (FYI, choice made)

Your §1a.64 keeps the half-float effect stage for preview only, with the bar "under one 10-bit code
value from full precision". Export now always renders full precision on every GPU
(`GpuCompositor::for_export`, with a test on both GPUs). Measured against full precision
(`half_float_preview_is_within_one_10_bit_code_of_f32`): colour ops 0.3–0.7 codes, a Gaussian
blur 0.99, **Sharpen / Unsharp 3.4–3.8 codes** (its six blur passes each round to half floats). And
on this PC's Intel UHD 750 half floats barely speed things up (the colour chain costs the same,
~490 ms per 6.2K frame either way: it is limited by arithmetic, not memory), so preview gains little.

Options: (a) **half floats off by default** (opt-in with `FILMCRAFT_FX_FORMAT=f16`), the switch
`HALF_ON_INTEGRATED` in `crates/gpu/src/fx.rs` ready for when Sharpen meets the bar; (b) on for
integrated GPUs except layers with Sharpen / Unsharp / blurs (needs two shader sets per GPU, ~2 h);
(c) remove the half-float code. **Provisional: (a)** (reversible: one constant). The graded-speed work
(§1a.65 c, d) continues at full precision.

**Answered 2026-10-13 (owner, live): keep (a)** ("what you did can be left alone"). Half floats stay
an opt-in preview option (`FILMCRAFT_FX_FORMAT=f16`), `HALF_ON_INTEGRATED = false`. Resolved.

## Q19 (2026-10-13, U0.2.3b c): Intel graded exports reach ~3.5 fps at 6.2K, short of helping (FYI, choice made)

Step c made the Intel UHD 750's graded frames 3.4× faster at full precision (6.2K, GPU time per
frame 960 → 286 ms: colour chains and effect passes now compile to small shaders specialised to what
they do, instead of one big interpreter). That is still ~3.5 fps against ~19 fps on the RX 6700 XT,
because Intel is now limited by memory bandwidth: every f32 pass over a 6.2K frame moves ~830 MB and
the bench grade takes ~8 passes. Options:
(a) **Stop here for Intel**: graded exports stay on the RX 6700 XT alone (the Auto rule of §1a.63
    already does this); the gain still helps preview on Intel-only machines. Cost: none.
(b) Fuse passes (colour chain + per-pixel effects into one pass, a combined Unsharp; maybe 2–3×
    fewer passes): a few days of shader work; at best ~8–10 fps on Intel, which would add roughly a
    third to graded 6.2K export speed when both GPUs run.
(c) Half floats on Intel for exports only: halves bandwidth, but you chose full precision for
    exports (§1a.64).
**Choice for now: (a)**, and U0.2.3b moves on to step d (AMD graded speed). Reversible: (b) can be
scheduled any time as its own step; nothing is removed.

## Q20 (2026-10-13, U0.2.3b d): a 1.5× faster 4K export from 6.2K that is not bit-identical (choice made, reversible)

When a 4K export is made from the X-M5's 6.2K clips, the Intel media engine can decode while the
RX 6700 XT grades and encodes. The bottleneck was Intel copying each full 6.2K picture across to the
AMD card. Letting Intel's video processor shrink the picture to the size it will be drawn at
(3240×2160) *during* that copy makes the export much faster (synthetic, full length): 4K graded
24.7 → 37–40 fps, 4K ungraded 26.0 → 41.8 fps. This beats the ≥ 30 fps floor.
The catch is that the output is no longer bit-identical to today's. Flat areas and edge positions
match (≤ 0.3 codes, no shift). Fine noise and texture differ because the downscale filter is
different: Intel's scaler averages the noise (it looks smoother) where the GPU path keeps more of
it. PSNR against today's output is 42.6 dB ungraded and 32.9 dB graded (the grade's sharpening
amplifies the texture difference). Step e asked for "output unchanged". Options:
(a) **Use the scaled hand-off** when Export engines = Auto and the export is smaller than the source.
    It applies only to clips placed by a plain fit, with no keyframed Motion, no temporal NR and no
    Maximum Render Quality (which keeps full size as today). Setting/env off-switch:
    `FILMCRAFT_HANDOFF_SCALE=1`.
(b) Keep exports bit-identical: no scaled hand-off, so 4K graded stays at ~25 fps.
(c) Change the GPU path's own downscale to an area filter as well, so both agree. This changes
    today's output for every export and preview of downscaled clips.
**Choice for now: (a)** as the plan for the plumbing. Until that plumbing lands, the code is only a
diagnostic (`FILMCRAFT_HANDOFF_SCALE` + `FILMCRAFT_DECODE_NODE`) and no export changes. Reversible:
the default is a single switch, and Maximum Render Quality always keeps full size.

**Update (2026-10-13 18:21, built):** Auto now does (a) in the export itself: one lane on the RX 6700 XT
whose decoders run on Intel, each source scaled to the most its clips ask for. The earlier
37–40 fps came from a diagnostic that shrank the bench clip too far: it has a 120 % Motion zoom, so
the correct scale is 62 %, not 52 %, and the diagnostic under-delivered pixels. **Correct numbers:**
4K graded 24.7 → **30.2–30.9 fps** (full length, synthetic). PSNR against today's single-chip output
is 33.8 dB graded, still a fine-texture difference only. Off-switches: Primary GPU only, Maximum
Render Quality, or `FILMCRAFT_HANDOFF_SCALE=1`.

**Owner (2026-10-14, live): (b) scheduled** as X422 step 2b, fused graded passes (§1a.71).

**Answered 2026-10-14 ~01:20 (owner, live): keep (a)** ("ok sounds good then"). Resolved.

## Q21 (2026-10-13, U0.2.3b e): 6.2K graded export stays at ~19.6 fps, short of the ≥ 24 aim (FYI, choice made)

Step e's full-length bench (synthetic, median of 2) passed three of four targets: 4K 32.5 / 35.1 fps,
4K graded 31.1 / 30.5, 6.2K 25.5. **6.2K graded is 19.6 fps** (19.1 before: not slower, but short of
the ≥ 24 aim). The RX 6700 XT's single media engine both decodes and encodes 6.2K, and that is the
limit. Things tried: two lanes on the AMD card were slower. Intel decoding for it reached 20.5 fps,
because copying a full 6.2K picture across costs ~34 ms inside the export, so the gain is too small
to switch on. Options:
(a) **Accept for now**: keep every gain, close U0.2.3b, continue with U0.2.4 (step e's rule for a
    lever short of its share).
(b) Later: a faster hand-off for full-size pictures (sharing Intel's decoded surfaces with the AMD
    card without a copy; needs a cross-vendor tiling both can read, probably not available).
(c) Later: render cache / smart-render parts for graded 6.2K timelines, which avoids the re-render
    on repeated exports.
**Choice for now: (a).** Reversible: (b) and (c) can be scheduled any time.

**Owner (2026-10-14, live): route to 24 fps wanted;** fused graded passes scheduled in X422 step 2b
(§1a.71). Render-cache reuse for exports (c) not scheduled.

## Q22 (2026-10-13, AI1.1): the tuned SmolLM2 explainer scores 70/75 on the new runtime instead of 72/75 (FYI, choice made, reversible)

AI1 replaces candle with FilmCraft's own quantized forward pass (no x86-64-v3 build flag needed).
On the 75 held-out cases (greedy, 80 tokens, repeat penalty 1.0, this PC, synthetic prompts):
`qwen05-v1` scores **74/75, identical** to its recorded run; `smol360-v1` (the default tune) scores
**70/75** (set 1 34/36 as before, set 2 36/39 instead of 38/39), and answers in **1.62 s** on average
instead of ~3 s. Its set-2 result moves with the order in which the maths adds numbers up: 36 with
our AVX2 kernels, 37 with our portable path (which is bit-identical to candle's plain path), 38 in
the recorded run (candle's AVX2 path). The cases that flip are claims just at the fact checker's
thresholds (a cooler highlight it omits, a red-curve brightening it overstates), so this is
rounding noise on borderline cases, not a broken model. AI-PLAN's accept line says "scores the
same". Options:
(a) **Accept within ±2 cases on 75** for the switch of runtime, and let AI1.6's retrained
    `smol360-v2`, which is evaluated only on the new runtime, settle the score.
(b) Hold AI1 until `smol360-v1` scores exactly 72 again (would mean matching candle's AVX2
    summation order, slower and arbitrary).
(c) Make `qwen05-v1` the default meanwhile (74/75, same speed now), against §1a.27's SmolLM2
    base rule.
**Choice for now: (a).** Reversible: (b) or (c) any time. Outputs:
`~/Apps/filmcraft-train/runs/ai1-own-pass/` (`eval.jsonl`, `score-set{1,2}.md`,
`eval-baseline-set2.jsonl`, `score-baseline-set2.md`).

## Q23 (2026-10-13, L1.5): upstream's non-blocking file dialogs (#260) taken early (FYI, choice made)

L1.5 builds on upstream's #260 ("file dialogs on Linux no longer stop the app"). The audit expected it
to arrive with U0.2, but U0.2 only synced the VA-API stack; the next full upstream sync is weeks of
commits away (565 pending). Options: (a) wait for that sync; (b) write our own; (c) **cherry-pick
upstream's commit `059376af` unchanged** (`-x`, so the next sync sees the same change) and add L1.5's
parts (dialog parented to the window, each kind of dialog starts in its last folder) as a
fork-side hook `run_dialog_from` next to upstream's `run_dialog`, leaving upstream's signature and
tests alone. **Choice: (c).** The next sync's merge of `native_dialogs.rs` / `main.rs` may need a small
hand resolution; the fork's version keeps both hooks.
## Q24 (2026-10-13, AI1.4): may the run download SmolLM2-135M-Instruct to try a "fast" explainer?

AI-PLAN step 4 tries a smaller tune: SmolLM2-135M-Instruct (Apache-2.0, Hugging Face
`HuggingFaceTB/SmolLM2-135M-Instruct`, about 270 MB of weights plus tokenizer / config, saved under
`~/.local/share/filmcraft-fork/models/llm-base/`), trained with TRAINING.md's pipeline and shipped
as a "fast" option only if it matches the 360M tune within a few cases. Downloads need your yes.
It matters less now: the 360M tune answers in ~1.6 s and shows its first words in ~0.3 s, so the
"fast" option would mainly help much slower PCs (the Dell is one to test it on).
Options: (a) yes, download and try it (~1 h of GPU training + evaluation); (b) skip AI1.4 for
good (the 360M tune stays the only one); (c) later, after AI1.
**Choice for now: skip AI1.4 and continue with AI1.5 / AI1.6**; if you answer (a), a later run
does AI1.4 (before or after the `ai-1` tag, it doesn't block it). Reversible.

**Answered 2026-10-14 ~01:10 (owner, live): (a) yes.** The run may download SmolLM2-135M-Instruct
(Apache-2.0, ~270 MB) and do AI1.4; keep the "fast" tune only if it passes. Doesn't block `ai-1`.

## Q25 (2026-10-14, AI1 end): the ARM (NEON) explainer kernels are not in `ai-1` (FYI, choice made)

§1a.69 says AI1 also adds the aarch64 (NEON) kernel path. `ai-1` shipped without it: on ARM the
explainer runs on the portable loop (correct, roughly the forced-portable speed: 1.9 s per answer
on this PC, slower on a small ARM board). Writing NEON kernels needs at least the
`aarch64-unknown-linux-gnu` Rust target and an emulator (`qemu-user`) to test them here, both
downloads, and real ARM hardware to measure. Options: (a) do it in **Phase 8 item 7** (Asahi /
ARM64), where ARM builds and tests happen anyway; (b) do it now as AI1.7 after you allow
`rustup target add aarch64-unknown-linux-gnu` + installing `qemu-user` (sudo: you); (c) skip.
**Choice for now: (a)**, X422 continues. Reversible: answer (b) and a run does AI1.7 next.

## Q26 (2026-10-14, A2 audit, Dell lane): parity oracle and test speech for voice isolation (FYI, choice made)

A2 ports DeepFilterNet3 to candle (weights downloaded with your 2026-10-12 consent, pinned and
checksummed, see `PREP-A2.md` §3). Two things the rules make me decide:
1. **Oracle.** To prove the port matches the original, something must run the original model.
   There's no PyTorch / onnxruntime here and installing them is a new download. **Choice:** build
   DeepFilterNet's own Rust tool (`libDF` / `deep-filter`, MIT / Apache, same repo as the weights)
   **from source on the Dell** as a test oracle only, like ffmpeg: never linked, never shipped.
2. **Test speech.** Kokoro voices aren't on disk, so the SI-SDR test set uses the built-in formant
   voice and generated speech-like signals. That's weaker evidence than real recordings.
   Options: (a) keep it so; (b) allow a small CC-BY speech set (e.g. ~50 utterances of LibriSpeech
   `test-clean`, CC-BY 4.0, ~30 MB) on the Dell only; (c) allow downloading the Kokoro voices the
   app already offers. **Choice for now: (a)**; answer (b) or (c) and a later run re-measures.

## Q27 (2026-10-14, X422 (b)): the 6.2K 4:2:2 HEVC master exports at ~7 fps, not 24 (FYI, choice made)

Exporting the graded 6.2K 4:2:2 clip to the "HEVC 4:2:2 10-bit master" preset now runs at **7.3 fps**
(was 0.5 fps: the colour conversion moved from the CPU to the GPU). The limit is the Intel UHD 750: it is the
only chip here that can both decode and encode HEVC 4:2:2, so it decodes, copies each frame across for the AMD
card to grade, and encodes, all at once. Measured levers (synthetic, `docs/fork-benchmarks.md` "X422"):
the encoder's speed setting changes nothing ("speed" 6.9, default 6.8, "quality" 5.0 fps); our ProRes 422 HQ
encoder at 6.2K runs at 0.5 fps (the ProRes path still renders the grade on the CPU); faster uploads into the
encoder (`vaDeriveImage`, tried) save CPU time but no frames, because the Intel chip, not the CPU, is full.
Options: (a) keep it: 7 fps (a 1-minute 6.2K 4:2:2 master takes ~4 minutes); for speed, export the 4:2:0
"6.2K Master HEVC" (case (a), 17 fps now, X422 works on it next); (b) later: render ProRes 422 HQ on the GPU
too (a separate step, its own measurement); (c) a future machine with a 4:2:2 HEVC encoder on the big GPU
(e.g. NVIDIA 50-series / Intel Arc) would lift this.
**Choice for now: (a)**, X422 continues with case (a) and the fused passes (§1a.71). Nothing to undo.

**Update 2026-10-14 ~06:30 (A2.7), now a real question: A2's acceptance needs your answer.**
Measured with (a): DeepFilterNet doesn't recognise the built-in formant voice as speech (clean
speech comes out at 2.1 dB SI-SDR instead of ≥ 20; it loses to DeNoise on most noise types; table
in FORK-PLAN §7). The port is verified (identical to the original, 68.2 dB), so only real speech
can show whether Voice Isolation meets the gate. Options:
(b) the lane downloads ~50 utterances of LibriSpeech `test-clean` (CC-BY 4.0, read English speech,
~30 MB, Dell only, never committed), converts them to 48 kHz WAV with ffmpeg (oracle use);
(c) download the Kokoro natural voices the app already offers (neural TTS: closer to speech, still
synthetic, ~330 MB);
(d) you record 2–5 minutes of your own voice in a quiet room (any app, 48 kHz WAV) and put the files
in `~/.local/share/filmcraft-fork/voice-test/` on the PC (the lane copies them to the Dell; they stay
out of git).
**Recommendation: (b)** (no work for you, permissive licence, many speakers), or (d) if you'd rather
test on your own voice. Until then the lane continues with A2.6 / A2.8 / A2.9 and re-measures when
you answer; the harness takes any folder of 48 kHz WAVs (`FILMCRAFT_EVAL_SPEECH`).

**Owner answer (live, 2026-10-14 ~06:20): (b) approved.** "If there's footage from elsewhere other than me
that you can use for this, that's totally fine." The Dell lane downloads the ~50 LibriSpeech `test-clean`
utterances (CC-BY 4.0, ~30 MB, Dell only, never committed), converts them to 48 kHz WAV with ffmpeg (oracle
use) and re-measures A2's gate on them. (c) Kokoro voices only if (b) proves unsuitable. The owner may still
add recordings of their own voice later in `~/.local/share/filmcraft-fork/voice-test/`; re-measure then too.

## Q-AI3-models (2026-10-14, AI3): which models may AI3 download? (answered)

**Answered live: SmolLM3-3B yes** (base + instruct, quantized and full-precision for the tune; on
this PC and the Dell). Phi-4-mini: not approved. OLMo 2 7B: only if AI3's eval shows SmolLM3 is
not enough; ask again with numbers then.

## Q28 (2026-10-14, X422 2b): fused graded passes kept; the graded aims are still short (FYI, choice made)

**What happened.** The fused passes (§1a.71) cut the bench grade's GPU work per 6.2K frame from 286 to 178 ms on the
Intel UHD 750 (~3.5 → ~4.9 fps for an Intel lane) and from 19.1 to 8.2 ms on the RX 6700 XT. Exports: 6.2K graded
19.6 → 19.7 fps (AMD is held by decode / encode there, not by rendering), X422 (a) 4:2:2 graded 17.1 → 19.9 fps.
The aims (Intel ~8–10 fps, 6.2K graded ≥ 24 fps) are not met.
**Choice made (per §1a.71):** keep the gain, record it, go on with X422's final benches. Output equals the old passes
within 1e-5 on both GPUs; `FILMCRAFT_FX_FUSE=0` brings the old passes back.
**Options for later, if you want them:** (a) nothing more (recommended for now); (b) bake the colour chain (input
transform + Basic + curves) into a GPU 3-D LUT per grade, which would cut Intel's chain cost (~60 ms of maths) at a
small accuracy cost (LUT interpolation, needs your bar); (c) let the Auto rule give Intel a share of graded 6.2K
exports now that its lane is ~4.9 fps (a few percent at best, since AMD is decode / encode-bound).
**(a)'s measured limit (X422 acceptance):** median of 2 = 20.0 fps; the Intel UHD 750 spends ~50 ms per frame on 4:2:2
decode + the ~104 MB hand-off copy to the AMD card, and AMD waits on it (`docs/fork-benchmarks.md` "X422 final").
(b) 4:2:2 HEVC master: 7.2 fps (Q27).

## Q29 (2026-10-14, Y3.4): how user shader effects work (FYI, choice made)

ADR [0006](../adr/0006-user-shader-effects.md), written by the scheduled run without a separate
designer (weekly usage above pace). The choices you may want to change: (a) user effects are
**GPU-only** (no CPU fallback; without a GPU they are skipped with a message); (b) the user writes
one function `effect(px) -> vec4` on **display-referred straight colour** (FilmCraft converts from
and to the linear working image), not raw linear premultiplied pixels; (c) the time limit is
**after the fact** (3 frames over 50 ms at 1080p disables it; a GPU hang quarantines the file),
because WGSL can't be stopped mid-frame. **Choice for now:** as in the ADR. Reversible: answer here
and a run changes the ADR before step 4 (the runtime); steps 2–3 (parser, registry) don't depend on (a)–(c).

**Owner answer (live, 2026-10-14 ~12:55): ok** (GPU-only stays).


## Q30 (2026-10-14, A2.8b, Dell lane): Voice Isolation misses two of its own acceptance bars on real speech (FYI, choice made)

On ~50 LibriSpeech recordings (your Q26 yes), Voice Isolation beats DeNoise in every case and by a
lot where noise is loud: **+7.1 dB at 0 dB SNR, +4.0 dB at 5 dB** (traffic, keyboard, wind). Two
bars I set before measuring are missed: at **10 dB SNR** (mild noise) the margin is only **+0.8 dB**
(bar 3 dB), and **clean speech** through it scores **17.0 dB** (bar 20 dB). Part of the clean-speech
number is probably LibriSpeech itself: its "clean" recordings have background 23–38 dB under the
voice, which the model removes and the measurement counts as damage. Clean Up Voice itself passes
its gate (one click is as good as the written manual process in all 15 cases, PREP-A2 §1).
Options: (a) accept as is: big gains on noisy voice, small ones on mildly noisy voice, slight
change to an already clean voice (Clean Up Voice turns noise reduction off on clean recordings
anyway); (b) keep A2 open and re-measure on recordings of your own voice in a quiet room
(`~/.local/share/filmcraft-fork/voice-test/`), which would separate "the model" from
"LibriSpeech's own noise"; (c) lower the default Voice Isolation amount for mild noise.
**Choice for now: (a)** with the numbers written into "A2 ready for acceptance"; (b) any time you
drop recordings there. Nothing to undo.

**Owner answer (live, 2026-10-14 ~12:55): (a) accepted.** "Yeah for now that's fine. We can train it
better later." Re-measure if own-voice recordings appear.


## Q31 (2026-10-14, H1 audit, Dell lane): test files for interlaced H.264 field pictures (PAFF) (question; MBAFF goes ahead)

H1 makes interlaced H.264 (1080i camcorder footage) decode on the graphics card. There are two kinds
of interlaced coding: **MBAFF**, which I can make test files for myself (x264 writes it), and **PAFF**
(each field coded as its own picture), which neither x264 nor these GPUs' encoders can write. To test
PAFF properly I'd need either (a) the ITU-T H.264.1 **conformance streams** (the official test
bitstreams, freely downloadable from the ITU, tens of MB, kept on the Dell only, never committed),
or (b) a 1080i file from a camcorder you have, or (c) nothing: PAFF then gets unit tests on synthetic
headers only and no real-file check. **Until you answer: (c)** for PAFF, MBAFF fully tested; nothing
to undo. Downloads still need your yes.

**Owner answer (live, 2026-10-14 ~12:55): (a) yes.** Download the ITU-T H.264.1 conformance streams
(Dell only, never committed) for PAFF and the other field / H2 conformance checks.


## Q32 (2026-10-14, H2.7, Dell lane): slice-level threads in the H.264 software decoder (FYI, choice made)

H264-PLAN H2 item 3 asks for slice-level threads, then frame-level. Frame-level threading already
exists and measured on the Dell: 4K 10-bit 4:2:2 Long-GOP **43 fps**, a very high-rate (632 Mbit/s)
4K 10-bit 4:2:2 Intra stream **27 fps** (docs/fork-benchmarks.md, H2.7). Slice threads would only
shorten the wait for a single frame (scrubbing / a paused frame: ~0.1 s Long-GOP, ~0.3 s Intra at 4K
on one core) and only on streams coded with several slices; a safe-Rust version needs per-slice views
of the picture (a large refactor of the slice decoder). Options: (a) defer slice threads until a real
file shows the single-frame wait matters; (b) build them now. **Choice: (a)**, revisit at H2.8 with
real 4:2:2 / interlaced files and the i5-11600K numbers. Nothing to undo.
