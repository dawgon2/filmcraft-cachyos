# PREP-H1: audit for H264-PLAN H1 (interlaced H.264 on the hardware path), with H2 / H3 in view

Written 2026-10-14 by the Dell lane (task §6, FORK-PLAN §1a.72), before any code. H264-PLAN.md
asks for interlaced H.264 (PAFF and MBAFF, e.g. AVCHD 1080i) on VA-API, bit-exact against ffmpeg
(as an oracle only) on AMD and Intel, played with zero drops.

## 1. What exists today

| Where | What | Interlaced today |
|---|---|---|
| `crates/h264` (software, 8.9k lines) | parsers (`params`, `slice`), CAVLC / CABAC, intra / inter, deblocking, the DPB (`dpb.rs`: marking 8.2.5, list init 8.2.4, output C.4) | refused: `Sps::check_supported` rejects `!frame_mbs_only` ("interlaced (field / MBAFF) coding"); `decoder.rs` also rejects `field_pic` slices |
| `crates/platform/src/vaapi/h264.rs` (568 lines, safe) | the VA-API front: uses the software parsers **and the software DPB** so both decoders make the same decisions; fills VA picture / matrix / slice buffers for an `Accel` (libva on Linux, a recorder in tests) | refused: calls the same `check_supported`, and returns "field pictures" for `field_pic`; it already sets the VA sequence flags `FRAME_MBS_ONLY` / `MB_ADAPTIVE_FRAME_FIELD` and the picture flag `FIELD_PIC` from the headers |
| `crates/platform/src/vaapi/mod.rs` | session, surfaces, the in-process decoder; Y1's zero-copy `Accel` | — |
| the hybrid / fallback chain | VA-API → software replay → the system ffmpeg | interlaced files reach ffmpeg today (hardware.md's table) |
| tests | `crates/h264/tests/conformance.rs` (bit-exact vs ffmpeg on libx264 fixtures, skipped without ffmpeg), `robustness.rs`, `perf.rs`; `vaapi/tests.rs` with the recording `Accel` | no interlaced fixtures |

Hardware (vainfo, H264-PLAN): AMD VCN and Intel UHD 750 / 770 decode H.264 CBP / Main / High,
8-bit 4:2:0, **including interlaced** (field and MBAFF are part of Main / High; the drivers take them
through the same VA H.264 entry point with the field flags).

## 2. What H1 needs

1. **Let interlaced streams through the hardware front only.** `check_supported` is shared; split
   it so the VA front accepts `frame_mbs_only = 0` (field and MBAFF) while the software decoder still
   refuses until H2.2 (`Sps::check_hw_supported`, or a flag), so nothing silently mis-decodes.
2. **The DPB learns fields** (`crates/h264/src/dpb.rs`, shared by both decoders): a frame store
   holds a top and / or bottom field; per-field reference marking (sliding window and MMCO on fields,
   8.2.5.3–8.2.5.4 field cases), long-term per field, field POCs (top / bottom from the POC types),
   the field reference list initialisation (8.2.4.2.2 / 8.2.4.2.5: alternating parity from frame
   lists) and modification (8.2.4.3 with picNumF / longTermPicNum for fields), second-field pairing
   (same frame_num, opposite parity, first field not an IDR pair mismatch), output of complete frames
   (and unpaired single fields), frame-num gaps with fields. MBAFF pictures are frames for the DPB.
3. **The VA front for PAFF:** a field picture is decoded into the frame's surface: `VAPictureH264`
   flags `TOP_FIELD` / `BOTTOM_FIELD` for the current picture and every reference (frame refs used by
   a field picture are entered as the right field), `CurrPic.TopFieldOrderCnt` /
   `BottomFieldOrderCnt` per field, `field_pic_flag` / `bottom_field_flag` in the slice and picture
   parameters, the second field reusing the first field's surface, `RefPicList` entries per field.
4. **The VA front for MBAFF:** frame pictures with `MB_ADAPTIVE_FRAME_FIELD`; the hardware handles
   the macroblock-pair decoding; the front only has to pass the flags and not refuse.
5. **Tests:** the recording `Accel` checks the submitted buffers (flags, POCs, lists, surfaces) on
   synthetic headers without hardware; real-hardware bit-exactness on the Dell's UHD 770 against
   ffmpeg's software decode of the same files; AMD owed on the PC at acceptance.

Deinterlacing stays the display / render path's job (the decoded frame is the woven pair).

## 3. Fixtures (generated on the Dell, never committed)

- **MBAFF:** libx264 encodes it (`ffmpeg … -c:v libx264 -flags +ildct+ilme -x264-params
  interlaced=1:tff=1` at 1920 × 1080 / 29.97i, plus bottom-field-first and CAVLC variants).
- **PAFF:** libx264 cannot write field pictures, and neither can the VA-API encoders on these
  GPUs. The usual source is the ITU-T H.264.1 conformance suite (field and PAFF streams); that is a
  new download, so it waits for the owner (OWNER-QUESTIONS Q31). Until then PAFF is covered by the
  recording-`Accel` unit tests on synthetic headers and by any PAFF file the owner has (AVCHD
  camcorders write PAFF or MBAFF depending on the model).

## 4. Step map

| Step | What | Where |
|---|---|---|
| H1.1 | split the support check: the VA front accepts interlaced SPSs, software still refuses (clear error) | `crates/h264/src/params.rs`, `crates/platform/src/vaapi/h264.rs` (one line) |
| H1.2 | DPB fields: frame stores with field halves, field POCs, field marking and MMCO, field reference list init / modification, pairing, output; unit tests on synthetic field sequences (8.2.4 / 8.2.5 examples) | `crates/h264/src/dpb.rs` (+ `slice.rs` helpers) |
| H1.3 | VA front PAFF: field flags, per-field POCs, second field into the first's surface, field reference entries; recording-`Accel` tests | `crates/platform/src/vaapi/h264.rs` (**Opus review**; no new `unsafe`) |
| H1.4 | VA front MBAFF (flags, no refusal) + MBAFF fixtures from libx264; bit-exact on the Dell's UHD 770 vs ffmpeg | platform tests (Dell), fixtures in the target dir |
| H1.5 | PAFF on hardware: bit-exact on real PAFF streams (Q31), zero-drop playback check, guide (hardware.md decode table), "H1 ready for acceptance"; AMD owed on the PC | tests, docs |

H2 (software: High 10 / 4:2:2, interlaced in software, threads) and H3 (ffmpeg-free confirmation)
follow H264-PLAN; H1.2's field DPB is what H2.2 reuses.

## 5. Risks

- **`crates/platform` is shared with the main runner** (task §6): H1 edits only
  `vaapi/h264.rs` (and its tests); check `git log origin/hevc-vaapi -- crates/platform` and the main
  checkout's status before each edit. Every platform diff gets a fresh-context Opus review; with the
  weekly usage above pace (no subagents), platform steps wait for a run where a review is allowed,
  while H1.1 / H1.2 (crates/h264) go first.
- **Field DPB correctness** is the hard part (the spec's field cases are intricate); unit tests
  per clause, then bit-exactness on hardware.
- **Driver variance:** AMD and Intel may differ in which field cases they accept; AMD is owed on
  the PC.
- **PAFF fixtures** need Q31.
- **Never crash:** hostile field sequences (unpaired fields, parity flips, frame_num jumps) must
  error or conceal, never panic; caps stay as today.
