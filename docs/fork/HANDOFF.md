# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–49 incl. §1a.6 order and its 2026-10-12 revisions §1a.40–49; §5 phases; §7 progress log), then [`GRADE-PLAN.md`](../../GRADE-PLAN.md)
(G2c done; G3 later), [`RESOLVE-UI-PLAN.md`](../../RESOLVE-UI-PLAN.md) (§1a.39), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md),
[`EXPORT-PLAN.md`](../../EXPORT-PLAN.md), [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md),
[`AUDIO-PLAN.md`](../../AUDIO-PLAN.md), [`AI-PLAN.md`](../../AI-PLAN.md), [`RAW-PLAN.md`](../../RAW-PLAN.md),
[`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md), [`H264-PLAN.md`](../../H264-PLAN.md),
[`DECODER-PLAN.md`](../../DECODER-PLAN.md), [`FORK-ONLY.md`](FORK-ONLY.md), [`docs/guide/README.md`](../guide/README.md),
[`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md) (Q9 answered 2026-10-12: TNR per clip, step 6.7b),
then this file. Rewritten 2026-10-12 after G2c (scheduled run: tagged `grade-2c`, built, published).
**Owner decisions 2026-10-12 (live session), all in FORK-PLAN §1a.40–49 — read them:** 6.7b TNR per clip
(design `docs/fork/DESIGN-6.7b.md`); small phases accepted in groups (first: resolve-ui-1 + RS3 + 6.7b + CO1 + Y1,
one acceptance after Y1); L1 / L2 Linux integration as a Dell lane (LINUX-PLAN, `docs/fork/LINUX-AUDIT.md`;
**owned by the scheduled task `filmcraft-dell-lane` (lock `.dell-lane.lock`); the main runner leaves it alone.
State 2026-10-12 17:10 (handed off for 5-hour pacing, §1a.52): nothing merged yet. 11 commits sit on branch
`worktree-agent-a3613e020ce2b61ce` (worktree `.claude/worktrees/agent-a3613e020ce2b61ce`; backup `origin/dell-lane-wip`),
rebased on `a14c61fd`: L2.1 XDG dirs + `app.paths` + one-time move of old Logs / Media Cache; L1.7 taskbar progress,
notifications ("Show in Folder"), idle / suspend inhibit, single instance (`--new-instance`); FORK-ONLY KIO thumbnailer
(`fork/kde-thumbnailer`); L2.2 MIME type, Open With (video), desktop actions, recently-used.xbel, `filmcraft-cli thumbnail`,
`.thumbnailer`; review fixes (Fable review, all 4 majors + minors 5–12 done; 13 waits for RS3: autosave.rs); L2.3 MPRIS;
L2.8 journal with levels + error window "Report an Issue…"; L1.9 HDR write-up (OWNER-QUESTIONS Q11) + L2.5 trash rule;
L2.7 background work at nice 10; L2.9 PKGBUILD (+ split `filmcraft-kde-thumbnailer`) + Flatpak grants. Guide: `docs/guide/linux.md`
(kept out of the Coach index: it shifted colour answers). Dell: scratch repo `~/Projects/filmcraft-linux` (own target),
gates script `~/fork-logs/linux-gates.sh`; run **s7** at `47432b27` was running at hand-off (log `~/fork-logs/linux-s7.log`;
tests use `--test-threads=4`, s6 at full parallelism was killed by systemd-oomd). Fixed since s6: coach index, dev_icon test,
zbus `NameTaken` hand-off. **Not this lane's:** i18n menu-label tests (`spanish_translates_every_menu_label`, Chinese / German /
Portuguese / Russian / Ukrainian catalogue tests, and `crash::tests::hook_records_and_logs_worker_panics` failing through them)
fail on Resolve-term labels ("Media Pool", "Inspector", "Timeline Settings…") from RS2 / RS2b on hevc-vaapi.
Next: s7 green (apart from those) → one opus review of the delta since the Fable review (MPRIS / journal / priority / packaging)
→ fast-forward merge → §7 log; then `makepkg` check on the Dell, L1.6 arboard `wayland-data-control`, L2.10 KDE Connect,
L2.4 udisks, L2.6 GPU choice (after U0.2), L1.5 (after U0.2); after RS3: L1.3 / L1.4 (prefs, theme), MPRIS / journal Settings switches.
Owed on the PC: Program monitor 1:1 at 125 / 150 %, hybrid-GPU default, Plasma taskbar progress / notification / Dolphin
thumbnail / media keys on the real desktop**); Fable for the hardest builder lanes (runner prompt §2c); Y1 on upstream's `vaapi/` decoder
(`docs/fork/PREP-Y1-6E-U02.md`); Dolphin thumbnails via fork-only `fork/kde-thumbnailer/`; CO1 Coach quick fix +
a non-prescriptive node-order page (options, beginner vs pro, why — the rule for every workflow page); AI help
for Fusion in C1 (COMPOSITOR-PLAN C1 item 7; design `docs/fork/DESIGN-C1.md`); AI1 moved before C1, new AI2
after C2; explainer retrained at the end of AI1 (`smol360-v2`) and in AI2 (`v3`) (AI-PLAN).
The G2c / RS1 owed-item fixes are merged (owner session, 2026-10-12 evening; see §7).
**L1 / L2 two-part acceptance (§1a.54):** part 1 (all but L1.5 / L2.6) rides with the **6E** group (tags `linux-1a` / `linux-2a`); part 2 with the U0.2 group (full `linux-1` / `linux-2`).
Phase order (FORK-PLAN §1a.6, the only valid order): … → 6 ✓ → G2c ✓ → **RS3** (after RS2 / RS2b) → 6.7b → CO1 → Y1 → 6E → U0.2 → A1 → Y3 → **AI1** → C1 → C2 → **AI2** (§1a.48) → A2 → **S1 → S2** ([`STORAGE-PLAN.md`](../../STORAGE-PLAN.md), §1a.50; A2 first, §1a.51) → G3 (L1 / L2: Dell lane, accepted with the next group when ready, §1a.43; KDE Connect is L2 item 10, §1a.50) → 7 (+ Y4) → Y2 → H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 → R2 → D → 8;
RESOLVE-UI-PLAN RS1 + RS2 run as a lane alongside (§1a.39).
**Owner, 2026-10-12 (§1a.40–41):** Q9 answered (TNR per clip, new step **6.7b** after RS3, before Y1);
Coach fixes go in C1. **Small phases are accepted in groups:** don't run a separate `cargo xtask ci` /
build / publish for `resolve-ui-1`, RS3 or 6.7b; the first group (`resolve-ui-1` + RS3 + 6.7b + Y1)
gets one acceptance after Y1, with all four tags on that commit. Each step still runs its in-between gates.
**Owner, 2026-10-12 (§1a.43): L1 / L2 run now as a Dell lane** (edits here, every build / test on the Dell,
nested KWin for window checks; window chrome / theme parts wait for RS3). The runner doesn't start L1 / L2 on this PC.
When L1 or L2 is done, accept it with the next main-order phase's group (tag `linux-1` / `linux-2` there); don't wait for AI1.
**Y1 (§1a.45): build on upstream's `vaapi/` H.264 decoder with a zero-copy `Accel`, per `docs/fork/PREP-Y1-6E-U02.md` §1.4 (≈ 7 h); no `va_h264.rs`.** 6E / U0.2 maps are in the same file.
**Designs ready:** 6.7b → `docs/fork/DESIGN-6.7b.md` (schema v17 migration, ≈ 11–14 h, 8 steps); C1 → `docs/fork/DESIGN-C1.md` (ADR 0006, one DAG model shared with C2, ≈ 36–44 h, 13 steps, leaf lanes T/A/B/C/D). Follow them; their provisional choices are listed at the end of each.
**CO1 (§1a.47), in the current group after RS2 merges:** Coach fix steps 1–2 from `docs/fork/coach-eval/DIAGNOSIS.md` + the eval set as a test (floor = measured rate) + the node-order guide content (several example trees, beginner vs pro, *why* each order; never one prescribed way). C1 also gets AI help for Fusion (COMPOSITOR-PLAN C1 item 7).

