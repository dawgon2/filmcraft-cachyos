# Readiness survey: Y1, 6E and U0.2 (read-only, 2026-10-12)

Prepared so the runner can start the next three hardware phases without re-reading everything.
Read-only: nothing was built, run or benchmarked (owner's PC busy). Line numbers are of the tree at
`5b27f3ab` (`hevc-vaapi`); `upstream/main` fetched at `0879bd7a` (2026-10-12, release 0.7.1).

## 0. Headline findings

1. **H.264 already decodes in hardware, in process, in this fork.** The U0 merge kept upstream's
   VA-API decoder, and it is second in the Linux decode chain: `crates/platform/src/lib.rs:291-333`
   (fork `va_hevc::NativeVaDecoder` → upstream `vaapi::VaDecoder` → ffmpeg pipe). Upstream's
   `vaapi::VaDecoder::new` takes exactly Y1's scope, Constrained Baseline / Main / High
   (`crates/platform/src/vaapi/mod.rs:73-80`), 8-bit 4:2:0 progressive (`:116-130`), multiple
   slices, and is bit-exact against the software decoder on x264 fixtures with B-pyramids, weighted
   prediction, temporal direct, custom matrices, open GOP, 1080p / 2160p
   (`crates/platform/tests/vaapi.rs:45-50, 200-240`). What it lacks is the fork's **zero-copy
   display**: every picture is read back with `vaGetImage` (`crates/platform/src/vaapi/va.rs:420-432,
   533-536`).
2. **The Y1-PREP field survey is answered by code that exists:** `crates/platform/src/vaapi/h264.rs`
   fills `VAPictureParameterBufferH264` (`:375-421`), `VAIQMatrixBufferH264` (`:424-426`) and
   `VASliceParameterBufferH264` (`:429-512`) entirely from `crates/h264`'s `Sps` / `Pps` /
   `SliceHeader` / `Dpb`. No parser field is missing for the Y1 scope (§1.1). The CLOUD-QUEUE
   Y1-PREP job can be closed by pointing at §1.1 instead of writing `Y1-FIELDS.md`.
3. **Recommendation (not a decision): build Y1 on upstream's decoder.** Don't write `va_h264.rs`.
   Write one fork-side `Accel` implementation (upstream's trait, `vaapi/mod.rs:32-59`) over the
   fork's `va::{Display, Config, Context, SurfacePool, Surface}` that returns `PixelData::Gpu`
   frames, plus a two-line change in each upstream front end so a surface still held by the
   display isn't reused. That is ≈ 7 agent hours (§1.4), inside the 8-hour cap, and it is also
   **U0.2's first step** (the zero-copy path "re-added on top" of upstream's decoder,
   UPSTREAM-PLAN "U0 merge notes"), so nothing is ported twice. The owner's order Y1 → 6E → U0.2
   can stay: 6E hardly touches the decoder (§2.5).
4. **Upstream's `vaapi/` has not moved since U0** (`git diff upstream/main -- crates/platform/src/vaapi/`:
   one line, the fork's own `make_missing` type fix in `hevc.rs:256`), and upstream still has no
   4:2:2 VA-API decode and no zero-copy path (grep of `upstream/main` for `422` / `dmabuf` /
   `PixelData::Gpu` in `crates/platform`, `crates/frame`, `crates/gpu`: only VideoToolbox's 4:2:2
   pixel formats and comments saying "a zero-copy path would replace it"). So U0.2 is a port of fork
   work onto a stable target, not a merge against drift.

## 1. Y1: VA-API H.264 decode (hard cap 8 agent hours)

### 1.1 Field map (Y1-PREP): what the VA-API H.264 buffers need and where `crates/h264` has it

Names are the public libva header's (`va_dec_h264.h` / `va.h`); the fill is upstream's
`crates/platform/src/vaapi/h264.rs`, reusing the parser the software decoder uses, so both decide
alike (module docs `:1-12`).

| VA buffer / field group | Filled at | Parser source |
|---|---|---|
| `VAPictureParameterBufferH264.CurrPic`, `ReferenceFrames[16]` (`VAPictureH264`: `picture_id`, `frame_idx`, `flags` short/long term, `TopFieldOrderCnt`, `BottomFieldOrderCnt`) | `h264.rs:309-320`, `va_ref` `:342-352` | `filmcraft_h264::dpb::{Dpb, DpbEntry, RefMark}` (`crates/h264/src/dpb.rs:34-50`), POC from `slice::PocState::compute` (`crates/h264/src/slice.rs:428`) |
| `picture_width_in_mbs_minus1`, `picture_height_in_mbs_minus1`, `bit_depth_luma/chroma_minus8`, `num_ref_frames` | `:404-408` | `Sps` (`params.rs:303`), `frame_height_in_mbs()` (`params.rs:441`) |
| `seq_fields`: `chroma_format_idc`, `residual_colour_transform_flag`, `gaps_in_frame_num_value_allowed_flag`, `frame_mbs_only_flag`, `mb_adaptive_frame_field_flag`, `direct_8x8_inference_flag`, `MinLumaBiPredSize8x8`, `log2_max_frame_num_minus4`, `pic_order_cnt_type`, `log2_max_pic_order_cnt_lsb_minus4`, `delta_pic_order_always_zero_flag` | `:376-387` (`MinLumaBiPredSize8x8` derived from `level_idc >= 31`, A.3.3.2) | `Sps` fields; bit positions in `vaapi/ffi.rs::seq_bits` (`:163`) |
| `num_slice_groups_minus1`, `slice_group_map_type`, `slice_group_change_rate_minus1` | hard 0 (`:410-412`); FMO refused `:188-190` | `Pps.num_slice_groups` (`params.rs:537`) |
| `pic_init_qp_minus26`, `pic_init_qs_minus26`, `chroma_qp_index_offset`, `second_chroma_qp_index_offset` | `:413-416` | `Pps` |
| `pic_fields`: `entropy_coding_mode_flag`, `weighted_pred_flag`, `weighted_bipred_idc`, `transform_8x8_mode_flag`, `field_pic_flag`, `constrained_intra_pred_flag`, `pic_order_present_flag`, `deblocking_filter_control_present_flag`, `redundant_pic_cnt_present_flag`, `reference_pic_flag` | `:388-397` | `Pps`, `SliceHeader.field_pic` / `nal_ref_idc` (`slice.rs:108`); `ffi.rs::pic_bits` (`:178`) |
| `frame_num` | `:418` | `SliceHeader.frame_num` |
| `VAIQMatrixBufferH264.ScalingList4x4[6][16]`, `ScalingList8x8[2][64]` (raster order, fall-back rules applied) | `iq_matrix` `:424-426` | `Pps.scaling: ScalingMatrices { m4, m8 }` (`params.rs:9-13, 558`) |
| `VASliceParameterBufferH264.slice_data_size/offset/flag`, `slice_data_bit_offset` | `:430-435` (`header_bits + 8` for the NAL header byte) | `SliceHeader.header_bits` (`slice.rs:142`) |
| `first_mb_in_slice`, `slice_type`, `direct_spatial_mv_pred_flag`, `num_ref_idx_l0/l1_active_minus1`, `cabac_init_idc`, `slice_qp_delta`, `disable_deblocking_filter_idc`, `slice_alpha_c0_offset_div2`, `slice_beta_offset_div2` | `:436-445` | `SliceHeader` (`slice.rs:113-133`: `slice_type_raw`, `direct_spatial_mv_pred`, `num_ref_idx_active`, `cabac_init_idc`) |
| `RefPicList0/1[32]` | `:464-469`; lists from `Dpb::build_ref_lists` (`dpb.rs:163`) at `:216-228` (modification applied there) | `dpb.rs` |
| `luma_log2_weight_denom`, `chroma_log2_weight_denom`, `luma/chroma_weight_l0/l1_flag`, `luma_weight/offset_l0/l1[32]`, `chroma_weight/offset_l0/l1[32][2]` | `:471-510` | `SliceHeader.pred_weight_table` (`slice.rs:128`) |
| Decoding-process state the buffers depend on: frame_num gaps (8.2.5.2), MMCO incl. MMCO 5, sliding window, IDR `no_output_of_prior_pics`, reorder / output order | `fill_frame_num_gap` `:273-296`, `submit_pending` `:325-336`, `start_picture` `:253-261` | `Dpb::fill_frame_num_gap` (`dpb.rs:131`), `store_picture` (`:331`), `idr` (`:97`), `flush` (`:91`), `PocState::update / update_gap_frame` (`slice.rs:495, 516`), `SliceHeader::has_mmco5` (`slice.rs:149`), `Sps::max_dpb_frames / max_num_reorder_frames` (`params.rs:475, 507`) |

**Missing fields: none** for CBP / Main / High 8-bit 4:2:0 progressive. Out of scope is refused
before any driver call: interlaced / MBAFF, non-4:2:0, >8-bit, lossless (`Sps::check_supported`,
`crates/h264/src/params.rs:518-533`); FMO (`h264.rs:188`), field pictures (`:191`), SP / SI
(`:194`), data partitioning (`:166`), PPS change within a picture (`:212`), missing references
(`:225-227`, the hybrid replays in software), >16 reference frames (`:306`). `NalStreamInfo`
already carries H.264 `avcC` info, `interlaced`, `reorder`, disposable detection
(`crates/codecs/src/hw.rs:282-317, 432`).

Two things worth a unit test when touching this code, not blockers: `slice_data_bit_offset` is
counted in the RBSP (after emulation-prevention removal, `h264.rs:430`), which is what Mesa and
iHD expect; and `MinLumaBiPredSize8x8` from the level (not from the SPS) is the usual convention.

### 1.2 `va_hevc.rs` structure, and why `va_h264.rs` should not mirror it

The fork's HEVC path is split in two: the bitstream half lives in `crates/hevc/src/accel.rs`
(`AccelDecoder<A: Accelerator>`, `:23-40, 123`: parameter sets, POC, RPS, missing references,
reference lists, DPB / bumping, all shared with the software decoder) and `va_hevc.rs` only
translates each `AccelPicture` into VA buffers and owns the libva objects (`VaHevc`, `:92-215`),
then wraps it as a `VideoDecoder` (`NativeVaDecoder`, `:542-929`) that hands out `PixelData::Gpu`
frames (`VaPicture: GpuSurface`, `:737-804`) with read-back on demand (`:806-871`).

`crates/h264` has **no** `accel.rs`: its DPB (`dpb.rs`) is generic over the frame type, but the
"bitstream half of a hardware decoder" (frame_num gaps with surface fills, MMCO 5 POC rebasing,
`is_new_picture` detection, reference-list conversion, output read-out) is exactly what upstream's
`vaapi/h264.rs::Front` (512 lines) already is. A `va_h264.rs` mirroring `va_hevc.rs` would have to
re-create that front end (4–6 h for parity alone, before zero-copy, tests and the review) and
leave two H.264 VA front ends in the tree — the opposite of U0.2. Evidence that the upstream front
end is sound: the bit-exact / streaming / mid-stream-failure / damaged-input / thread tests in
`crates/platform/tests/vaapi.rs` (`:200-240, 279, 311, 357, 452, 483`).

### 1.3 The gap: zero-copy on upstream's `Accel`

Upstream's design (`vaapi/mod.rs:30-59`): an `Accel` has a **fixed** surface list (`surfaces()`),
decodes into an index, and `read(index) -> VideoFrame` copies the picture out; the front end picks
the target as the first index not in the DPB (`h264.rs:264-265`, `hevc.rs:273`) and reads outputs
immediately so their surfaces can be reused (`h264.rs:139-147`). The libva `Session`
(`vaapi/va.rs:250-340`) opens its own shared display on the first render node (`:154-162,
174-187`; it ignores `FILMCRAFT_VAAPI_DEVICE`, hence "two VA displays are opened at startup",
`docs/fork/UPSTREAM-SYNC.md:75`), creates the context with fixed render targets (`:335`) and
reads back with `vaGetImage` (`:429`).

The fork's libva layer does what zero-copy needs: a context **without** render targets (`va.rs:401-410`,
so any surface of the format works), recycling pools where a `Surface` returns on drop
(`va.rs:947-960, 1188-1205`; oldest-first reuse against the GPU stall, `:1082-1097`), DMA-BUF export
once per surface (`:1219-1291`), display choice with `FILMCRAFT_VAAPI_DEVICE` and AMD-first ranking
(`:97-118`), and the GOP cache already budgets GPU frames by `gpu_bytes()`
(`crates/codecs/src/gop.rs:745-759`). The NV12 import path is proven by 8-bit HEVC zero-copy
(`va_hevc.rs:71, 770`; `crates/gpu/src/interop.rs`).

