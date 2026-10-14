# PREP-H2: the built-in H.264 software decoder covers everything in scope (audit)

Dell lane, 2026-10-14 (task §6, H264-PLAN Phase H2). Written while H1.3 / H1.4 wait for their
review (see HANDOFF), so H2 can start the moment H1 is accepted. Scope from H264-PLAN H2:
High 10 and High 4:2:2 (8 / 10-bit 4:2:2, Intra and Long-GOP), interlaced (PAFF and MBAFF) in
software, multithreading good enough for 4K 10-bit 4:2:2 on the i5-11600K, never crash on hostile
streams. Acceptance: bit-exact against the conformance vectors for every in-scope profile; 4K
10-bit 4:2:2 Long-GOP and Intra fixtures (XAVC S / S-I-like) play, fps recorded. Tag `h264-2`.

## 1. What exists (`crates/h264`, ~9 k lines, pure Rust, L0, builds for wasm32)

- **Parsing is already complete for the H2 formats.** `Sps::parse` reads `chroma_format_idc`
  (0–3), `separate_colour_plane`, bit depths up to 14, the lossless bypass flag and 8 / 12
  scaling lists; `SliceHeader::parse` reads every field-coding element; `PocState` gives field
  POCs for all three types (H1.2c, `Poc::picture`). The parsers are shared with the VA-API front.
- **The field-aware DPB is done (H1.2):** per-field marks and POCs, `Dpb::store` with
  `FieldPocs`, second-field pairing (`is_second_field`), field sliding window and MMCOs,
  `build_field_ref_lists` (P / B field init, parity alternation, field modification). The software
  decoder still uses only the frame API (`store_picture`, `build_ref_lists`).
- **Frame-level threading exists** (`threads` feature, rayon): headers / DPB on the calling
  thread, each picture's macroblocks as a job publishing finished rows into its `Frame`; later
  pictures wait per row on their references (`Frame::publish` / `row` / `wait_complete`). There
  is no slice-level parallelism within a picture.
- **Everything below the slice header is 8-bit 4:2:0 frame coding:**
  - Sample storage: `Planes` / `Frame` / `FrameRow` hold `u8` planes with 4:2:0 chroma
    (`chroma_window`, `copy_cropped`, `set_plane_allocator` all `u8`); `Picture` (the public
    output) is `u8` y / u / v; `codecs/src/video.rs` wraps it as `PixelData::Yuv8` `C420`.
  - Prediction: `inter.rs` (`clip1` clamps to 255; chroma MC assumes half-resolution in both
    directions: `slicedec.rs` divides the luma position by 2 vertically too), `intra.rs` (8-bit
    DC defaults of 128, chroma prediction for 8×8 blocks only).
  - Residual: `transform.rs` (4×4 / 8×8 / Intra16×16 DC / **2×2** chroma DC only), level scale
    tables without `QpBdOffset`; QP wrap `(qp + delta + 52) % 52` without the bit-depth offset;
    chroma QP mapping without `QpBdOffsetC`.
  - Entropy: CAVLC's `residual_block` already has the `nC = -2` (4:2:2 chroma DC) code table, but
    the macroblock layer only asks for 4:2:0 chroma DC (4 coefficients, `nC = -1`); CABAC chroma
    DC `ctxBlockCat` 3 is sized for 4 coefficients, chroma AC blocks counted as 4 per component
    (4:2:2 has 8).
  - I_PCM reads 8-bit samples and 4:2:0 chroma counts.
  - Deblocking: `u8` samples, α / β / tC0 without bit-depth scaling, chroma edges for 4:2:0
    (4:2:2 chroma has horizontal edges at 2, 6, … in 4-row units: the transform-edge set differs).
  - No field / MBAFF macroblock addressing, neighbour derivation (6.4.12.2), field MC (reference
    field selection, chroma offset for opposite parity), MBAFF CABAC context (`mb_field_decoding_flag`),
    or field deblocking (mixed edges).
- **Refusals** (`Sps::check_supported`, `decoder.rs`): interlaced, `chroma_array_type != 1`,
  bit depth != 8, lossless bypass, FMO / ASO, SP / SI, field slices. These stay the single place
  that says what software can't do; each H2 step narrows them.