**Usage (owner, 2026-10-12, FORK-PLAN §1a.52): keep working, pace the 5-hour window so it never runs out early** (line 95 % × elapsed fraction; slower modes above it, a brief throttle only far above it; backstop 95 % / weekly 96 %). Don't stop otherwise.

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0`,
  `grade-1`, `upstream-sync-1`, `grade-2`, `grade-2b`, `grade-2c`. Builds in
  `~/Apps/filmcraft-builds/`; `latest` → `grade-2c`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public snapshots / releases on
  dawgon2/filmcraft-cachyos branch `fork` (§1a.33); latest `fork-grade-2c`.
- **G2c done** (FORK-PLAN §7, 2026-10-12): `crates/color/src/wb.rs` (shared Temperature / Tint
  gains, exact inverse, box average, grey-world Auto, CIEDE2000), commands `grade.wb.pick` /
  `grade.wb.auto` / `lumetri.wb.pick` / `lumetri.wb.auto`, buttons under Primaries ▸ Tint and in
  Lumetri ▸ Basic Correction (click or drag a box in the Program picture). Graphic / mask
  overlays stand aside while any picker is armed (they used to swallow picker clicks).
- **RS1 done (merged, part of tag `resolve-ui-1` later):** feature `fork-resolve-default`
  (enabled by `apps/filmcraft`, so `cargo xtask ci` runs every test with Resolve as the
  default: **new UI tests that assume Premiere must set `prefs.workflow = Premiere` or run
  `settings.setWorkflow premiere`**), one-time prefs migration (`crates/engine/src/fork_resolve.rs`,
  marker file `fork-resolve-migrated` in the data dir), `ThemeKind::Resolve`. The owner's own
  prefs migrate on the first launch of the grade-2c build.
- **Owed (not blocking):** From Phase 6: frame server per-frame proxy hint; `cache.claimFolder`; Export Frame without
  TNR; automated 4:2:2 export chroma test; decode-to-screen fps with hardware 4:2:2 in the app;
  software 4:2:2 decoder memory cap / multithreading (ask first); TNR / cache / proxy 6.2K bench
  cases; VPP context leak on purpose (iHD crash on destroy).
- Schema **v17** since 6.7b (clip `temporal_nr`; `v16_to_v17` moves node `tnr.*`); prefs version 3. New user-visible strings (and engine command labels)
  need es + zh-cn entries. New guide pages go in `GUIDE_PAGES` (`crates/engine/src/coach_ask.rs`)
  and their `{ui:}` ids must be known to the `coach_ui` walk tests.
- Cloud: merged up to `55f8639c`; CLOUD-QUEUE has GUIDE-7/8, COACH-WALK-1, Y1-PREP, I18N-2.
  Merge `origin/cloud-work` first if it has new commits.

## Run state (2026-10-12 evening, scheduled run; RS2 and RS3 merged)

- **RS2 merged** (`eba0e218`): engine `projects.*` (`crates/engine/src/projects.rs`: listing,
  summaries + poster cache, hostile files, OS trash with a checked `.Trash-<uid>`, no-overwrite
  publishing via hard links, projects folder must be absolute and never `/` / home / an ancestor
  of home), UI home screen `crates/ui-egui/src/panels/home.rs` (listing on a background thread
  under `catch_unwind`), guide `docs/guide/projects.md`. Opus review: 2 medium + 4 low findings,
  all fixed with tests. Known limits: the trash uid comes from the home folder's owner (no
  unsafe); `projects.new` checks the name then calls `file.saveAs` (tiny race); Export makes a
  folder, no `.zip` yet; side-by-side screenshot vs `resolveHomePage.png` still owed (run
  `projects_ui` with `FILMCRAFT_UI_SHOTS=<dir>`), do it in RS3's screenshot pass.
- RS1 + RS2 + RS2b are all on hevc-vaapi: `resolve-ui-1` is complete; its tag goes on the
  group-acceptance commit after Y1 (§1a.41), not now.
- Usage pacing per §1a.52 (5-hour line L5 = 95 × elapsed fraction; owner correction 16:30:
  pace, don't stop early).

## Exact next step: the group acceptance (ci running at hand-off time), then 6E

RS3 merged (`18583b31`, see §7). RS1 + RS2 + RS2b + RS3 are all on hevc-vaapi; their tags
`resolve-ui-1` and `resolve-ui-2` go on the group-acceptance commit after Y1 (§1a.41).
RS3 owed (polish, not blocking): Resolve Color page Primaries cramped under ~1000 pt window
height (shorter page bar, or scroll to the WB row); toolbar shows Markers instead of Index; no
Sound Library / Keyframes / LUTs / Lightbox panels; Fairlight top strip isn't per-track meters;
RS2 home-screen screenshot vs `resolveHomePage.png` still not compared side by side.
1. **6.7b done** (2026-10-12 21:55, §7): temporal NR is a clip setting (schema **v17**, `tnr.*`
   commands, Effect Controls / Inspector + Noise tab rows, guide `color/noise.md`). Owed polish:
   keyframe markers for the TNR rows in the Effect Controls lane; label column too narrow for
   "Temporal NR chroma". Tag `phase-6-7b` at the group acceptance.
2. **CO1 done** (2026-10-12 22:11) and **Y1 done** (2026-10-12 23:48, §7): zero-copy VA-API
   H.264 on upstream's front end (`va_accel.rs`, `Accel::pick`, `vaapi/gray.rs`); owed items in §7.
3. Group acceptance after Y1 (§1a.41): one `cargo xtask ci`, tags `resolve-ui-1`, `resolve-ui-2`,
   6.7b's, CO1's, `yt-1` on that commit, one build + publish.

## Environment and gotchas

- **Disk rule (FORK-PLAN §1a.12):** keep ≥ 20 GB free on `/home`; `df -h /home` at session start
  and after each build; clean `target/debug/incremental` / `target/debug` / stale build dirs in
  the repo when needed; never delete `target/fixtures`, `target/fork-bench`, `target/bench`,
  `~/Apps/filmcraft-builds` (keep `target/release`: the MCP config runs
  `target/release/filmcraft-cli`); can't stay above 20 GB → reason in `.fork-plan-blocked`, stop.
  2026-10-08 after cleanup: 182 GB free (`target/debug` deleted; `target/` 31 GB).

- **AI explainer models (G2.11, owner-approved downloads 2026-10-11):** GGUF candidates +
  tokenizers in `~/.local/share/filmcraft-fork/models/llm/` (qwen2.5-0.5b q4_k_m, qwen2.5-1.5b
  q4_k_m, smollm2-360m q8_0, smollm2-1.7b q4_k_m; SHA-256 checked); full-precision bases for
  training in `~/.local/share/filmcraft-fork/models/llm-base/{SmolLM2-360M-Instruct,
  Qwen2.5-0.5B-Instruct}/` (pinned revision in `REVISION`). Trainer venv
  `~/Apps/filmcraft-train/.venv` (Python 3.12, torch 2.5.1+rocm6.2, transformers, peft; 18 GB);
  run it with `HSA_OVERRIDE_GFX_VERSION=10.3.0` (RX 6700 XT = gfx1031; ~6 TFLOPS bf16, 12 GB).
  Tuned models are published as releases on the public dawgon2/filmcraft-cachyos only after the
  owner says "publish" (OWNER-QUESTIONS Q7).
- **Rust** 1.99 via rustup in `~/.cargo` (not pacman): `export PATH="$HOME/.cargo/bin:$PATH"`.
  `bindgen-cli` 0.73.2 is installed there (user-level) for regenerating `va_sys.rs`.
- **Bench `--only`:** the export section is gated by `wants("export")` (substring of the case
  name in the filter, not the other way round): use `--only export` (all three cases).
- **Shell is zsh**: unmatched globs are errors (quote `--include='*.rs'`); `set -- $var` does
  not word-split (use `bash -c '…'`); no `bc` (use python3). `pgrep -f` / `pkill -f` match
  their own command line (`pkill -f gates-2.2b` killed the tool's shell): use `pgrep -x cargo`,
  `pkill -x <name>`; killing cargo leaves orphaned `rustc` processes (`pkill -x rustc`).
- **Foreground commands time out at 10 min.** Run long work detached
  (`(cmd > log 2>&1; echo "exit $?" >> log) &; disown`) and wait with
  `timeout 590 bash -c 'until grep -q "^exit" log; do sleep 15; done'`. Never chain `sleep`s.
- **Dell lane (FORK-PLAN §1a.32):** `ssh dell` (fish shell: send `bash -s` scripts) is a second
  machine with an Intel UHD 770, set up and green (full build 392 s, tests 380 s). Use it for
  Intel-path tests (6.1b 4:2:2 parity first), long tests / soaks and weaker-PC checks; sync
  with `git push ssh://dell~/Projects/filmcraft-main hevc-vaapi`.