**Design for Y1** (new `crates/platform/src/va_accel.rs`, ≈ 250 lines, safe code: every FFI call
it needs exists in `va.rs`):

- `ZeroCopyAccel { display, config, context, pool, slots: Vec<Arc<Surface>> }` implementing
  `vaapi::Accel`. `surfaces()` = the slots' ids; `decode_h264` = `Context::param_buffer` /
  `slice_data` / `decode` (`va.rs:413, 438, 456`, as `va_hevc.rs:181-214` does); `read(i)` =
  `Surface::sync` then a `VideoFrame { data: PixelData::Gpu(VaPicture { surface: slots[i].clone(),
  origin: crop, ten_bit: false }) }` (make `VaPicture`'s constructor `pub(crate)`,
  `va_hevc.rs:738-749`; its `download` already handles NV12, `:851-869`); `fill(i, from)` =
  `Surface::write` mid-grey (`va_hevc.rs:156-168`) or copy through `Context::process_copy`
  (`va.rs:483`) / read + write.
- **One trait addition**: surfaces must not be reused while a frame holds them. Add
  `fn pick(&mut self, candidates: &[usize]) -> Option<usize>` to `Accel` with a default of
  `candidates.first()`; `ZeroCopyAccel` returns the first candidate whose `Arc::strong_count == 1`
  and otherwise **appends** a fresh pool surface (slot indices never change, so DPB references
  stay valid; cap the slot count, e.g. 32, then "no free surface" as today). Call it at
  `h264.rs:265` and `hevc.rs:273` (the HEVC side gets it for free: U0.2 groundwork).
