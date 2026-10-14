# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–75 incl. §1a.6 order; §5 phases; only the last ~80 lines of §7), then
[`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md) "Phase Y3" (next) and [`PREP-Y3.md`](PREP-Y3.md), then
[`FORK-ONLY.md`](FORK-ONLY.md), [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), this file. Other plan
files (GRADE, RESOLVE-UI, YOUTUBE, AUDIO, AI, COMPOSITOR, STORAGE, H264, RAW, DECODER) only when a
step points at them. Rewritten 2026-10-14 after the X422 acceptance (scheduled run).

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → Y1 ✓ → 6E ✓ → U0.2 ✓ → AI1 ✓ →
X422 ✓ → **Y3** → C1 → **AI3** (§1a.73–74; knowledge work on the Dell first) → C2 → AI2 → S1 → S2 → G3 → 7 (+ Y4) → Y2 → C3 → C4 → G4 → A3 → R1 →
R2 → D → 8. Small phases are accepted in groups (§1a.41). AI1 moved right after U0.2 and the
4:2:2 export cases (X422) right after AI1 by the owner 2026-10-13 (§1a.69). **A2 and then
H1 → H2 → H3 run on the Dell lane (owner 2026-10-14, §1a.72); the main runner skips them.**
**Owner, live 2026-10-14 ~12:55 (§1a.75):** AI3 trains on **all of editing**, the Compositor (C1) included, not only grading; only features built by the tune, later phases add pages / pairs / eval questions and the tune is refreshed. **Q30 (a) accepted** (A2's numbers stand). **Q31 (a) yes:** the Dell lane may download the ITU-T H.264.1 conformance streams (Dell only, never committed) for H1.5 PAFF and H2. Q29 ok.
**A1 (Dell lane, §1a.60) accepted by the owner 2026-10-14 ~01:20 (§1a.71)**; its tag `audio-1` is on `f8c3e788` with `x422` (2026-10-14).
**L1 / L2 part 2 accepted by the owner (live, 2026-10-14 ~01:10; was ready 2026-10-13 22:55, Dell lane; scheduled task `filmcraft-dell-lane`,
lock `.dell-lane.lock`):** the last two steps merged: **L1.5** `d0c68898` (upstream #260 off-thread
file dialogs cherry-picked, Q23; dialogs parented to the window, each kind starts in its last
folder, close waits for an open dialog) and **L2.6** `7f6d79dc` (Settings ▸ Playback ▸ Graphics
device by PCI ids, VA-API tries the chosen GPU's render node first, `graphics.listDevices`).
Checklist on the Dell (CachyOS, UHD 770, 2026-10-13): fmt ✓, clippy ✓ (workspace minus
`filmcraft-speech`, see below), engine / app / ui-egui lib ✓, every `--test ui` module one per
process ✓ (86 / 86), `native_dialogs_ui` ✓, platform lib ✓, layers ✓, wasm ✓; both steps had a
fresh-context Opus review, findings fixed. Earlier part 2 items: Flatpak build ✓ (`18d523e4`,
feature-checked in a virtual KWin; SDK kept on the Dell, Q17), L1.8 / L2.4 / L2.10 and the
text-scale hint (`2e67f732`). **Tags `linux-1b` / `linux-2b` are on `f8c3e788` with `x422` (2026-10-14).**
**Owed at acceptance:** on the PC: Graphics device Auto vs a chosen GPU on the RX 6700 XT + UHD 750
(window, monitors and decode on the same GPU; `graphics.listDevices` `inUse`), Program monitor 1:1
at fractional scale; on a real desktop: a file dialog stacked over the window (KDE portal, Wayland)
and opening in the last folder, GlobalShortcuts dialog, card insert → Copy → Eject, KDE Connect send /
receive / Multimedia control, Plasma font 11 pt → 110 %, an export notification popping.
**Seen on the Dell, not the lane's:** `cargo clippy --workspace --all-targets` fails in
`crates/speech/examples/candle_ops_bench.rs` since `a2cd09d4` (AI1.2): the example uses candle without
`required-features = ["whisper"]` in `crates/speech/Cargo.toml`.
**A1 accepted by the owner (live, 2026-10-14 ~01:20, §1a.71; was ready 00:55, Dell lane; AUDIO-PLAN Phase A1, §1a.59–60).** Steps merged:
A1.1 `a7b753c2` (upstream #840 stream recovery, ported to cpal 0.18), A1.3a `39f4439a` (failed reopen
retried within the budget; restart keeps the running format), A1.2 / A1.2b (play-ahead: per-block
panic recovery, `mixerStopped` → status + wall clock in both monitors, adaptive lead ≥ 4 device
buffers +50 ms per underrun episode ≤ 0.5 s, checked mix access, start-failure status), A1.3b
`a9077873` (mix rebuilt at the rate the output opened), A1.4a `c0a81628` (clock counts frames heard:
cpal timestamps), A1.4b `cdd0dc69` (sync probe `tests/ui/av_sync_ui.rs`: 0 frames off after 100
seeks, at 1× and 2×), A1.5 `c008e455` (soak harness `tools/fork/a1-audio-soak.sh`, docs/testing.md
§6). **Acceptance checklist (Dell, CachyOS, PipeWire 1.6.9):** 30-minute soak, 20 tracks with
overlapping clips, 78 seeks, 48 2× shuttles: **0 underruns** ✓, **drift ≤ 1 frame** (worst 1, 87 383
readings) ✓; **simulated device loss** (null sink removed / restored, card profile off / on, both
automated) recovered in 0.50–0.56 s each without a restart ✓, profile restored ✓; regression tests
per report: #336 overlapping-clip cut-outs and #617 Linux stutter → soak + `play_ahead` stall /
lead tests; #602 no audio → start-failure status, rate agreement, #840 recovery tests; #605 (Xonar
DX output) → sample-format conversion and running-format restart tests (the card itself isn't
here); #410 ("buggy audio", no repro steps) → covered only by the soak. **Owed on the PC at
acceptance:** the soak once on the PC's own output (`tools/fork/a1-audio-soak.sh 300`; it uses a null
sink and the first card's profile), a listen at 1× / 2× for pitch after a default-sink change, and
the real output latency (`perf.stats` `playback.audio`). Tag `audio-1`: on `f8c3e788` with `x422`.
**A2 ready for acceptance (Dell lane, 2026-10-14 ~08:30; §1a.72; scheduled task `filmcraft-dell-lane`,
lock `.dell-lane.lock`; worktree `.claude/worktrees/dell-lane`, branch `dell-lane` = hevc-vaapi, nothing
unmerged; tag `audio-2` rides with the next phase group).** Audit `docs/fork/PREP-A2.md` (step map §5). Merged: A2.1 `26d2e7c5` (new L2 crate
`crates/voice`: mixed-radix FFT, DFN3 STFT / ERB / features / deep filter), A2.2 `93e0eb8b` (checked
checkpoint loader, feature `dfn`), A2.3 `e9e95f2e` (the network as plain-Rust streaming kernels +
`Denoiser`; parity with DeepFilterNet's own `deep-filter` 68.2 dB; real-time factor 0.071 on one
Dell core), A2.4 `e66e056d` (the `voice_isolate` effect: audio-dsp external-effect registry +
DeNoise fallback, `filmcraft_voice::effect::install`, project def, one render arm, guide
`docs/guide/audio.md`), A2.5a `12721bed` (`audio.voiceIsolate.status / download / cancelDownload /
installFile / remove`, pinned package, lazy loading; app + CLI feature `voice-isolation` on by
default; real download checked end to end on the Dell), A2.5b `28edd384` (Settings ▸ Audio ▸ Voice
isolation row, UI test in both workflows), A2.7 `3e635592` (test set + SI-SDR harness
`voice_isolation_eval`; **on synthetic formant speech the model fails the gate (2.1 dB on clean
speech): real speech needed; **Q26 answered (b) 2026-10-14 ~06:20: LibriSpeech test-clean approved**), A2.6 `3cb31fd0` (live in the mixer: one network step per track on the
channels' mean, 0.2 s pre-roll; Dell soak 300 s: 1 Voice Isolation track 0 underruns, 4 tracks 21
in the first 2× shuttle's first 0.2 s; no offline cache needed). DeepFilterNet3 weights (pinned commit 82b0c7ad, SHA-256 in PREP-A2 §3) are on the Dell
under `~/.local/share/filmcraft-fork/models/deepfilternet3/`; the oracle is
`~/fork-oracles/target/release/deep-filter` (built from source with `RUSTC_BOOTSTRAP=1`, `time`
pinned to 0.3.36). Dell tests: `FILMCRAFT_DFN3_CKPT=<ckpt> FILMCRAFT_DFN_ORACLE=<oracle> cargo test
-p filmcraft-voice --features dfn`. A2.8a `5be92eab` (Clean Up Voice `audio.cleanupVoice` + Essential Sound ▸ Repair ▸ Voice Isolation
slot + Loudness ▸ Limit True Peak / Ceiling; guide audio.md "Cleaning up a voice by hand" = the written
manual process; loudness half of the gate passes in tests: ±0.5 LU of −14 LUFS, ≤ −1 dBTP on the
rendered clip). A2.8b `66654f4d` (the gate on LibriSpeech, 50 utterances on the Dell under
`~/.local/share/filmcraft-fork/voice-test/librispeech/`: **one-click passes all 15 cases** against
the manual process; **Voice Isolation vs DeNoise misses two of its bars** (+0.8 dB at 10 dB SNR, bar
3; clean speech 17.0 dB, bar 20), OWNER-QUESTIONS Q30 FYI, choice (a) accept with the numbers).
A2.8c `f5eafc67` (Clean Up Voice as a background job: progress, stop, applied in one undo step on poll; `wait: true` for agents). A2.9 `f9f290ac` (per-take review: recordings tagged as takes of their record point,
`audio.voiceover.takes` / `pickTake`, Takes with waveforms in Essential Sound, processing moves
with the take). **Acceptance checklist (AUDIO-PLAN A2):** port = original 68.2 dB, real-time
factor 0.071 / channel on the Dell, live in the mixer (soak: 0 underruns with 1 track) ✓;
Voice Isolation vs DeNoise on LibriSpeech: wins all 15 cases, non-steady margin +7.1 / +4.0 dB
at 0 / 5 dB SNR ✓ but **+0.8 dB at 10 dB (bar 3) and clean speech 17.0 dB (bar 20) ✗**
(OWNER-QUESTIONS Q30, choice (a) accept; (b) re-measure on your own quiet-room recordings);
one-click gate vs the written manual process: **all 15 cases pass**, −14.0 … −14.2 LUFS,
−1.5 dBTP ✓ (no case limited to "Suggest settings"); commands `audio.voiceIsolate.*`,
`audio.cleanupVoice`, `audio.voiceover.takes` / `pickTake`, all on the control channel ✓; guide
`docs/guide/audio.md` written ✓. Limits: 44.1 kHz sequences use the DeNoise fallback (resampling
around the model is a later step); the one-click path without the model (Reduce Noise) wasn't
gated. Evals: `FILMCRAFT_EVAL_SPEECH=~/.local/share/filmcraft-fork/voice-test/librispeech
FILMCRAFT_DFN3_CKPT=~/.local/share/filmcraft-fork/models/deepfilternet3/ckpt/DeepFilterNet3/checkpoints/model_120.ckpt.best
cargo test --release -p filmcraft-engine --features voice-isolation <voice_isolation_eval|cleanup_gate>
-- --ignored --nocapture` (gate ~17 min on the Dell). Reviews were self-reviews (weekly usage
above pace, no subagents): **a fresh-context review of A2.1–A2.3 and A2.8–A2.9 is owed** before
the tag. **Owed on the PC at acceptance:** live voice isolation during playback on the i5-11600K.
**AI3-PREP done (Dell lane, 2026-10-14 ~13:45; §1a.74, task §5b; marker `.dell-ai3prep-complete`)**,
for the main runner's AI3: (1) **compact headers** on all 27+ guide pages (spec docs/guide/README.md
"Section headers"; parser `coach_ask::parse_header`, `Section::header`; check
`section_headers_are_valid_unique_and_point_at_real_things` covers every page but README.md: **every
new guide section needs a header**); (2) **feature pages**: text, mixing, graphics, timeline, effects,
media, shortcuts, edit-menu, trimming, export-queue, file-menu, essential-sound, plus getting-started
X-M5 import, voice-over recording, caption tracks (~580 of ~700 engine commands named by family);
(3) **150 practice notes** in docs/guide/practice/ (10 pages; each mapped to FilmCraft with gaps said as
"not yet"; own words, no source text, plan/ai3-sources.md is maintainer-local); (4) **advanced eval**
docs/fork/coach-eval/advanced.toml (63 questions with source header ids, must / must-not facts;
`advanced_eval_set_is_well_formed`) and **885 draft training pairs**
docs/fork/coach-eval/training-pairs.jsonl (generator tools/fork/ai3-training-pairs.py, TRAINING-PAIRS.md,
`training_pairs_are_grounded`; needs a human review sample before tuning). **Keyword-index note:**
the 116-question colour eval sits at its floor (82), so the general and practice pages are in
`coach_ask::NOT_INDEXED`; AI3's retrieval should index them (and the headers). No model downloads
were made. **H1 in progress on the Dell lane** (task §6, H264-PLAN; same worktree / branch `dell-lane`):
audit **docs/fork/PREP-H1.md** (H1.0 `b731932c`, step map §4); **H1.1** `ec7ce8ed` (`Sps::check_coding_tools`
for hardware fronts, `check_supported` still refuses interlaced for software) and **H1.2** (field-aware DPB in
`crates/h264/src/dpb.rs`: H1.2a `7158790c` per-field marks / POCs, `Dpb::store` with `FieldPocs`,
`is_second_field` pairing (ask it before `idr()` / gap fill), field sliding window and MMCOs; H1.2b `c61fcb00`
`build_field_ref_lists` (to_ref gets entry + `Parity`); H1.2c `e9294236` `Poc::picture`) merged, self-reviewed
(weekly usage above pace). **H1.3 + H1.4 are built but NOT merged**: local branch **`dell-lane-h1-3`**
(`52cdb73b` H1.3, `1c6c8243` H1.4, on top of `585b4bd6`; also on the Dell's scratch repo), waiting only for the
**fresh-context Opus review** the platform rule requires (weekly usage above pace: no subagents this run).
H1.3: `vaapi/mod.rs` `geometry()` takes interlaced H.264 (HEVC still refused) and `h264_setup` calls
`check_coding_tools`; `vaapi/h264.rs`: `check_coding_tools`, no `field_pic` refusal, a second field
(`Dpb::is_second_field`) decodes into its first field's surface without IDR / gap handling, CurrPic and
reference entries carry `VA_PICTURE_H264_TOP/BOTTOM_FIELD` (new consts in `ffi.rs`, no new `unsafe`) with only
their own fields' order counts, field slices use `build_field_ref_lists`, the frame list goes by
`DpbEntry::is_reference`, storage through `Dpb::store` with both field POCs (equivalent for frames); recorder
tests: synthetic PAFF stream (pairing, flags, lists by 8.2.4.2.5), MBAFF, hostile field sequences + 200 bit-flip
rounds; the Recorder now allows a second field's target to be listed as its first field alone. H1.4:
`tests/vaapi.rs::interlaced_h264_matches_ffmpeg`: three libx264 MBAFF fixtures (tff / bff, B-pyramid, 4 refs,
weightp, CAVLC) **bit-exact vs ffmpeg's decode on the Dell's UHD 770**; interlaced dropped from the decline test;
guide `hardware.md` row. Gates on the Dell green (fmt, clippy, h264 + platform tests incl. hardware, layers,
wasm, engine coach tests). **Review points:** the second-field surface reuse vs the zero-copy accelerator's
held frames (a first field is never read out before its pair, so it can't be held); a mid-stream hardware
failure on an interlaced stream can't continue in software (the software decoder refuses interlaced until H2),
so it errors as before H1; the ffmpeg-pipe path (`vaapi_pipe.rs`) still declines interlaced. **Next run with
subagent room:** review `git diff 585b4bd6 dell-lane-h1-3` (Opus, fresh context), fix, rebase onto
hevc-vaapi, re-run the platform tests on the Dell, fast-forward merge; then H1.5 (PAFF real files: **Q31**
open; zero-drop playback check; "H1 ready for acceptance"). AMD checks (MBAFF / PAFF on VCN) owed on the PC at
acceptance.
**H2 started in parallel** (H1 waits only for its review; sequencing choice logged here): audit
**docs/fork/PREP-H2.md** (H2.0 `e29d8814`: parsing and the field DPB are done, frame threading exists, everything
below the slice header is 8-bit 4:2:0 frame coding; step map H2.1–H2.8; conformance vectors ride on **Q31** a).
**H2.1 (sample-type groundwork) done** (2026-10-14 ~11:45, self-reviewed: weekly usage above pace, PREP-H2 §4 allows
it for H2.1): `crates/h264/src/sample.rs` (`Sample` for `u8` / `u16`, `clip1`, `chroma_row`, `code_max`: constant
255 for `u8` so 8-bit kernels clip at compile time); H2.1a `1e7c0d65` inter, H2.1b `a0ae378c` intra, H2.1c
`9ceb574c` residual kernels generic; H2.1d `833feb88` storage (`Planes` / `FrameRow` / `Frame` over `Sample` with
`bit_depth`, windows / cropping for `u16`) and deblocking (alpha / beta / tC0 × (1 << (BitDepth − 8)), clip to max,
packed `u8` vertical edges kept, plain `u16` lanes; 16-bit lanes safe to 12 bits); H2.1e `3f744cc9` slice decoding
(`PicState` / `Scratch` / `SliceDecoder` / `RefPic` generic); H2.1f `cf22ac31` decoder dispatch: a private
`Core<S>` per sample type behind `Decoder`; a slice whose SPS needs > 8 bits switches to the `u16` core (old core's
pictures flushed first; parameter sets, settings, error slot and stats carry over). Hidden test hook
`Decoder::set_wide_samples` runs **every conformance fixture (32 on the Dell) and the synth streams through the
`u16` path too: bit-exact vs ffmpeg**, plus `sample_path_switch_at_idr`. **8-bit speed unchanged** (perf_1080p
1 thread 99.5–100.7 fps vs 99.8–100.2 before, release, Dell; edge filters same speed / checksum). > 8-bit streams
are still refused (`check_coding_tools`); the `u16` core's `Picture` is reduced to 8 bits until H2.2.
**H2.2 (High 10 4:2:0) done** (2026-10-14 ~11:55, self-reviewed): H2.2a `b011e9a4` QpBdOffset (slice QP /
mb_qp_delta ranges and wrap, negative QP_Y / QP_C as `i8` in `MbState`, dequant at QP′); H2.2b `0195ae20` I_PCM at
BitDepth bits (`read_pcm`, CAVLC + CABAC); H2.2c `6d9e6f1d` `Sps::check_software_supported` (9–10-bit, equal luma /
chroma depth; `check_supported` / `check_coding_tools` stay 8-bit for the VA-API fronts), explicit WP offsets ×
(1 << (BitDepth − 8)), `Picture::bit_depth` + `Picture::high` (16-bit planes), codecs → `PixelData::Yuv16 { bits }`;
guide `hardware.md` row. **Six x264 `high10` fixtures (intra, CAVLC B, weightp + fade, qp 1, 4 slices + deblock
−2/−2, odd crop) bit-exact vs ffmpeg's yuv420p10le decode** (1 / 3 / all threads), MP4 High 10 seek test
(`crates/codecs`) sample-exact; 8-bit speed unchanged (99.7 fps). Owed / later: a `u16` plane allocator (output
planes use `Vec::with_capacity`, not the frame pool), 10-bit decode speed (not measured).
**H2.3 (High 4:2:2 intra, 8 / 10-bit) done** (2026-10-14 ~12:30, self-reviewed): H2.3a `ad61e1ed` 4:2:2 geometry
(`Planes::cheight`, `Frame::c422`, 16 chroma lines per published MB row, windows / cropping, `Picture::chroma_format_idc`,
codecs `C422`); H2.3b `f28cafa7` 8×16 chroma intra prediction (DC rules per 4×4, plane yCF = 4); H2.3c `8af81317`
chroma residual (2×4 DC: CAVLC nC = −2, CABAC cat 3 Min(i/2, 2), [[c0,c2],[c1,c5],[c3,c6],[c4,c7]] transform at
QP′c + 3; 8 AC blocks with 2×4 neighbours; 4:2:2 I_PCM); H2.3d `4e9d88df` deblocking (vertical chroma edges over 16
lines, horizontal at chroma rows 0/4/8/12 from luma edges 0..3, odd edges' bS derived under the 8×8 transform too),
4:2:2 allowed in `check_software_supported`, **4:2:2 P / B slices refused per slice until H2.4**; guide
`hardware.md` row. **Six x264 High 4:2:2 Intra fixtures (8 / 10-bit, CABAC / CAVLC, no 8x8dct, qp 1, 4 slices +
deblock −2/−2 + odd crop) bit-exact vs ffmpeg** (1 / 3 / all threads + the u16 path). deblock −2/−2 + odd crop) bit-exact vs ffmpeg** (1 / 3 / all threads + the u16 path).
**H2.4 (High 4:2:2 inter) done** (`8a9e9321`, 2026-10-14 ~12:55, self-reviewed): chroma MC at full vertical
resolution (yIntC = yAL + (mv >> 2), yFracC = (mv & 3) << 1), 8×16 chroma predictions / weighted stores, P / B
refusal lifted; slice decoding and deblocking are now **monomorphised over the chroma format** (`SliceDecoder<S,
C422>`, `deblock_mb_planes<S, C422>`, dispatched per slice / MB) so 4:2:0 carries no 4:2:2 branches. **Five x264 High
4:2:2 Long-GOP fixtures (B-pyramid + 4 refs, CAVLC, weightp + weightb on a fade, temporal direct + odd crop, qp 1;
8 / 10-bit) bit-exact vs ffmpeg**, MP4 4:2:2 10-bit seek test (`crates/codecs`) sample-exact; guide `hardware.md`.
**8-bit 4:2:0 speed is 2.3 % below pre-H2.3** (perf_1080p, 1 thread, builds with forced code alignment
`RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5"` and their own target
dir, interleaved: 99.6 → 97.3 fps; unaligned builds vary ±1.5 % by layout). Bisect: H2.3a 0, H2.3b–d ~−1.5, H2.4
before monomorphising −2.3 (fixed). Ruled out: `MbState` / scratch sizes (probe at the old sizes: same speed),
chroma-window / add-chroma loop bounds. No `perf` on the Dell (no sudo): **profile owed at H2.7** (or on the PC).
**H2.4 never-crash rounds** `cdd9a216` (High 10 / 4:2:2 bit flips incl. parameter sets, truncation, a stream switching
bit depth and chroma format; no panics, caught or not). **H2.7a** (perf_4k_422_10): 4K 10-bit 4:2:2 **Long-GOP
43.2 fps, Intra (632 Mbit/s) 26.8 fps** with 16 threads on the Dell (1 thread: 8.5 / 3.3), fork-benchmarks.md;
**slice threads deferred (Q32, FYI)**; fps on the i5-11600K owed at acceptance.
**Q31 answered (a): ITU-T H.264.1 conformance streams may be downloaded (Dell only, never committed).**
**Conformance (H2.8a):** ITU-T H.264.1 AVCv1 + FRExt + the Hi422 / Hi10 Intra professional streams (4:4:4
skipped) **downloaded** to the Dell's `~/fork-data/h264-conformance/` (219 archives, `SHA256SUMS*`; outside git) and
extracted into `extracted/` (218 dirs; `~/fork-logs/conf-extract.sh`). Harness `tests/conformance_vectors.rs` (ignored;
`FILMCRAFT_H264_CONFORMANCE_DIR=~/fork-data/h264-conformance/extracted cargo test --release -p filmcraft-h264 --test
conformance_vectors -- --ignored --nocapture`); `f65643cb` (merged) crops references stored uncropped (CVFC1) and
accepts references that repeat pictures / mix fields per the pic_struct SEI (CVSE2 / CVSE3 / CVSEFDFT3) when both
fields of every decoded picture appear in order. **hevc-vaapi (2026-10-14 14:40): 125 pass, 92 unsupported (all
interlaced, FMO, SP/SI, monochrome), 1 skipped (alpha), 0 failed.**
**H2.5 (PAFF in software) built and green, NOT merged** — branch **`dell-lane-h2-5`** (local + the Dell's scratch
repo; on top of): H2.5a field-aware `Frame`; H2.5b intra field pictures (first /
second field, interleave after the second, unpaired field filled by copying); H2.5c inter fields (reference
fields, chroma offset, field scans and CABAC field contexts, field deblocking); H2.5d direct prediction in
fields (Table 8-6 colocated, Frm_To_Fld); H2.5e frames referencing field pairs (Fld_To_Frm), frame-coded
pictures stored with both field POCs, PAFF sequences accepted without the test hook (MBAFF ones still refused until
H2.6), guide `hardware.md` rows; PREP-H2 §6 "As built" (the design's chroma-offset sign was reversed, and
it missed field scans / contexts / deblocking). **Branch results on the Dell: fmt, workspace clippy, h264 tests
(incl. new synthetic field test + damaged-field-stream rounds), layers, wasm green; conformance 168 pass / 49
unsupported (42 MBAFF, 3 FMO, 2 SP/SI, 2 monochrome) / 1 skipped / 0 failed (+43 PAFF streams bit-exact);
8-bit progressive perf_1080p 1 thread −0.9 % (96.4 vs 97.3 fps, alignment-pinned A/B, inside layout noise).**
Engine test `ai_explain_tests::panic_error_and_timeout_fall_back` failed once under full-workspace load and passes
alone (timing; not touched here). Self-reviewed only (weekly usage above pace, no subagents).
**H2.6a–c (MBAFF intra, inter, direct) built and green, NOT merged** — branch **`dell-lane-h2-6`** (on top of
`dell-lane-h2-5`): design PREP-H2 §7 (merged); H2.6a pair addressing, mb_field_decoding_flag (read ahead /
inferred), the Table 6-4 neighbour mapping behind every neighbour lookup, field-MB reconstruction (line step 2),
MBAFF as a const generic of `SliceDecoder` (progressive perf parity 97.1 vs 97.2 fps); H2.6b field-MB references
(refIdx >> 1 frame, refIdx & 1 parity), weights, MC, motion neighbours by luma sample (C / D from the line above);
H2.6c direct with MBAFF colocated pairs (per-MB field flags + field reference ids in published rows), temporal
over field POCs. **Conformance 177 pass / 40 unsupported (33 = MBAFF with the loop filter, 3 FMO, 2 SP/SI, 2
monochrome) / 0 failed** (+9 MBAFF streams over H2.5; every loop-filter-off MBAFF stream bit-exact). Lessons: two
recalled Table 6-4 details were wrong and the conformance streams (and the Toshiba `mv_trace` MVP files) decided;
Dell-only tools `crates/h264/examples/fc_*.rs` on the scratch repo (untracked, never committed).
**Next:** H2.6d MBAFF deblocking (33 streams as the oracle), H2.6e accept MBAFF by default (hook off), guide rows.
**Next run with subagent room:** fresh-context **Opus** review of `git diff f65643cb dell-lane-h2-5` and then
`dell-lane-h2-5..dell-lane-h2-6` (decoder.rs
field picture lifecycle / unpaired fields / job threading, slicedec.rs colocated mapping, deblock.rs, picture.rs),
fix, fast-forward merge; also still owed: the H1.3 + H1.4 review (`dell-lane-h1-3`). Then **H2.6 MBAFF** (42
conformance streams as the oracle; flip `set_field_pictures` default when done) and H1.5. Dell leftovers: A/B
worktree `~/Projects/fc-base` (at `9c7f81a5`) + `~/Projects/fc-perf-*` targets (≈0.5 GB, reusable for perf A/B).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, the yt-1 group (`resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`,
  `coach-1`, `yt-1`) and the **6E group on `d866cb4c` (2026-10-13): `phase-6e`, `linux-1a`,
  `linux-2a`**, **`upstream-sync-2` on `e090c0c9` (2026-10-13, U0.2 + U0.2.3b)** and `ai-1` on
  `bcb5f713` (2026-10-14) and the **X422 group on `f8c3e788` (2026-10-14): `x422`, `audio-1`, `linux-1b`,
  `linux-2b`** and **`yt-3` on `db7c1d0d` (2026-10-14)**. Builds in `~/Apps/filmcraft-builds/`; `latest` → `yt-3`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public (§1a.33): dawgon2/filmcraft-cachyos
  branch `fork` snapshot `9abdf285` (of `yt-3`, parent `62992ebb` = `fork-x422`), tag `fork-yt-3`,
  release `fork-yt-3` (scrubbed Dell build; NOTES.md); earlier tags `fork-x422`, `fork-audio-1`,
  `fork-linux-1b`, `fork-linux-2b`.
