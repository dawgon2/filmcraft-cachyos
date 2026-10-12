# EXPORT-PLAN.md: Phase 6E, 6.2K export above 24 fps with the AMD + Intel engines

Written 2026-10-11 (session 8). This is the "6.2K optimisation phase" of FORK-PLAN §1a.16; the owner
approved it on 2026-10-11 and placed it **after Phase 6 and Y1** (order: FORK-PLAN §1a.6,
G0 ✓ → G1 ✓ → U0 ✓ → G2 → G2b → 6 → Y1 → 6E → U0.2 → Y3 → C1 → C2 → 7 (+ Y4) → D → 8 → Y2 → H1 → H2 → H3). All FORK-PLAN rules apply. Analysis by a Fable subagent, from session 7's
measurements (FORK-PLAN §7) and `vainfo` on 2026-10-11.

## Why export stops at ~19 fps today (measured, synthetic)

The RX 6700 XT has one VCN; decode and encode run strictly one after the other on it:
6.2K decode 24.4 ms + 6.2K Main10 encode 30.7 ms = 55.1 ms/frame → 18.1 fps (export measured
18.2–18.9). Rendering isn't the limit (GPU busy 10–20 %). Intel UHD 750 alone: 6.2K Main10 encode
72.9 ms/frame (13.7 fps) via `VAEntrypointEncSlice`; AMD-decode → read-back → Intel-encode was a
dead end (9.9 fps; ≈ 28 ms to move each 78 MB frame).

## Plan: two independent pipelines

Each GPU decodes, composites / grades and encodes **its own chunks** of the timeline; the chunks are
joined at the end. No frames cross between devices, so no new cross-device `unsafe`.
Estimate: AMD 18.7 fps + Intel 7–13 fps ≈ **26–31 fps** (26–28 with Intel `EncSlice`, 29–31 if
Intel's low-power `EncSliceLP` / VDEnc is much faster). Graded 4K exports go beyond real time too.

### 6E.1 Measure first (ffmpeg only, machine idle: no builds or benchmarks running)
1. Intel 6.2K Main10 encode, `hevc_vaapi -low_power 1` vs `0` on renderD128 (the biggest unknown).
2. Intel 6.2K decode-only ceiling (`-hwaccel vaapi … -f null`) on renderD128.
3. Intel decode + encode together → the strategy's estimate confirmed or revised.
4. AMD encoder quality / speed level (`-compression_level`) at 6.2K.
5. Encode two chunks per device, concatenate the Annex B streams, check with ffprobe and mpv
   (parameter-set compatibility, CVS concatenation).
6. **4:2:2 sources (added 2026-10-12 after Phase 6):** export from 6.2K 4:2:2 originals, per
   stage (Intel decode + hand-off, render, encode). First data point: the xm5 4:2:2 clip →
   ProRes 422 HQ through `filmcraft-cli export` (CPU render, default settings) ran at ~2.2 fps
   (60 frames in 26.7 s, synthetic, single run). Measure the same with GPU Rendering on and to
   H.265 Main 10 (zero-copy path), and with 6E.4's 4:2:2 HEVC master preset once it exists.
Record all numbers (synthetic) in `docs/fork-benchmarks.md` "Phase 6E". If 1–3 show Intel adds
less than ~4 fps, stop, record it, and ship only 6E.2's single-VCN gains.

### 6E.2 Cheap wins (stack with everything else)
- `EncSliceLP` support in `crates/platform/src/va_enc.rs` (today it uses `EncSlice` only, :122 /
  :134), chosen per device when `vainfo` offers it.
- AMD encoder speed / quality preset exposed in export settings ("Speed / Balanced / Quality").

### 6E.3 Chunk scheduler and second pipeline
- Split the export into closed-GOP chunks of 5–10 s, preferring timeline cuts as boundaries; a
  work queue both devices pull from (so the faster one takes more), chunk-start decode from the
  source's previous IRAP.
- A second media stack + compositor on the Intel device (grading already passes parity there;
  `FILMCRAFT_VAAPI_DEVICE` / `FILMCRAFT_VAAPI_ENCODE_DEVICE` already select render nodes).
- Joiner: the export already starts chunks at IDRs and writes its own SPS / PPS / slice headers
  (`hevc_export.rs`, `va_enc.rs`); use identical parameter sets on both devices if both drivers
  accept them, otherwise in-band parameter sets per IDR (`hev1`).
- **Consistency:** the same fixed QP (CQP) or long-chunk VBR on both encoders so there are no
  visible quality / grain steps at joins; a test renders a static shot across a join and compares
  the two sides (PSNR / SSIM within a set threshold).
- **Never crash:** an Intel error, hang or device loss re-queues its chunk on AMD and continues
  AMD-only, with the reason in the export log and `perf.stats`; Settings choice
  "Export engines: Auto / AMD only / AMD + Intel".

### 6E.4 4:2:2 10-bit HEVC master on the Intel encoder (owner, 2026-10-12, FORK-PLAN §1a.30)
- An export preset "HEVC 4:2:2 10-bit master" on the Intel encoder (`VAProfileHEVCMain422_10`,
  `VAEntrypointEncSlice` on `renderD128`; AMD has no 4:2:2 encode): `va_enc.rs` accepts
  `chroma_format_idc` 2, writes the range-extension SPS fields, takes 4:2:2 input surfaces
  (Y210 / P210, whichever the driver accepts; the compositor writes 4:2:2 Y'CbCr into them).
- Reuses 6.1b's 4:2:2 display selection and surface formats. Shown only when a device offers the
  profile; never the default (YouTube, proxies and the render cache stay 4:2:0).
- Test: decode the export with the built-in 4:2:2 decoder; PSNR against
  the Program frame above a set threshold; ffprobe reports `yuv422p10le`; plays in mpv.

### Accept
`xm5-export-6k` ≥ 24 fps on the AMD + Intel path (target 26+), output passes ffprobe, plays in mpv,
colour matches the Program monitor, no visible step at chunk joins (the static-shot test), pulling
the Intel device mid-export finishes on AMD alone; `xm5-export-4k` and graded 4K measured; full
`xm5` export suite (synthetic). `cargo xtask ci`, tag `phase-6e`, build per §1a.9.

## Estimates
6E.1 2–3 h · 6E.2 3–6 h · 6E.3 15–30 h · 6E.4 3–6 h · total ≈ 23–46 agent hours.
