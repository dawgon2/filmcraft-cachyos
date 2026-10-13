# Design 6E.3: one sequence exported on two GPUs at once (AMD RX 6700 XT + Intel UHD 750)

Written 2026-10-13 (design-only lane, no build). Implements EXPORT-PLAN 6E.3 after 6E.1 measured
(`docs/fork-benchmarks.md` "Phase 6E"): both GPUs together 36.3 fps vs AMD alone 18.1 (decode +
encode, no render); Intel decodes 6.2K at 69.7 fps vs AMD's 35.4; Intel `EncSliceLP` is ~30 %
faster than `EncSlice` (18.2 vs 14.0 fps encode-only); the two drivers' VPS / SPS / PPS differ when
ffmpeg writes them. Line numbers are as of `55db5173`; re-check before relying on them. All
FORK-PLAN rules apply: never crash (no `unwrap` / `expect` / `panic!` outside tests, hostile
numbers checked), `unsafe` only in `crates/platform`, layering (export < engine; export never
sees the media pool), fork-only code separable (§1a.36).

## 0. What exists today (facts found, don't re-research)

| Where | What |
|---|---|
| `crates/export/src/job.rs:181-217, 477-626` | `Exporter`: `step()` renders batch *k+1* on the rayon pool while encoding batch *k* on the calling thread; `RenderMode::of` (:107) picks the GPU 4:2:0 path from the **global** `crate::gpu_frames()` and the encoder's `gpu_target()`; `overlap_batch` (:121) bounds in-flight frames by `IN_FLIGHT_BUDGET` (512 MiB, counted as RGBA8: a 6.2K export runs 2-frame batches). `encode_batch` (:628) muxes video per `INTERLEAVE` (16) group then the audio up to its end. |
| `crates/export/src/pipeline.rs:109-220, 284-313` | `Pipeline` (project, rate, size, options); `frame_yuv` plans the frame (`plan_frame`) and calls `gpu.render_into` / `gpu.yuv420` on the global renderer. CPU fallback `frame` uses a `RendererPool` of `build_frame_renderer()` compositors on the default GPU. |
| `crates/export/src/lib.rs:863-882, 804-817, 2094-2106` | `VideoEncoder` trait (`sample_entry`, `timescale`, `encode`, `flush`, `gpu_target`); `register_gpu_frames` / `gpu_frames()` (one static); `export()` loops `step`. `export_range` (:1995), `frame_span` (:2024). `Progress` (:690): `done`, `total`, `cancel`, `finished`, `status`, `error`. `note_stage` / `stage_times` (:996) feed `perf.stats`. |
| `crates/export/src/settings.rs:160-202, 487-513` | `HwEncodeSpeed` (the pattern for a new enum setting: `ALL`, `label`, `from_name`, `#[serde(default)]`, old-settings test :749); `resolve()` gives `keyint` (`keyframe_distance` or 2 × fps). |
| `crates/platform/src/va.rs:94-158` | `Display::get()` (process decode display, `FILMCRAFT_VAAPI_DEVICE`), `for_encode()`, **`for_node(node)`** (one display per render node, kept; max 8), `supports`, `config_attribs` (:318). `SurfacePool::shared` keeps `SHARED_POOLS = 4` pools (:1127). |
| `crates/platform/src/va_enc.rs:164-293, 377-436` | `VaHevcEncoder::new_on(params, display)`: reads the driver's `EncHEVCBlockSizes` / `EncHEVCFeatures` into `StreamParams` (:206-236: `log2_ctb`, `log2_min_cb`, `log2_min_tb`, `log2_max_tb`, depths, `amp`, `sao`, `transform_skip`, `cu_qp_delta`), coded size 16-aligned, `headers()` = VPS + SPS + PPS from `filmcraft_hevc::write`. `submit` (:377): IDR when `gop_pos == 0`, else P from the previous recon; `sequence_params()` carries the same block sizes to the driver. `EncSlice` only. |
| `crates/platform/src/hevc_export.rs:89, 539-575, 591-597, 653-760, 790-860, 920-1006` | `factory` → `HevcEncoder::new` (backend order VaNative → `hevc_vaapi` → `libx265`); `probe_native` encodes one black frame on `VaHevcEncoder::new` (process display) and `Parser::config()` builds the `hvcC` (`SampleEntry::hevc` = `hvc1`, `codec.rs:1276`); the parser drops in-band VPS / SPS / PPS equal to the configured ones (:529). `VaWorker`: the encoder on its own thread behind a queue. `fall_back` (:862): an in-process failure continues the stream with ffmpeg (writes its own parameter sets). `stamp` (:898): packets get `duration = rate.den`, `composition_offset 0` for P-only streams. |
| `crates/platform/src/gpu_export.rs:128, 253-271` | `GpuExport { pci }` → `GpuCompositor::headless_device_on(pci)` (`crates/gpu/src/lib.rs:1372`, adapter by PCI ids); **`frames_on(node)`** gives a `GpuFrames` on that node's GPU; `pci_ids(node)` from sysfs. DMA-BUF import of a decoded picture that fails is read back instead (`crates/gpu/src/interop.rs:224-231`): a frame from the other GPU's display is slow, not fatal. Test `a_renderer_on_each_gpu` renders the same plan on AMD and Intel (within 4 codes). |
| `crates/platform/src/va_hevc.rs:80-82, 108-130` | `display_for(chroma)` → `Display::get()` (4:2:0) / `for_hevc_422()`; chosen in `ensure()` at the first SPS, i.e. lazily on the thread that decodes. `crates/platform/src/lib.rs:309-360` `vaapi_factory(&SampleEntry)`: a plain fn pointer (no context), native decoder first, upstream / ffmpeg-pipe fallbacks in a `HybridDecoder`. |
| `crates/codecs/src/gop.rs:112-122, 913-925, 385-392` | `VideoSamples::make_decoder` is called lazily inside `video_frame` (one decoder per source, kept); the decoded-frame budget is process-wide (`auto_limits`: RAM / 5 → 6.2 GB here, max live decoders). |
| `crates/engine/src/media_pool.rs:139-164, 334-342, 463-487` | `MediaPool::default()` is cheap (maps + `filmcraft_codecs::openers()`); `PoolProvider` implements `SourceProvider` (`crates/render/src/lib.rs:55`) over one pool; `full_res_provider` for export. `export_tools.rs:464-500` `spawn_export` runs `filmcraft_export::export` on a job thread (`guard_job`). |
| `crates/engine/src/render_cache.rs:788-816` | Precedent for chunked export: each cache part is its own `Exporter` to `<name>.part`, renamed when done. |
| `crates/project/src/lib.rs:540-560, 744-752, 1000-1003` | `TrackItem.start / duration` (ticks), `Track.transitions` (`start`, `duration`), `Sequence.video_tracks`: the cut points. |
| Control / CLI / UI | `export_tools.rs:297-300` param parsing (`hwEncodeSpeed`), schema string `commands.rs:1357`; CLI flags list `apps/filmcraft-cli/src/args.rs:30` + usage `main.rs:78`; Export page combo `export_mode.rs:731-739` (`combo(ui, reg, "export.video.hwEncodeSpeed", …)`), i18n `crates/ui-egui/src/i18n/*.tsv` (8 languages); guide `docs/guide/export.md:85-95`. |
| Bench | `crates/ui-egui/examples/bench/xm5.rs:974-978` cases `xm5-export-4k`, `xm5-export-4k graded`, `xm5-export-6k` (5 s quick / 60 s). |

