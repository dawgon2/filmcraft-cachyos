# PREP-R1: CinemaDNG, pure Rust (audit before the work)

R1-PREP (cloud, 2026-10-14), docs only, no code changes. Line numbers are from the working tree
on branch `hevc-vaapi` on that date.

**R1 itself waits for the owner's go (FORK-PLAN §1a.22: "camera RAW" is on the never-without-the-owner
list); this is preparation only.**

## 1. What R1 asks for (RAW-PLAN.md, "Phase R1", about 15-25 h, upstream-contributable, tag `raw-1`)

Clean-room from Adobe's published DNG spec (1.4+) and the TIFF / TIFF-EP specs. Never from
dcraw, LibRaw, RawTherapee or darktable.

- **R1.1 Reader.** A new L0 crate (e.g. `crates/dng`) depending only on `filmcraft-bitstream`.
  TIFF / DNG IFD parsing with hostile-input caps (IFD count, chain depth, offsets, dimensions,
  tile counts). Uncompressed and lossless-JPEG (LJ92) tiles and strips, 8-16 bit. Tags:
  linearization table, black / white levels, `CFAPattern`, `DefaultCrop`, `AsShotNeutral`,
  `ColorMatrix` / `ForwardMatrix` / `CalibrationIlluminant`, `BaselineExposure`, timecode and
  frame-rate tags.
- **R1.2 RAW develop stage** (shared with R2; its own module; no ffmpeg). Black / white
  normalisation, white balance, demosaic (bilinear for scrubbing, an edge-directed method for
  playback / export, both on the GPU with a CPU reference), camera matrix to XYZ, then
  DaVinci Wide Gamut / Intermediate (FORK-PLAN §1a.14).
- **R1.3 Clip model.** A numbered DNG folder imports as one clip (rate from tags or the sequence
  preset, missing frames reported, a sidecar WAV linked by name / timecode). RAW controls on the
  clip (white balance / tint, exposure, highlight recovery, demosaic quality) as `raw.*` commands
  plus an Inspector section.
- **Accept:** synthetic fixtures from a test DNG writer in the crate decode exactly before
  demosaic; develop matches a CPU reference within a stated delta E; fuzz-style hostile DNGs never
  crash; a synthetic 4K CinemaDNG sequence plays (proxies / render cache allowed).

Headline finding: **nothing RAW-specific exists yet** (no TIFF parser, no lossless JPEG, no Bayer
type, no demosaic). What R1 builds on is the colour pipeline (very complete), the numbered-still
sequence machinery, and the L0 crate template (`dnx`, `apv`).

## 2. What exists today

