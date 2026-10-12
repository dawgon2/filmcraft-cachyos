# Export: presets, hardware H.265 and quality

**[FilmCraft] [Fork]** · turning a finished sequence into a file: picking a preset, how the
H.265 (HEVC) encoder is chosen and when it falls back, bitrate versus constant quality, and what
happens to HDR sequences

## Why

**[FilmCraft]**

Exporting is the last step of every edit: FilmCraft renders each frame of the sequence (with every
grade and effect) and hands it to an encoder that writes the file. The two choices that matter
most are **which preset** you start from (it sets the format, size and quality in one go) and
**how the size of the file is decided** (a target bitrate, or a constant quality). Everything
else has a sensible default. If you know Premiere, the Export page works the way its Export mode
does: settings on the left, a preview in the middle, a queue on the side.

## Open the Export page

**[FilmCraft]**

1. Click **Export** in the header {ui:header.mode.export}, or choose File ▸ Export ▸
   **Media…** (`file.exportMedia`; Ctrl+M).
2. The settings column shows **File Name** {ui:export.fileName}, **Location**
   {ui:export.location}, **Preset** {ui:export.preset} and **Format** {ui:export.format},
   then the Video, Audio and Effects sections.
3. When the summary looks right, click **Export** {ui:export.button} to export now, or
   **Send to Queue** {ui:export.sendToQueue} to collect several exports and run them later
   with **Start Queue** {ui:export.queue.start} (`export.queue.start`).

## Choose a preset

**[FilmCraft] [Fork]**

A preset is a saved set of export settings. Start from the one closest to where the file is
going, then change only what you need (the Preset box then says "Custom"). The fork adds five
H.265 presets for 6.2K footage to upstream's list:

| Preset | What it is for |
|---|---|
| **YouTube 4K HEVC** | uploads to video sites: 3840×2160, Main 10, Rec. 709, about 80 Mbps |
| **6.2K Master HEVC** | a full-size master to keep: 6240×4160, Main 10, QuickTime, about 200 Mbps, uncompressed 24-bit audio |
| **Archive HEVC (x265, quality)** | the smallest file at high quality when time doesn't matter: software x265, constant quality 18 |
| **Vertical 2160×3840 HEVC** | phone-shaped (9:16) uploads, the picture scaled to fill |
| **H.265 / HEVC 2160p 4K 10-bit** | a general 4K Main 10 file (upstream's list, kept) |

1. Open the **Preset** list {ui:export.preset} and pick one. Favourites are at the top.
2. To keep your own settings, open **More presets…** {ui:export.preset.more}, type a name in
   **Save current settings as** {ui:presetManager.saveName} and click **Save**
   {ui:presetManager.save} (`export.presets.save`). `export.presets.list` lists every preset.

## How H.265 is encoded, and when it falls back

**[Fork]**

On this machine H.265 is normally encoded by the graphics card's **video engine** through
VA-API, inside FilmCraft, with the rendered pictures going straight into the encoder's memory
(see [hardware.md](hardware.md)). It is fast enough to export 6.2K at around 18–20 frames per
second. When it can't be used, the export doesn't fail: it moves to the next encoder.

The **Encoder** choice {ui:export.video.hevcEncoder} (Video section, only shown for H.265):

| Choice | What happens |
|---|---|
| **Auto (hardware if available)** | the video engine in FilmCraft; if it refuses the size or profile, or fails part-way, the system ffmpeg's VA-API encoder carries on, then software x265 |
| **Hardware (VA-API)** | the video engine only; the export stops with a message rather than fall back to software |
| **Software (x265)** | the system ffmpeg's x265 encoder: slower, often a little smaller at the same quality |

1. Leave **Auto** unless you have a reason. Pick **Hardware (VA-API)** when you'd rather know
   than wait for a slow software export; pick **Software (x265)** for archives.
2. After an export, `perf.stats` names the encoder that was actually used.

The software fallbacks run the system ffmpeg as a separate program. The fork plan removes them in
Phase 7 (FilmCraft's own encoders become the fallback), so treat them as a safety net, not a
feature.

## Bitrate or constant quality

**[Standard] [FilmCraft]**

An encoder can aim for a **size** (a bitrate: so many megabits per second) or for a **quality**
(it spends as many bits as each moment needs). A bitrate is predictable, which upload sites and
players like; constant quality gives an even look and a file size you only learn afterwards.

The **Bitrate Encoding** list {ui:export.video.bitrateMode}:

- **CBR**: the same bitrate all the time. Rarely needed.
- **VBR, 1 pass**: an average target with a ceiling; more bits for busy shots, fewer for still
  ones. The right choice for uploads.
- **VBR, 2 pass**: H.264 only (a hardware H.265 encoder makes one pass, so it is greyed out).
- **CRF (constant quality)**: a quality number from 0 to 51 in **Quality (CRF)**
  {ui:export.video.crf}; lower is better and larger. 18 looks the same as the source to most
  eyes; 22–23 is a good everyday value. x265 reads it as its CRF; the video engine reads it as a
  fixed quantiser (QP), which looks similar but sizes differently.

**If you made presets in the fork before the first upstream merge:** they had no mode list,
only a CRF number. They still load: a preset with a CRF number and no mode opens as **CRF**, one
without a number opens with its bitrate.

## HDR sequences export as Main 10

**[Fork] [Standard]**

H.265 comes in two everyday profiles: **Main** (8-bit) and **Main 10** (10-bit). 8 bits are
plenty for ordinary (SDR) video, but stretched over HDR's much wider brightness range they show
visible steps (banding) in skies and gradients, so HDR is always 10-bit.

1. A sequence is HDR when Sequence ▸ **Color Management…** (`sequence.colorSettings`) sets its
   working space to Rec. 2100 PQ or HLG (see [color/color-management.md](color/color-management.md)).
2. Exporting such a sequence to H.265 writes **Main 10** with PQ or HLG signalling, even when the
   **Profile** box {ui:export.video.hevcProfile} says Main (8-bit). The dialog says so under the
   Profile box: "HDR export: written as Main 10 (10-bit)" {ui:export.video.hevcMain10Note}.
3. When no encoder here can write HDR H.265, the sequence is tone-mapped and exported as ordinary
   8-bit SDR instead; the note isn't shown then.
4. For an SDR copy of an HDR sequence, export with the `sdr` setting of `file.exportMedia` (the
   dialog has no switch for it yet).