- **GPUs:** AMD RX 6700 XT = `/dev/dri/renderD129`, Vulkan GPU0; Intel UHD 750 = `renderD128`,
  Vulkan GPU1. Force one for wgpu tests: `MESA_VK_DEVICE_SELECT=1002:73df` (AMD) or `8086:4c8a`
  (Intel) plus `MESA_VK_DEVICE_SELECT_FORCE_DEFAULT_DEVICE=1`. Run GPU parity tests on both.
- **GPU stall with the video engine flat out (session 4):** decoding pictures back to back on
  VCN made compositing wait 40–100 ms per frame (`device.poll`); spread decoding did not.
  Reverse playback paces VA submissions (`cancel::with_pace`). Not caused by DMA-BUF import,
  surface churn or VA queue depth (all measured). Watch for it wherever a burst of decoding
  happens while playing (proxies, cache rendering in Phase 6).
- **Driver quirks found:** Mesa's VA-API wants HEVC scaling lists in raster order of each 4×4 / 8×8
  matrix (not the coded diagonal order the VA header suggests); RADV/Intel interpolate 1:1
  sample positions slightly off texel centres (fixed by snapping in `composite.wgsl`); ffmpeg's
  MOV muxer writes `hvcC numTemporalLayers = 6` for single-layer x265 streams (use the SPS);
  libva reuses surface ids after destroy (key caches by our own memory id).