| Where | What it is | Used by |
|---|---|---|
| Search result | `grep -i` for `dng`, `cinemadng`, `ljpeg`, `sof3`, `lj92`, `0xFFC3`, `bayer`, `demosaic`, `cfa`, `lossless jpeg` over `crates/` and `apps/` finds nothing relevant (the `tiff` hits are only the `image` crate and stills export). No DNG, no LJPEG, no Huffman decoder for JPEG, no Bayer types. | all |
| `crates/media/src/still.rs:13` `sniff` | Treats any file starting `II*\0` or `MM\0*` (`:20-21`) as a still image. A `.dng` file therefore matches this sniff. | R1.3 |
| `still.rs:43` `decode_raster` | DPX, then TGA, then `image::load_from_memory`. That is the path a `.dng` would reach today; I did not run it, so what the `image` TIFF decoder does with a DNG is unverified (it may fail or show an embedded preview IFD). R1 must route DNG before this point. | R1.3 |
| `crates/media/src/lib.rs:178` `STILL_EXTENSIONS` | `png jpg jpeg gif webp tif tiff bmp tga dpx`. No `dng`. `is_importable` (`:184`) uses it plus audio / video lists. | R1.3 |
| `crates/media/Cargo.toml:19` | `image 0.25` with `png jpeg gif webp tiff bmp` features. Not usable for DNG tags or LJ92, and R1.1 must not pull it into an L0 crate. | - |
| `crates/media/src/dpx.rs:1-30` | The house pattern for a clean-room still decoder: spec named in the header, `MAX_PIXELS = 1 << 28` (`:19`) bounds the allocation a header can ask for, errors as `MediaError::Decode(...)`, "Not decoded" list in the doc. Copy this style. | style |
| `crates/media/src/sequence.rs:23` `Numbered`, `:101` `sequence_frames`, `:122` `ImageSequenceSource`, `:132` `new`, `:167-177` `frame_count` / `missing_frames` / `frame_path` | Numbered stills as a movie: missing frames are `None` and reported, frames decode on demand through a `FrameLoader` closure into a `FrameCache`. Rate is passed in. The first present frame is decoded at open for size / alpha. | R1.3 |
| `crates/engine/src/media_pool.rs:~190` `open_image_sequence`, `image_sequence_frames` | Lists the directory via `Services::list_dir` (or probes consecutive numbers, 100 misses, cap 1 000 000) and builds an `ImageSequenceSource` with a rate and name. Also `apps/filmcraft-cli/src/probe.rs:143`. | R1.3 |
| `crates/media/src/lib.rs:94` `MediaInfo` | Has `start_timecode: Option<i64>` (`:108`), `video: Option<VideoStreamInfo>` with `frame_rate`, `color: ColorInfo`, `pixel_format` label. A DNG source can fill these from tags. | R1.3 |
| `crates/media/src/lib.rs:190-205` `open_bytes` / `Opener` | Extra openers get first try (`filmcraft_codecs::openers()` is registered in `MediaPool::default`), then WAV, then stills. A DNG opener can go in the codecs openers list (`crates/codecs/src/lib.rs:133`, `apv::opener` is the model, `crates/codecs/src/apv.rs:1-50`). | R1.3 |
| `crates/frame/src/lib.rs:46` `PixelData` | `Rgba8`, `RgbaF32` (premultiplied linear), `Yuv8`, `Yuv16`, `Yuv16Bi`, `Gpu`. No single-plane Bayer variant. `VideoFrame::rgba_f32` at `:206`. A develop stage can output `RgbaF32` with no frame-type change; a GPU-side develop needs a new Bayer plane variant (touches L1 and every match on `PixelData`). | R1.2 |
| `crates/bitstream/src/lib.rs:34` `BitReader`, `:86` `read_bits`, `:70` `peek`, `:64` `seek_bits` | MSB-first reader with `Result` and `BitError::Eof`. Good base for the LJ92 Huffman decoder. It handles no JPEG `0xFF00` byte stuffing; the DNG crate must unstuff first (cf. `unescape_rbsp` at `:252` for the H.264 analogue). No Huffman table type here (the only Huffman code is `crates/aac/src/huffman.rs`, AAC specific). | R1.1 |
| `crates/color/src/spaces.rs:19` `Gamut`, `:70` `to_xyz`, `:81` `npm`, `:89` `mul`, `:99` `mul_vec`, `:103` `inverse`, `:117` `gamut_matrix`, `:124` `to_f32` | Matrix toolbox (`Mat3` of f64). `Gamut::DaVinciWide` exists (`:32`). So camera-to-XYZ then `inverse(DaVinciWide.to_xyz())` is expressible today. | R1.2 |
| `spaces.rs:318` `WorkingSpace::DaVinciWide`; `ops.rs:28-45` `di` constants, `di_encode` / `di_decode`; `crates/color/src/grade.rs:38` `GradeSpace::DaVinciIntermediate` | DaVinci Wide Gamut / Intermediate working space is implemented (ADR `docs/adr/0003-gpu-color-ops.md`). | R1.2 |
| `spaces.rs:150-176` `ColorSpace` | Includes `FLogFGamut` and `FLog2FGamut`, `Linear709`, no "camera native / scene linear in camera primaries". | R1.2 |
| `crates/color/src/transform.rs:189` `InputTransform`, `:282` `input_ops` | Source colour space to working space as `ColorOp` list. The develop stage's tail (camera to DWG) should end as a `ColorOp::Matrix` so the GPU path reuses it. | R1.2 |
| `crates/color/src/ops.rs:248` `ColorOp` (`Matrix { m, offset }`, `Table`, `Lggo`, `Saturation`, ...), `:385` `ColorOp::matrix(Mat3)` | The composable colour-op chain. GPU twin: `crates/gpu/src/colorops.wgsl`, `colorops.rs`, parity tests `crates/gpu/src/colorops_tests.rs` (GPU vs `ColorOp::apply`). The R1 parity test should follow this pattern. | R1.2 |
| `crates/color/src/wb.rs:20` `gains(temperature, tint)`, `:69` `solve`, `:314` `xyz_to_lab`, `:329` `delta_e2000` | WB gains from temperature / tint (the UI model, not a camera-neutral to CCT solver), and a delta E 2000 function usable for the "within a stated delta E" acceptance test. No CCT-from-`AsShotNeutral`, no illuminant interpolation of two `ColorMatrix`es. | R1.2 / R1.3 |
| `crates/dnx/Cargo.toml`, `crates/apv/` | Template for a new L0 codec crate: `filmcraft-bitstream` + `thiserror` (+ optional rayon), `[lints] workspace = true`, `filmcraft-testkit` as dev-dependency. `crates/bitstream/src/lib.rs:7` shows the crate-level `deny(clippy::unwrap_used, ...)` attribute. | R1.1 |
| `xtask/src/main.rs:29` `LAYERS`, `:64` `L0_FOUNDATION`, `:94` `SAME_LAYER` | A new crate must be listed in `LAYERS` (xtask says "not assigned a layer" otherwise, `:153`) and in `docs/architecture.md` §1 table (`:40-59`). L0 codec crates may depend only on `bitstream`. `cargo xtask wasm` checks layers <= 4 (`:264`), so the DNG reader must be wasm-clean (no threads without a feature gate, as in `dnx`). | R1.1 |
| `Cargo.toml:31-32` | Workspace path-dependency entries (`filmcraft-dnx`, `filmcraft-apv`) and `[profile.dev.package.*]` opt-levels (`:117-120`). A new crate adds both. | R1.1 |
| `crates/testkit/src/fixtures.rs:18` `fixtures_root`, `:26` `fixtures_dir`, `:52` `generate`; `cargo xtask fixtures` (`xtask/src/main.rs:402`) | Fixture directories under `target/fixtures/`, generate-if-missing helpers, per-crate ignored `generate_fixtures` test. | R1 tests |
| `crates/media/src/lib.rs` `STILL_EXTENSIONS`, `crates/interchange/src/common.rs:175` | A second extension list classifying files as stills for interchange. `dng` would need adding if CinemaDNG should be classed as video there. | R1.3 |

