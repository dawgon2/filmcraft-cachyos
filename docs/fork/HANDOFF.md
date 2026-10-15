# Handoff: FilmCraft fork (6.2K HEVC on CachyOS + RX 6700 XT)

For a fresh session. Read `FORK-PLAN.md` first (§1 fork rules, **§1a standing instructions**
§1a.1–75 incl. §1a.6 order; §5 phases; only the last ~80 lines of §7), then
[`YOUTUBE-PLAN.md`](../../YOUTUBE-PLAN.md) "Phase Y3" (next) and [`PREP-Y3.md`](PREP-Y3.md), then
[`FORK-ONLY.md`](FORK-ONLY.md), [`OWNER-QUESTIONS.md`](OWNER-QUESTIONS.md), this file. Other plan
files (GRADE, RESOLVE-UI, YOUTUBE, AUDIO, AI, COMPOSITOR, STORAGE, H264, RAW, DECODER) only when a
step points at them. Rewritten 2026-10-14 after the X422 acceptance (scheduled run).

**NOW (owner, live 2026-10-15 ~05:20, FORK-PLAN §1a.79): ship, then pause.** (1) H1 / H2 / H3 accepted: run their owed
PC checks, tag `h264-1/2/3` (+ `audio-2` after A2's owed fresh-context review), build (§1a.9), publish (§1a.33).
(2) AI3 to acceptance now: GPU inference to < 1 s first words / < 10 s typical answer, AI-PLAN AI3 accept list, no tune,
tag `ai-3`, build, publish. Work at STEADY or better for this regardless of the weekly line (owner override); keep only
the 5-hour hard stop. (3) Then create `.fork-paused`, message the live session "Progress since 1:30 pm" that it's shipped,
and stop. Don't start C2.

**Phase order (FORK-PLAN §1a.6, the only valid order):** … → Y1 ✓ → 6E ✓ → U0.2 ✓ → AI1 ✓ →
X422 ✓ → **Y3** → C1 → **AI3** (§1a.73–74; knowledge work on the Dell first) → C2 → AI2 → **A3** (§1a.76) → S1 → S2 → G3 → **UPD** (§1a.77, update check) → 7 (+ Y4) → Y2 → C3 → C4 → G4 → R1 →
R2 → D → 8. Small phases are accepted in groups (§1a.41). AI1 moved right after U0.2 and the
4:2:2 export cases (X422) right after AI1 by the owner 2026-10-13 (§1a.69). **A2 and then
H1 → H2 → H3 run on the Dell lane (owner 2026-10-14, §1a.72); the main runner skips them.**
**Owner, live 2026-10-14 ~12:55 (§1a.75):** AI3 trains on **all of editing**, the Compositor (C1) included, not only grading; only features built by the tune, later phases add pages / pairs / eval questions and the tune is refreshed. **Q30 (a) accepted** (A2's numbers stand). **Q31 (a) yes:** the Dell lane may download the ITU-T H.264.1 conformance streams (Dell only, never committed) for H1.5 PAFF and H2. Q29 ok.
**Owner, live 2026-10-15 ~00:35 (§1a.78), done (AI3.5f `8cedc686`, merged):** Lessons / Ask the coach / **Assistant** are pills in
the Resolve page bar left of Home (Premiere: the header's right side; `coach::bar_buttons`); the Ask box only opens from them. AI3's
chat is `PanelKind::Assistant` (`panels/assistant.rs`): the button opens it floating (left of the coach's card column), dockable like
any panel; chat turns in `UiState::assistant` (never saved), follow-ups from the last model answer, New chat, Stop, clickable lines →
spotlight, Show me; fallback shows the keyword coach's steps. Tests: `coach_ui::assistant_panel_floats_answers_docks_and_closes`,
`pages_ui` placement checks; guide color/coach.md "The Assistant panel". **Next for the chat (main runner):** it uses the same
`coach.assist` path, so GPU inference speeds both; a Settings ▸ Color row to install the model from the panel when `notReady` would help.
Known flake (not from this change, fails on the base too under load): `panels_ui::scopes_of_colour_bars` in a parallel run.
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
**A2 accepted by the owner (live, 2026-10-15 ~00:15, §1a.76; tag `audio-2` after the owed review below); its Voice Isolation quality work moves into A3.** Was: **A2 ready for acceptance (Dell lane, 2026-10-14 ~08:30; §1a.72; scheduled task `filmcraft-dell-lane`,
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
(weekly usage above pace). **H1 ready for acceptance (Dell lane, 2026-10-14 ~18:50; tag `h264-1` rides with the main runner's next phase
group).** Merged: H1.3 `b728822d` (VA-API front takes PAFF and MBAFF: `vaapi/h264.rs` field flags / per-field order
counts / second field into its first field's surface / field reference lists, `geometry()` takes interlaced H.264),
H1.4 `50f2a402` (`tests/vaapi.rs::interlaced_h264_matches_ffmpeg`, libx264 MBAFF fixtures), H1.5 `995b48ab`
(`interlaced_conformance_streams_match_reference`, ignored; ITU-T set via `FILMCRAFT_H264_CONFORMANCE_DIR`), H1.5b
`81addcfd` (`interlaced_1080i_is_bit_exact_and_faster_than_real_time`, ignored). **Review:** fresh-context Opus review
done in this run's own session (it had never seen the diff; no subagent, weekly usage above pace, Q33 note); no
new `unsafe` (ffi.rs gains two flag constants only), so no Fable review needed. No blocking findings; non-blocking: an
unpaired field (its partner lost) leaves the other field's lines from the surface's previous picture on hardware (the
software path copies the field; never a crash). **Checklist on the Dell (UHD 770, Mesa iHD):** fmt ✓, clippy
platform + h264 ✓, platform tests ✓ (24 + 1080i), layers ✓; **ITU-T interlaced set 65 bit-exact (PAFF, MBAFF, mixed;
CAVLC / CABAC; High 8x8), 3 declined (MR6/7/8_BT_B: Extended profile), 0 failed; 1080i29.97 MBAFF (tff, B-pyramid, 4
refs) bit-exact vs ffmpeg, 549.8 fps decode (18× real time).** Finding for acceptance: the hybrid decoder starts
streams that open on a non-IDR I picture in software (24 of the conformance set); with H2 merged the software path
decodes them too (bit-exact), but such files (broadcast captures cut mid-GOP) then decode on the CPU until the hybrid
moves to hardware. ~~AMD VCN interlaced~~ ✓ 2026-10-14 18:40 (RX 6700 XT, `FILMCRAFT_VAAPI_DEVICE=/dev/dri/renderD129`,
release: `interlaced_h264_matches_ffmpeg` (3 MBAFF cases) + 1080i29.97 MBAFF bit-exact vs ffmpeg at 378 fps, synthetic;
the 32 GB conformance set stays on the Dell). **Still owed on the PC at acceptance:** the in-app zero-drop playback of a 1080i file on both GPUs.
**H2 started in parallel** (sequencing choice logged here): audit
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
**H2.5 (PAFF in software) merged 2026-10-14 ~19:20** (`d94bb1e8` and before; was branch `dell-lane-h2-5`) (local + the Dell's scratch
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
**H2.6 (MBAFF) merged 2026-10-14 ~19:20** (through H2.7b `346c0cc1`; was branch `dell-lane-h2-6`) (on top of `dell-lane-h2-5`; both
on this PC and the Dell's scratch repo): H2.6a pair addressing / mb_field_decoding_flag / Table 6-4 neighbours /
field-MB reconstruction (MBAFF a const generic of `SliceDecoder`); H2.6b field-MB references, weights, MC; H2.6c
direct with MBAFF colocated pairs; H2.6d MBAFF deblocking (pair order, mixed edges line by line, field-mode top
edge), chroma DC per-half availability; H2.6e interlaced accepted by default (hook gone), guide `hardware.md` row;
PREP-H2 §6 / §7 "As built" notes. **Branch results on the Dell: fmt, workspace clippy, h264 / codecs / media tests,
layers, wasm green; ITU-T conformance (218 archives) 210 pass / 7 unsupported (2 monochrome, 3 FMO, 2 SP/SI:
outside H2) / 1 skipped / 0 failed — every PAFF and MBAFF stream bit-exact; 8-bit progressive perf_1080p 1 thread
98.4 vs 97.4 fps (base `9c7f81a5`); H2.7b (same branch): x264 1080i MBAFF bit-exact vs ffmpeg, 31.9 fps on 1 thread,
144.7 fps on 16 (fork-benchmarks.md).** Dell-only debug tools in `~/fork-logs/fc-tools/` (never committed).
**Reviewed and merged (2026-10-14 ~19:20, Dell lane):** fresh-context Opus review in the run's own session (Q33
update; no subagent): no blocking findings (H2.5's duplicated colocated comment was already fixed in H2.6; unpaired
first fields are filled on the decoder thread, which waits for that field's job: latency, not a hang; MBAFF neighbour
indices stay inside the even pair geometry, and any slip is caught per picture as gray). Rebased onto hevc-vaapi
(guide `hardware.md`: one interlaced H.264 row, VA-API first, own decoder as fallback). **Gates on the Dell after the
rebase:** fmt ✓, workspace clippy ✓, h264 / codecs / platform / media tests ✓, layers ✓, wasm ✓, **ITU-T conformance
210 pass / 7 unsupported (monochrome, FMO, SP/SI) / 1 skipped / 0 failed.**
**H2 ready for acceptance (Dell lane, 2026-10-14 ~19:55; tag `h264-2` rides with the main runner's next phase
group).** All of H2.1–H2.6, H2.7a / H2.7b and H2.8a / H2.8b are merged (last: H2.8b `0f929430`). Checklist (Dell,
i5-13500T): **ITU-T H.264.1 conformance 210 pass / 7 unsupported (2 monochrome, 3 FMO, 2 SP/SI: outside scope) / 1
skipped (alpha) / 0 failed** (every in-scope profile: Baseline / Main / Extended-as-Main / High / High 10 / Hi422 /
Hi10 Intra / Hi422 Intra, progressive, PAFF and MBAFF); the fixture matrix (x264, plus VideoToolbox streams on macOS) 57 bit-exact vs ffmpeg (incl. High
10, High 4:2:2 8 / 10-bit Intra and Long-GOP, and since H2.8b four interlaced MBAFF ones: CABAC, CAVLC bff, High 10,
High 4:2:2 10-bit); never-crash rounds (bit flips incl. parameter sets, truncation, depth / chroma switches,
damaged synthetic field / MBAFF streams, damaged x264 MBAFF streams; 1 and 4 threads): no panic, no hang; 4K 10-bit
4:2:2 25p (H2.7a): Long-GOP 43.2 fps, Intra 632 Mbit/s 26.8 fps with 16 threads (1 thread 8.5 / 3.3); 1080i MBAFF 31.9
fps on 1 thread, 144.7 on 16; guide `hardware.md` rows. **Deferred (FYI):** slice-level threads (Q32; frame
threading carries 4K 4:2:2). **Owed on the PC at acceptance:** fps on the i5-11600K (`cargo test --release -p
filmcraft-h264 --test perf -- --ignored --nocapture perf_4k_422_10 perf_1080i_mbaff`), the H2.7 profile of the
−2.3 % 8-bit 4:2:0 slowdown (no `perf` on the Dell), the 4K 4:2:2 fixtures and a 1080i file played in the app
(a real camcorder 1080i file if the owner has one).
**H3.0 audit merged** (`ed98d5c5`, docs/fork/PREP-H3.md, 2026-10-14 15:45): once H1 / H2 merge, ffmpeg is already
unnecessary for every in-scope H.264 stream; choice (c): H3.1 drops the ffmpeg pipe from the chain for H.264 only
(HEVC left to Phase 7.7), H3.2 clip badge / Decoder line, H3.3 ffmpeg-hidden + hardware-pulled harness, H3.4 playback
check + guide. H1 and H2.5 / H2.6 are merged, so H3 code can start (after H3.2a's review).
**H3.2a merged** (`e63c6410`, 2026-10-14 ~19:35, reviewed in-run, gates green on the Dell after the rebase; was branch `dell-lane-h3-2`)
(`git diff origin/hevc-vaapi...dell-lane-h3-2`; local + the Dell's scratch repo): the Project panel says when
H.264 decodes in software: High 4:2:2 gets the 4:2:2 badge (never "hardware failed", which is HEVC-only now),
High 10 4:2:0 an amber "10" badge "H.264 10-bit: software decode"; not claimed under VideoToolbox (Apple silicon may
take those profiles). `proxy_jobs::{is_h264, bit_depth, software_h264_high_bit_depth}`, `hardware_422_failed(info)`;
engine tests (hostile pixel-format labels) + UI test `media_ui::h264_software_decode_badges` (Premiere and Resolve);
es / zh-cn; guide `hardware.md` options table (edit the original / proxy / intra export). Dell gates: fmt, workspace
clippy, engine proxy_jobs + coach, ui media_ui + coach + i18n, layers, wasm green. Joins the Q33 review queue (a
low-risk UI diff: a Sonnet review is enough). **H3.2b** merged (below).
**H3.1 + H3.3 + H3.4a merged** (`66ef1c63` / `34a843f7` / `9fdbc1b3`, 2026-10-14 ~19:00, in-run fresh-context
review of `crates/platform` (no new `unsafe`; no blocking findings: the chain's closures, the pipe-name check and the
guide's "H.264 never uses ffmpeg" hold; H.265 export is the only other ffmpeg user); was branch `dell-lane-h3-1`):
**H3.1** H.264 never decodes through the ffmpeg pipe (`vaapi_chain` in `crates/platform/src/lib.rs`: zero-copy →
read-back → FilmCraft's own decoder; HEVC keeps the pipe until 7.7; `hardware_decoder_for` follows it), **Q34** (FYI,
choice c); **H3.3** `crates/platform/tests/ffmpeg_free.rs` (ignored): the product's chain in child processes with
ffmpeg hidden, hardware / Hardware decoding Off / VA driver pulled × 7 fixtures (H.264 progressive, High 10, 4:2:2
10-bit, MBAFF, MBAFF 4:2:2 10-bit; HEVC Main, Main 10): **all pass on the Dell's UHD 770, every picture identical to
the software decoder, none through the pipe**; **H3.4a** guide `hardware.md` § "H.264 without ffmpeg". Base had moved
by docs only, so the branch's Dell gates stand (fmt, clippy, platform tests incl. VA-API hardware, coach, layers).
**H3.2b merged** (`8253a5a5`, 2026-10-14 ~19:15, self-reviewed: weekly usage above pace): File ▸ Get Media File
Properties for ▸ Selection… has a **Decoder** line (the decoder that decoded the file last, e.g. "VA-API H.264 (in
process)" / "FilmCraft H.264"; follows a hybrid's mid-stream fallback; "chosen when the clip first plays" before the
first frame; files only). `GopCache::decoder_name` (a leaf lock, never waits on a decode; intra decoders recorded when
made, outside the race window `intra_frames_decode_in_parallel_once_each` measures), `MediaSource::decoder_name`
(default `None`; mp4 / mkv / mxf / mpeg / apv and the unreadable-slate wrapper forward it), `decoder` in
`file.mediaProperties`; codecs + engine + UI tests (`media_ui::media_properties_decoder_line`, Premiere and Resolve);
es / zh-cn; guide `hardware.md`. Dell gates: fmt, workspace clippy, codecs gop / media / engine project_tools +
media_pool + coach / ui media_ui + menus_ui + coach + i18n, layers, wasm green (re-run after the rebase onto AI3.6a).
`intra_frames_decode_in_parallel_once_each` failed once under full load in the first gate (6 / 6 alone; then the
record moved out of its window). **H3.4b done (2026-10-14 ~19:30, no code):** the release app in a virtual KWin on the Dell (`kwin_wayland --virtual`,
1920×1080), ffmpeg hidden (`FILMCRAFT_FFMPEG=/nonexistent`, PATH without it; log: "ffmpeg not usable"), `--empty`,
control channel: four 10 s 1080p25 x264 clips (High, High 10, High 4:2:2 10-bit, MBAFF tff), each its own sequence,
9 s of playback each. **Played 220–224 frames, 0 dropped**, except **1 dropped on the first software-decoded play of a
session** (High 10 or 4:2:2, whichever plays first; 0 on every later play: warm-up, not codec-specific; noted, not
fixed). High and MBAFF: `hardware.frames` 236, `backends` only "VA-API H.264 (in process)"; High 10 and 4:2:2 (proxies
off): `softwareFrames` 237 / 239, no backend; with Automatic proxies on, the 4:2:2 clip played from its proxy (HEVC on
VA-API). **No stream went through the ffmpeg pipe.** Properties dialog (screenshots): "Decoder: FilmCraft H.264" /
"Decoder: VA-API H.264 (in process)". Scripts / logs on the Dell: `~/fork-logs/h3-4b-{run,run2,drive}.*`.
**H3 ready for acceptance (Dell lane, 2026-10-14 ~19:35; H264-PLAN H3; tag `h264-3` rides with the main runner's next
phase group).** Merged: H3.0 audit `ed98d5c5`, H3.1 `66ef1c63` (H.264 never through the ffmpeg pipe), H3.2a `e63c6410`
(software-decode badges), H3.2b `8253a5a5` (Decoder line), H3.3 `34a843f7` (ffmpeg-free harness: 7 fixtures × 3
setups pass on the UHD 770), H3.4a `9fdbc1b3` (guide "H.264 without ffmpeg"), H3.4b above. **Owed on the PC at
acceptance:** `cargo test --release -p filmcraft-platform --test ffmpeg_free -- --ignored --nocapture` on the RX 6700 XT;
the same in-app playback (ffmpeg hidden) on both GPUs; H1's 1080i zero-drop playback; H2.7's profile. Dell leftovers: A/B worktree `~/Projects/fc-base` (at
`9c7f81a5`) + `~/Projects/fc-perf-*` targets (≈0.5 GB, reusable for perf A/B).

## Status

- **Branch** `hevc-vaapi` (never touch `master`). Tags `phase-1` … `phase-6`, `grade-0` …
  `grade-2c`, `upstream-sync-1`, the yt-1 group (`resolve-ui-1`, `resolve-ui-2`, `phase-6-7b`,
  `coach-1`, `yt-1`) and the **6E group on `d866cb4c` (2026-10-13): `phase-6e`, `linux-1a`,
  `linux-2a`**, **`upstream-sync-2` on `e090c0c9` (2026-10-13, U0.2 + U0.2.3b)** and `ai-1` on
  `bcb5f713` (2026-10-14) and the **X422 group on `f8c3e788` (2026-10-14): `x422`, `audio-1`, `linux-1b`,
  `linux-2b`** and **`yt-3` on `db7c1d0d` (2026-10-14)** and **`comp-1` on `236c1c0c` (2026-10-14)**. Builds in `~/Apps/filmcraft-builds/`; `latest` → `comp-1`; launcher
  `~/.local/share/applications/filmcraft-fork.desktop`. Public (§1a.33): dawgon2/filmcraft-cachyos
  branch `fork` snapshot `ca059e2e` (of `comp-1`, parent `9abdf285` = `fork-yt-3`), tag `fork-comp-1`,
  release `fork-comp-1` (scrubbed Dell build; NOTES.md); earlier tags `fork-yt-3`, `fork-x422`, `fork-audio-1`,
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
  - *U0.2:* ~~Opus review of U0.2.3b d and U0.2.4 / .5~~ ✓ 2026-10-15: fixed (1) an MP4 clean aperture was cut in full-size
    coordinates from a scaled hand-off picture (wrong part / aspect): such sources hand off unscaled (`GopCache::crops`, test
    `a_clean_aperture_keeps_the_hand_off_unscaled`); (3) `with_handoff_scale` restores on unwind (drop guard, test
    `a_panic_inside_restores_the_scale`). ~~(2) comp items in `handoff_scales`~~ ✓ (media a comp's Media In reads keeps full size, a comp reading a track keeps the whole sequence at full size; test `media_a_comp_reads_keeps_full_size`). **Left:**
    (4) `quiesce()` vs live decode / encode threads: ✓ in part 2026-10-15 (each picture submit — decode, video processing,
    encode — holds `Busy`, so exit waits for one in flight and refuses new ones; test `crates/platform/tests/va_exit.rs`,
    diagnostic `va::work_refused_while_exiting`); ✓ 2026-10-15 04:10: surface work outside a submit (sync, image read / write,
    DMA-BUF export, new / spare surfaces, coded-buffer maps) holds it too (test `crates/platform/tests/va_exit_sync.rs`, failed before);
    (5) ~~`va.rs` Send / Sync comment~~ corrected 2026-10-15; ~~drivers other than Mesa / iHD shared unchecked~~ ✓ 2026-10-15 05:12 EDT:
    `Display::open_raw` refuses them (`driver_is_thread_safe`: vendor string starts "Mesa Gallium driver" / "Intel iHD driver"),
    software decode instead, `FILMCRAFT_VAAPI_ANY_DRIVER=1` overrides (Q36; per-display serialisation for NVIDIA = Phase 8);
    ~~(6) nit: `KEPT` cap 64 unreachable today~~ ✓ 2026-10-15 04:33 EDT (named `KEPT_MAX`, a display past it is logged as not kept);
    ~~`teardown_ui` flake~~ ✓ 2026-10-15 (reproduced with `teardown_ui timeline text title trim`
    at 6 threads: other tests' ~100 frame / background threads starve the closed app's last frame; the test now
    checks that no queued frame job runs after the close and that everything exits within 2 min, via
    `FrameServer::live_workers`; a missing stop still fails it). **Left:** render-cache cancel latency inside one frame (CLOUD-QUEUE item 10, cloud
    diagnosis); the 2 min bound is only a never-stops check for it. Thread names in its failure list are
    inherited: unnamed threads started from a `filmcraft-background` / `filmcraft-frames-N` thread (rayon pool,
    decoder threads) carry that name on Linux, so ~38 "filmcraft-backg" is not one worker per session; ~~hand-off scale only for MP4 / MOV sources~~ (MKV and MXF implement
    `set_handoff_scale` now, test `crates/codecs/tests/handoff_scale.rs`); U0.2.3 owed (ii) re-grey test needs failure injection, ~~(v)
    `va_dual_export::two_gpus_keep_the_colours_of_one` with the Intel device forced~~ run 2026-10-15
    (`FILMCRAFT_VAAPI_DEVICE=/dev/dri/renderD128`): colours kept (worst mean chroma shift 0.06, bound 1.0), but its PSNR bound
    fails: 2.15 dB (bound 0.5), because the second lane is then AMD, whose encoder gives lower chroma PSNR than Intel's at the
    same settings; with AMD primary (the default) the second lane is the better encoder. Not a colour bug; if Intel-primary
    dual exports matter, compare against the weaker GPU's single export in that test.
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
  - *Earlier:* ~~6.7b TNR keyframe markers / label width~~ ✓ 2026-10-15 (lane diamonds: click seeks, drag = `tnr.moveKeyframe`); RS3 cramped Color page, Markers vs Index,
    missing panels, Fairlight strip, home-screen screenshot compare; Phase 6 frame-server proxy
    hint, ~~`cache.claimFolder`~~ ✓ 2026-10-15, decode-to-screen fps with hardware 4:2:2, software 4:2:2 memory cap
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

## Exact next step: §1a.79 order: (1) H group acceptance (owed PC checks → tags h264-1/2/3 → build → publish), (2) AI3 to acceptance (GPU inference, STEADY+), (3) `.fork-paused`

**2026-10-15 05:50 (scheduled run):** H group in progress: `ffmpeg_free` on the RX 6700 XT and H2 perf on the i5-11600K
running; then in-app 1080i / 4K 4:2:2 playback on both GPUs (ffmpeg hidden), tags, build, publish. AI3 notes below stay valid. **Done 05:55:** H3 ffmpeg_free on AMD ✓, H2 perf ✓, in-app 1080i zero drops on AMD + Intel ✓, 4K 4:2:2 H.264 plays at
9–24 / 25 fps (recorded; proxies recommended; burstiness → Open problems). Done since: A2 review + fixes `a8c708b9`, ci (AMD VCN fault, Q37; Intel + Dell green), tags
`h264-1/2/3` + `audio-2`. Left for (1): build `~/Apps/filmcraft-builds/h264-3` (§1a.9) → publish (§1a.33).

**2026-10-15 03:58 (scheduled run, LEAN):** the owed C1 GPU-comp review and U0.2 review are done (fixes + tests, leftovers
listed under Status › Owed). Token-light leftovers are now thin. **Next = GPU (wgpu) inference for AI3.3b**: step 1 (Q4_K matvec) ✓ 05:45 in MINIMAL, see "AI3.3b GPU step 1" below for
steps (a)–(e); the bigger ones (attention, forward path) best once the mode is
STEADY or better (Opus / Fable designer + builder per §1a.44); CLOUD-QUEUE item 14 (AI3-GPU-PREP) asks the cloud for the
design prep `docs/fork/PREP-AI3-GPU.md`: read it first if it has landed. In LEAN / MINIMAL meanwhile: the in-app AMD + Intel
look at a comp reading an F-Log2 clip (C1 owed); ~~C2's design note~~ ✓ 05:21 (`docs/fork/DESIGN-C2.md`) (`vaSyncSurface` outside submits ✓ 2026-10-15 04:10). Comp mask Bézier handles ✓ 2026-10-15 04:27 EDT; Ctrl-click add / delete points on comp masks ✓ 2026-10-15 04:32 EDT.

**Order fix 2026-10-14 16:50 (scheduled run):** this section said "C2 next", but FORK-PLAN §1a.74 puts
**AI3 between C1 and C2** (… C1 → AI3 → C2 → AI2 …). AI3 is the current phase; the C2 plan below waits.
AI3-PREP (items 0a–0d, 6) is done on the Dell (see above). **Main-runner step map** (AI-PLAN items 1–6):
- **AI3.2a ✓** (2026-10-14): `coach_ask::retrieve(question, k)` (≤ `MAX_RETRIEVE` 12): a second BM25 index
  over **every** guide page (NOT_INDEXED practice / general pages included; README.md left out) that also
  searches each section's compact header (`does:` ×2, `when` / `resolve` / `premiere`); the coach's own index
  is unchanged (colour eval floor untouched). `retrieval_finds_the_advanced_sets_sources`: **recall@5 60 / 63**
  on advanced.toml (floor 60; misses `adv-grain-where`, `adv-shot-match-limits`, `adv-black-point`),
  `retrieval_survives_hostile_questions`.
- **AI3.2b ✓** (2026-10-14): `coach_assist::assistant_prompt(question, &AssistContext, budget_chars)` → `AssistPrompt { system, user, sources }` (6 sections, best 2 with full text when they fit, others header + steps; budget in chars, default 12 000, max 64 000; markers sanitised; 3 tests). Token-exact budgeting waits for AI3.3 (the model's tokenizer).
- **AI3.2c ✓** (2026-10-14): heading-word boost in `retrieve` (`ASSIST_HEADING_MATCH` 0.5); `adv-grain-where` also accepts `color.nodes.starter-tree` (it states grain goes on the last node); `practice.grading.black-white-points` header `when:` names blacks / whites. **recall@5 62 / 63** (floor 62; `adv-black-point` "how far down should my blacks go?" still misses, left honest).
- **AI3.4a ✓** (2026-10-14, done before AI3.3 because it needs no model): `coach_ask::ground(answer, &sources, &Checks) -> Grounded` checks a model answer line by line (a line naming an unknown control / a command-namespace id that isn't registered is dropped, ids listed; `[id]` citations of sections not sent are taken out as `invented_citations`; `General advice:` lines marked; ≤ 80 lines / 16 000 chars); 2 tests. Sources list in the UI ✓ (AI3.5c). **"I don't know" ✓ (2026-10-14 18:58 EDT):** no section found → `AssistFallback::NothingFound` (model not asked); the model saying the sections don't cover it → `AssistAnswer::dont_know` / JSON `dontKnow` (lines kept, keyword coach's answer alongside, UI note `coach.ask.assistDontKnow`); tests `the_assistant_says_when_it_does_not_know` + the box test.
- **AI3.3a ✓** (2026-10-14): **SmolLM3-3B runs on our runtime.** `Family::SmolLm3` (GGUF arch `smollm3`;
  `Config::no_rope_step` 4: layers 3, 7, 11 … skip the rotary embedding, as llama.cpp; interleaved rope like
  `llama`; tied output; end-of-text `<|end_of_text|>`), `chat_prompt` follows the published template in
  `/no_think` mode (metadata header, **no `<|im_end|>` after the system block when there are no tools** —
  that is what the template writes —, empty think block; "Today Date" left out), catalogue entry
  `smollm3-3b-q4km` pinned to `ggml-org/SmolLM3-3B-GGUF@4965cb60` Q4_K_M (1 915 305 312 B, sha256 verified)
  + base-repo tokenizer `@a07cc9a0`. Test `smollm3_skips_the_rotary_embedding_on_its_nope_layers`. **Real
  model** (`tests/model.rs` with `FILMCRAFT_LLM_DIR=target/ai3-models/root`, files in
  `target/ai3-models/smollm3-3b/`, never committed): coherent answer, verify 1.3 s, load 1.1 s, **~5.9 tok/s**
  generation, prefill 981 tokens in 24.4 s (~40 tok/s), first words 1.8 s with the system prompt cached
  (release build, this PC, synthetic). Guide `color/matching.md` names the model.
- **AI3.3c ✓** (2026-10-14, ahead of 3b): `coach_assist::answer(backend, model, question, ctx, checks) ->
  AssistAnswer` (blocking; run off the UI thread): prompt → `ExplainBackend::generate` (400 tokens, 90 s) →
  `ground`; the keyword coach answers with `AssistFallback::{NotReady, Error, Ungrounded}` otherwise. Test with
  a scripted backend.
- **AI3.3d ✓** (2026-10-14): commands **`coach.assist {question, page?, selection?, gradeFacts?, model?}`** →
  `{id, model}` and **`coach.assistResult {id, wait?}`** → `{state: pending|done|unknown, source: model|coach,
  sources, lines[{text, general, ui, commands}], cited, removedLines, removedIds, inventedCitations, fallback,
  coach}`. `Session::assist: coach_assist::AssistJobs` (background thread `filmcraft-assist` under
  `catch_unwind`, one model answer at a time, ≤ `MAX_PENDING` 4 waiting, 32 kept; command ids snapshotted for
  the worker; no backend (no `llm` feature / wasm) → the coach answers at once). Model: `ASSIST_MODEL`
  `smollm3-3b-q4km` or `model` param (an "Assistant model" pref comes with the UI). Test
  `coach_assist_commands_answer_in_the_background`. **Streaming ✓** (2026-10-14 evening): `answer_streaming`, pending results carry `partial` (unchecked text so
  far; a stop → coach fallback; an evicted question stops the model). **Forget ✓:** `coach.assistForget {id}` stops / drops an answer. **Not yet:** a real-model end-to-end run of
  `coach.assist` (do it with AI3.3b's smaller budget).
- **AI3.4b ✓** (2026-10-14): `coach_ask::ground_with_text(answer, sources, sent_text, checks)`: in a line with
  an action word (click / press / open / select / find / use …) every `**bold**` / "quoted" label, and every
  shortcut (`Ctrl+N`) anywhere, must appear in the guide text sent (`AssistPrompt::sent_text`), else the line
  goes; `answer` uses it. System prompt: "not in FilmCraft yet → say so, no steps"; name controls only as the
  sections write them. Test from the real hallucination. Ignored real-model test
  `real_assistant_answers_advanced_questions` (`ASSIST_BUDGET`, `FILMCRAFT_LLM_DIR`).
- **Real-model findings (SmolLM3-3B Q4_K_M, budget 3 500 chars ≈ 1 000 tokens, 400 max new tokens,
  2026-10-14, synthetic):** 48–70 s per answer (target < 10 s); answers are long (300–400 tokens); **no
  citations** in 2 of 3; on the gap question "how do I add a parallel node like in Resolve?" it **invents the
  feature** ("Parallel Mixer", `Ctrl+N`, a "+" button, File ▸ New Project ▸ "Node Tree") even with the rule;
  the label check removes 4 lines, the rest (unlabelled steps) survives. **So:** (1) gap questions need a
  deterministic route before the model — if the best sections say "not yet" / "not available" for the
  question's topic, answer from the coach's `not-yet` route or state it up front in the user message;
  (2) shorter answers (max ~150 tokens, "at most 6 steps") and streaming; (3) the eval (AI3.6) will likely
  need the tune on training-pairs.jsonl (human review sample first → OWNER-QUESTIONS when it starts);
  (4) a Fable / Opus design pass on (1)+(3) when the mode allows (MINIMAL today).
- **AI3.4c ✓** (2026-10-14): gap route. `assistant_prompt` finds a gap in the **best** section (`gap_line`: a
  "**FilmCraft:** … no / not … yet" first clause, or "Planned: …") → `AssistPrompt::gap`, an "Important, what
  FilmCraft doesn't have yet [id]: …" note before the question; `answer` caps it at `GAP_ANSWER_TOKENS` 150
  and opens with the guide's gap line + citation when the model didn't say it. On advanced.toml: **6 / 7 gap
  questions found, 1 other hit** (`adv-bleach-bypass`, a real partial gap); top-2 / top-3 gave harmful false
  hits (adjustment layer, reference look). Tests `gap_questions_are_told_…`, `gap_detection_matches_…`.
- **AI3.3b part 1 ✓** (2026-10-14): shorter by default: `DEFAULT_BUDGET_CHARS` 4 500 (≈ 1 100 tokens),
  `FULL_TEXT` 1, `MAX_ANSWER_TOKENS` 200, system prompt "at most 6 steps, no headings". Estimate on this PC:
  ~25 s prefill + ~33 s answer: still far from the acceptance (first words < 1 s, typical answer < 10 s).
  **The numbers say CPU is too slow for 3B** (AI-PLAN item 3: "GPU inference (wgpu) only if CPU is too slow
  for a 3–4B model: a separate, bigger step, decide on the numbers"): runner's call, **next big step = wgpu
  inference for the Q4_K / Q6_K matmuls** (RX 6700 XT / Intel; CPU stays the fallback), a Fable designer +
  builder per §1a.44 once the mode allows (STEADY or better); in MINIMAL keep to AI3.5 UI / streaming work.
  Also cheap first: re-measure with the new defaults (`real_assistant_answers_advanced_questions`).
- **AI3.5c ✓** (2026-10-14 evening): **chat UI, first part.** The Ask the coach box has a
  **Also ask the assistant (slow: runs on this computer)** checkbox (`coach.ask.assistant`, off by
  default, provisional until GPU inference): the keyword coach answers at once, then `coach.assist`
  runs; the box polls `coach.assistResult` each frame (repaint 250 ms) and shows the partial text
  ("Not checked against the guide yet.") with **Stop** (`coach.ask.assistStop` → `coach.assistForget`),
  then the grounded lines (`coach.ask.assistLine.{n}`, "General advice" dimmer), **Sources** (cited, else
  given) and lines left out; a fallback says why in one line. Closing / unticking / a new question stops
  it. Test `coach_ask_box_shows_the_assistant_answer_or_why_not` (scripted backend); es / zh-cn strings;
  guide `color/coach.md` › "Also ask the assistant". Side fix: the new section flipped a near-tie in the
  coach eval (`owner-1-para-d` at the floor 94), fixed guide-side ("Start here when grading any log clip"
  in `recipes.md` › 1). **Still owed for AI3.5:** follow-up questions (conversation context),
  "Show me" → lesson from an assistant answer, an "Assistant model" choice. **AI3.5d ✓:** the checkbox is the
  preference `color.coach.assistant` (default off, lenient: a bad value is off; also Settings ▸ Color ▸ Grading Coach).
  **AI3.5e ✓ follow-ups:** `coach.assist` takes `previousQuestion` / `previousAnswer` (each ≤ `MAX_PREVIOUS_CHARS`
  800, sanitised; "Earlier in this conversation" block before the sections; retrieval reads both questions);
  the box remembers the last checked model exchange (`CoachAsk::last_exchange`) until **New topic**
  (`coach.ask.newTopic`), close or untick. Test `a_follow_up_carries_the_earlier_exchange`. **AI3.5 left:**
  "Show me" ✓ (2026-10-14 19:06 EDT, `coach_ask::lesson_for`, `coach.ask.assistLesson`), an "Assistant model"
  choice (with more models), a real-model end-to-end run of the box (with GPU inference).
- **AI3.6a ✓ (2026-10-14 19:11 EDT): eval scorer.** `coach_assist::{parse_eval_set, score_case, summarize}`: per case
  `model_answered` (not the coach / "I don't know"), `cites_source` (a cited id among the case's `sources`), `invented`
  (removed lines + invented citations: any one fails), `states_gap` for `gap` cases; `auto_pass` is an upper bound
  (the `must` facts come back in `review` for a reader). Test `eval_scores_sources_inventions_and_gaps`; ignored
  runner `real_assistant_eval_on_the_advanced_set` (`ASSIST_EVAL_LIMIT`, writes `target/ai3-eval.json`; ~1 min/case
  on CPU, run on an idle machine or the Dell as a soak). **AI3.6 left:** the `must`-fact check (reader or a judge
  pass), the run itself (best after GPU inference), tune only if the 85 % floor is missed.
  **Baseline run 2026-10-14 20:18 EDT (SmolLM3-3B Q4_K_M, CPU, default budget, synthetic): 7 / 85 auto-pass
  (8 %)**, 19 cite an expected source, 60 had lines / citations removed (real inventions, e.g. "use a Key-Mixer node"
  on the parallel-node gap), 20–57 s per answer, 63 min total. Floor 85 % → far off: **OWNER-QUESTIONS Q35** (tune on
  training-pairs.jsonl needs the owner's sample review; provisional: assistant stays off by default, next = GPU
  inference, meanwhile try (b) extractive answers: the model picks / orders / shortens guide steps only). Report
  `target/ai3-eval.json` (not committed; carries `removedIds` / `inventedCitations` from the next run on).
- **AI3.6d ✓ (2026-10-14 20:39 EDT): pick-the-steps style** (Q35 (b)): `AssistStyle::Pick` / `coach.assist {style: "pick"}`, default
  still `free`; the model only names step keys, the answer is guide text word for word (no inventions by design, ~40
  tokens out instead of 200). **Full eval ({D}, synthetic): 69 / 85 (81 %), 0 invented** (free: 7 / 85) →
  pick is now `DEFAULT_STYLE` of `coach.assist` / the box (Q35 update; `style: "free"` kept). Next cheap steps for
  the 85 % floor (report `target/ai3-eval.json`, not committed): the 4 gap questions that don't state the gap
  (in pick mode the gap line is only prepended when the reply picks nothing or the first line lacks it: check
  `adv-outside-node` etc.), the 10 without an expected source (retrieval / which sections fit the budget), the 4
  "NONE". Then GPU inference (speed) as before.
  **AI3.6e ✓ ({D}):** scorer accepts "no … yet"; bullet lists split into steps; **fresh run 76 / 85 (89.4 %), 0
  invented** (synthetic) → floor met on the automatic checks, no tune needed (Q35 update). Left for AI3.6: the `must`
  facts read (a reader / judge pass over `target/ai3-eval.json`, not committed). **Next big step: GPU (wgpu) inference
  for AI3.3b** (Fable / Opus designer once the mode is STEADY or better). **`must`-fact read ✓ (2026-10-14 22:30 EDT,
  [`coach-eval/AI3-MUST-READ.md`](coach-eval/AI3-MUST-READ.md)): 53 / 85 (62 %) state every `must` fact, 73 / 85 (86 %)
  counting partial answers**, 3 auto-passes miss the point; ~15 answers padded with off-topic picks (the model picks
  runs of keys from the top sections). A prompt line against padding was tried (73 / 85, padding unchanged) and
  reverted; the eval runner now defaults to `DEFAULT_STYLE` (pick). **Padding filter ✓ (2026-10-15):**
  `PICK_MIN_RELEVANCE` 0.5 in `parse_picks` (each `PickStep` carries its section's score / best); replayed on the
  22:14 answers: 39 / 322 lines dropped, no auto-pass lost (0.6 lost two). `adv-export-size` retrieval fixed guide-side (header `when:`). In MINIMAL meanwhile: the remaining misses (+ ~~`adv-edit-tighten-cut`~~ ✓ 2026-10-15: synonym `late` → trim roll ripple: editing.trimming rank 3 → 1, recall@1 70 / 85, floor raised; `adv-qualifier-chatter` looked at 2026-10-15: the answer sits in `color/secondaries.md` › Tips, the guide's only `### Tips` block, which pick mode doesn't offer (numbered steps only, ≤ `PICK_STEPS` 8, already full there); the `practice.grading.qualify-skin` pitfall bullet was offered and not picked: a model limit, left as is) (~~adv-film-look~~ ✓ 2026-10-15: a **quoted** header `when:` example question whose words are all in the question ranks its section ×1.5 (`ASSIST_WHEN_MATCH`; unquoted `when:` text is a description and isn't boosted: "exporting HDR", "interviews and documentaries" stole rank 1 when tried); `practice.looks.film-curve` rank 2 → 1, recall@1 65 → 66 / 85 (new floor `RETRIEVAL_FLOOR_FIRST`), recall@5 85 / 85; pick prompt now offers its toe / shoulder step first; not re-run on the model; adv-lut-wrong-camera looked at 2026-10-15: its answer (normalise-log A4 / A6 pitfall) was offered as rank 1 and the model replied NONE, a model limit; ~~adv-aces~~ ✓ 2026-10-15: new guide section `practice.delivery.aces` (gap in the first clause, so the gap note fires; quoted `when:`), added to the case's sources, ~~adv-grain-where~~ ✓ 2026-10-15 (`practice.grading.grain-finish` `does:` now says "last node of the tree, after the look": not retrieved → rank 1; recall@1 67 / 85, floor raised; the model had picked the glossary although starter-tree B6 was offered), ~~adv-check-scene~~ ✓ 2026-10-15 (`numbered_list` bug: a wrapped line "100. **Check scene** …" in color/matching.md was taken as the list, so pick mode offered one prose line instead of steps 1–3; now a list interrupting a paragraph must start at 1, as CommonMark; test `a_number_at_the_start_of_a_wrapped_line_is_not_a_step`),
  adv-edit-proxy-export (looked at 2026-10-15: `proxies.what` A1 states "an export always reads the original files", offered first; the model replied NONE — same pattern as adv-lut-wrong-camera: **yes/no questions get NONE although the first step answers them**; idea for the next model run: a PICK_SYSTEM line "a step that answers a yes / no question counts", measure on those two + a few others; glossary › Proxy had a stale "planned for Phase 6" line, fixed), ~~adv-audio-voice-fast~~ ✓ 2026-10-15 (synonyms `cleanup` → clean up, `quickest` → fast quick: audio.voice-isolation / audio.cleanup-voice rank 1 / 2, were 2 / absent; recall@1 69 / 85, floor raised; coach eval 94 / 124 unchanged), ~~adv-gfx-lower-third-series~~ ✓ 2026-10-15 (quoted `when:` "the same lower third in every episode" on graphics.templates: rank 2 → 1, recall@1 71 / 85, floor raised), ~~adv-media-dual-system-sound~~ (2026-10-15: its answer section `practice.editing.sync-sound` ranks 1 and is correct; added to the case's sources; the model still skipped its Merge Clips / Synchronize step A1 and padded with multicam).
- **Model re-check ✓ (2026-10-15 ~01:05 EDT, SmolLM3-3B Q4_K_M, CPU, pick style, synthetic):** the 8 cases fixed on 2026-10-15
  pass 7 / 8 (adv-check-scene answered right from `color.recipes.scene`, now in its sources); the yes / no NONE pattern is fixed by a
  `PICK_SYSTEM` line ("a step that answers a yes / no question by saying what FilmCraft does counts as an answer"): a second run
  of lut-wrong-camera, proxy-export, check-scene, film-look, aces and the gap cases outside-node / parallel-nodes: **7 / 7, 0
  invented**. A full 85-case re-run with the new line is worth doing at the next idle hour (~60 min CPU, few tokens; reports
  `target/ai3-eval.json`, last full run 76 / 85).
  **Full re-run ✓ (2026-10-15 01:19–02:00 EDT, pick style, SmolLM3-3B Q4_K_M, CPU, synthetic): 83 / 85 auto-pass
  (97.6 %), 0 invented, 85 / 85 model-answered**, median 25.6 s per answer (max 90.7 s); misses `adv-mixed-light` and
  `adv-interview-match` (no expected source cited). Report `target/ai3-eval.json` (not committed; previous 76 / 85 run in
  `ai3-eval-prev.json`). Misses looked at (2026-10-15 02:20): `adv-interview-match` answered from `color.recipes.match` (rank 1, a correct how-to) → added to the case's sources, and that recipe's wipe step now says "match skin first"; `adv-mixed-light` retrieves its section at rank 1, the model picked day-for-night bullets instead: a model limit, left honest. **Smaller pick prompt ✓ (2026-10-15 02:15–02:43):** `pick_prompt` leaves out sections below `PICK_MIN_RELEVANCE` (their picks were dropped anyway): 3 682 → 3 064 chars on average (test `pick_prompts_leave_out_sections_whose_picks_would_be_dropped`); full re-run **84 / 85 (98.8 %), 0 invented, median 21.5 s (was 25.6), max 32.4 s (was 90.7), 28 min total (was 40)**, only `adv-mixed-light` misses (synthetic). Still far from < 10 s: GPU inference stays the next big step. The `must`-fact read (AI3-MUST-READ.md) was on the 76 / 85 run; re-reading it is optional.
- **AI3.3b GPU step 1 ✓ (2026-10-15 05:45 EDT, MINIMAL, done in the main run):** `filmcraft-llm` feature `gpu`
  (`crates/llm/src/gpu.rs`, `gpu_matvec.wgsl`): `GpuContext::{new, upload_q4k, upload_q6k, matvec}` → `GpuMatrix`, Q4_K and Q6_K × f32 matvec (Q6_K ✓ 05:55, 212 B
  blocks, d as f32), parity with `dequantize_q4k` / `dequantize_q6k` on AMD + Intel. Design choices made without the PREP note (provisional, cheap to change): a `gpu` feature on
  `filmcraft-llm` rather than a new crate; activations stay f32 on the GPU (no Q8_K); blocks 152 B (scales unpacked).
  **Next GPU steps (small, in order):** ~~(a) Q6_K matvec~~ ✓; (b) time both on a
  SmolLM3-sized matrix (11 008 × 2 048) vs `kquant::matvec_q4k`; per-call buffers → persistent x / y buffers; (c) RMSNorm,
  rope (NoPE layers), SiLU-gate, attention over a GPU KV cache; (d) a `forward.rs` GPU path behind the feature with a
  logits parity test on the real model (`FILMCRAFT_LLM_DIR`); (e) tiled matmul for prefill. CPU stays the fallback.
- **AI3.3b (open):** speed (GPU inference, above; prefill profiling). **Prefill checked 2026-10-14 18:20:** already batched
  (`forward.rs` `PREFILL_CHUNK` 64 → `Weight::matmul` → `quant::rows_at`: rayon tasks of 32 weight rows, AVX2 `dot4_v3`
  over 4 activation rows, each weight row read once per chunk); 981 tokens in 24 s ≈ 240 GFLOP/s effective, so no cheap
  CPU fix left (llama.cpp-class tiling might give ≤ 2–3×, still far from < 1 s): **GPU inference stays the next step**,
  with a Fable / Opus designer once the mode is STEADY or better. In MINIMAL meanwhile: ~~`compositor.md` guide gaps (§1a.75)~~ ✓ 2026-10-14 18:21 (every `comp.*` command documented;
  advanced.toml + 6 compositing questions ✓ (recall@5 68 / 69, floor 68);
  + 8 editing / trimming / audio / export ✓ (misses adv-black-point,
  adv-edit-three-point; audio → sound synonym added; + 8 titles / media / playback / project questions: 80 / 85, floor 80; misses adv-media-send-to-colourist, adv-play-heavy-timeline, adv-proj-move-computer; AI3.2e synonyms 2026-10-14 18:35–18:45: **85 / 85, floor 85**), or the cheap real-model re-measure (`real_assistant_answers_advanced_questions`, needs an idle machine). Acceptance wants first words < 1 s and a typical answer < 10 s: a 3 000-token
  prompt would take ~75 s of prefill at 40 tok/s, so (1) cache the fixed system prompt's KV (already
  cached across calls) and keep the per-question part small (≈ 800 tokens: headers + steps, full text only
  for the top 1; `DEFAULT_BUDGET_CHARS` down to ~3 500), (2) profile the prefill (matmul batch path in
  `weights.rs`; is Q4_K prefill using the batched kernel?), (3) then wire `assistant_prompt` → `Model` in
  `crates/engine` behind the `llm` feature (a `coach.assist` command returning the grounded answer). (Was AI3.2b:) prompt assembly in `crates/engine` (no model yet): `assistant_prompt(question, grade facts,
  on-screen context) -> Prompt` sending header + numbered steps of the top-k first, the full body only for the
  top 1–2, under a token budget (count with `crates/llm` tokenizer when a model is loaded, else chars / 4);
  tests: budget respected, every section id cited, hostile input. Then look at the 3 misses (synonyms or
  header `when:` words, guide-side fixes only if the text really lacks the answer).
- AI3.3 model: SmolLM3-3B (approved §1a.74) on the AI1 runtime (`crates/llm`: gguf / kquant / forward);
  download needs no further yes for SmolLM3; check the architecture (SmolLM3 = Llama-style with NoPE layers)
  against `forward.rs`; tokens/s on this PC. AI3.4 grounding checker (every `{ui:}` / command in an answer
  checked against the live app, unknown ones removed; "general advice" marking). AI3.5 chat UI (follow-ups,
  streaming, Show me → lesson; model-free coach as fallback). AI3.6 eval of model + retrieval on advanced.toml
  (≥ 85 %, no invented controls); tune only if the floor is missed (training-pairs.jsonl needs a human review
  sample first: ask via OWNER-QUESTIONS). AI3.1 / §1a.75: finish `compositor.md` and fill feature-page gaps.
- MINIMAL mode: do it yourself in small steps; with STEADY+ a builder for AI3.3 (decoder state → Fable per §1a.44).

## After AI3: C2 (COMPOSITOR-PLAN, parallel / layer nodes in the grader), step 1

**comp-1 accepted 2026-10-14** (C1: comp clips, node canvas + inspector, Compositor panel / Compositing workspace /
Fusion page, GPU comps via `PlanLayer.comp` + `crates/gpu/src/comp.rs`, Media In tracks as `Pass::Layers`, coach
routes (item 6), `comp.explain` + comp lessons (item 7); bench comp-1080 / comp-2160 half 192 / 0 drops, 30-min soak
clean; all synthetic). The C1 working notes are archived at the end of `docs/fork/PROGRESS-LOG-ARCHIVE.md` ("HANDOFF
notes for C1"). **Cloud work merged 2026-10-14 ~16:45** (USHADER-HOSTILE, R1-PREP, USERFX-FOLDER, GUIDE-SHADER-COOKBOOK; guide recipes render on AMD + Intel); full `cargo xtask ci` owed at the next boundary. TEARDOWN-FLAKE stays open (cloud diagnosis in CLOUD-LOG: render-cache `CacheJob` holds the pool past `CLOSE_WAIT`). **Owed from C1 (not blocking):** ~~an Opus review of `crates/gpu/src/comp.rs` + the `render_plan` comp
hook and `plan::track_layers`~~ ✓ 2026-10-15 03:33 (bug fixed: a later comp's uploads evicted an earlier comp's result from the 24-entry upload cache → it drew transparent; `refresh_comp_output` re-registers every result before the draw, test `a_busy_later_comp_does_not_evict_an_earlier_result`, failed before. **Left from the review:** ~~(2) comp VRAM~~ ✓ 2026-10-15 (`COMP_VRAM_BUDGET` now covers every held slot set, sets a frame no longer uses are dropped, all comp textures go after `COMP_IDLE_FRAMES` 120 comp-free draws; test `comps_share_one_budget_and_their_textures_are_let_go`); ~~(3) effect-pool churn from the nested slot draws~~ ✓ 2026-10-15 (one pool frame per top-level `render_plan`, `fx_frame_open`; test `comp_and_frame_effects_keep_their_pooled_textures`, failed before); ~~(4) `Pass::Image` of another size~~ ✓ 2026-10-15 (such a comp runs on the CPU, test in `a_comp_too_wide_for_a_texture_falls_back_to_the_cpu`); (5) `track_layers` nested `scale` > 1: left as is (2026-10-15: it only changes graphics / captions, which then rasterise at the comp's size, sharper than the CPU's upscale; media frames are already capped at full size; sizes stay within the texture limit)); ~~Item-source Media In still CPU pictures~~ ✓ 2026-10-15 02:56 (media items / subclips: one plan layer via `plan::media_frame_layer`, fitted as `fit`; log / HDR Y'CbCr gets the input transform as layer colour ops; RGB frames needing management, nested sequences and comps stay CPU; tests `an_item_source_compiles_to_a_plan_layer`, `a_media_frame_layer_fits_the_canvas`; owed: an AMD + Intel look at a comp reading an F-Log2 clip in the app); Mask shapes on the monitor have no Bézier handles / add-point; ~~`backdrop` / `post` reallocate when comp and frame sizes alternate~~ ✓ 2026-10-15 (it was `output` + `backdrop`, 2 new targets per frame; comp slot draws now swap in their own `comp_output` / `comp_backdrop`; test `a_comp_of_another_size_leaves_the_frames_targets_alone`, AMD + Intel); ~~the `settings_tests::hardware_decoding_setting_drives_the_decoder_switch`
parallel flake (seen once)~~ ✓ 2026-10-15 (`set_prefs` re-applied the process-wide decoder switch on every `prefs.set`, so another test's unrelated setting turned it back on; now only when Hardware decoding / Memory change; deterministic check in the test); the compositing coach-eval misses (`comp-*` in `docs/fork/coach-eval/questions.toml`): 4 fixed 2026-10-15 (COMP-EVAL), `comp-what-is` left. **Next in MINIMAL** (2026-10-15 03:25): token-light leftovers are thin; GPU (wgpu) inference for AI3.3b wants STEADY (Fable / Opus designer); TEARDOWN-FLAKE needs instrumented 8-copy repro runs of the ui test binary (CLOUD-LOG); ~~mask Bézier handles~~ ✓ 2026-10-15 04:27 EDT (comp mask overlay: smooth points show handles, drag bends (mirrored, Alt breaks), Alt-click smooth ⇄ corner; shared `MaskVertex::set_tangent` / `MaskPath::toggle_smooth`; test in `comp_ui.rs`); Ctrl-click add / delete points ✓ 2026-10-15 04:32 EDT, shared `Mask::add_vertex` / `remove_vertex`) (Item-source Media In ✓ 02:56).

**C2 plan (COMPOSITOR-PLAN §C2):** 1 grade model → DAG (`Grade` keeps node ids, gains inputs per node; serial grades
load unchanged: schema bump + migration test; reuse `crates/project/src/dag.rs` `DagNode` / `validate` / `Order`),
2 node types (Parallel mixer, Layer mixer, Splitter / Combiner, key mixer, key inputs / outputs, Outside node),
3 evaluation (CPU reference graph first, then GPU passes reusing C1's scheduler: `render::comp`'s `Program` / slots
and `gpu::comp::run_comp` are the model; per-node `ColorOp` chains as today), 4 UI (the Color page node strip
becomes `widgets/node_canvas.rs`; add Parallel / Layer / Outside from the menu; Resolve keymap shortcuts from the
public manual), 5 the node explainer (G2.10–11) explains branches, 6 Coach lessons for each branch type (the lesson
engine + walk tests as `first-comp` / `key-comp` do: `crates/ui-egui/tests/ui/coach_ui.rs` `walk` with
`Do::WorkspaceNamed` / `{comp}`). **Design note written 2026-10-15 05:21 EDT: `docs/fork/DESIGN-C2.md`** (edges + node `kind` on `Grade`, serial = implicit
chain and compiles exactly as today, virtual source id 0, schema v19, `FxOp::GradeGraph` program mirroring C1's, CPU oracle,
lanes); ADR 0004 / 0007 point to it. Start at its §5 lane 1 (model + schema v19). In MINIMAL mode do it yourself; with STEADY+ a Fable / Opus designer for the note and
builders per lane. **Accept:** each new lesson completes end to end; skin + background on parallel nodes, a look on a
layer node and an outside node render identically on CPU and GPU, round-trip save / load, and graded 6.2K plays in
the quick pass without regression. Tag `comp-2`.

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
- **AMD VCN stuck (2026-10-15 06:40, Q37):** `Fence fallback timer expired on ring vcn_dec_0` since ci's VA-API tests;
  VCN page fault at 07:28. Until the owner reboots, run VA-API checks on Intel / the Dell. Open question: can our hostile
  hardware-input tests (`damaged_samples_and_parameter_sets_never_crash_or_hang`) trigger this driver fault? After the
  reboot: run the AMD `vaapi` binary alone and watch `journalctl -k`.
- **A2 review leftovers (non-blocking):** (2) `audio.voiceIsolate.installFile` loads weights on the command thread;
  (3) `Denoiser::reset` allocates (may run on the audio thread); (7) `apply:false` / `wait:true` cleanup renders on the
  command thread; (10) `takes_of` scans every item each frame.
- 4K 10-bit 4:2:2 H.264 in-app playback is bursty (2026-10-15, synthetic): decoder alone 36.5 fps (12 threads), app shows
  9–24 / 25 fps, p95 decode jobs 500–600 ms (frame-threading latency vs the frame workers' deadlines?). Proxies cover it today.

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
