# UPSTREAM-PLAN.md: staying in sync with storytold/filmcraft and contributing back

Written 2026-10-11 (session 8, with the owner). All FORK-PLAN rules apply to the fork; **contributions
follow upstream's rules instead** (upstream `AGENTS.md`, `CONTRIBUTING.md`, `docs/contributing.md`
at upstream's current `main`).

## Remotes (set up 2026-10-11)

| Remote | URL | Use |
|---|---|---|
| `origin` | github.com/dawgon2/filmcraft-fork (private) | this fork, branch `hevc-vaapi` |
| `upstream` | github.com/storytold/filmcraft | the main project, **fetch only** |
| `cachyos` | github.com/dawgon2/filmcraft-cachyos (public GitHub fork of upstream) | contribution branches; PRs go from here to upstream |

The fork has no shared git history with upstream: `master` is a one-commit snapshot ("original")
whose tree is identical to upstream **`8fcad73`** (2026-10-07 10:57, "Merge pull request #159").
That commit is the fork base for every comparison and merge. `master` stays untouched (§1a.1).

## What upstream did since the fork base (measured 2026-10-11)

**806 commits** (275 merged PRs) in four days: 563 files, +96.8k / −7.0k lines; it moves about
200 commits a day. 93 of the fork's 161 changed files were also changed upstream. Overlaps that
matter:
- **Upstream has its own VA-API decoding** for **H.264 and HEVC** (`crates/platform/src/vaapi/`:
  `va.rs`, `h264.rs`, `hevc.rs`, `ffi.rs`, shared display, ABI tests), plus **NVDEC** (Linux) and
  NVENC export work (one NVENC path reverted for heap corruption). The fork has its own VA-API
  stack (`va.rs`, `va_hevc.rs`, `va_enc.rs`, zero-copy DMA-BUF display via `crates/gpu/src/interop.rs`)
  and a `vaapi.rs` file (the ffmpeg pipe) that **collides with upstream's `vaapi/` module name**.
- **Upstream moved Lumetri (Basic, Creative, Vignette) and export rendering onto the GPU**
  ("GPU3–GPU7", issue #30) and changed `crates/gpu/src/lib.rs`, `crates/render/src/effects.rs`,
  `crates/codecs/src/gop.rs`, `panels/lumetri.rs`: the fork rebuilt the same areas differently
  (ColorOps, DWG / DI working space, all of Lumetri incl. curves / wheels / HSL on the GPU).
- Proxies, export batching, CRF encoding, cropping / clean aperture, AV1 hardware export, Linux
  packaging, new docs (`docs/target-app-parity.md`, `docs/gaps.md`, contributor credits).

## Phase U0: the first upstream merge (after G1, before Phase 6; ≈ 10–30 h)

Order: FORK-PLAN §1a.6 (G0 ✓ → G1 ✓ → U0 → G2 → G2b → 6 → Y1 → 6E → Y3 → C1 → C2 → 7 (+ Y4) → D → 8 → Y2 → H1 → H2 → H3).
Every day of delay adds ~200 upstream commits to reconcile, and upstream's VA-API H.264 may make Y1
mostly free.

1. Branch `sync/upstream-<date>` from `hevc-vaapi`; record the base pairing (fork base `8fcad73`).
   Merge strategy: replay upstream's changes since `8fcad73` onto the fork (e.g. `git merge` of a
   graft / a `git diff 8fcad73 upstream/main` applied in reviewed chunks, or a merge after
   `git replace --graft master 8fcad73`; choose and record in an ADR, **no history rewriting** of
   `hevc-vaapi` or `master`, no force-push).
2. Resolve per area, with a Fable designer for each conflict cluster:
   - **Decode:** U0 keeps both VA-API stacks (the fork's HEVC decoder first in the chain); the
     consolidation below is **U0.2, scheduled right before Phase 8** (owner, 2026-10-11; FORK-PLAN
     §1a.6), with the same 6.2K and conformance gate. Target: one VA-API stack, not two. Compare upstream's `vaapi/` with the fork's on
     correctness (conformance bit-exactness), zero-copy display, 6.2K playback numbers, fallback
     behaviour; keep the better parts, and keep the fork's ffmpeg-pipe fallback (FORK-PLAN §1) under
     a non-colliding name. Upstream's H.264 VA-API → re-scope Y1 accordingly (log it).
   - **GPU colour / Lumetri / export:** keep the fork's ColorOps pipeline and DWG / DI design
     (§1a.14) as the architecture; take upstream's fixes, presets, tests and UI changes into it.
   - Everything else: take upstream.
3. Gates: `cargo xtask ci`, GPU parity on AMD and Intel, the xm5 quick pass, plus G0 / G1 tests.
   No regression beyond noise against the grade-1 numbers.
4. **Merge into `hevc-vaapi` automatically when every gate is green** (owner, 2026-10-11), then
   write the summary (what came in, conflicts and how they were resolved, numbers) to
   docs/fork/UPSTREAM-SYNC.md and the progress log. If a conflict needs a design call, never stop (owner, 2026-10-11): record it in
   docs/fork/OWNER-QUESTIONS.md with a provisional, reversible choice and carry on. Tag `upstream-sync-1`, build per §1a.9.

### U0 merge notes (redundancy analysis, 2026-10-11, read-only comparison of upstream/main)
Roughly 15–20 % of the fork's work by value is redundant upstream; the 6.2K / AMD performance work is not.
- **Decode:** take upstream's VA-API decoder (`crates/platform/src/vaapi/`: H.264 + HEVC Main / Main10,
  open-GOP / RASL, `hybrid.rs` mid-stream failover with `fail_after` tests, ABI and fuzz tests,
  shared display #474) and retire the fork's `va_sys.rs` / `va_hevc.rs` / `hevc/accel.rs` decode core.
  Upstream reads every picture back with `vaGetImage`: **re-add the fork's zero-copy path** on top
  (a surface-handle output next to `Accel::read`, `crates/gpu/src/interop.rs` DMA-BUF import, VRAM
  budget, shared surface pools). Rename the fork's ffmpeg-pipe `platform/src/vaapi.rs` (e.g.
  `vaapi_pipe.rs`) to end the module clash. Make `va_enc.rs` use upstream's libva loader and shared
  display. Upstream's VA-API H.264 means **Y1 shrinks to wiring its H.264 into the zero-copy path**.
