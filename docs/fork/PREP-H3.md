# PREP-H3: audit for H264-PLAN H3 (ffmpeg-free H.264, confirmed)

Written 2026-10-14 by the Dell lane (task §6, FORK-PLAN §1a.72), before any code, while H1 / H2
wait on branches for their reviews (OWNER-QUESTIONS Q33, choice a). H264-PLAN H3 asks: hardware
(VA-API) → built-in software is the decode chain, the ffmpeg pipe is at most an optional extra
(off by default for H.264), every supported file opens and plays with ffmpeg absent, the UI says
which path is in use; accept with the full H.264 fixture set (and the HEVC set) playing with ffmpeg
removed from PATH, and the fallback with the hardware pulled. FORK-PLAN §1a.24 (later than the
plan's wording) goes further: Phase 7.7 removes the product's ffmpeg uses altogether and "H3 then
only has to confirm it for H.264".

## 1. What exists today (hevc-vaapi `a78962cb`)

| Where | What | ffmpeg's role |
|---|---|---|
| `crates/platform/src/lib.rs` `vaapi_factory` | the Linux chain per stream: in-process VA-API zero-copy → in-process read-back → **ffmpeg pipe** → software (`HybridDecoder` fallbacks) | the pipe is always in the chain for H.264 and HEVC when `ffmpeg` runs and opens a VA-API device; `hardware_decoder_for` also asks it |
| `crates/platform/src/vaapi_pipe.rs` | `ffmpeg -hwaccel vaapi` child process; GPU-or-fail (`hwdownload`), so streams VA-API can't take error out and land in software | its `new` declines when `ffmpeg()` / `vaapi_device()` are `None`; no product setting turns it off |
| `crates/platform/src/ffmpeg.rs` | `ffmpeg()` (probe once per process; `FILMCRAFT_FFMPEG`), `vaapi_device()`, `vaapi_supports()` | runtime probing, also used by the HEVC export fallback (`hevc_export.rs`, not H3's: Phase 7.7) |
| registration (`lib.rs` ~L158) | registers `vaapi_factory` when in-process VA-API **or** the pipe works | with ffmpeg absent and libva working nothing changes; with both absent no factory is registered and software decodes everything |
| `crates/codecs/src/hw.rs` | Settings ▸ Playback ▸ Hardware decoding (`Auto` / `Off`), per-backend counters (`note_backend`, `backend_stats`), `perf.stats` names the backend | only `perf.stats` (control channel) shows the path; no per-clip display for H.264 |
| `crates/ui-egui/src/panels/project_views.rs` | clip badge for 4:2:2 HEVC decoded in software ("4:2:2: software decode, proxies recommended") | the pattern H3's UI part can reuse |
| software H.264 (`crates/h264`) | hevc-vaapi: progressive 8-bit + High 10 + High 4:2:2 (H2.1–H2.4); branches: PAFF / MBAFF (H2.5 / H2.6, 210 of 217 in-scope ITU-T streams bit-exact) | none |
| VA-API H.264 front | hevc-vaapi: progressive; branch `dell-lane-h1-3`: PAFF / MBAFF (65 interlaced ITU streams bit-exact on the UHD 770) | none |
| `docs/guide/hardware.md` rows 28–31 | H.264 High 10, 4:2:2 and interlaced still list "the system ffmpeg, while it is still in the chain" as the primary | to rewrite |

Other runtime `Command::new(ffmpeg)` hits in product code: none for H.264 decode (`export/src/raster.rs`
and `engine/src/media_pool.rs` hits are inside `#[cfg(test)]`). ffmpeg in `crates/testkit` and the
tests is oracle / fixture use and stays.

So once H1 and H2 are merged, ffmpeg is already **unnecessary** for every in-scope H.264 stream:
VA-API takes 8-bit 4:2:0 (progressive and interlaced), software takes the rest (High 10 and 4:2:2
have no VA-API profile on AMD / Intel and are software already). What remains is (a) the pipe still
sits in the chain when ffmpeg is installed, (b) nothing proves the ffmpeg-free path end to end, (c)
the UI doesn't say which path a clip uses, (d) the guide rows.

## 2. The pipe: what H3 does with it (choice, FYI)

| Option | For | Against |
|---|---|---|
| (a) H264-PLAN's wording: a Settings switch "Use ffmpeg for H.264", off by default | literal plan | a new user-facing setting for a path §1a.24 deletes in Phase 7.7; a shared `crates/platform` + Settings edit for nothing lasting |
| (b) leave the pipe alone; H3 only proves the ffmpeg-free path (tests with ffmpeg hidden) | no platform edit | with ffmpeg installed the pipe still wins over software for streams the in-process front declines (e.g. a VA driver bug), so "ffmpeg-free" holds only on machines without it |
| **(c) drop the pipe from the chain for H.264 only** (one `match info.codec` in `vaapi_factory` / `hardware_decoder_for`; HEVC keeps it until 7.7), plus (b)'s proof | H.264 behaves the same with or without ffmpeg installed; tiny diff; 7.7 then only removes the HEVC side | an H.264 stream the in-process front declines but ffmpeg's VA-API would take now decodes in software (slower, correct) |

**Choice: (c)** (logged as an FYI in OWNER-QUESTIONS when H3.1 lands). It's the reading of §1a.24
that leaves no dead setting behind.

## 3. Step map

| Step | What | Where / test |
|---|---|---|
| H3.0 | this audit | — |
| H3.1 | H.264 skips the ffmpeg pipe in `vaapi_factory` and `hardware_decoder_for`; doc comments; guide rows | `crates/platform/src/lib.rs` (shared: surgical, main checkout `git status` first, fresh-context Opus review); unit test that the H.264 chain has no pipe step (chain builder split out as a pure function so it is testable without hardware) |
| H3.2 | the UI says the path: extend the project-view clip badge to H.264 decoded in software for a known reason (High 10 / 4:2:2 / hardware declined / Hardware decoding Off), and a "Decoder" line in the clip's properties from the decoder's backend name ("VA-API H.264 (in process)" / "FilmCraft software") | `crates/codecs/src/hw.rs` (a cheap non-blocking query, like `hevc_422_in_hardware`), `project_views.rs`; UI test for both workflows (Premiere / Resolve); es + zh-cn rows; `{ui:}` ids in the guide |
| H3.3 | acceptance harness: ignored test that decodes the libx264 fixture set (progressive, High 10, 4:2:2, MBAFF / PAFF) and the ITU conformance set through the **product's** factory chain in a child process with `FILMCRAFT_FFMPEG=/nonexistent` and ffmpeg stripped from `PATH` (`ffmpeg()` is a per-process `OnceLock`, so fixtures are generated first, then the decode runs in a separate process); same for the HEVC fixture set; then again with hardware pulled (Hardware decoding Off, and a bogus `LIBVA_DRIVER_NAME` so the display fails to open) and with `fail_after` mid-stream fallback | `crates/platform/tests/` (Dell, UHD 770); AMD run owed on the PC at acceptance |
| H3.4 | playback check: the app plays the fixture timeline with ffmpeg hidden (control channel, `perf.stats` backend counters show no pipe), guide page section "H.264 without ffmpeg" (what decodes where, how to check the path, trade-offs of software decode vs proxies for 4:2:2 / High 10) | Dell virtual KWin; guide `hardware.md` |
| H3.5 | "H3 ready for acceptance" in HANDOFF | — |

## 4. Order and risks

- **H3 code waits for H1 / H2 to merge** (Q33 a): H3.3's fixture set includes interlaced streams,
  which only decode ffmpeg-free with H1 (hardware) and H2.5 / H2.6 (software). H3.1 could land
  first, but it is a `crates/platform` diff needing the same Opus review the backlog waits on, so
  it joins that queue rather than stacking a fourth unreviewed branch.
- **Phase 7.7 overlap:** the main runner's 7.7 deletes `vaapi_pipe.rs` / `ffmpeg.rs`; H3.1's change
  is a few lines it will delete with them. If 7.7 lands first, H3.1 shrinks to nothing and H3 is
  H3.2–H3.5.
- **Software speed:** 4:2:2 / High 10 H.264 are CPU-only on every GPU (no VA-API profile); 4K
  4:2:2 10-bit Long-GOP is 43 fps on the Dell with 16 threads, 6.2K far less, so the badge
  recommends proxies (H3.2), as for 4:2:2 HEVC.
- **Per-process probes:** `ffmpeg()`, `vaapi_device()` and the VA display are process-wide
  `OnceLock`s; the harness must run each configuration in its own process.
- **AMD:** interlaced on VCN and the ffmpeg-free run on the RX 6700 XT are owed on the PC at
  acceptance (H1's list).