- Wiring: `VaDecoder` holds `h264::Front<Session>` (`vaapi/mod.rs:93-96`); make it generic over the
  `Accel` (or add a second enum pair) with a constructor taking a `ZeroCopyAccel` built from
  `Display::get()` + `h264_profiles` (`mod.rs:73-80`) + the `Sps` DPB size (`mod.rs:137`). Chain in
  `lib.rs:306-333`: fork HEVC → **zero-copy H.264** → upstream read-back `VaDecoder` (the fallback
  when DMA-BUF export fails or `FILMCRAFT_NO_GPU_IMPORT` is set) → pipe. Name it
  "VA-API H.264 (in process)" for `perf.stats`; `hardware_decoder_for` (`lib.rs:221-227`) needs no
  change.
- Devices: `Display::get` picks AMD here (`va.rs:97`; ranking in `ffmpeg.rs:52-66`), the same as
  HEVC 4:2:0, so the compositor imports from the same GPU. On the Dell (Intel only) both are Intel:
  same-device import, test there (`ssh dell`, HANDOFF "Dell lane").
- New `unsafe`: none expected (all libva calls already reviewed in `va.rs`, ADR 0002 §"Review"),
  so YOUTUBE-PLAN Y1 step 2's Fable review shrinks to a check of the slot / `Arc` reasoning.