- **U0.2 as shipped:** one VA-API stack: upstream's H.264 / HEVC front ends on the fork's
  `va::Display` with `va_accel::ZeroCopyAccel` (zero-copy, hand-off incl. scaled, read-back mode);
  fork `va_hevc.rs`, `crates/hevc/src/accel.rs` and upstream's `vaapi/va.rs` deleted. Displays are
  kept for the process's life and `filmcraft_platform::quiesce()` runs before exit (a CLI crash at
  exit, fixed with a regression test). Exports (U0.2.3b, synthetic, full length): 4K 32.5–35.1,
  4K graded 30.5–31.1 (Auto: Intel decodes, scaled hand-off, Q20), 6.2K 25.5, 6.2K graded 19.6 (Q21).
- **6E as shipped:** two-GPU HEVC export (`crates/export/src/lanes/`: chunker, scheduler,
  coordinator with the **slow-lane rule** `retire_slow_lanes`, spill files; `hev1` when the
  drivers' headers differ), Export engines setting, encoder speed preset, 4:2:2 10-bit master on
  Intel. Synthetic: 6.2K 25.8 fps dual vs 19.4 single, 4K 33.9 vs 24.6, graded 4K 24.2 vs 22.3
  (`docs/fork-benchmarks.md` "Phase 6E"). Diagnostic env vars: `FILMCRAFT_EXPORT_TRACE=1`,
  `FILMCRAFT_EXPORT_GPU_PROBE=1` (per-frame GPU time per device), `FILMCRAFT_BENCH_ENGINES=dual|single`,
  `FILMCRAFT_BENCH_KEEP=<dir>`.
- **Schema v18** (C1 comps), prefs version 3. New user-visible strings need es + zh-cn rows. New guide pages go
  in `GUIDE_PAGES` (`crates/engine/src/coach_ask.rs`) and their `{ui:}` ids must be known to the
  `coach_ui` walk; dynamic automation ids (`format!`) must be listed in
  `crates/ui-egui/src/coach_ids.rs::DYNAMIC_ID_PATTERNS` (a test fails otherwise). Coach eval floor 94.
- **RS1** feature `fork-resolve-default` is on in ci: new UI tests that assume Premiere set
  `prefs.workflow = Premiere`.
- **Desktop integration is off under `--control`** unless `FILMCRAFT_DESKTOP_INTEGRATION=1`
  (MPRIS, taskbar progress, notifications, inhibit).
- **Public-snapshot hygiene:** `publish-snapshot.sh` refuses the owner's user name, e-mail name or a
  LAN address it can't rewrite (it rewrites the home path, not a `/media/…` or `/run/media/…` mount
  path, and not a name inside prose): use `user` in test fixtures and never write the owner's user
  name in docs.
