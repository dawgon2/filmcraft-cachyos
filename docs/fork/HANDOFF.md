# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–57 incl. §1a.6 order and the 2026-10-12 revisions §1a.40–54; §5 phases; §7 progress log),
then [`EXPORT-PLAN.md`](../../EXPORT-PLAN.md) (6E, next), [`PREP-Y1-6E-U02.md`](PREP-Y1-6E-U02.md)
§2 (6E map) and §3 (U0.2), [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md), [`GRADE-PLAN.md`](../../GRADE-PLAN.md),
[`RESOLVE-UI-PLAN.md`](../../RESOLVE-UI-PLAN.md), [`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md),
[`AUDIO-PLAN.md`](../../AUDIO-PLAN.md), [`AI-PLAN.md`](../../AI-PLAN.md), [`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md),
[`STORAGE-PLAN.md`](../../STORAGE-PLAN.md), [`H264-PLAN.md`](../../H264-PLAN.md), [`RAW-PLAN.md`](../../RAW-PLAN.md),
[`DECODER-PLAN.md`](../../DECODER-PLAN.md), [`FORK-ONLY.md`](FORK-ONLY.md), [`docs/guide/README.md`](../guide/README.md),
[`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), then this file.
Rewritten 2026-10-13 00:50 after the first group acceptance (scheduled run).

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → 6 ✓ → G2c ✓ → RS1–RS3 ✓ → 6.7b ✓ →
CO1 ✓ → Y1 ✓ → **6E** → U0.2 → A1 → Y3 → AI1 → C1 → C2 → AI2 → A2 → S1 → S2 → G3 → 7 (+ Y4) → Y2 →
H1 → H2 → H3 → C3 → C4 → G4 → A3 → R1 → R2 → D → 8. Small phases are accepted in groups (§1a.41).
**L1 / L2 (Dell lane, owned by the scheduled task `filmcraft-dell-lane`, lock `.dell-lane.lock`;
the main runner leaves it alone):** part 1 rides with the **6E group** (tags `linux-1a` /
`linux-2a`, §1a.54) once the lane writes "L1 / L2 part 1 ready for acceptance" here; then add its
PC-only checks to 6E's acceptance (Program monitor 1:1 at 125 / 150 %, real-desktop taskbar
progress / notifications / Dolphin thumbnails / media keys). Part 2 with U0.2.
**Part 1 trimmed (owner, 2026-10-13 05:55, §1a.54):** L1.8, L2.4 and L2.10 move to part 2; part 1
= merged steps + L1.3 + L1.4 + L1.6 + MPRIS / journal switches + `makepkg` check, then "ready".
**L1 / L2 part 1 ready for acceptance (2026-10-13 07:00, Dell lane).** Merged into hevc-vaapi:
L2.1 XDG dirs, L1.7 taskbar progress / notifications / inhibit / single instance, L2.2 MIME / Open
With / recent files / thumbnailer (+ FORK-ONLY KIO shim), L2.3 MPRIS (+ `Seeked`) and its Settings
switch (`3eafdb9d`), L2.8 journal + Report an Issue (journal switch dropped, Q14), L1.9 / L2.5
write-ups (Q11), L2.9 PKGBUILD / Flatpak grants, L2.7 dropped (Q13), L1.3 window size / maximised
restore + GNOME CSD (`d0c30393`), L1.4 accent (`04f5b593`) + Animations / reduced motion
(`909ce174`; text-scale hint to part 2, Q15), L1.6 Wayland clipboard + IME / touchpad tests
(`4926e868`; DnD / pinch blocked on winit, Q10, fallbacks in the guide). Worktree
`.claude/worktrees/dell-lane` (branch `dell-lane`).
*Checked on the Dell (UHD 770, Plasma 6 Wayland):* gates per step (fmt, clippy, touched tests,
layers, wasm); UI tests for every new setting in both workflows; `makepkg -f --nodeps` builds both
packages (`filmcraft`, `filmcraft-kde-thumbnailer`), `check()` passes, `desktop-file-validate` and
`appstreamcli validate` clean, binary links only libasound / libc / libgcc (the rest is loaded at
run time), the KIO plugin has no missing libraries.
*Owed on the PC at acceptance (add to 6E's):* native Wayland window with KWin decorations and the
icon; Program monitor 1:1 and crisp at 125 / 150 %; accent, light / dark and Animation speed ▸
Instant follow System Settings live; window size / maximised restored; taskbar progress +
completion notification on an export; screen doesn't lock during 10 min of playback; Dolphin
thumbnails of `.fcproj`; double-click a project in Dolphin opens it in the running FilmCraft;
media keys control playback; logs in `journalctl --user`; Paste of files from Dolphin; the same build under
X11 and a nested non-KDE compositor without errors; `makepkg -si` install (needs sudo: owner).
*Part 2 (after U0.2):* L1.5 portal dialogs (parent + last folder), L2.6, L2.4 card import,
L2.10 KDE Connect, a Flatpak build, the text-scale hint (Q15). **L1.8 merged** (`b6d302dd`,
Settings ▸ Playback ▸ Global shortcuts via the GlobalShortcuts portal; off by default; mock-portal
bus test). *Owed on a real Plasma desktop (part 2 acceptance):* the BindShortcuts dialog appears,
the keys fire with another window focused, System Settings ▸ Shortcuts lists FilmCraft.
**L2.10a merged** (`bd93a644`, Project panel ▸ Send via KDE Connect, `kdeconnect.devices` / `kdeconnect.send` through the running kdeconnectd; review done 07:40, no blocking findings). *Owed on a real desktop (part 2):* send an export to a paired phone. **L2.10b merged** (`1f33cc46`, files shared from a phone → offer bar / Inbox bin / ignore, Settings ▸ Media; review done 08:00, no blocking findings). **L2.10c merged** (`4b5c8791`, guide only: KDE Connect's Multimedia control drives the existing MPRIS player). *Owed on a real desktop (part 2):* share a clip from a paired phone → offer bar → Inbox; the phone's Multimedia control lists FilmCraft and plays / seeks. **L2.4a / L2.4b merged** (`76d4bdfd` Copy Card in the engine: `card.inspect` / `card.offload`, CRC-32 read-back, guide `cards.md`; `0fa6636c` Project panel card bar: mount-table watch, Copy, Eject via UDisks2, Settings ▸ Media ▸ Camera cards / Copy camera cards to, Q16; `c9d182d6` review fix: an identical file already copied is re-checked with progress and Cancel). Fresh-context review done 08:30 by the next run (no subagent, weekly over its line); minor, not fixed: the default copy folder is "<card> <date>", so a resumed copy after midnight starts a new folder (pass `name` to resume into the old one). *Owed on a real desktop (part 2):* insert a card → bar → Copy → Eject; a Flatpak build sees cards mounted after start. **Flatpak grant done** (`10515534`, 09:45: both manifests grant `--talk-name=org.kde.kdeconnect`; *owed:* check it in a real Flatpak build in part 2). **Next for the lane:** only L1.5 / L2.6 remain (wait for U0.2 to merge); runs until then have nothing to build. Worktree `.claude/worktrees/dell-lane`.
**Dell scratch:** `~/Projects/filmcraft-linux/packaging/linux/arch/{src,pkg,*.pkg.tar.zst}` from
the makepkg check (gitignored; delete freely).
Known flake (not the lane's): engine `hardware_decoding_setting_drives_the_decoder_switch`. **Dell note:** the UI
test binary grows ~1 GB per test (also on plain hevc-vaapi) and is OOM-killed on 16 GB in one
process; run it per module (`~/fork-logs/linux-gates2.sh` on the Dell). After a power loss clear
`target/debug/incremental` and `cargo clean -p` the lane crates (corrupt rlibs → link errors).
**Usage (§1a.52–53, §1a.55):** weekly line 99 % × elapsed fraction of the week, reaching 99 %
at the Tuesday 04:00 reset; **conserve first** (§1a.55: natural burn is ~3× the line; FULL only at
U ≤ L − 6; no log-only commits; a run that can't work exits silently; read only the last ~80 lines
of §7); pace the 5-hour
window (line 95 % × elapsed fraction); near the limits (weekly ≥ 98 % or 5-hour ≥ 93 %) keep
working in MINIMAL mode, no hard stop.

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, and the first group (2026-10-13, one commit `203e4e15`):
  `resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`, `coach-1`, `yt-1`. Builds in
  `~/Apps/filmcraft-builds/`; `latest` → `yt-1`; launcher `~/.local/share/applications/filmcraft-fork.desktop`.
  Public (§1a.33): dawgon2/filmcraft-cachyos branch `fork` snapshot `c72dcac2`, tags
  `fork-resolve-ui-1` … `fork-yt-1`, release `fork-yt-1` (see §7 for the scrubbed Dell build).
- **Schema v17** (6.7b: clip `temporal_nr`; `v16_to_v17` moves node `tnr.*`, notes →
  `Loaded.migration_notes` → toast); prefs version 3. New user-visible strings (and engine
  command labels) need es + zh-cn rows. New guide pages go in `GUIDE_PAGES`
  (`crates/engine/src/coach_ask.rs`) and their `{ui:}` ids must be known to the `coach_ui` walk.
  The coach eval (`docs/fork/coach-eval/questions.toml`) is a test: `EVAL_FLOOR` 82 may only go
  up; guide edits can move answers (check `-p filmcraft-engine coach`).
- **RS1** feature `fork-resolve-default` is on in ci: new UI tests that assume Premiere set
  `prefs.workflow = Premiere` or run `settings.setWorkflow premiere`.
- **Y1:** H.264 Constrained Baseline / Main / High 8-bit 4:2:0 progressive decodes zero-copy
  (`crates/platform/src/va_accel.rs`, `Accel::pick`, grey stand-ins `vaapi/gray.rs`); chain
  fork HEVC → zero-copy H.264 → read-back → pipe.
- **UI test memory (2026-10-13 side lane):** a dropped `FilmcraftApp` now stops and waits (≤ 3 s
  each) for its background jobs and frame workers; the UI test binary runs whole in one process at
  ≤ 2 GB per test, except hevc_presets (3–7 GB alone: its 6.2K preset goes dual-lane, 6E.3) (FORK-PLAN §7; parallel mode still needs ~1 GB per concurrent test). A new UI test that keeps an app alive past its end, or a long job that
  ignores `filmcraft_media::cancel`, brings the pile-up back; `teardown_ui` guards the demo case.
- **Owed (not blocking):** Y1: idle zero-copy slots not trimmed / not counted against the GPU
  budget; AMD open-GOP-seek parity covers the I picture only (gap POCs); the fork HEVC pool may
  share the iHD context-lifetime hazard; Dell run of `tests/vaapi.rs`. 6.7b: keyframe markers
  for the TNR rows in Effect Controls; label column width. RS3: Color page Primaries cramped
  under ~1000 pt (Resolve chrome); Markers instead of Index; no Sound Library / Keyframes /
  LUTs / Lightbox; Fairlight strip; home-screen screenshot vs `resolveHomePage.png`. Phase 6:
  frame-server per-frame proxy hint; `cache.claimFolder`; ~~automated 4:2:2 export chroma test~~ (done 07:45, `hevc_422_master_keeps_per_row_chroma`);
  decode-to-screen fps with hardware 4:2:2 in the app; software 4:2:2 decoder memory cap /
  multithreading (ask first); TNR / cache / proxy 6.2K bench cases; VPP context leak on purpose.
- Cloud: CLOUD-QUEUE has C1-PREP, AI1-PREP, R1-PREP (docs only; earlier items done). Merge `origin/cloud-work`
  first if it has new commits.

## Exact next step: 6E (EXPORT-PLAN), group acceptance with L1 / L2 part 1

**Own title bar (FORK-PLAN §1a.57, live session 2026-10-13):** merged; FilmCraft draws its window
title bar on Linux / Windows (Settings ▸ General ▸ Window title bar, `general.titleBar`). Add its
real-desktop checks to the 6E acceptance (see §1a.57).
**Dell lane every 10 min (owner, 2026-10-13 06:00)** until "L1 / L2 part 1 ready"; then the owner (or a
live session) sets `filmcraft-dell-lane` back to every 30 min (§1a.56); the lane can't change its own
schedule.

**Queued first (owner, 2026-10-13, FORK-PLAN §1a.56): DC1, the Dell build-and-test service** (no AI
on the Dell; design in §1a.56). Do it at the next clean checkpoint of 6E.3, before more 6E work;
then send in-between gates, Intel parity and soaks there and read 3-line results.

**Run 09:03–09:25 (LEAN, U 8 / L 3.0): graded dual regression FIXED (Next item 1 done).** GPU probe confirmed the diagnosis (Intel 509 ms GPU / graded 4K frame, AMD 19 ms); coordinator slow-lane rule (`retire_slow_lanes`, 3 fake-lane tests). Graded 4K dual 22.3 / 23.8 fps (single 22.3), 6.2K dual 25.9, 4K 31.3 / 35.7 (noise; `docs/fork-benchmarks.md` "Graded lanes diagnosed and fixed"). Self-reviewed only (LEAN; coordinator-thread logic reusing the tested `kill` path): an Opus review of this diff is owed when the mode allows. **Item (2) done 09:30:** `two_gpus_keep_the_colours_of_one` (`crates/platform/tests/va_dual_export.rs`: two-GPU chroma as close to the BT.709 source as one GPU's, mean shift ≤ 1 code; AMD + Intel: 0.00 dB loss, 0.06 shift; graded colour across GPUs rests on the existing GPU parity tests). **Next: (4) group acceptance with L1 / L2 part 1**; (3) Intel lane speed / `Rgba16Float` on iGPUs stays optional.
**Run 07:03–08:35 (LEAN, U 7 / L 2.6): S11 speed gate MET after a fix: dual 25.0 / 24.9 fps vs single 19.4** (full length, synthetic). Root cause: the process-wide hardware decode gate (`crates/codecs/src/hwgate.rs`) made the two GPUs take turns; it is now per decode node (Opus diagnostician + Opus review). `docs/fork-benchmarks.md` "6E.3 S11". **Export suite done (08:36–08:50):** dual 4K 38.6 vs 24.6, 6.2K 25.1 vs 19.4, all outputs pass ffprobe / mpv / ffmpeg decode; **but graded 4K dual 18.4 < single 22.3** (Intel lane 1.8 fps on its one chunk; details in `docs/fork-benchmarks.md`). **Graded-lane diagnosis (Opus, 08:58, read-only, unconfirmed):** no CPU fallback; the plan is device-independent and the Intel lane runs the whole GPU effect stage (F-Log2 input transform, Lumetri, HSL, vignette, Sharpen's box blurs; `crates/render/src/plan.rs` ~236–255 / 497–589, `fx.rs` `Rgba32Float` ≈ 133 MB per 4K texture) through the iGPU's shared DDR; the GPU time is asynchronous and lands as decode waits (surface reuse fences). AMD lane slower than single because it gets `total − 4` threads (`lanes/lane.rs` ~120) and shares DRAM bandwidth. **Confirm first (cheap):** env-gated probe after the `catch_unwind` in `crates/platform/src/gpu_export.rs::render_into` that polls the device (`PollType::Wait`) and prints per-frame GPU time per PCI id (expect Intel 300–500 ms, AMD 20–40 ms); or drop Sharpen from the bench `grade()` once. **Fix design (take unless the probe disagrees):** (a) coordinator guard in `crates/export/src/lanes/coordinator.rs` / `scheduler.rs`: track each lane's fps after its first batch; when the queue is empty and a lane's projected finish is > 1.5× the other's, stop it at its next batch boundary (`ChunkDone` with a truncated `end`, IDR start for the tail) and `requeue` the tail; (b) on Auto, if the second lane's first-chunk rate is < 25 % of the first lane's, retire it and requeue; (c) optional: `Rgba16Float` effect stage on integrated adapters. (a)/(b) are concurrency: Opus / Fable leaf builder + review, tests with a fake slow lane (`LaneWork` mock). **Next:** (1) fix the graded dual regression: why a graded frame is slow on the Intel lane (iGPU missing GPU grade ops → CPU fallback? trace with `FILMCRAFT_EXPORT_TRACE=1 FILMCRAFT_BENCH_ENGINES=dual … --only "xm5-export-4k graded"`), and a slow-lane guard in `crates/export/src/lanes/coordinator.rs` (take back a slow lane's tail chunk, or Auto picks one GPU); this is concurrency / design: diagnostician first; (2) colour vs Program monitor check on a dual export; (3) optional Intel lane speed (6.2K target 26+); (4) group acceptance with L1 / L2 part 1 (the Dell lane wrote "part 1 ready"). Earlier in this run: (first pass) **Run 07:03–08:15 (LEAN, U 7 / L 2.4): S11 bench ran — NOT accepted: dual 16.2–17.0 fps < single 19.4 fps** (full length, 1798 frames; `docs/fork-benchmarks.md` "6E.3 S11 … first pass"; bench rows now have `stages_s`). Dual is render-bound (≈116 ms / frame / lane vs 30 ms single; GPU busy 12 %), single is encode-bound. **Next: diagnose the lane render slowdown** (diagnostician per §2c mode; leads in the benchmarks doc: lane encoders' `gpu_target()` → zero-copy vs byte readback, shared locks between the lanes' render pools / `lane_copy` sources / decode cache, decode node vs render GPU), fix with a test, rerun `FILMCRAFT_BENCH_ENGINES=dual|single … --only xm5-export-6k` (full length, not `--quick`: 149 frames is too short for two lanes). Also did the owed 4:2:2 export chroma test. **Run 07:03 (LEAN, U 6 / L 1.8):** machine busy (another session's nextest, 3 GB free) until ~07:40; then the owed 4:2:2 export chroma test (`crates/platform/tests/hevc_export.rs`, all three backends, AMD box + Intel encoder); **S11 still next.** **Run 06:50 (LEAN, U 5 / L 1.7):** machine busy again (titlebar ci); I18N-2 closed (already test-enforced), cloud queue refilled (C1-PREP, AI1-PREP, R1-PREP); **S11 still next.** **Run 06:33 (LEAN, U 5 / L 1.5):** machine still busy (titlebar session's ci), so Y3-PREP (`docs/fork/PREP-Y3.md`, Sonnet scout: CPU keys exist but none on GPU, tracker affine only, no user-shader hook, packs need a container) instead; **S11 still next.** **Run 06:18 (LEAN, U 5 / L 1.4):** machine busy (titlebar session's ci + its app), so A1-PREP (`docs/fork/PREP-A1.md`, Sonnet scout) instead; **S11 still next.** **Run 06:09 (STEADY):** machine still busy (another session's ci), so GUIDE-7 (`docs/guide/editing.md`) instead; **S11 still next.** **Run 06:02 (STEADY):** machine busy (Dell-lane build), so GUIDE-8 (scopes guide section) instead; **S11 still next.** **Run 05:03 (STEADY): 6E.3 S10 done** (left uncommitted by the 04:01 run, finished + committed: `tests/va_dual_export.rs` static-shot join PSNR within 0.2 dB of one-GPU, `a_failing_lane_hands_its_chunks_to_the_other_gpu` via `FAIL_NODE`; AMD + Intel). **S12 (EncSliceLP, low-delay B, CRF lane exports on Intel) merged 893d6a05 at 05:55.** **Next: 6E.3 S11 acceptance bench** (idle machine, owner not gaming / in filmcraft, no other session building; Intel lane on LP only for CRF, the bench exports VBR so Intel uses EncSlice): `FILMCRAFT_BENCH_ENGINES=dual` (and `single` for the baseline) with `cargo xtask bench --sections xm5 --only xm5-export-6k --quick` (rows now carry `engines` + the `lanes` report, 50e74d2e); then `xm5-export-4k` / graded; record in `docs/fork-benchmarks.md` "Phase 6E". Then group acceptance. **Run 04:01 (STEADY): FFI second-opinion review done (2 fixes, fa18d7f1); S7 merged 78e9d0ca (Opus builder + Opus review, fixes e8b0505f). DC1 (§1a.56) blocked: the auto-mode classifier refused installing the Dell timer; draft in `git stash`, owner question Q12. **Next run: 6E.3 S10** (static-shot join / PSNR test, failure injection through the real encoder; `FAIL_AFTER` is cfg(test)-only in platform), then S11 acceptance bench. Don't retry the DC1 install until Q12 is answered.** **Done so far (2026-10-13 00:51–01:45 run; details in FORK-PLAN §7):** 6E.2 encoder speed preset
(`hwEncodeSpeed`, Export ▸ Encoder speed; AMD reports 32 quality levels, Intel 7). 6E.4 complete
in code: Main 4:2:2 10 SPS writer, Y210 encoder on Intel (found automatically,
`Display::for_encode_422`), `hevc_chroma_422` / `chroma: 422`, CPU 4:2:2 conversion, all three
backends, preset "HEVC 4:2:2 10-bit master", Export ▸ Chroma, guide. 6E.3 groundwork: an encoder
per GPU (`Display::for_node`, `VaHevcEncoder::new_on`) and an export renderer per GPU
(`frames_on`, `pci_ids`, `headless_device_on`), both tested on AMD + Intel in one process.
6E.1 item 5: AMD + Intel chunks join and decode, but the drivers' parameter sets differ. Our
encoder on Intel `EncSliceLP`: "internal encoding error" with P pictures (reverted).
That run had no speed numbers (the owner was gaming); **the 01:41–02:58 run took them (6E.1 below) and built 6E.3 S1–S6.**

1. ✓ **6E.1 measured (2026-10-13 01:41–02:05, idle machine; `docs/fork-benchmarks.md` "Phase 6E").**
   Intel adds ~18 fps when both GPUs run at once (36.3 vs 18.1 fps decode + encode): **6E.3 is a
   go.** Intel decodes 6.2K twice as fast as AMD (69.7 vs 35.4 fps); the AMD lane is decode-bound
   (VCN shared by decode and encode). The 4:2:2 master preset runs at 2.7 fps (CPU 4:2:2
   conversion after RGB readback; `RenderMode::of` in `crates/export/src/job.rs`) vs 24 fps for
   4:2:0 from the same source: a GPU → Y210 conversion is worth a step (6E.4 follow-up).
2. **6E.3 design: done, `docs/fork/DESIGN-6E3.md` (Fable designer, 2026-10-13 02:16); S1 done (02:23), S5 done (02:28: chunker, scheduler, spill files in `crates/export/src/lanes/`); S6 settings half done (02:31, `ExportEngines`); S2 done (02:37: identical packed headers on AMD + Intel; not yet one hvcC, see below); S3 done (02:44, thread decode node); S4 done (02:49, `lane_copy`); S6 encoder half done (02:57, `HevcEncoder::for_lane` / `lane_stream`;
   `Pipeline.gpu` + `encode_node` folded into S7). **S8 mostly done (03:08–03:11): radeonsi
   (transform depth 4) and iHD (depth ≤ 2) can't share one `hvcC`, so mixed lanes use `hev1`
   (`LaneTarget::in_band = lane_stream(..).is_none()`, `SampleEntry::hevc_inband`); the rest of
   S8 (`second_encode_node`, perf.stats lanes) goes into S7.** **S9 done (03:23): `exportEngines` param
   / CLI `--export-engines` / Export ▸ Export engines combo / es + zh-cn / guide "Two GPUs"; device-
   named labels left for S7.** Still open: S7, S10–S12. **FFI self-review done 03:30 (no blocking findings; S7 notes in the 03:26 progress-log entry: thread-local decode node on every lane thread, 4:2:2-source routing).** **Next run: optional second-opinion review (Fable / Opus,
   531bc10f..HEAD; chunker + scheduler already self-reviewed 03:3x and sound, so focus on the FFI: va_enc.rs, hevc_export.rs, va_hevc.rs, hevc/write.rs), then S7 (Fable leaf builder): coordinator in `crates/export/src/lanes/`, lanes
   with `LaneTarget { in_band: lane_stream(..).is_none() }`, `cancel()` the scheduler when every
   lane is dead, `second_encode_node`, perf.stats lanes + `export.sampleEntry`.** **Read the 02:57 progress-log
   entry before S8: radeonsi rewrites the SPS / PPS, so AMD + Intel chunks don't share one `hvcC`
   yet (try matching radeonsi's values, else `hev1`).** → settled: `hev1` (see S8 above). Before S7: a
   reviewer pass (Fable / Opus) over commits 531bc10f..HEAD (self-reviewed only, weekly ≥ 95 %;
   S7's coordinator must `cancel()` the scheduler when every lane is dead). S7 is concurrency work: Fable leaf
   builder when the mode allows (§1a.44). Build it in
   the order of its §3 table: S1 (`common_stream`, crates/hevc, pure) ∥ S3 ∥ S4 ∥ S5 ∥ S6-settings
   first, then S2 → S6-encoder → S7 ∥ S8 ∥ S9 → S10 → S11 (acceptance: 6.2K 4:2:0 export ≥ 24 fps,
   synthetic). Its §4 open questions carry recommended choices; take them unless a test says
   otherwise (log what you chose in the progress log).
3. EncSliceLP: item 1 shows LP is ~30 % faster on Intel (18.2 vs 14.0 fps encode-only); the
   design makes it follow-up S12 (`6E.3b`, low-delay B pictures) after the acceptance run.
4. Acceptance per EXPORT-PLAN (quick passes, synthetic), plus L1 / L2 part 1 if the Dell lane
   has declared it ready: one `cargo xtask ci`, tags `phase-6e` (+ `linux-1a`, `linux-2a`),
   build `~/Apps/filmcraft-builds/phase-6e/`, publish (§1a.33).

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