### 1.4 Step plan (≈ 7 h of the 8)

| # | Step | Files | Est. |
|---|---|---|---|
| 0 | Close Y1-PREP: cite §1.1 in the progress log; the runner drops the CLOUD-QUEUE item | `docs/fork/CLOUD-QUEUE.md`, FORK-PLAN §7 | 0.25 h |
| 1 | `Accel::pick` with default; use it in both front ends; unit test with the recording stand-in | `vaapi/mod.rs:32-59`, `vaapi/h264.rs:265`, `vaapi/hevc.rs:273`, `vaapi/tests.rs` | 0.5 h |
| 2 | `va_accel.rs::ZeroCopyAccel` (+ `VaPicture` constructor) | new file; `va_hevc.rs:738`; `lib.rs` mod list | 2 h |
| 3 | `VaDecoder` over any `Accel`; chain + names | `vaapi/mod.rs:93-164`, `lib.rs:306-333` | 1 h |
| 4 | Tests: parity of the zero-copy decoder's `download()` against software on the existing H.264 fixtures (pattern: `tests/va_native.rs:9`, `tests/ui/va_interop.rs:30`); fallback to the read-back decoder when export is refused (`FILMCRAFT_NO_GPU_IMPORT`; pattern `tests/vaapi.rs:279`); seek / reorder already covered by `parity_streaming` (`:168`) | `crates/platform/tests/vaapi.rs` | 1.5 h |
| 5 | Quick bench before / after: `bench --only h264-2160` play-full / scrub (`crates/ui-egui/examples/bench/sections.rs:188-196, 251`; fixtures `fixtures.rs:68-69`, 2160p23.976 x264); Dell run of `tests/vaapi.rs` | bench, Dell | 1 h |
| 6 | ADR 0002 note, HANDOFF / progress log, `perf.stats` backend name check | docs | 0.75 h |

