# YOUTUBE-PLAN.md: closing the gaps for everyday YouTube editing

Written 2026-10-11 (session 8, with the owner). The owner asked for a plan to fix three gaps found
when checking FilmCraft against YouTube work: **H.264 footage** (OBS recordings, Canon `MVI_*.MP4`,
phone clips) has no in-process hardware decode, **variable-frame-rate** clips are untested, and
there are **no third-party plugins or Fusion-style compositing**. Every FORK-PLAN rule applies
(never crash, `Tick` time, everything is a command, layering, `unsafe` only in `crates/platform`
and `crates/gpu/src/interop.rs`, `cargo xtask ci` per phase, a build per phase per §1a.9).

**Order (owner, 2026-10-11):** G0 → G1 → 6 → **Y1** → **6E** ([`EXPORT-PLAN.md`](EXPORT-PLAN.md)) → G2 → **Y3**
→ 7 (with the **real-edit check, Y4**) → 8 → **Y2** → H264-PLAN (H1–H3). Y2 moved after Phase 8
(owner): the owner's current clips are constant frame rate; VFR matters for phone footage.
The owner's clips checked 2026-10-11: OBS records HEVC 1080p30 (already on the fast path); the
Canon camera records **4K23.976 H.264 High**; all three were constant frame rate. Y1 moved after
Phase 6 and is limited to 8 hours (owner); the rest of complete H.264 support is H264-PLAN.md, after Phase 8.

---

## Phase Y1: hardware H.264 decode, 8-bit 4:2:0 progressive (hard limit: 8 h)

**Owner (2026-10-11): no more than 8 agent hours.** Everything beyond this (interlaced, High 10 /
High 4:2:2, multithreaded software decode, working with ffmpeg removed) is a separate plan,
[`H264-PLAN.md`](H264-PLAN.md), after the primary plan (after Phase 8).

Both GPUs (`vainfo`, 2026-10-11) decode H.264 Constrained Baseline / Main / High, 8-bit 4:2:0;
that covers the owner's Canon (4K23.976 High), OBS and phone H.264.

1. `crates/platform/src/va_h264.rs`, mirroring `va_hevc.rs`, fed by `crates/h264`'s parser (SPS /
   PPS / slice headers, POC, reference lists, DPB): Constrained Baseline / Main / High, 8-bit
   4:2:0, **progressive only**, multiple slices. Same frame type, zero-copy display and fallback
   order as HEVC; anything else (interlaced, 10-bit, 4:2:2) keeps today's path (ffmpeg pipe /
   software) with the reason logged. One VA-API code path for AMD and Intel; test both.
2. `// SAFETY:` on every new `unsafe` block; a Fable review of the new FFI, recorded in ADR 0002.
3. Tests: bit-exact against the software decoder on progressive 8-bit conformance vectors and two
   or three camera-like fixtures (4K23.976 / 4K30 High, 1080p60); seek and reorder tests.
4. Quick bench: 4K H.264 play-full, pipe (before) vs in-process (after).

**Time box:** if 8 hours run out, ship what works behind the fallback, and move the rest to
H264-PLAN.md with a note in the progress log.

**Accept:** 4K23.976 / 4K30 High H.264 plays with zero drops at Full on the hardware path on AMD
(and decodes on Intel); bit-exact on the progressive 8-bit set; out-of-scope streams fall back
cleanly; review done. Tag `yt-1`.

## Phase Y2: variable-frame-rate footage (≈ 8–20 h)

The MP4 demuxer already reads per-sample durations (`crates/isobmff/src/demux.rs` `stts` / `ctts`).
What isn't known is whether everything above it respects them.

1. **Audit first** (scout + Fable diagnostician): every place that maps a frame index to time or
   back by multiplying with a nominal frame rate (media index, frame server, GOP cache, proxies,
   thumbnails, waveforms, multicam sync, export). Write the findings into the progress log.
2. **Fixtures:** ffmpeg-generated VFR clips that behave like phones and OBS (frame durations
   jittering ±1 ms around 33.3 ms; drops to 15 fps in low light; a 60 → 30 fps switch mid-clip;
   long clips of 20+ minutes for drift), MP4 and MKV, with a known audio click track.