## 3. Gaps per R1 step, files touched, layering

Layering (docs/architecture.md §1, enforced by `cargo xtask layers`): L0 `dng` -> `bitstream`
only. The develop stage cannot live in `dng` if it needs `color` (a foundation crate, not
`bitstream`) because of the "codec crates depend on bitstream only" rule. So split:

| Piece | Suggested home | Layer | May depend on |
|---|---|---|---|
| TIFF / DNG parser, LJ92 decoder, raw sample unpack, test DNG writer | new `crates/dng` | L0 | `filmcraft-bitstream`, `thiserror` |
| CPU develop (normalise, WB, demosaic, camera to DWG matrix) | new `crates/rawdev` (name open), or a module inside `crates/media` | L1 if a crate (xtask needs `("rawdev", 1)` plus any same-layer edges) | `dng`, `color`, `frame` |
| GPU develop shaders and pipeline | `crates/gpu` (new `rawdev.rs` / `rawdev.wgsl`) | L3 | `rawdev` types |
| DNG opener for one file / a folder | `crates/codecs` (opener list, `lib.rs:133`) or `crates/media` | L2 / L1 | `dng`, `rawdev` |
| `raw.*` commands, per-clip RAW settings | `crates/engine`, `crates/project` (clip params, serde) | L4 / L1 | - |
| Inspector section | `crates/ui-egui` (registers automation ids) | L5 | - |

Per step:

- **R1.1 Reader.** Create `crates/dng/{Cargo.toml,src/lib.rs,src/tiff.rs,src/ljpeg.rs,src/tags.rs,src/writer.rs (test-only or feature)}`;
  edit `Cargo.toml` (workspace dep + dev profile), `xtask/src/main.rs` `LAYERS`, `docs/architecture.md`
  §1 table. Needs: byte-order aware IFD walker with caps; strip / tile table readers; uncompressed
  8/10/12/14/16-bit unpackers; LJ92 (SOF3 predictor 1-7, DHT, restart intervals, multi-component
  interleave as used by CinemaDNG tiles); linearization table; the tag set above. DNG also has
  SubIFDs (main image is often not IFD0); the SubIFD pointer path is needed. Timecode
  (`TimeCodes` 51043) and `FrameRate` (51044) tags are CinemaDNG-specific. Size: L.
- **R1.2 Develop.** Create the develop module / crate above, a GPU shader, and parity tests.
  Gaps: no Bayer type or demosaicer; no illuminant-interpolated camera matrix; the matrix chain
  `camera RGB -> XYZ (D50 per DNG) -> adapt -> DaVinci Wide` must use the existing
  `color::spaces` toolbox (`Gamut::DaVinciWide.to_xyz()`, `inverse`, `mul`) plus a Bradford (or
  similar) adaptation, which does not exist in `color` today (not verified exhaustively; grep for
  `bradford` in `crates/color/src` found no helper, so add one with a test). End the chain as
  `ColorOp::matrix(...)` so scrub and export share it. Clip handling: highlight recovery needs
  a stated rule. Size: L.