Time-box fallback: the read-back H.264 path already meets "decodes on the hardware path on AMD
and on Intel, bit-exact, out-of-scope streams fall back cleanly"; only zero-copy is at risk, and
it ships behind the fallback if step 2–3 overrun. Acceptance "4K23.976 High plays with zero drops at
Full" should be measured with the read-back decoder first (it may already pass at 4K).

## 2. 6E: 6.2K export above 24 fps

### 2.1 The current export / encode path (where each 6E step lands)

| Stage | Code |
|---|---|
| Export driver: blocking `export()` loop with rayon batches | `crates/export/src/lib.rs:2011-2127` (`batch = threads.clamp(2,16)`, `:2027`) |
| Stepped exporter (jobs): render batch k+1 while encoding k, `IN_FLIGHT_BUDGET`, `ahead` | `crates/export/src/job.rs:1-60` (design), `:89`, `:120`, `:476-620` (`step`) |
| GPU-rendered frames and zero-copy into encoder surfaces | `crates/platform/src/gpu_export.rs:1-13` (design), `GpuExport::create` `:124-136` (one headless compositor, `HighPerformance` adapter: `crates/gpu/src/lib.rs:1364-1366`), `render_into` `:159-222`, `FILMCRAFT_NO_ZERO_COPY` |
| HEVC encoder with backend order and mid-stream fallback | `crates/platform/src/hevc_export.rs`: `Backend` `:95-101` (VaNative / ffmpeg `hevc_vaapi` / libx265), `with_backends` `:784-820`, `fall_back` `:832`, `VaWorker` `:632-720`, AU parser / `hvcC` config `:384-560`, `gpu_target` `:963` |
| In-process VA-API encoder | `crates/platform/src/va_enc.rs`: `supported` `:121-123` (**EncSlice only**), `new` `:125-215` (`EncSlice` `:134`, attribute checks `:143-176`, packed headers required `:163-166`), `profile()` `:440` (Main / Main 10 only), `set_chroma_format_idc(1)` `:401`, `RateControl` `:28` (CQP / CBR / VBR), `encode_surface` `:264` |
| Encode display: `FILMCRAFT_VAAPI_ENCODE_DEVICE` else the decode display | `crates/platform/src/va.rs:123-141` (`Display::for_encode`, process-global `OnceLock`) |
| Export settings (`HevcEncoder` Auto / Vaapi / X265, bit depth, bitrate mode / CRF) and the command parameters | `crates/export/src/settings.rs:70, 122-150`; `crates/engine/src/export_tools.rs:195-290` (`file.exportMedia`, `hevcEncoder:`) |
| Packets → one muxer, hvcC from the encoder's parameter sets | `crates/export/src/lib.rs:735` (`EncodedPacket`), `hevc_export.rs:518, 818, 891` |
| SPS / PPS / slice header writer (fork-only) | `crates/hevc/src/write.rs` (436 lines) |

### 2.2 Measurement tools that exist (6E.1)

- `cargo run --release -p filmcraft-platform --example encode_bench -- vaapi 10 300 6240x4160 24`
  (encoder alone on synthetic frames, `crates/platform/examples/encode_bench.rs:1-20`);
  `FILMCRAFT_VAAPI_ENCODE_DEVICE=/dev/dri/renderD128` moves it to Intel. Also the speed test in
  `va_enc.rs:588-605` (`FILMCRAFT_ENC_SPEED_6K=1`).
- `examples/decode_bench.rs` (software vs registered hardware decoder, one file);
  `examples/va_native_bench.rs`, `va_422_handoff_bench.rs` (6.1b).
- `target/release/examples/bench --section xm5 --only export` (`xm5-export-4k`, `4k graded`,
  `6k`, `tnr`: `crates/ui-egui/examples/bench/xm5.rs:966-978`), `FILMCRAFT_EXPORT_TRACE=1` for
  per-batch stage times, `FILMCRAFT_BENCH_KEEP=<dir>` to keep outputs for ffprobe / mpv.