## 1. Shape of the change

One **coordinator** (the existing `Exporter`, new mode `Dual`) on the job thread owns the muxer,
the audio and the output file, exactly as today. Two **lanes** (new `crates/export/src/lanes/`),
each on its own thread with its own rayon pool, its own `SourceProvider` copy, its own
`GpuFrames` and its own `VideoEncoder` on its own GPU, pull chunks from a shared queue, render
and encode them, and spill the packets to a file per chunk. The coordinator muxes chunks in order.
Nothing crosses GPUs (DMA-BUF across devices was the dead end). Audio is untouched.

## 2. Decisions

### 2.1 Chunking (`crates/export/src/lanes/chunker.rs`, pure)

- **Unit:** output frames `[f0, f1)` from `frame_span`. **Every chunk starts on a keyint multiple
  from `f0`** (`r.keyint`, default 2 × fps = 60 at 29.97), so chunk IDRs are exactly the IDRs a
  single-device export places: no extra IDRs, same keyframe table. The encoder is told to restart
  its GOP at a chunk start (new `VaHevcEncoder::restart_gop()` sets `gop_pos = 0`).
- **Cuts preferred:** `cuts(seq, rate, f0, f1)` collects `start` and `end()` of every enabled item
  on enabled video tracks, as output frames. A nominal boundary snaps to the nearest cut within
  ± keyint (an IDR at a scene change is what encoders do anyway: cost ≈ 0, and it hides any
  encoder-state step and avoids re-decoding a source GOP, since a clip start decodes from its own
  IRAP). A boundary inside a transition range is moved to the transition's end (a join inside a
  dissolve is where a quality step would show).