3. **Rule:** media time comes from the container's timestamps, never from index × nominal fps.
   A VFR clip shows as VFR in Info / Properties, with its average and min / max frame rate.
   On a CFR sequence, the frame shown at a sequence time is the source frame whose timestamp
   span covers it (repeat / skip as needed, never drift). Optional **Interpret as CFR** and
   **Conform to sequence rate** commands for people who want the old behaviour.
4. **Audio stays locked to picture:** A/V offset measured with the click track at the start and
   end of a 20-minute VFR clip, in playback and in export.

**Accept:** on every VFR fixture, A/V offset ≤ 1 frame at the end of 20 minutes, in playback and
export; no stutter in play-full beyond the source's own frame spacing; Info shows VFR. Tag `yt-2`.

## Phase Y3: compositing and extensibility (≈ 25–50 h)

Most YouTube compositing is green screen, screen replacement, tracked text / blur and simple
motion graphics. Those come first. A full Fusion-style node compositor and native plugins are
bigger and partly conflict with the fork's rules, so they're scoped separately (Y3.6, deferred).

1. **Keys on the GPU:** Ultra Key / chroma key and luma key as GPU ops (they're CPU-only today,
   FORK-PLAN §7 Phase 4.3d), spill suppression, matte cleanup (choke / soften), "show matte".
   Oracle: the existing CPU key implementations.
2. **Attach to a track:** use the existing tracker (`crates/render/src/track.rs`) to drive any
   layer's position / scale / rotation (text, stickers, callouts), and tracked **blur / mosaic**
   for faces, plates and screens (privacy; very common on YouTube).
3. **Screen replacement:** a 4-point track with the existing tracker driving the existing Corner
   Pin, so a phone or monitor screen can be replaced.
4. **Custom shader effects (the safe plugin route):** users drop a WGSL file with annotated
   parameters into the effects folder; FilmCraft validates it with naga, builds its controls
   from the annotations, runs it in the GPU effect pass with a time limit, and disables it with
   a clear message if it fails. No native code, so the never-crash rule holds. Ship a handful of
   examples (glitch, chromatic aberration, VHS, light leak).
5. **Look and template packs:** import / export of effect presets, LUT folders and graphic
   templates (`crates/engine/src/graphic_templates.rs`) as one shareable pack file.
6. **Deferred (needs the owner):** (a) a node-based compositor (Fusion-style) built on the grade
   node graph once parallel / layer nodes exist (GRADE-PLAN §6); (b) **OpenFX plugins**: native
   C plugins break the pure-Rust and never-crash rules unless hosted in a **separate process**
   with shared-memory frames, so a plugin crash can't take FilmCraft down. That's a large,
   `unsafe`-heavy project; don't start it without the owner.

**Accept:** a green-screen 4K clip keys in real time at Full on the GPU; text attached to a
tracked point follows it over 5 s; a tracked face blur holds; a screen replacement holds;
a WGSL example effect loads, renders on AMD and Intel, and a broken one is rejected with a
message and no crash. Tag `yt-3`.

## Y4: the real-edit check (part of Phase 7's acceptance, ≈ 3–6 h)

Using the owner's own non-X-M5 clips read-only (`~/Videos/OBS Recordings`, `~/Videos/MVI_*.MP4`;
never committed; results marked "real footage, not X-M5"), driven through the control channel /
MCP: import, cut a 10-minute video with J/L-cuts and B-roll, grade with the node grader,
transcribe and burn in / export captions (SRT), loudness to −14 LUFS with music ducking, a tracked
blur, a 9:16 Short cut from it, export both with the YouTube presets. Record every bug, slowdown
and missing feature in the progress log and fix the crash- or data-loss ones before tagging
Phase 7.

## Rough timeline

| Phase | Agent hours |
|---|---|
| Y1 hardware H.264 decode (hard limit) | ≤ 8 |
| Y2 VFR | 8–20 |
| Y3 compositing / extensibility | 25–50 |
| Y4 real-edit check | 3–6 |
| **Total** | **≈ 45–85** |

Subagents as in GRADE-PLAN §7 (Fable reviewer on every diff, Fable diagnostician for the VFR audit
and any H.264 decode mismatch).