- **Owed (not blocking):**
  - *U0.2:* Opus review of U0.2.3b d (per-source hand-off scale, `handoff_scales`, decode-only lane)
    and U0.2.4 / .5 (display layer, `Busy` / `quiesce`) when the usage mode allows (STEADY / LEAN);
    `teardown_ui` fails under the full parallel ci run (8 references left after 3 s) but passes alone
    (known flake since 6E: make it deterministic); hand-off scale only for MP4 / MOV sources (MKV
    etc. ignore `set_handoff_scale`); U0.2.3 owed (ii) re-grey test needs failure injection, (v)
    `va_dual_export::two_gpus_keep_the_colours_of_one` with the Intel device forced.
  - *6E:* Intel lane barely helps graded exports (retired); `Rgba16Float` effect stage on
    integrated GPUs would let it; a hung lane can still be the slow-lane comparison until the
    watchdog; (`perf.stats` lanes now report `retired` apart from `failed`, 2026-10-13);
    4:2:2 master is CPU-converted (2.7 fps; GPU → Y210 is worth a step); ~7 GB RSS for a 6-frame
    6.2K dual export in `hevc_presets`.
  - *L1 / L2 part 1, on the owner's desktop* (visual / System Settings / sudo; NOTES "Please
    check"): KWin look + icon, Program monitor 1:1 at 125 / 150 %, accent / light-dark / Animation
    speed live, window size restore, 10 min without lock, Dolphin thumbnails / double-click /
    paste, nested non-KDE compositor, `makepkg -si`. Verified here: native Wayland, MPRIS + Pause,
    portal Inhibit on play, taskbar progress + notification, journal priorities, single instance,
    X11 run.
  - *Y1:* idle zero-copy slots not trimmed / counted; AMD open-GOP-seek parity covers the I
    picture only; the fork HEVC pool may share the iHD context-lifetime hazard; Dell run of
    `tests/vaapi.rs` (U0.2.3 has a Dell run anyway).
  - *Earlier:* 6.7b TNR keyframe markers / label width; RS3 cramped Color page, Markers vs Index,
    missing panels, Fairlight strip, home-screen screenshot compare; Phase 6 frame-server proxy
    hint, `cache.claimFolder`, decode-to-screen fps with hardware 4:2:2, software 4:2:2 memory cap
    (ask first), TNR / cache / proxy 6.2K bench cases.