- **Size:** nominal **8 s** (`CHUNK_SECONDS = 8`, ≈ 240 frames at 29.97, 4 GOPs). Per-chunk cost
  is one source-GOP re-decode at the start (`gop.rs decode_start` seeks to the sync sample before:
  ≤ 1 s of source ≈ 0.5–0.9 s at 35–70 fps decode) plus a `flush` (3 pictures), i.e. ≲ 7 % at 8 s
  and 0 at a cut. **Tail shrink** (guided self-scheduling): `len = clamp(remaining / (2 × lanes),
  keyint, nominal)` rounded down to a keyint multiple, so the last chunks are short and the slower
  lane cannot hold the finish for more than ~keyint / fps.
- **Scheduler** (`lanes/scheduler.rs`, pure): `Mutex<VecDeque<Chunk>>` + condvar; a lane pulls
  the front chunk (so completion order is close to file order and the coordinator's wait is short).
  The faster lane simply pulls more; the measured fps of 6E.1 (AMD ≈ 18, Intel ≈ 14–21) only sets
  the `lanes` divisor of the tail rule and the **first pull order** (the primary lane takes chunk 0,
  so the muxer starts earliest). A re-queued chunk (§2.5) goes to the **front**.
- Hostile inputs: `f1 <= f0` → no chunks (the existing `validate_range` already refuses 0
  frames); 1 frame → one chunk; keyint 0 → 1; keyint or nominal > range → one chunk; cuts outside
  the range ignored; all arithmetic saturating on `i64`.

### 2.2 A full pipeline per lane, on the lane's GPU

- **Lane** (`lanes/lane.rs`): `Lane { name, node: Option<String>, gpu: Option<Arc<dyn GpuFrames>>,
  sources: Arc<dyn SourceProvider + Send + Sync>, pool: rayon::ThreadPool, venc }`. The primary
  lane is the process's displays (`node = None`, global `gpu_frames()`, the caller's provider; AMD
  here). The second lane is built from a render node: `gpu = frames_on(node)` (`gpu_export.rs:253`),
  encoder on `Display::for_node(node)`.
- **Decoders on the lane's display:** the decoder factory has no context, and decoders are made
  lazily on the decoding thread (`gop.rs:913`), so the binding is **per thread**: new
  `filmcraft_codecs::hw::decode_node()` / `set_thread_decode_node(Option<String>)` (thread-local).
  Each lane's rayon pool sets it in `ThreadPoolBuilder::start_handler`, and the lane thread sets it
  too; all of a lane's rendering runs under `pool.install` / `pool.in_place_scope` (`render_batch`
  and the prefetch of `job.rs:587-600` take the pool as a parameter). `va_hevc::display_for`
  honours it (`Display::for_node(node)`; 4:2:2 only if that node `supports(Main422_10, VLD)`, else
  `for_hevc_422()` as now), and `NativeVaDecoder::new` **captures the node into the decoder** so
  `ensure()` uses it whatever thread later decodes. The hybrid's fallbacks (upstream `VaDecoder`,
  ffmpeg pipe, software) ignore the node: their frames land on `Display::get()` and the Intel
  compositor's import fails → read back → correct but slow (logged once). Env override
  `FILMCRAFT_EXPORT_SECOND_DEVICE=/dev/dri/renderD128` for tests.