- **Encode / export:** upstream has no VA-API encode (its Linux NVENC HEVC was reverted): keep the
  fork's `va_enc.rs`, `hevc_export.rs`, `render_into` and GPU 4:2:0 pass on top of upstream's
  `gpu/src/export_renderer.rs` plumbing and pooling; make sure upstream's "HDR / wide gamut goes to
  the CPU" rule doesn't push the DWG pipeline off the GPU.
- **`gop.rs`:** heavy conflict. Take upstream's `decode_start` open-GOP fix (same root cause as the
  fork's) and drop the fork's; re-apply the `MIN_FRAMES` bound, the store-time eviction (black
  frame) fix, reverse GOP playback, skimming, `hwgate` deadline ordering, unshown-picture skipping.
- **Lumetri:** keep the fork's ColorOps chain; remove upstream's single `OP_LUMETRI` (or turn its
  tests into parity checks against the chain); port upstream's Lumetri fixes (vignette roundness
  #719, preset drag #227).
- **Anamorphic:** take upstream's PAR in `motion_matrix` (ff8728af), re-add the 1.6× entry and the
  fork's single PAR list.
- **Already fixed upstream** (drop the fork's version): open-GOP seek, compositor `load4` snap,
  Linux keymap test. **Still unique** (contribution candidates): `MIN_FRAMES` bound, eviction fix,
  zero-copy display, VA-API encode, full GPU Lumetri / ColorOps / DWG, GPU scopes, Lanczos, the
  node grader.
- **Wanted from upstream:** VA-API H.264, NVDEC / NVENC (Phase 8 interface), hybrid failover,
  MP4 cropping #288, HybridDecoder replay fix #447, decode-cache memory limits #55, Sequence
  Settings / New Sequence dialogs, ProRes 4444 / EXR / DPX export, CRF.

## Ongoing sync (weekly, automatic when green)

Weekly, and only between phases: fetch `upstream`, merge into a `sync/upstream-<date>` branch,
resolve, run the same gates; **merge into `hevc-vaapi` automatically when every gate is green**
(owner, 2026-10-11: the workflow never stops), then write the summary to docs/fork/UPSTREAM-SYNC.md. Conflicts that need a design call (e.g. upstream rewriting something the fork
extended) go to docs/fork/OWNER-QUESTIONS.md with a provisional, reversible choice; the work
continues.

## Contributing back

Pipeline (after U0, so contributions start from a synced fork):
1. Pick candidates (below); for each, build a clean branch **on upstream `main`** in the `cachyos`
   remote: fork-only parts removed (ffmpeg fallbacks, FORK-PLAN / HANDOFF / plan files, X-M5
   benches), upstream's rules met (no `unsafe` outside `crates/platform` with an ADR, ffmpeg only
   as a test oracle, assets / attribution, fonts), upstream's full `cargo xtask ci` green, a Fable
   review against upstream's AGENTS.md, a focused PR description. **Small PRs**, one fix or feature
   each.
2. Push the branch to `cachyos` (public, but just a branch). **Opening each PR needs the owner's
   explicit yes** (it posts under the owner's GitHub name).
3. Upstream's contributor-credit rule: **asked 2026-10-11; the owner wants to be credited as
   `@dawgon2` only.** Never add or edit a `contributors/people.toml` entry.
4. Follow review comments; prepare fixes for approval. When merged upstream, take the upstream
   version back at the next sync.

Candidates, easiest first (re-check each against upstream first; some may already be fixed there):
- Bug fixes: GOP-cache memory bound (`MIN_FRAMES` with 6.2K frames) and the store-time eviction
  (black-frame) fix; `FrameServer` drop fix (check upstream first). The open-GOP seek, `load4` snap
  and Linux keymap fixes are already fixed upstream.
- Small features: 1.6× anamorphic PAR; `effects.move`; Lanczos-3 for Maximum Render Quality;
  sequence presets.
- Larger (talk to maintainers first, e.g. on Discord): zero-copy VA-API → Vulkan DMA-BUF display;
  GPU ColorOps with DWG / DI and all of Lumetri on the GPU; GPU scopes; the node grader.