- Prior numbers to compare with: FORK-PLAN §7 `:1516-1526` (AMD 6.2K encode 30.7 ms/frame, Intel
  72.9 ms; AMD-decode → read-back → Intel-encode 9.9 fps), `docs/fork-benchmarks.md:291-312`.
- **Not scripted**: the ffmpeg-only checks of 6E.1 (`hevc_vaapi -low_power 1` vs `0`, Intel
  decode-only ceiling, Annex B concatenation + ffprobe / mpv). One-off commands; record in
  `docs/fork-benchmarks.md` "Phase 6E". No 4:2:2 export bench case exists yet (owed from Phase 6,
  HANDOFF "Owed").

### 2.3 6E.2 cheap wins

- `EncSliceLP`: choose the entry point per display in `va_enc.rs:122, 134` (`Display::supports`,
  `va.rs:248`); the attribute checks `:143-176` must be re-run for LP (packed headers, rate
  control modes, HEVC features / block sizes differ for VDEnc). Prerequisite: 6E.1.1 shows LP is
  faster on Intel.
- Speed / quality preset: nothing uses `VAConfigAttribEncQualityRange` / the quality-level misc
  parameter today (grep: no `quality` in `va_enc.rs` outside rate control); add a
  `Misc<VAEncMiscParameterBufferQualityLevel>` next to the rate-control ones (`va.rs:369-399`,
  `va_enc.rs:420-435`), an `ExportSettings` field (`settings.rs` near `HevcEncoder`, `:122-150`),
  `export_tools.rs:281` parsing, the export dialog, es + zh-cn strings. The ffmpeg backends map it
  to `-compression_level`.

### 2.4 6E.3 chunk scheduler and second pipeline: what it touches

- Chunks: a run of `Exporter` batches cut at IDRs (`job.rs:476-620`; keyint already produces IDRs,
  `va_enc.rs` sequence params). A work queue needs the exporter to render a chunk on a chosen
  device and hand packets to the single muxer in order (`lib.rs:735`).
- Per-lane devices (the real work): `Display::for_encode` is process-global (`va.rs:123-141`) →
  a display parameter on `VaHevcEncoder::new` (`va_enc.rs:125, 133`); one `GpuExport` compositor
  per lane (`gpu_export.rs:124-136` is a single static; adapter choice by DRM node / PCI id in
  `crates/gpu/src/lib.rs:1364` and `interop.rs:35`); decode on the lane's GPU: `Display::get` is
  process-global (`va.rs:97`) — generalise 6.1b's `display_for` (`va_hevc.rs:80-82`) into a
  per-stream display choice (for Y1-on-upstream this is the `ZeroCopyAccel`'s display argument).
  Grading on Intel already passes parity (FORK-PLAN §7).
- Joiner: `VaHevcEncoder::headers()` (`va_enc.rs:231`) and `Parser.configured` (`hevc_export.rs:818`)
  give the hvcC; require both devices' SPS / PPS byte-identical (the fork writes them itself,
  `crates/hevc/src/write.rs`) or switch to in-band `hev1` (check `crates/isobmff` for `hev1`
  support before relying on it). Consistency at joins: CQP per chunk (`RateControl::Cqp`,
  `va_enc.rs:28, 315`).
- Never-crash: an Intel failure re-queues the chunk on AMD via the existing `fall_back` shape
  (`hevc_export.rs:832`); settings "Export engines: Auto / AMD only / AMD + Intel" (new, engine
  settings + i18n).
- Prerequisite: 6E.1.3 shows Intel adds ≥ ~4 fps, else stop at 6E.2 (EXPORT-PLAN).

### 2.5 6E.4 4:2:2 10-bit master on the Intel encoder

Needs: the encode display chosen by profile (`Display::for_hevc_422`'s search, `va.rs:143-190`,
generalised to `supports(VAProfileHEVCMain422_10, EncSlice)`); `profile()` `va_enc.rs:440` and
`set_chroma_format_idc` `:401`; range-extension SPS fields in `crates/hevc/src/write.rs`; a Y210 /
P210 input pool (`SurfacePool`, `plane_rows` already knows Y210, `va.rs:992`); the compositor's
Y'CbCr pass writing packed 4:2:2 into the exported surface (`gpu_export.rs:159-222` renders two
planes for NV12 / P010; `crates/gpu/src/unpack422.rs` is the read direction only); preset in
`crates/export/src/presets.rs` + settings + i18n; test = decode with the built-in 4:2:2 decoder,
PSNR vs Program. Risks from 6.1b: iHD VPP context leak on destroy (HANDOFF "Owed"), GPU resets on
damaged input. Prerequisite: none beyond 6.1b (done); independent of 6E.3.