- **DC1 done (2026-10-13):** `fork/dell-ci/` installed on the Dell (user timer, every 2 min, only
  while the owner is logged in: Linger=no). Use: `git push ssh://dell~/Projects/filmcraft-main
  HEAD:refs/heads/dell-ci/<kind>-<id>` (kinds `ci`, `test-<crate>`, `intel` = `fork/dell-ci/intel.list`,
  `soak-<name>` = `fork/dell-ci/soaks/<name>.sh`), keep working, later
  `ssh dell 'cat ~/filmcraft-ci/results/<kind>-<id>.txt'` (3 lines; `.log` next to it, read only on FAIL).
  Self-tests: soak-selftest-fail → FAIL with its line, test-filmcraft-time → PASS. Gentle mode per
  §1a.56 (games: GPU jobs wait; MPRIS playing: 8 build jobs, soaks wait; < 3 GB free: no new job).
  `intel-1` (U0.2.3 owed iii): PASS in 3m47s.
- Cloud: CLOUD-QUEUE has C1-PREP, AI1-PREP, R1-PREP (docs only). Merge `origin/cloud-work` first
  if it has new commits.

## Exact next step: C1 (COMPOSITOR-PLAN, simplified node compositor), step 1

**yt-3 accepted 2026-10-14** (Y3.1 GPU keys, Y3.2 attach to track / tracked blur, Y3.3 screen replacement,
Y3.4 user WGSL shader effects + 4 examples, Y3.5 look packs; all synthetic). The Y3 working notes are
archived at the end of `docs/fork/PROGRESS-LOG-ARCHIVE.md` ("HANDOFF notes for Y3"). Owed from Y3 (not
blocking): a fresh-context review of the Y3 diffs (`git log 14705d16^..yt-3`, the GPU user-effect pass in
`crates/gpu/src/fx.rs` and the device-lost hook especially) when the mode allows (STEADY or better); the
X422 2b fusion review; the MKV 4K-from-6.2K Intel export check (cloud MKV-HANDOFF); Color Key Edge Thin;
a look at the Effects ▸ User folder on screen (not screenshotted).