- **Second copy of the sources:** `SourceProvider` gets `fn lane_copy(&self) -> Option<Arc<dyn
  SourceProvider + Send + Sync>>` (default `None` → no dual export). `PoolProvider::lane_copy`
  builds a **new `MediaPool`** with the parent's openers (`MediaPool::with_openers(…)`) over the
  same project snapshot and services, `full_res: true`. Why a separate pool: `gop.rs` keeps one
  decoder per `SharedSource`; two lanes on one source would seek-thrash one decoder on one GPU.
  Readers are range reads (cheap); the GOP caches share the process budget.
- **Render:** `Pipeline` gets `gpu: Option<Arc<dyn GpuFrames>>` (set from the lane; default the
  global) used by `frame_yuv`; `RenderMode::of` takes it. CPU-fallback frames of the second lane
  (HDR, overlays, limiter, a plan the GPU declines) render through `RendererPool` on the default GPU
  (AMD): correct, slower, fallback only (open question Q3).
- **Encoder:** `ExportSettings.encode_node: Option<String>` (`#[serde(skip)]`, runtime only) →
  `hevc_export::Params.encode_node` → `VaHevcEncoder::new_on(params, Display::for_node(node))` for
  both `probe_native` and the real encoder. In dual mode both lanes use `Backend::VaNative` only and
  `Params.no_fallback = true` (ffmpeg's `fall_back` would write its own parameter sets, breaking
  §2.3); a failure is a chunk failure (§2.5).
- **Threads:** primary pool `max(2, n − 4)`, second pool 4 (its work is planning + hardware
  decode; the Intel lane is encode-bound). The encoder keeps its own `VaWorker` thread per lane.
- **Memory (31 GB RAM, builds in mind):** a 6.2K P010 frame is 78 MB. Per lane: encoder source pool
  (2-frame batches × 2 in flight + `DEPTH` 3 + 1 ≈ 8 surfaces, 620 MB) + recons (2–3, 230 MB) +
  decoder DPB (7 × 78 = 550 MB) ≈ 1.4 GB, plus the shared GOP cache (≤ 6.2 GB, process-wide
  `auto_limits`). AMD's is VRAM; **Intel's is system RAM** (iGPU). Worst case ≈ 1.5 (Intel) + 1
  (AMD CPU side) + 6.2 (GOP) + app ≈ 10 GB; with a 20 GB build slice (§1a.31) that is 30 of 31 GB.
  Rule: the coordinator reads `MemAvailable` (`/proc/meminfo`) at setup and goes dual only if
  ≥ 6 GB; the second lane's `IN_FLIGHT_BUDGET` is 256 MiB; below the threshold it logs the reason
  and runs single. Never allocate from a size read from media without the existing caps.

### 2.3 Identical parameter sets on both encoders (`hvc1`), `hev1` fallback

- Our writer already fixes everything in the SPS / PPS except what `va_enc.rs:206-236` takes from
  the driver. New pure `filmcraft_hevc::write::common_stream(a: &StreamParams, b: &StreamParams)
  -> Option<StreamParams>`: `log2_ctb = min`, `log2_min_cb = max`, `log2_max_tb = min`,
  `log2_min_tb = max`, depths `min`, `amp` / `sao` / `transform_skip` / `cu_qp_delta` = `a && b`,
  coded size from the stricter alignment, level / tier recomputed; `None` when size, depth, chroma,
  colour or timing differ (they never should: same `EncodeParams`).