## 2. What H2 needs, by area

| Area | Change | Size |
|---|---|---|
| Sample type | Make the picture path generic over the sample (`u8` / `u16`) or store everything as `u16` above 8-bit: `Planes`, `Frame`, `FrameRow`, windows, `Picture` (+ `bits`). Recommendation: a `Sample` trait with `u8` / `u16` impls and `max = (1 << bits) - 1`, monomorphised per bit depth so 8-bit speed doesn't regress. | large, mechanical |
| High 10 | `QpBdOffsetY / C` in QP wrap, level scale, chroma QP table; `clip1` to the bit depth; intra defaults `1 << (bits - 1)`; I_PCM `bits`-bit samples; deblocking α / β / tC0 scaled by `1 << (bits - 8)`; weighted prediction offsets scaled (High 10: `o << (bits - 8)`). | medium |
| 4:2:2 | Chroma 8×16 per MB: intra chroma prediction (DC per 4×4 of an 8×16 block, plane with 4:2:2 `yCF`), chroma DC **2×4** transform + its own QP (`QPc + 3`), CAVLC `nC = -2` chroma DC (table exists), CABAC chroma DC 8 coefficients (cat 3) and 8 AC blocks, chroma MC at full vertical resolution, deblocking chroma edge set, 4:2:2 `copy_cropped` / output (`Chroma::C422`). | medium–large |
| Interlaced | Reuse H1.2's DPB (switch the software path to `Dpb::store` / `build_field_ref_lists`); field pictures = a field view of the frame buffer (stride ×2, offset); field MC (reference fields, chroma vertical offset ±¼ for opposite parity); MBAFF: MB-pair addressing, `mb_field_decoding_flag` (inferred when skipped), neighbour derivation 6.4.12.2, CABAC ctxIdxInc for the field flag, field/frame MB pairs in MC and intra, mixed-edge deblocking (field MBs filter as fields; the left edge between frame / field pairs). | large (the hardest part) |
| Threads | Frame-level exists; add slice-level within a picture (slices are independent apart from deblocking across slice edges, which runs after) and check the per-row waits for field pictures (a field references rows of both fields). Measure fps (synthetic) for 4K 10-bit 4:2:2. | medium |
| Output | `codecs/src/video.rs`: `Yuv16 { bits }` above 8-bit, `C422` for 4:2:2, interlaced frames woven (field order from the stream for the existing field options). | small |
| Never crash | Fuzz-style rounds per new path (bit flips, truncation, hostile `chroma_format_idc` / bit depth / field flags), allocation caps from the SPS (4:2:2 16-bit planes are 4× the 8-bit 4:2:0 size: cap by `MaxDpbMbs`). | ongoing |

## 3. Fixtures and oracles

- **libx264 on the Dell** writes everything in scope except PAFF: `-profile:v high10` /
  `high422` with `yuv420p10le`, `yuv422p`, `yuv422p10le`; `-x264-params interlaced=1` (MBAFF)
  and `intra-refresh` / `keyint=1` for Intra-style streams (XAVC S-I-like: `-x264-params
  keyint=1:...`). Generated into the target dir, never committed (the existing
  `filmcraft_testkit::fixtures` pattern).