**C1 plan:** `docs/fork/DESIGN-C1.md` (2026-10-12) is the design; §7 has the 13 steps, lanes and the
critical path 1 → 2 → 3 → 5 → 7 → 10 → 13. **Done:** step 1 (ADR **0007** `docs/adr/0007-node-graph.md`,
provisional) and step 2 (`crates/project/src/dag.rs`: `validate`, `Order { steps }`, `Order::last_use`,
`DagError`, caps; 12 tests) and step 3a (`crates/project/src/comp.rs`: types, `params(kind)`,
`sanitize`, `impl DagNode`; the Text node holds the `graphic_text` `EffectInstance`, not a
`LayerSpec`) and step 3b (`ItemKind::Comp(Arc<Comp>)` everywhere; `nest_cycle` follows comps; format
sanitizes comps on load; a comp clip renders transparent until step 5). Step 4 done (schema v18, fuzz test). Step 5a done (`crates/render/src/comp/mod.rs`: `compile`,
`Program` / `Pass`, liveness slots, `execute_cpu`, `render_cpu`, `CompSources`, gradient; 12 golden /
scheduling tests). Step 5b done (comp clips render on the CPU path and through the frame plan as CPU layers;
`comp/project.rs` `ProjectSources`; `comp_clip_tests.rs`). Step 6 done (Text node via
`graphic_clip::render_graphic_layers`, comp B shape test). Step 11 started 2026-10-14: `docs/guide/compositor.md` (partial: basics, making, convert, the two recipes, speed; in `GUIDE_PAGES` + `NOT_INDEXED`; the coach UI walk opens the Compositor) — add the inspector / viewer / Fusion page sections with 10b. **Next** (any order; critical path is 7, a large GPU step best done in STEADY or better with a Fable/Opus builder):
**step 7: 7a done 2026-10-14** (`PlanLayer.comp`, not a `PlanStep::Comp`: keeps `FramePlan::Layers` so GPU export
stays on the GPU; `render::comp_plan_layer`; the GPU still runs the program on the CPU in `render_plan`'s fallback).
**Step 13 progress 2026-10-14:** bench comp-1080 / comp-2160 half real time; 30-min comp soak passed (0 drops / errors,
RSS plateau); architecture §5 + ADR 0007 as-built written. 10b done 2026-10-14 (Color node fully editable; Mask shapes dragged on the Program monitor,
no Bézier handles yet). Still for acceptance: screenshots in both workflows (looked at 2026-10-14, both fine; redo at the tag), an Opus review of `gpu/src/comp.rs` (LEAN+), `cargo xtask ci`, tag `comp-1`,
build / publish. **7d done 2026-10-14 (comps real time: comp-1080 / comp-2160 half 192 / 0 drops, synthetic; Item Media In sources still
CPU images: give them `media_in_layers` too when a bench shows it).** Was: bench (`bench --section playback --only comp`, scenarios `comp-1080` /
`comp-2160`) shows 75 % / 91 % drops: ~400 ms CPU per frame because Media In nodes render their source on the CPU
(`comp::project::ProjectSources::media_in` → CPU image → `Pass::Image`, uploaded every frame). Plan: a new
`Pass::Layers { dst, layers: Vec<PlanLayer> }` that `compile` emits for Media In when the sources can give plan
layers (`CompSources::media_in_layers`, default None → today's image path): `ProjectSources` builds them with the
frame-plan code (`push_item` for the Media In item / the track's items at comp time, scaled to the comp size);
`execute_cpu` runs `plan::execute_layers` into a blank image; the GPU `slot_draw`s the layers (decoded / hardware
frames, zero-copy). Then re-bench. **7b + 7c done 2026-10-14** (`crates/gpu/src/comp.rs` `run_comp`, `comp_tests.rs`, AMD + Intel parity; owed: an Opus
review of `comp.rs` in LEAN or better; `backdrop` / `post` reallocate when comp and frame sizes alternate: pool per
size if the step-13 bench shows it). The design as built (kept for the review): in `crates/gpu` (new `comp.rs`), `GpuCompositor::run_comp(&Program) -> Arc<VideoFrame>`:
slots = `Option<(Texture, View, (w, h))>` per `Tex` + scratch, created like `Self::target` with ACCUM_FORMAT and
`COPY_SRC | COPY_DST`; each slot has a stable placeholder frame (`RgbaF32(Arc::new(Vec::new()))`, program size) and is
**registered in `self.uploads`** under that frame's key (`Uploaded { views: [slot view, dummy ×3], kind: 1,
code_scale 1, texel_scale 1, alpha_scale 0, chroma (w, h), last_used: clock }`) right before every use (the LRU
evicts at 24; never let `upload` see a placeholder unregistered: its empty pixels would be uploaded). Passes:
Image = `render_plan(Layers [PlanLayer::new(image frame, IDENTITY, 1, Normal)], clear)` into the slot (swap
`self.accum` with the slot as transitions do, lib.rs `transition_inputs`); Fill = a 1×1 `rgba_f32` frame scaled by
`Affine::scale(w, h)`; Copy = `copy_texture_to_texture`; Draw / Over = one slot layer with `xform` (Draw: clear
true; Over: clear false, `mode`, `opacity`; hq → `with_quality`); src == dst or Fx = draw into scratch and swap
slot handles; Fx = the slot layer with `fx: Some(LayerFx { size: program size, decimation: 1, ops })`, identity.
Then in `render_plan`, before the accumulator is taken, replace each comp layer's frame with `run_comp`'s out
frame (drop its CPU fallback branch). VRAM: `textures × w × h × 8 B` over `fx::FX_VRAM_BUDGET` → keep the CPU
fallback (log once). Sizes alternate (comp vs frame) so `backdrop` / `post` reallocate: pool them per size if it
shows in the bench. **7c:** parity tests in `crates/gpu/src/tests.rs` per pass kind + comps A / B vs `execute_cpu`
(≤ 1e-3 rel) on AMD and Intel (`MESA_VK_DEVICE_SELECT`, see memory `machine-gpu-render-nodes`).
(Was: **step 7** GPU `PlanStep::Comp` (Draw / Fx / Over on pooled canvases, VRAM check → CPU
fallback, AMD + Intel parity; a Fable/Opus builder per §1a.44 when the mode allows, else do it here);
step 9 done (`widgets/node_canvas.rs`, generic canvas + 5 tests); step 10a done
(`panels/comp.rs`, `PanelKind::Compositor`, `tests/ui/comp_ui.rs` both workflows); **step 10b**: node
inspector (schema rows done 2026-10-14: `panels/comp.rs::inspector` with the shared
`transition_controls::param_widget`, ids `comp.param.<key>`; still owed: Color node = grade tab widgets, Text rows done (layer params, text field), Mask = mask list done 2026-10-14 (`panels/comp.rs::mask_rows`: Add ellipse / rectangle, Inverted, Delete → `comp.setMasks`; ids `comp.mask.{ellipse,rect}`, `comp.mask<k>.{inverted,delete}`), feather / opacity / expansion rows `comp.mask<k>.<param>` done; still owed: viewer shape editor (Color needs `grade.rs::palette` refactored to target a comp node instead of a clip: do it in STEADY or better); the per-row stopwatch `comp.param.<key>.stopwatch` is done), `Target::CompNode` viewer (render side done 2026-10-14: `filmcraft_render::render_comp_node(project, seq, clip, node, t, opts, sources)` = the clip's comp with `output = node`, at comp time; `Target::CompNode(seq, clip, node)` in `frames.rs`, `panels/comp.rs::viewer_target` used by the Program monitor while the Compositor is open and a non-output node is viewed (`canvas.viewed`: double-click / node menu View / UI command `comp.view {node?}`) done 2026-10-14, not screenshotted; the "Show: Media Out / <node>" combo `comp.viewer` done too; Compositing workspace + Fusion page between Edit and Color done 2026-10-14: `dock.rs` "Compositing", `pages.rs` `fusion`, Fusion toolbar; not screenshotted)
(done 2026-10-14: Open in Compositor = `comp.open`, and the timeline node-count badge `timeline.rs::comp_badge`,
not yet looked at on screen); step 8a done (`engine/src/comp.rs`: 15 core `comp.*` commands + `comp_tests.rs`); step 8b part done (`comp.newClip`, `comp.setMasks`, and `comp.convertToCompClip` 2026-10-14: stacked clips
only, same start / end, strict refusals per DESIGN-C1 §9; same-track sequences and animated Motion / Opacity are
refused for now, relax in C4; the step-11 guide page must cover it); `comp.open` done 2026-10-14 (UI command in `panels/comp.rs::route`, Clip ▸ Compositor ▸ Open in Compositor, by `clip` / `comp` / selection; an opened comp holds until the selection changes, `opened_with` in the panel state); **rest of 8b**: none (`comp.view` done as a UI command). `tools/comp-accept.json` done 2026-10-14 (the `command_batch` args building comp A on
V2 and comp B on V3 over V1 footage via Media In `track: 0`; node ids are fixed: 1 = Media Out, then 2, 3, … in
add order; engine test `the_acceptance_batch_builds_comps_a_and_b` runs it step by step as the MCP server does). Static-subgraph cache (DESIGN-C1 §3.2) done 2026-10-14:
`comp::compile_cached` (used by `render_cpu`; step 7's GPU path should call it too, its cached pictures arrive as
`Pass::Image`), `static_tops`, 256 MB LRU `StaticCache`; Color nodes and Media In are never static, any keyframe
makes a node dynamic. **C1 item 7 (AI help for Fusion)** part done 2026-10-14: `comp.explain` (`engine/src/comp_explain.rs`, template
sentences per node; no UI button yet); 8 compositing eval questions added and `compositor.md` indexed (eval 93/124, floor 93; 6 compositing misses
left); lesson `first-comp` (Text / Transform / Merge / Media Out, walk test
`coach_first_comp_end_to_end`); and `key-comp` (Media In / Key / Solid / Merge / Color,
`coach_key_comp_end_to_end`); assistant plan §4a written: **item 7 done**. **Step 12 / C1 item 6 (Coach fixes)** part done 2026-10-14: what-for, x-vs-y, order and not-yet routes + typo stand-in in
`coach_ask` (eval 91/116, floor 91); heading-idf weight measured useless (dead end); and the why-first summary (`Answer::explain`):
**C1 item 6 done** (`docs/fork/coach-eval/DIAGNOSIS.md` §4). In
MINIMAL mode do the integrator steps yourself, no lanes; lanes A–D / T only when the mode allows
builders. The user-effect registry (`filmcraft_project::effect::UserEffectSpec` / `user_effect_module`)
is the shape a later "Shader" comp node can reuse (ADR 0006 consequences).

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