**Interaction with U0.2:** 6E changes `va_enc.rs`, `va.rs::Display` and `gpu_export.rs`, none of
which U0.2 retires (U0.2 retires the fork's *decode* core, §3). If Y1 keeps the fork's `va.rs` as
the one libva layer (recommended, §3.3), 6E and U0.2 don't conflict and the order can stay.

## 3. U0.2: one VA-API stack

### 3.1 What the two stacks are today

| Component | Fork (`hevc-vaapi`) | Upstream (`vaapi/`, identical in the fork tree) |
|---|---|---|
| libva binding | `va_sys.rs` (bindgen, 7 953 lines: decode + encode + VPP + DRM-PRIME; regenerated by `gen-va-sys.sh`) | `vaapi/ffi.rs` (hand-written, 519 lines, decode only, `abi_tests.rs`) |
| Display | `va.rs:68-260`: per-node open, `FILMCRAFT_VAAPI_DEVICE`, `for_encode`, `for_hevc_422`, profile query `supports` | `vaapi/va.rs:120-246`: first node that initialises, one shared display, replaced on GPU reset (`:276-290`) |
| Objects | `Config`, `Context` (no render targets), `SurfacePool` + `Surface` (recycled, exported, `LinearCopy`, `Misc` params, `CodedBuffer`) | `Session` (config + context with fixed targets + surfaces + one `VAImage`) |
| HEVC bitstream half | `crates/hevc/src/accel.rs` (`AccelDecoder`, fork-only, 459 lines; `hvcC` configure `:164`) | `vaapi/hevc.rs::Front` (612 lines) |
| HEVC VA buffers | `va_hevc.rs:242-535` (+ Rext `:352-398`, hostile tile checks `:223-240`, entry-point bounds `:466-469`) | `vaapi/hevc.rs:370-612` (tiles via `Layout` `:406-422`) |
| H.264 | none | `vaapi/h264.rs` |
| Output | `PixelData::Gpu` (`VaPicture`, `va_hevc.rs:737-804`), read-back on demand | `vaGetImage` → `VideoFrame` (`vaapi/va.rs:420-491`) |
| Missing references | mid-grey surface (`va_hevc.rs:151-179`) | error → hybrid replays in software (`vaapi/hevc.rs:258-261`) |
| 4:2:2 (6.1b) | yes (below) | no |
| Hybrid | fork's chain of hardware fallbacks (`hybrid.rs`, +63 lines: `HwFallback`, `with_fallback`, replay from the restart point) | single software fallback |
| Tests | `tests/va_native.rs`, `va_hevc_422.rs`, `va_422_handoff.rs`, `vaapi_displays.rs`, `tests/ui/va_interop.rs`, examples `va_native_bench.rs`, `va_422_handoff_bench.rs` | `tests/vaapi.rs` (fork-extended, +318), `vaapi/tests.rs`, `abi_tests.rs` |

### 3.2 What the fork must port onto upstream's decoder

1. **Zero-copy output** = Y1's `ZeroCopyAccel` + `Accel::pick` (§1.3). Done once in Y1.
2. **HEVC 4:2:0 through `vaapi/hevc.rs::Front`** with that accel: parity on the conformance set
   (`tests/va_native.rs`), the xm5 play-full / jkl / multicut / 6.2K numbers against the U0 table
   (`docs/fork/UPSTREAM-SYNC.md`, "xm5 quick pass"). Behaviour differences to decide: missing
   references (grey vs software replay: keep the fork's grey, as `Accel::fill(None)` already makes
   grey, `vaapi/va.rs:434-458`); the fork's hostile-PPS tile checks and entry-point bounds
   (`va_hevc.rs:223-240, 466-469`) should move into the front end (Intel resets on such damage).
3. **6.1b's 4:2:2 additions** (kept small per §1a.34): the profile / RT-format / fourcc table
   (`va_hevc.rs:69-76`), the display choice `display_for` (`:80-82`, `va.rs:143-190`), the Rext
   picture / slice buffers (`:184-214, 352-398`; upstream's `ffi.rs` has no Rext structs or Y210
   constants → either add hand-written structs with an ABI test like `va_hevc.rs:1027-1082` or
   alias `va_sys` types), the Y210 pool and read-back (`:806-838`), the hand-off `LinearCopy`
   (`:561-700`, `va.rs:841-925`, `LinearAlloc` / `DmaBufAlloc` `:705-760`), the "stopped after a
   failed picture" rule (`:546-551, 887-895`), the probe hooks (`crates/codecs/src/hw.rs:47-64`,
   `lib.rs:155-158`), `decode_annexb[_with]` (`:934-978`, used by tests and benches), and the
   4:2:2 tests / examples re-pointed. `VaDecoder::new` must stop refusing `chroma_format_idc != 1`
   and `range_extension` (`vaapi/mod.rs:117, 148`), and `Session`-level format tables
   (`vaapi/va.rs:264-270`) go away with `Session`.