- `va_enc.rs`: split `new_on` into `stream_params_for(&Display, &EncodeParams) ->
  Result<StreamParams>` (the attribute reading) and `new_on_with(params, display, stream)` (the
  rest; it re-checks the given stream against the device's ranges and errors cleanly). Dual setup:
  `s = common_stream(stream_params_for(amd), stream_params_for(intel))`, both encoders
  `new_on_with(…, s)`, then `probe_native` on **each** device; **`headers()` bytes must be equal**
  on both and both probes must encode → one `hvcC`, `hvc1` as today.
- **Fallback `hev1`:** if a driver rejects the common stream or the headers differ, each lane keeps
  its own `StreamParams`, the sample entry becomes `hev1` (new `SampleEntry::hevc_inband`,
  `codec.rs` next to :1276, `hvcC` holding lane 0's sets) and the parser keeps every IDR's in-band
  VPS / SPS / PPS (`Parser.configured` left empty). ffprobe / mpv / VLC / YouTube take `hev1`;
  older QuickTime does not → a warning in the export log and `perf.stats`.
- **`EncSliceLP` is a follow-up, not the first cut.** First cut: Intel `EncSlice` (14 fps encode;
  with AMD's 18 that is ≈ 30 fps combined, above the 24 fps target). The LP step (S12) adds
  low-delay B: `SlicePic.low_delay_b` → `slice_type` B (0) with L1 = L0 (one negative RPS entry,
  `num_ref_idx_l1 = 0 (+1)`, `mvd_l1_zero_flag` 1, `collocated_from_l0`), VA `coding_type 3`,
  `ref_pic_list1 = ref_pic_list0`, entrypoint `VAEntrypointEncSliceLP` (`va_sys.rs:909`) with its
  own attributes fed to `common_stream` (LP's block sizes may differ from EncSlice's). It must keep
  `composition_offset 0` (no reordering) and identical headers, else `hev1`. Measured before kept.
  **Done (S12, 2026-10-13):** `EncodeParams.low_power` (set for two-GPU lanes); LP is used only
  after a cached trial encode (IDR + 2 B, read back by our decoder) works, else `EncSlice`.
  Deviation: `mvd_l1_zero_flag` **0** (iHD 26.3 codes L1 MVDs whatever it is told; 1 desyncs the
  slice data). iHD 26.3's LP offers **CQP only** (no HuC BRC here), so VBR / CBR exports stay on
  `EncSlice`. LP's attributes give the same stream as EncSlice (depth 2, 64/8), so `hvc1` rules
  are unchanged. Measured (iHD, CQP 24, synthetic noise, desktop in use): 4K 18.3 → 33.5 fps,
  6240×4160 6.9 → 11.7 fps.

### 2.4 Joining (`lanes/spill.rs` + the coordinator)

- A lane writes each chunk's packets as they come to `<output>.chunks/<k>.<attempt>.pkts`
  (next to the output: same filesystem, space where the user expects; dir created at setup, removed
  at the end and stale ones removed at the next export to that path). Record: `u32 len, u8 key,
  u32 duration, i32 cts, bytes`; the reader checks every length against the file (truncated or
  oversized → `ExportError::Io`, never a panic or an unbounded `Vec`). Memory: one chunk's packets
  in RAM in the coordinator at a time (≈ 50–150 MB at 6.2K CQP 24).
- The coordinator's `step()` in dual mode: wait (condvar, cancel polled every 200 ms) for chunk
  `k`, read it, feed `mux_group` (the interleave / audio part of `encode_batch` extracted into a
  helper shared by both modes): per `INTERLEAVE` group of **global** frame indices, video packets
  then `AudioOut::pull` to the group's end — the same file layout as single mode because there is
  exactly one packet per frame in order. Timestamps: `duration = rate.den` per packet,
  `composition_offset 0`, `media_start None` (P-only / low-delay B), so chunk order gives the
  dts / pts; `key` is true at every chunk start (IDR). `open_mux` after lane 0 reports its
  `SampleEntry` (`LaneReady { sample_entry, headers, timescale }` on a channel at lane start; the
  coordinator compares both lanes' `headers` here for §2.3).
- Consistency at joins: same rate control on both (CQP is deterministic per device; VBR / CBR
  restart the HRD at ¾ fullness per chunk, `misc_buffers`). The static-shot test (§2.7) is the gate.
- Dual mode only when `!settings.part_of_batch`, `settings.sink.is_none()` (no in-memory / web
  exports), not two-pass (H.264 only anyway), and the loudness pass runs first as now.

### 2.5 Failure: never crash, never lose the export

- A lane's chunk error (`VaPicture::ready` → lost, encoder error, decode error, a panic caught by
  `catch_unwind` around the lane body as `guard_job` does) → `LaneEvent::Failed { chunk, why }`:
  the coordinator deletes that attempt's spill file, re-queues the chunk at the **front**, marks the
  lane dead (its thread is joined if it returns; a thread that never returns is detached: the next
  attempt has a new file name, so a zombie writer can't corrupt it). The reason goes to the export
  log and `perf.stats` `export.lanes[i].failed`.
- Watchdog: a lane with no progress for 60 s (`progress.done` per lane) counts as failed (device
  hang; the AMD lane continues).
- The primary lane failing → `ExportError` as today, but the coordinator **restarts the whole export
  single-lane** on the classic path (`engines = Single`) once, logging it (simple and rare; open
  question Q6). Cancel: lanes check `progress.cancel` per batch, join, chunk dir removed.
- Nothing dual runs on wasm or when `rayon::current_num_threads() < 4`.

### 2.6 Setting "Export engines"

- `ExportSettings.export_engines: ExportEngines { Auto, Single, Dual }` (`#[serde(default)]`,
  `ALL`, `label()`, `from_name` like `HwEncodeSpeed`). Labels name the devices when known: "Auto",
  "AMD only" / "Primary GPU only", "AMD + Intel" / "All GPUs" (vendor via `Display::for_node`'s
  `vendor`, through a `filmcraft_export::register_engine_names` hook set by the platform crate).
- **Auto = dual** when all hold: format HEVC via VA-API (`hevc_encoder != X265`, native encoder
  available), not 4:2:2 (AMD has no 4:2:2 encoder: that preset stays single on Intel), a second
  render node whose display `supports(HEVCMain/Main10, EncSlice)` other than `for_encode()`'s node
  (new `platform::second_encode_node()`), `sources.lane_copy()` is `Some`, `MemAvailable ≥ 6 GB`,
  native, ≥ 4 threads. `Dual` with a condition missing → single with the reason in the log, not an
  error. 4K exports qualify too (decoder-bound today; measured in acceptance).
- Control param `exportEngines: auto | single | dual` (alias `engines`) in `export_tools.rs`
  next to `hwEncodeSpeed`, schema string `commands.rs:1357`; CLI `--export-engines` in `args.rs:30`
  + usage; Export page `combo(ui, reg, "export.video.exportEngines", …)` under Encoder speed, shown
  for HEVC with Auto / VA-API; i18n rows in the 8 `.tsv`; guide `docs/guide/export.md` section
  "Two GPUs" (options and trade-offs, not prescriptive: when single is the better choice).
- `perf.stats` `export.lanes`: per lane `name, node, chunks, frames, fps, failed`, plus
  `export.sampleEntry` (`hvc1` / `hev1`).

### 2.7 Tests

| Test | Where | Kind |
|---|---|---|
| chunker: keyint alignment, cut snapping, transition avoidance, tail shrink, hostile (0 / 1 frame, keyint 0, keyint > range, nominal 0, cuts outside, i64 extremes) | `lanes/chunker.rs` | unit, pure |
| scheduler: pull order, front re-queue, dead lane, deterministic join order with simulated lanes of different speeds | `lanes/scheduler.rs` | unit, pure |
| spill round trip; truncated / oversized length → error, bounded memory | `lanes/spill.rs` | unit |
| `common_stream`: intersection, mismatch → `None`, equal headers | `crates/hevc/src/write.rs` | unit, pure |
| headers byte-equal and a probe encode on each device with the common stream | `crates/platform/src/va_enc.rs` tests | skips without 2 encoders |
| decode with the thread node set to Intel → surface display is Intel | `crates/platform/tests/va_native.rs` | skips |
| **static-shot join:** a static 6.2K (or 1080p, quick) synthetic sequence, chunks on two devices, joined, decoded with our decoder; frame count / pts / key positions equal the single-device export; PSNR to the source on both sides of each join within 0.5 dB of the single export; ffprobe (if present) reports `hvc1` (or `hev1`) and the frame count | `crates/platform/tests/va_dual_export.rs` | skips |
| injected failure in lane 2 (`FAIL_AFTER`-style) → export completes, all frames, `perf.stats` reason; cancel removes the chunk dir | `crates/platform/tests/va_dual_failure.rs` | skips |
| settings: old JSON reads `Auto`; `from_name`; CLI / control param parse | export / engine / cli tests | unit |
| **acceptance:** `xm5-export-6k` with `exportEngines=dual` ≥ 24 fps (target 26+), `xm5-export-4k` and graded measured, synthetic | bench `xm5.rs` | bench |

## 3. Steps (one leaf builder each; [A] = parallel from the start)

| # | Commit | Crates | Depends on |
|---|---|---|---|
| S1 [A] | `6E.3: common_stream (intersection of two drivers' stream params)` + tests | hevc | — |
| S2 | `6E.3: VaHevcEncoder::stream_params_for / new_on_with / restart_gop`, per-device header-equality test | platform | S1 |
| S3 [A] | `6E.3: per-thread decode node` (`codecs::hw`, `va_hevc::display_for`, decoder captures it) + test | codecs, platform | — |
| S4 [A] | `6E.3: SourceProvider::lane_copy, MediaPool::with_openers, PoolProvider::lane_copy` + test | render, engine | — |
| S5 [A] | `6E.3: chunker, scheduler, spill files` (pure, tested) | export | — |
| S6 [A] | `6E.3: Pipeline.gpu, ExportSettings.encode_node / export_engines, HevcEncoder on a node, no_fallback` | export, platform | S2 for the encoder part (settings part first) |
| S7 | `6E.3: lanes and the dual coordinator` (`lanes/lane.rs`, `Exporter` dual mode, `mux_group`) | export | S3–S6 |
| S8 | `6E.3: second_encode_node, hvc1 / hev1 decision, SampleEntry::hevc_inband, perf.stats lanes` | platform, isobmff, export | S2 |
| S9 | `6E.3: exportEngines param, CLI flag, Export page combo, i18n, guide` | engine, cli, ui-egui, docs | S6 (settings) — parallel with S7 |
| S10 | `6E.3: static-shot join test, failure injection test` | platform tests | S7, S8 |
| S11 | `6E.3: acceptance run` (bench, `docs/fork-benchmarks.md`, FORK-PLAN §7, HANDOFF) | docs | S10 |
| S12 | follow-up `6E.3b: EncSliceLP with low-delay B` (write.rs B slice header, va_enc LP entry point, probe, measured) | hevc, platform | S11 |

Parallel at the start: S1, S3, S4, S5, S6 (settings half). Then S2 → S6 (encoder half) → S7 ∥ S8 ∥ S9
→ S10 → S11 → S12.

## 4. Open questions (recommended choice first)

- **Q1 chunk length:** 8 s nominal with tail shrink (recommended) vs 5 s. 5 s balances better but
  costs ~2× the GOP re-decodes; cut snapping removes most of that either way. Decide by S11's run.
- **Q2 do both drivers accept the intersected block sizes?** If radeonsi or iHD rejects a CTB / TB
  below its reported maximum, `new_on_with` errors and the export falls to `hev1` automatically;
  S2's test tells, and `hvc1` may then need the per-driver maxima to coincide (they likely do:
  both report 64 / 8).
- **Q3 CPU-fallback frames of the Intel lane render on AMD** through `build_frame_renderer()`.
  Recommended: leave (fallback only, correct); a later step passes PCI ids to the frame renderers.
- **Q4 VBR / CBR steps at joins:** recommended: allow, gated by the static-shot test; if a step
  shows, carry the lane's last HRD fullness into the next chunk's `MiscHrd` (follow-up).
- **Q5 spill location:** next to the output (recommended) vs `std::env::temp_dir()` (tmpfs here:
  RAM).
- **Q6 primary-lane failure:** restart single-lane from scratch (recommended, simple) vs resume
  from the finished chunks (the chunk files allow it; more code, rare).
- **Q7 Auto on 4K:** dual whenever the conditions hold (recommended); the 4K bench decides if a
  size threshold is needed.
- **Q8 memory threshold:** `MemAvailable ≥ 6 GB` (recommended; the Intel lane's surfaces live in
  RAM and the build slice may take 20 GB).
- **Q9 `hev1` acceptability:** fine for YouTube / mpv / VLC; warn once for older QuickTime
  (recommended) rather than refusing the export.