- **ffmpeg's decoder** is the oracle (raw `yuv422p10le` etc. through `-f rawvideo`), as H1.4 does.
- **Conformance vectors** (H264-PLAN's acceptance; also PAFF): the ITU-T H.264.1 suite, which
  **Q31** already asks about (option a). Until the owner answers, H2 is checked against x264
  fixtures and ffmpeg, and the conformance part of acceptance stays owed.

## 4. Step map (each step: tests incl. never-crash rounds → guide → gates on the Dell → review → merge)

| Step | What |
|---|---|
| H2.0 | this audit |
| H2.1 | sample-type groundwork: generic `Planes` / `Frame` / windows / `Picture` with `bits`; 8-bit output unchanged (bench before / after on the Dell: no 8-bit slowdown beyond noise) |
| H2.2 | High 10 4:2:0: QpBdOffset, clipping, intra / PCM / deblocking / weights by bit depth; refusal narrowed; x264 `high10` fixtures bit-exact vs ffmpeg; `Yuv16` output |
| H2.3 | 4:2:2 intra (8 and 10-bit): chroma 8×16 intra prediction, 2×4 chroma DC, CAVLC `nC = -2`, CABAC chroma cat 3 / 4 for 4:2:2, deblocking chroma edges, output `C422`; `keyint=1` fixtures |
| H2.4 | 4:2:2 inter: chroma MC at full vertical resolution, weighted prediction; Long-GOP `high422` fixtures |
| H2.5 | PAFF in software: the decoder on H1.2's field DPB / field lists; field picture views; field MC; field deblocking; synthetic + Q31 streams |
| H2.6 | MBAFF in software: MB pairs, neighbour derivation, field flag inference / CABAC, MBAFF MC / intra / deblocking; x264 `interlaced=1` fixtures (tff / bff, CAVLC / CABAC) |
| H2.7 | slice-level threads + field-aware row waits; fps on the Dell (synthetic 4K 10-bit 4:2:2 Long-GOP / Intra), owed on the i5-11600K at acceptance |
| H2.8 | acceptance: conformance vectors (if Q31 a), fixture matrix, fuzz rounds, guide (hardware.md / formats), "H2 ready for acceptance" |

H2.1–H2.4 and H2.7 can merge with self-reviews when weekly usage is above pace (crates/h264 is the
lane's own crate); H2.5 / H2.6 change decoding of every MBAFF / PAFF macroblock and get a
fresh-context Opus review when allowed.

## 5. Risks

- **8-bit speed:** the generic sample path must not slow the 8-bit decoder that all progressive
  H.264 without VA-API uses (bench per step; monomorphise rather than branch per sample).
- **MBAFF complexity** (neighbours, contexts, deblocking) is where bugs hide; x264 MBAFF fixtures
  exercise it heavily, ffmpeg gives exact answers, and H1.4's hardware decode is a second oracle on
  the Dell.
- **Memory:** 16-bit 4:2:2 4K frames are ~25 MB each; DPB of 16 + threads in flight must stay
  within the existing caps (cap by level, never by stream-supplied counts alone).
- **wasm32:** the crate builds for wasm; keep `threads` optional and no new dependencies.
- **Conformance download** waits for Q31; without it acceptance relies on x264 + ffmpeg.

## 6. H2.5 design: PAFF (field pictures) in software (written 2026-10-14 after H2.4)

Status at writing: the field DPB, field reference lists, field POCs and second-field pairing exist
(H1.2: `Dpb::store` with `FieldPocs`, `is_second_field`, `build_field_ref_lists` with `Parity`,
`Poc::picture`) and run for the VA-API front; the software decoder still refuses `field_pic`.
Conformance field streams (AVCv1 `CVPA*`, `CVFI*`, `CVSE*`, `CAPAMA*`, `CVMP*`, `CAPA*`, FRExt field
streams) are on the Dell under `~/fork-data/h264-conformance/extracted/` (Q31 a), run by
`tests/conformance_vectors.rs`.

**Storage and publication.** A field picture's MB row r covers frame lines 32r + parity + 2k, i.e.
half of frame MB rows 2r and 2r + 1, and the second field of a frame references the first field of
the same frame while the frame is incomplete. So readiness has to be per field:
- `Frame` keeps its frame rows for frame-coded pictures; a field-coded frame gets two field halves,
  each with its own `OnceLock` rows of 16 *field* lines (8 / 16 chroma) published by that field's
  job. One `Frame` type with `rows: Rows` = `Frame(Box<[OnceLock<FrameRow>]>)` or
  `Fields([Box<[OnceLock<FrameRow>]>; 2])` keeps the DPB entry type unchanged.
- Views for readers (all windows clamp at the edge of the *view*):
  - field view of a field-coded frame: its own rows; field view of a frame-coded frame: field line
    l = frame line 2l + parity (waits on frame row (2l + parity) >> 4);
  - frame view of a field-coded frame (a frame picture referencing a complementary field pair):
    frame line y = field (y & 1) line y >> 1.
  `luma_window` / `chroma_window` gain a `view: View { Frame, Field(Parity) }` argument (const
  generic for the frame path so progressive speed is unchanged).
- Output (`copy_cropped`) interleaves the two fields; the crop rectangle is in frame lines.

**Decoding a field.** `PicState` for a field: planes of field size (height / 2), `mbs` of
`mb_w × mb_h / 2`; published rows go to the field half. The slice decoder is otherwise unchanged
(a field is decoded exactly like a progressive picture of half height) except:
- inter prediction picks the reference *field* `RefPicList[i]` = (frame, parity) from
  `build_field_ref_lists`; luma MC reads the field view; 4:2:0 chroma: the vertical chroma vector
  is offset by −2 (current bottom, reference top) / +2 (current top, reference bottom) in 1/8
  units (Table 8-9 / 8-10); 4:2:2 has no offset;
- direct prediction: colocated picture selection for fields (8.4.1.2.1: field of the first entry
  of RefPicList1, or the frame / complementary field pair with the vertical mv scaling rules
  between frame and field colocated), temporal direct / implicit weights with the field POCs
  (`Poc::picture`) of the current field and the reference fields;
- `ref_id` per 8×8 block identifies (frame, parity) so deblocking and colocated lookups compare
  reference *fields*;
- deblocking runs inside the field (it is a picture of its own; no mixed edges in PAFF).

**DPB / output.** `Decoder` starts a picture per field: first field → new `Frame` with field
halves; second field (`is_second_field` asked before `idr()` / gap fill) → the same `Frame`, other
parity. `store` with `FieldPocs`; output when the frame (or an unpaired field) leaves the DPB; an
unpaired field is output with the missing field filled from the present one (line doubling) so
readers never block.

**Steps.** H2.5a field-aware `Frame` rows + views + interleaved output (unit tests on synthetic
planes); H2.5b decoder accepts field pictures with intra-only fields (conformance `CVFI*` /
`CVSE*`-style I fields); H2.5c field inter (reference fields, chroma offset, weights); H2.5d
direct / colocated across frame / field; H2.5e frame pictures referencing field pairs. Each with
the conformance field streams as the oracle; fresh-context Opus review before merge (§4).

## 7. H2.6 design: MBAFF in software (written 2026-10-14 after H2.5)

Status at writing: PAFF is built (§6 "As built", branch `dell-lane-h2-5`); MBAFF frames
(`mb_adaptive_frame_field` with `field_pic` 0) are refused per slice. The oracle is the 42
conformance archives the harness reports as "MBAFF frames": intra-only professional streams
(`PPH10I6/7_Panasonic_A`, `PPH422I6/7_Panasonic_A`, `Hi422FREXT17/19_SONY_A`), CAVLC (`CVMA1_*`,
`CVMANL*`, `CVMAQP*`, `CVMAPAQP3`, `cavlc_mot_mbaff0_full_B`, `HVLCMFF0`), CABAC (`CAMA*`,
`CANLMA*`, `CAMACI3` constrained intra, `CAMASL3` slices, `camp/cabac_mot_mbaff*`, `cama*_vtc`),
mixed PAFF + MBAFF (`CAPAMA3_Sand_F`, `CVMAPAQP3`, `FRExt2_Panasonic_C`), 8×8 / High
(`FREXT01/02_JVC`, `HCAMFF1`, `HCHP3`, `HPCAMAPALQ`, `brcm_freh5`, `FRExt4`, `MR9_BT_B`).

**Addressing.** In an MBAFF frame macroblocks come in vertical pairs: CurrMbAddr = 2 × pair +
bottom, pair = mb_y / 2 × W + mb_x; `first_mb_in_slice` counts pairs (× 2). A pair is either two
frame MBs (rows 32 r .. 32 r + 15 and + 16 .. + 31 of the pair) or two field MBs (the top MB
takes the pair's even lines, the bottom MB its odd lines; chroma likewise). Reconstruction keeps
`PicState.planes` in frame layout; a field MB reads / writes with a line step of 2 from its
pair's first line + bottom. `mb_field_decoding_flag` is read for the top MB of a pair (or the
bottom one when the top is skipped), and inferred, when both are skipped, from the left pair,
else the pair above, else frame (7.4.4); CABAC ctxIdxInc 70–72 from the left / above pairs.

**Neighbours (6.4.12.2, Table 6-4).** The one big new piece: A / B / C / D of a 4×4 / 8×8 block
or a whole MB depend on whether the current and the neighbouring pairs are frame or field (and,
for a frame MB above, whether the current MB is the top or bottom one). One function maps
(current MB, frame / field, luma position) → (neighbour MB address, its position), used by intra
prediction (also the sample neighbours of intra 4×4 / 8×8 / 16×16 / chroma, with their own
field / frame line mapping), CAVLC nC, CABAC ctxIdxInc (mb_skip, mb_type, cbp, cbf, intra chroma
mode, mvd and ref_idx with the frame ↔ field scaling of 9.3.3.1.1.6–7: refIdx > 1 vs > 0 and
absMvdComp × 2 / ÷ 2), QP prediction (previous MB in decoding order, i.e. pair order), and the
motion vector predictor (8.4.1.3: neighbour mv[1] × 2 or ÷ 2 and refIdx × 2 or ÷ 2 across
frame / field). Unavailable / slice rules apply per pair.

**Field MBs.** Field scans and the field CABAC significance contexts (as in PAFF); reference
lists RefPicList for a field MB are the frame list expanded to fields, each frame giving its
same-parity field first then the opposite one (8.4.2.1: refIdx >> 1 selects the frame, refIdx & 1
the parity relative to the current MB); explicit weights use refIdx >> 1, implicit weights and
temporal direct use the field POCs of the current MB's parity and the reference fields; MC reads
the reference field view at half the vertical position, with the 4:2:0 chroma offset of Table
8-10 by the current MB's parity; deblocking ids per reference field.

**Direct (8.4.1.2).** Colocated per Table 8-6 with mb_field_decoding_flag: frame MB → colocated
frame (or, over a complementary field pair, the field nearer in POC); field MB → the field of
RefPicList1[0] with the current MB's parity; mbAddrCol5 / 6 / 7 and yM with
vertMvScale (Frm_To_Fld / Fld_To_Frm, mvCol[1] ÷ 2 / × 2, refIdxL0 << 1 / >> 1 per 8.4.1.2.3);
spatial direct takes refIdx / mv neighbours through the MBAFF mapping and colZero from the
colocated field / frame. Colocated *frames* published by MBAFF pictures need per-MB field flags
in `FrameRow` (a field MB's mv / ref are stored as such, and readers convert).

**Deblocking (8.7).** Per MB in pair order after the whole pair is reconstructed; the left edge
between a frame and a field pair (mixedModeEdgeFlag) filters each half of the left pair's lines
against the current MB's lines as field rows; the top edge of a frame MB under a field pair (and
a field MB pair's top MB over a frame pair) is filtered in field mode (top MB of a frame pair
over a field pair: two field edges with bS from the matching field rows); bS 4 only on vertical
edges or frame-frame horizontal edges, else 3 for intra; mixedModeEdgeFlag 1 makes bS 1 where
motion would decide. The filter then runs with field line steps.

**Steps** (branch on top of H2.5; each with the conformance streams as the oracle and the
never-crash rounds; fresh-context Opus review before merge):
- **H2.6a** pair addressing, `mb_field_decoding_flag` (read / infer, both entropy coders),
  field-MB sample placement, the Table 6-4 neighbour mapping for intra (samples, modes, nC,
  CABAC contexts, QP), field scans / contexts for field MBs, deblocking still off for MBAFF in
  the test (or frame-only pairs): oracle the intra-only `PPH*I6/7` / `Hi422FREXT17/19` streams
  once H2.6d lands, and `CAMA1_*` I pictures earlier via the trace example.
- **H2.6b** inter: field-MB reference lists and weights, mv / refIdx prediction with frame ↔
  field scaling, mvd / ref_idx contexts, MC from fields, P_Skip in field MBs.
- **H2.6c** direct (spatial / temporal) incl. colocated MBAFF frames (field flags in `FrameRow`).
- **H2.6d** MBAFF deblocking.
- **H2.6e** accept MBAFF sequences by default (drop the hook's role), perf A/B for progressive,
  never-crash rounds over damaged MBAFF streams, guide rows.