4. **Display layer**: one display per node. Recommend the fork's `va::Display` as the single
   libva object layer (it is the superset: encode, VPP, export, pools, profile query, device
   env vars) and retiring upstream's `Session` / `Display` / `shared()`; this also closes the
   "two displays opened at startup" follow-up. Note `render_nodes()` lives in the fork-only
   `ffmpeg.rs:52-66` and must move (Phase 7.7 removes `ffmpeg.rs`).
5. **Bindings**: two choices. (A) one binding, `va_sys` everywhere, `vaapi/ffi.rs` reduced to type
   aliases + the `*_bits` modules (bigger diff, contribution-unfriendly: upstream wants the
   hand-written header-checked `ffi.rs`); (B) keep `ffi.rs` for the decode buffers the front ends
   fill and `va_sys` for everything else (status quo; "one stack" at the decoder level is still
   reached). Recommend **B** for U0.2 and leave A to a later contribution-prep cleanup.
6. **Chain cleanup**: `lib.rs:306-333` becomes one in-process factory entry (H.264 + HEVC, zero-copy
   with read-back fallback) → pipe; `perf.stats` backend names; remove `va_hevc.rs`'s decode core
   and `crates/hevc/src/accel.rs` (check `crates/hevc/tests/accel.rs` first); `FORK-ONLY.md`, ADR
   0002 / 0005 updates.

### 3.3 Conflicts and step order (estimates)

Upstream drift: **none** in `vaapi/` since U0 (`b2ef7689`, base `4e2d0bc2`); upstream's platform
commits since then are HEVC *software* decoder speed-ups (#1039, #1053, #1067) and VideoToolbox
counters, which the next weekly sync takes without touching this work.

| Step | Work | Est. |
|---|---|---|
| U0.2.1 | zero-copy `Accel` + `pick` | done in Y1 |
| U0.2.2 | HEVC 4:2:0 on upstream's front end; parity + xm5 regression gate | 3–4 h |
| U0.2.3 | 4:2:2 port (Rext structs + ABI test, display choice, Y210 pool, hand-off, stop rule, tests); Dell run | 4–6 h |
| U0.2.3b | speed floor (FORK-PLAN §1a.58, §1a.62–65): export ≥ 24 fps 6.2K, ≥ 30 fps 4K incl. graded (Intel and AMD faster on graded frames at f32), Auto one-GPU rule, half floats preview-only; multicut CPU / frame back to U0 | 10–14 h |
| U0.2.4 | one display layer (fork `va::Display` under the front ends; `render_nodes` out of `ffmpeg.rs`) | 1–2 h |
| U0.2.5 | chain cleanup, removals, docs / ADRs, `cargo xtask ci`, tag `upstream-sync-2` | 2–3 h |
| | **Total** | **≈ 14–23 h** (+ Y1's 7) |

### 3.4 Order recommendation (recommend, don't decide)

Keep the owner's order **Y1 → 6E → U0.2**, with Y1 built on upstream's decoder as in §1.3. Then
Y1 *is* U0.2.1 and nothing is ported twice; 6E's decode-side needs (a per-lane decode display,
§2.4) are one argument on the same `ZeroCopyAccel`. The alternative **Y1 → U0.2 → 6E** would let
6E's lane work land on the final decoder, but 6E touches the encoder / compositor / exporter far
more than the decoder, so the gain is small and it delays the owner's export goal. Writing a
separate `va_h264.rs` is the one option to avoid: it would be ported again (or deleted) in U0.2.