- **R1.3 Clip model.** Edits: `still.rs` `sniff`/`decode_raster` (route DNG before the TIFF magic
  match, or exclude files with `DNGVersion`), `STILL_EXTENSIONS` or a new RAW extension list,
  `interchange/common.rs:175`, `media_pool.rs` sequence open (a DNG folder becomes a clip with the
  tag-derived rate; currently the rate is a parameter), `ImageSequenceSource` or a new
  `DngSequenceSource` (the existing one decodes to `Raster` RGBA8, which cannot carry >8-bit linear
  data; R1 needs an f32 / 16-bit frame path), sidecar WAV link by name / timecode, `raw.*` commands
  in `crates/engine` (everything is a command; ids, params, `enabled()`), project serde for the
  per-clip settings (`crates/project`), Inspector section with automation ids (`crates/ui-egui`),
  docs page per FORK-PLAN §1a.21-style rule (user-visible feature pages). Size: L.
- **Playback acceptance.** A 4K DNG sequence is decoder-bound (LJ92 is serial per tile); needs
  tile-parallel decode (rayon behind a feature, as `dnx` does) and proxies / render cache.

## 4. Fixtures

- **ffmpeg cannot be relied on to write DNG.** It has a DNG/TIFF *decoder* path in some builds, but
  no CinemaDNG muxer and no DNG encoder I can vouch for (I did not run ffmpeg; builds vary). Do not
  plan on ffmpeg as the fixture generator. It can still serve as an independent oracle for the
  *RGB tail* of the pipeline where its build supports raw TIFF, but treat that as optional and
  skipping (like the other oracle tests, `FILMCRAFT_REQUIRE_ORACLES`).
- **Primary plan (as RAW-PLAN says): a test DNG writer in `crates/dng`.** Builds a minimal DNG in
  memory: little- and big-endian TIFF header, IFD0 + a SubIFD, uncompressed strips and tiles at
  8/10/12/14/16 bit, an LJ92 *encoder* for round trips (also tests the decoder against a second
  implementation of our own), CFA patterns RGGB/BGGR/GRBG/GBRG, tags set to known values, a
  linearization table, a `DefaultCrop`. The expected Bayer samples are then known exactly.
- **Develop-stage reference:** a synthetic scene (colour chart patches of known XYZ, rendered
  through an inverse camera matrix into a mosaic) so camera -> DWG output has a known answer; compare
  with `color::wb::delta_e2000` (`wb.rs:329`).
- **Public-domain / CC0 real DNGs** (e.g. from the raw.pixls.us archive, CC0) are useful for a
  manual smoke test only. Per the repo rule, **media is never committed**; keep any download under
  `target/fixtures/` (see `crates/testkit/src/fixtures.rs:26` `fixtures_dir`) and make the test skip
  when absent. Check each file's licence before using it, and do not copy tables from any
  GPL / LGPL source.
- Hostile corpus: generated by the writer plus byte-flipping mutations from a seeded RNG, never
  stored.

## 5. Risks

- **Hostile input.** TIFF is a pointer-chasing format. Cap and check: IFD count per file, chain depth
  (cycle detection by visited-offset set), SubIFD recursion depth, offset + length overflow
  (`checked_add`), tag count x type size overflow, strip / tile offsets and byte counts beyond the
  file, tile counts (`ceil(w/tw) * ceil(h/th)` overflow and a sane cap), width x height x bytes per
  sample against an allocation cap (follow `dpx.rs` `MAX_PIXELS = 1 << 28`, `try_reserve`),
  `BitsPerSample` outside 8..=16 (or LJ92 precision 2..=16), zero tile size, zero `CFARepeatPatternDim`,
  `WhiteLevel <= BlackLevel` (division by zero in normalisation), NaN / zero `AsShotNeutral`
  (division), singular `ColorMatrix` (inverse of a singular matrix: `color::spaces::inverse` at
  `:103` must be checked for singularity before use), a huge linearization table, LJ92 Huffman
  tables that are over-subscribed or empty, bit run past the end, restart-marker misuse, predictor
  that walks outside the row, component count mismatch with the tile.
- **Never crash.** No `unwrap` / `expect` / `panic!` / indexing / unchecked arithmetic outside tests;
  put the crate-level `deny(clippy::unwrap_used, ...)` header (as `crates/bitstream/src/lib.rs:7`),
  use `get()`, `Result`, bounded loops. Source decode runs under the existing `catch_unwind` job
  guards, but that is the last resort. Every crash found gets a regression test that panicked before.
