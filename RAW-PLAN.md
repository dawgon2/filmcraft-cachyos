# RAW-PLAN.md: CinemaDNG and ProRes RAW

Written 2026-10-12 (live session, with the owner). Order: FORK-PLAN §1a.6 (R2 after G4, before D;
R1 is on the maybe list, §1a.81). Every FORK-PLAN rule applies (never crash, `Tick` time, everything is a
command, layering, `cargo xtask ci` per phase, a build and a `cachyos` publish per phase).
BRAW, R3D and ARRIRAW are **not** in this plan (closed SDKs only; FORK-PLAN §1a.35 f).

## Phase R1: CinemaDNG, pure Rust (≈ 15–25 h; upstream-contributable)

**Not scheduled: on the maybe list (owner, 2026-10-15, FORK-PLAN §1a.81).** Upstream now has its own clean-room
DNG / CinemaDNG decoder (`crates/dng`, #1065). Reconsider only on the owner's word, after checking upstream's crate
against his files. R2 stays in the plan.

Clean-room from Adobe's published DNG specification (1.4+) and the TIFF / EP specs only; never
from GPL decoders (dcraw, LibRaw, RawTherapee, darktable).
1. **Reader** (new L0 crate, e.g. `crates/dng`, depending only on `filmcraft-bitstream`): TIFF /
   DNG IFD parsing with hostile-input caps (IFD count, chain depth, offsets, dimensions, tile
   counts), uncompressed and lossless-JPEG (LJ92) tiles / strips, 8–16-bit, linearization table,
   black / white levels, `CFAPattern`, `DefaultCrop`, `AsShotNeutral`, `ColorMatrix` /
   `ForwardMatrix` / `CalibrationIlluminant`, `BaselineExposure`, timecode and frame rate tags.
2. **RAW develop stage** (shared with R2, its own module, no ffmpeg anywhere): black / white
   normalisation → white balance → demosaic (bilinear for scrubbing, a higher-quality
   edge-directed method for playback / export, both on the GPU with a CPU reference) → camera
   matrix to XYZ → **DaVinci Wide Gamut / Intermediate** working space (FORK-PLAN §1a.14), so
   CinemaDNG grades like every other clip.
3. **Clip model:** a numbered DNG folder imports as one clip (frame rate from tags or the
   sequence preset, missing frames reported, a sidecar WAV linked by name / timecode); RAW
   controls on the clip (white balance / tint, exposure, highlight recovery, demosaic quality) as
   commands (`raw.*`) and an Inspector section.

**Accept:** fixtures generated from our own synthetic images (written by a test DNG writer in
the crate, so the expected values are known; never commit media) decode exactly before
demosaic; the develop stage matches a CPU reference within a stated ΔE; hostile DNGs (fuzz-style)
never crash; a 4K CinemaDNG sequence plays (through proxies / render cache if needed;
synthetic). Tag `raw-1`.

## Phase R2: ProRes RAW through the system ffmpeg (≈ 8–15 h; **fork-only**)

The system ffmpeg has a `prores_raw` decoder (checked 2026-10-12). FFmpeg only ever runs as an
external process (FORK-PLAN §1). **This phase must be trivially separable from anything sent
upstream** (owner, 2026-10-12; FORK-PLAN §1a.36):
1. **All of it in one fork-only crate**, `crates/fork-prores-raw`: probe (`ffprobe` JSON:
   dimensions, bayer pattern, frame rate, black / white level, white balance and colour metadata
   if exposed), spawn ffmpeg, read raw Bayer frames from the pipe (verify the output pixel format
   at the start of R2; if ffmpeg only offers developed RGB, take `rgb48` and skip the develop
   stage's demosaic), and hand them to R1's develop stage. No other crate imports anything
   ffmpeg-specific from it.
2. **One hook, behind a feature.** Shared code gets a generic, upstream-acceptable extension
   point (e.g. an `ExternalDecoder` registration list in the media layer, useful on its own); the
   fork registers the ProRes RAW decoder in **exactly one line** in the app crate, under
   `#[cfg(feature = "fork-ffmpeg")]` with a `// FORK-ONLY` comment. The `fork-ffmpeg` feature
   is on by default in the fork's app crate only.
3. **Separate commits.** R2's code lands in its own commits prefixed `FORK-ONLY:` (never mixed
   with R1 or generic changes); the generic extension point, if needed, lands first in its own
   normal commit so it can be contributed.
4. **Listed in [`docs/fork/FORK-ONLY.md`](docs/fork/FORK-ONLY.md)** (the crate, the hook line,
   the feature).
5. **Proven separable:** the gate runs `cargo xtask ci` a second time with the feature off
   (`--no-default-features` on the app crate, or the equivalent) and with `crates/fork-prores-raw`
   deleted from a scratch copy, both green. Without ffmpeg (or without `prores_raw` in it) the
   clip shows a clear "needs ffmpeg with ProRes RAW" message, never a crash.

**Accept:** a ProRes RAW sample (generated ourselves if ffmpeg can encode one, else a
permissively licensed public sample, downloaded only with the owner's yes, never committed)
decodes and grades like CinemaDNG; the separability checks in 5 pass. Tag `raw-2`.
