# Hardware: what runs where

**[Fork] [FilmCraft]** · which chip does which job on this machine, what happens when it can't,
and the switches that change it

## Why

**[Fork]**

A 6.2K, 10-bit HEVC clip is far too heavy to decode on the CPU at playback speed. This fork
moves the heavy jobs to the graphics card's **video engine** (on the AMD RX 6700 XT, the "VCN")
and keeps the pictures on the graphics card from decoding to the screen, and from the timeline
to the exported file. You don't have to set anything up: it is on by default and quietly falls
back to slower paths when it can't be used, so a file never becomes unplayable because of the
hardware. Knowing what runs where helps when something is slower than you expect.

## The jobs and where they run

**[Fork]**

| Job | Normally runs on | Falls back to |
|---|---|---|
| Decoding H.265 (HEVC) 4:2:0, 8- or 10-bit (X-M5 main setting) | AMD video engine, through VA-API, inside FilmCraft | a second VA-API path (upstream's), then the system ffmpeg using the same video engine, then FilmCraft's own software decoder |
| Decoding H.264 | AMD video engine (VA-API) | FilmCraft's own software decoder |
| Decoding H.265 4:2:2 (X-M5 occasional setting) | not possible on this graphics card (see below) | — |
| Showing the picture, compositing layers, the colour grade | AMD graphics card (Vulkan) | the CPU compositor (much slower) |
| Exporting H.265 | AMD video engine (VA-API), frames rendered straight into the encoder | the system ffmpeg's VA-API encoder, then software x265 |
| Rendering H.265 export pictures | AMD graphics card, straight into the encoder's memory | CPU rendering when the export is HDR or burns in an overlay, timecode, name or the video limiter, or the picture is letterboxed |
| Exporting other formats (ProRes, H.264, image sequences…) | CPU encoders; pictures rendered on the CPU unless **GPU Rendering** is ticked | CPU rendering |

**Zero-copy** means a decoded picture never travels back to main memory: the video engine
writes it into graphics memory, the compositor reads it from there, and on export the
compositor writes into the encoder's own memory. This is what makes 6.2K playback and export
fast. If the driver refuses it, FilmCraft copies the pictures instead (slower, same result)
and writes one line in the log.

### The Intel graphics

**[Fork]**

The Intel UHD 750 in the CPU is a second, smaller graphics device. FilmCraft doesn't use it on
its own; it is there as a spare. Every colour feature is tested on both the AMD and the Intel
device so the picture is the same on either, and an advanced switch (below) lets export encode
on the Intel video engine while the AMD one keeps decoding.

### 4:2:2 clips

**[Fork]**

The AMD video engine can't decode 4:2:2 H.265 at all; this is a limit of the chip, and Resolve
has the same limit on this card. Today such a clip imports, plays its sound, and shows an
"unreadable" slate saying why. Phase 6 adds 4:2:2 to FilmCraft's own decoder (on the CPU, many
threads) and automatic proxies, so these clips edit smoothly; this page will be updated then.

## The switches

**[FilmCraft] [Fork]**

### Preferences

| Where | Label | Id | What it does |
|---|---|---|---|
| Preferences ▸ Playback | **Hardware decoding**: Auto / Off | {ui:settings.playback.hardwareDecoding} | Auto (default) uses the video engine for the streams it can take, FilmCraft's own decoder for the rest. Off decodes everything in software (slow at 6.2K; useful only to rule out a driver problem). Clips already open keep their decoder until reopened. |
| Preferences ▸ Memory | **Decoded frames in GPU memory (0 = Auto)** | {ui:settings.memory.vramCacheMb} | how much graphics memory keeps recently decoded frames, so scrubbing back and forth doesn't decode again |
| Preferences ▸ Memory | **Decoded source frames (0 = Auto)** | {ui:settings.memory.decodeCacheMb} | the same for frames held in main memory |
| Export dialog, Video, H.265 | **Encoder**: Auto (hardware if available) / Hardware (VA-API) / Software (x265) | {ui:export.video.hevcEncoder} | Auto uses the video engine when it can take the size and profile, else x265. Hardware refuses to export rather than fall back. |
| Export dialog | **GPU Rendering**: Composite on the GPU when available | {ui:export.gpuRendering} | off by default (upstream's choice: a CPU export is byte-identical on every machine). Tick it to render other formats' pictures on the graphics card, much faster on effects-heavy edits. H.265 exports use the graphics card whether or not it is ticked. |

To change a setting:

1. Choose Edit ▸ Preferences ▸ **Playback…** (`app.settings.playback`; **Memory…** is
   `app.settings.memory`). The Preferences window opens on that category.
2. Change the control.
3. Press **OK** {ui:settings.ok} to keep it (**Cancel** {ui:settings.cancel} drops the change).
4. For Hardware decoding, close and reopen the project (or restart) so open clips pick it up.

The Program monitor's **Select Playback Resolution** menu {ui:program.resolution} (Full, 1/2,
1/4 …) is the other lever: lower resolutions decode and composite less. **[FilmCraft]**

### Environment variables (advanced)

**[Fork]**

Set these before starting FilmCraft (for example `FILMCRAFT_NO_VA_ENCODE=1 filmcraft`). They
are for troubleshooting; normally leave them unset.

| Variable | Effect |
|---|---|
| `FILMCRAFT_VAAPI_DEVICE=/dev/dri/renderD129` | which graphics device decodes (here renderD129 is the AMD card, renderD128 the Intel) |
| `FILMCRAFT_VAAPI_ENCODE_DEVICE=/dev/dri/renderD128` | encode H.265 exports on another device's video engine (e.g. the Intel), so decoding and encoding don't share one |
| `FILMCRAFT_NO_VA_ENCODE=1` | don't use the in-process hardware encoder; export through the system ffmpeg instead |
| `FILMCRAFT_NO_ZERO_COPY=1` | export copies frames back to main memory instead of rendering into the encoder |
| `FILMCRAFT_NO_GPU_IMPORT=1` | playback copies decoded frames instead of handing them to the compositor in graphics memory |
| `FILMCRAFT_FFMPEG=/path/to/ffmpeg` | which ffmpeg program the fallback paths run |

## When something is slow

**[Fork]**

1. Check the clip: 4:2:2 clips can't use the video engine (above).
2. Check the log (or the status bar) for a line saying a path was switched off, such as
   "switched to software compositing" or "GPU acceleration is unavailable; preview renders on
   the CPU". Those mean a fallback is in use.
3. Lower the Program monitor's playback resolution and see whether playback becomes smooth.
4. Make sure Preferences ▸ Playback ▸ Hardware decoding is **Auto**.

## If you know Resolve

**[Resolve-style]**

Resolve's "Decode H.264/H.265 using hardware acceleration" preference corresponds to
Hardware decoding: Auto. Like Resolve on Linux with this AMD card, there is no hardware 4:2:2
decoding; the answer is the same (proxies or optimized media).

## If you know Premiere

**[FilmCraft]**

Hardware decoding and GPU Rendering correspond to Premiere's hardware-accelerated decoding and
Mercury Playback Engine GPU acceleration; the menu labels follow upstream FilmCraft.