- **Clean-room.** Adobe's DNG spec is public and may be read. Do not use the Adobe DNG SDK source,
  and do not copy from dcraw (public-domain claims are murky and it is GPL-style derived widely),
  LibRaw (LGPL / CDDL), RawTherapee or darktable (GPL), or ffmpeg's `dng`/`tiff`/`mjpeg` code
  (LGPL). The JPEG lossless process (ITU-T T.81 Annex H) is a public standard; implement from
  that. Record sources in the crate README as `dnx` / `apv` do. No Adobe assets in the UI.
- **Performance.** LJ92 is sequential; bilinear scrub at 4K needs the GPU path; CPU reference only
  for tests. An f32 4K frame is about 128 MB (4096 x 2160 x 4 x 4); mind the frame cache budget.
- **Frame model churn.** Adding a Bayer variant to `PixelData` touches every exhaustive match in
  render / gpu / scopes. Prefer: CPU develop first to `RgbaF32`, GPU develop as a later step.
- **Colour accuracy.** DNG uses D50 PCS conventions; a mistake in illuminant handling shows as a
  global tint. The delta E test must cover two illuminants.
- **wasm gate.** `cargo xtask wasm` builds layers <= 4; keep `dng` free of `std::fs`, threads
  (feature-gate rayon) and `unsafe`.

## 6. Suggested tests

Unit, in `crates/dng`:
- TIFF header both byte orders; IFD with each field type; out-of-line values; offset overflow.
- IFD loop (next pointer to itself / to an earlier IFD) returns an error; deep SubIFD chain stops.
- Huge `ImageWidth` / `ImageLength`, huge tile counts, zero tile size: error, no allocation.
- Uncompressed 8/10/12/14/16-bit, strips and tiles, both byte orders: exact samples.
- LJ92 encode/decode round trip for every predictor 1-7, precisions 8-16, 1/2/4 components, restart
  intervals; truncated data; corrupt Huffman table; byte-stuffed `FF00`.
- Linearization table applied; black / white level per channel; `DefaultCrop` odd origins keep CFA
  phase; all four CFA patterns.
- Property test (`proptest`, already used in `time`/`bitstream`): random bytes and mutated valid DNGs
  never panic.
- Tags: `AsShotNeutral`, `ColorMatrix1/2`, `ForwardMatrix1/2`, `CalibrationIlluminant1/2`,
  `BaselineExposure`, `TimeCodes`, `FrameRate` parse to the written values; absent tags give
  defaults.

Develop stage:
- Black / white normalisation exact on a ramp; clipped highlights rule.
- Demosaic of a flat colour and a horizontal gradient reproduces it; bilinear vs edge-directed on
  a diagonal edge (edge-directed measurably better); CFA phase after crop.
- Chart scene -> DWG within a stated delta E 2000 (`wb.rs:329`) under two illuminants.
- GPU vs CPU parity in the pattern of `crates/gpu/src/colorops_tests.rs`.

Clip / UI:
- A folder of N frames with gaps: `missing_frames()` correct, rate from tags and from preset.
- Sidecar WAV linking by name and by timecode.
- `.dng` no longer falls into the generic TIFF still path (regression for `still.rs:43`).
- `raw.*` commands: undo / redo, `enabled()`, serde round trip in a saved project; UI test by
  automation id.
- Synthetic 4K sequence playback with a drop count, `#[ignore]` perf test (like `perf_*` tests).

## 7. Suggested lane split and order

1. **Lane 1 (`crates/dng`, L0):** scaffold, layer entry, TIFF/IFD parser with caps, uncompressed
   unpack, test writer, hostile tests. Then LJ92 decoder + encoder. Independent of everything else.
   Land the scaffold and public types first in one small commit so other lanes can compile.
2. **Lane 2 (colour + develop CPU, L0/L1):** camera-matrix / illuminant interpolation, Bradford
   adaptation, camera to DWG chain ending in `ColorOp::matrix`, CPU black / white / WB / bilinear and
   edge-directed demosaic, delta E tests. Needs only the `dng` types (agree a `RawFrame` struct:
   CFA pattern, black / white levels, neutral, matrices), so it can start from a stub.
3. **Lane 3 (GPU, L3):** WGSL demosaic and normalisation with parity tests. After Lane 2's CPU
   reference exists.
4. **Lane 4 (clip model, L1-L5):** sniff / extension routing, folder import with tag rate, missing
   frames, sidecar WAV, `raw.*` commands, project serde, Inspector, docs page. Starts after Lane 1's
   reader API is stable; the UI part after the develop settings struct is agreed.
5. **Last:** 4K synthetic playback / proxy check, `cargo xtask ci`, tag `raw-1`.

Lanes 1 and 2 can run in parallel; 3 and 4 follow. All of it waits for the owner's go.