- **Bench:** `--only "jkl J 1x"` runs one JKL case; `L 4x warm` / `J 2x warm` play the speed
  elsewhere first (remembered step); output has `steps` and cadence (`gap_ms`).
- **Env vars:** `FILMCRAFT_VAAPI_DEVICE` (render node), `FILMCRAFT_FFMPEG`,
  `FILMCRAFT_NO_GPU_IMPORT` (app/bench: no Vulkan DMA-BUF import → read-back path),
  `FILMCRAFT_VRAM_CACHE_MB` (decode-cache VRAM budget), `FILMCRAFT_SOAK_MINUTES` (bench soak),
  `FILMCRAFT_BENCH_TRACE=1` (bench lists frames that finished after they were due),
  `FILMCRAFT_BENCH_KEEP=<dir>` (bench keeps each export's output), `FILMCRAFT_NO_VA_ENCODE`
  (export through ffmpeg), `FILMCRAFT_FULL_SIZE_GRADE` (grade converted media at full size),
  `FILMCRAFT_XM5_DIR` (where the xm5 clips live; real clips can be linked there by name),
  `FILMCRAFT_NO_ZERO_COPY` (export reads frames back instead of rendering into encoder surfaces),
  `FILMCRAFT_EXPORT_TRACE` (per-batch export stage times on stderr).
- **Worktree lanes (session 10):** leaf builders in `.claude/worktrees/agent-*` with
  `CARGO_TARGET_DIR=target/agent-<name>` (~5 GB each); remove the worktree after merging.
- **Disk:** see the disk rule above; a debug build of the workspace regrows `target/debug` to ~60 GB. Fixtures: `target/fixtures/xm5/` (12 GB synthetic), `target/fixtures/hevc/`
  (conformance, `.hevc` + ffmpeg `.yuv`), `target/fixtures/platform/`, `target/fixtures/hevc4k/`.
- **Bench:** `target/release/examples/bench --section xm5 [--only CASE] [--quick] [--json f]`;
  cases ceilings, decode, play-full, play-graded, jkl, scrub, multicut, export, 422-play, and only
  when named: soak, open. Output line includes VA surfaces, cache VRAM and GOP seeks / decoded /
  skipped / evicted. Full suite ≈ 75 min (graded 4K export alone ≈ 40 min). Saved binaries in
  `target/fork-bench/bench-phase{0,1,2}`, results in `target/bench/xm5-phase*.json`.
- **Benchmark pitfalls hit:** run on an idle machine (an x265 encode, a build or orphaned
  `rustc`s inflate everything; check `uptime` and `ps`); never run benchmarks during the soak
  (they share the decode engine); `--quick` multicut covers only 3 cuts near GOP starts, the full
  60 s run covers 20 cuts deeper into the clips (much worse); bench plays used to leak a
  `FrameServer` each (fixed) — results of a case can depend on the cases before it in one process;
  the synthetic clips are x265 `ultrafast` open-GOP (CRA + RASL), keyint 30, ~49 % disposable.
- **CI:** `cargo xtask ci` ≈ 22 min since session 13 (nextest, `[profile.ci]`, one UI test binary `crates/ui-egui/tests/ui/main.rs`: a new UI test file needs a `mod` line there; prints per-stage times); run idle — `mixer_tests::perf_24_tracks_3_effects_realtime_factor`
  needs ≥ 2.5× real time on one core (4.7× idle). `cargo test --workspace --release --no-fail-fast`
  shows every failure.
- **App smoke test:** `filmcraft --data-dir <scratch> --control <port> --empty`, JSON lines
  `{"id":1,"method":"engine.execute","params":{"command":"file.import","params":{"paths":[…]}}}`,
  `file.newSequenceFromClip` `{"items":[id]}`, `ui.playback` `{"action":"play"}`, `perf.stats`,
  `ui.screenshot` `{"path":…}`, `app.quit`. Playback resolution: `ui.set {"playbackRes":"Full"}`
  (serde wants variant names: `{"program":{"res":"Full"}}` works, `"full"` is refused). JKL:
  `engine.execute` `playback.forward` / `playback.reverse` (each call doubles the speed while
  playing), `playback.stop`; shown / dropped in `ui.inspect` → `playback`. Script used in
  session 4: a Python JSON-lines client (see FORK-PLAN §7, Phase 3.3).
- **Silent audio tests:** cpal's PulseAudio client ignores `PULSE_SINK` and sets no
  `application.process.id`; a long test once played on the owner's speakers. Find the stream by
  `application.name` (check it first) before moving it to a null sink, or mute it another way.
- **Permission classifier:** the agent cannot set up self-restarting automation (`/loop`, timers
  that start `claude`); the owner does that (the runner script).

## Dead ends (with measurements, all synthetic)

- JKL (Phase 3.2): more spare VA surfaces (48 instead of 8) against the reverse GPU stall: no
  change; limiting VA submissions in flight (1, 2, 3, 64): 12–44 drops of 300, no fix (pacing
  the rate did it); a skimming tolerance of a quarter step (instead of half): grid frames deep
  in a GOP cost every reference before them and 4× climbed to step 24–30.

- ffmpeg pipe tuning (Phase 1): a 1 MiB pipe buffer did not help (22.2 vs 21.8 fps); ffmpeg's own
  download + write is the cap.
- Vulkan Video as primary: 5 % slower than VA-API in ffmpeg (40.9 vs 43.2 fps), frames identical.
- Multicut / cuts (Phase 2–3): requesting the first frame after the next cut early (20–90 frames
  ahead, with or without the priority gate), a deeper prefetch window (20–30 frames), a per-clip
  floor of 16 frames, waiting for every picture inside the priority gate (−6 % throughput):
  all made cut drops worse or did nothing. What helped: the open-GOP seek fix (correctness) and
  skipping non-reference pictures before a clip's in-point (~98 → 25–42 drops of 360).
- VRAM: Auto budget of half the VRAM ran a 12 GB card out of memory with four 6.2K clips; a
  quarter is the default. The monitor frame cache charging GPU frames their full size left room
  for ~9 plans (fewer than the prefetch window); it charges 16 MB per GPU frame.

## Open problems

- 6.2K cuts every few seconds drop frames on original media (decoder-bound) → Phase 6.
- 4× on this synthetic stream (51 % reference pictures, open GOP) runs at the decoder's limit
  even at step 12–15: steady skipping at ~8–10 pictures/s, not more.
- 4:2:2 HEVC does not decode at all (no decoder on Linux) → Phase 6.1.
- Graded 4K export 27.5 fps (just under real time): one VCN shared by the 6.2K decode and the
  encode; a possible 6.2K optimisation phase later (FORK-PLAN §1a.16, only with the owner).
- First play after loading drops frames for ~0.5–1 s (cold start) → Phase 7.
- Fujifilm's F-Log2 LUT changes gamut on the log signal; built-in is linear light (FORK-PLAN §7).
- Grade node ids are per grade (`Grade::next_id`); the current node is clip-scoped
  (`EditorState.grade_node: Option<GradeSel>`).
- Linux default keymap has shortcut conflicts (Ctrl+9, Ctrl+Shift+M, Ctrl+T) — not fixed.
