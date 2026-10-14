# Hardware: what runs where

**[Fork] [FilmCraft]** · which chip does which job on this machine, what happens when it can't,
and the switches that change it

## Why

> id: hardware.why | does: why 6.2K 10-bit HEVC needs the graphics card's video engine | resolve: GPU decoding | premiere: hardware-accelerated decoding | when: heavy camera footage | see: hardware.jobs

**[Fork]**

A 6.2K, 10-bit HEVC clip is far too heavy to decode on the CPU at playback speed. This fork
moves the heavy jobs to the graphics card's **video engine** (on the AMD RX 6700 XT, the "VCN")
and keeps the pictures on the graphics card from decoding to the screen, and from the timeline
to the exported file. You don't have to set anything up: it is on by default and quietly falls
back to slower paths when it can't be used, so a file never becomes unplayable because of the
hardware. Knowing what runs where helps when something is slower than you expect.

## The jobs and where they run

> id: hardware.jobs | does: which jobs run on the AMD or Intel video engines or the processor, and the fallbacks | ui: program.resolution | resolve: GPU processing mode | premiere: Mercury Playback Engine | when: checking what runs where | see: hardware.switches

**[Fork]**

| Job | Normally runs on | Falls back to |
|---|---|---|
| Decoding H.265 (HEVC) 4:2:0, 8- or 10-bit (X-M5 main setting) | AMD video engine, through VA-API, inside FilmCraft | a second VA-API path (upstream's), then the system ffmpeg using the same video engine, then FilmCraft's own software decoder |
| Decoding H.264 (Constrained Baseline / Main / High, 8-bit 4:2:0, progressive) | AMD video engine, through VA-API, inside FilmCraft, zero-copy ("VA-API H.264 (in process)" in the performance stats; fork Y1) | the same VA-API decode with the picture copied through memory, then the system ffmpeg, then FilmCraft's own software decoder |
| Decoding other H.264 (High 10, 4:2:2, interlaced) | the system ffmpeg, while it is still in the chain (FilmCraft's own decoders cover these in the H264-PLAN phases) | — |
| Decoding H.265 4:2:2 10-bit (X-M5 occasional setting) | Intel video engine (VA-API), picture handed to the AMD card (see below) | the same Intel decode with the picture copied through memory, then FilmCraft's own software decoder |
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

The Intel UHD 750 in the CPU is a second, smaller graphics device. Its video engine decodes
4:2:2 H.265, which the AMD one can't (next section); otherwise it is a spare. Every colour feature is tested on both the AMD and the Intel
device so the picture is the same on either, and an advanced switch (below) lets export encode
on the Intel video engine while the AMD one keeps decoding.

### 4:2:2 clips

**[Fork]**

The AMD video engine can't decode 4:2:2 H.265 at all; this is a limit of the chip. The Intel
video engine can, so FilmCraft decodes 4:2:2 clips there and hands each picture straight to
the AMD card (a shared buffer, no copy through main memory; about 21 pictures a second at
6.2K on this PC, measured on test clips). Why both: the AMD card does all the compositing
and grading, the Intel engine only decodes.

1. Import the clip. No badge means the hardware path is in use.
2. If the Project panel shows an amber **4:2:2** badge ("4:2:2: software decode, proxies
   recommended"), FilmCraft is using its own software decoder instead: no Intel engine was
   found, or it failed on this clip earlier in the session (the badge's tooltip says so).
   Software decoding is slow at 6.2K.
3. Either way, FilmCraft makes a half-size proxy of each 4:2:2 clip in the background as
   soon as it is imported, and playback can use it; see [proxies.md](proxies.md).

If the hand-off to the AMD card is refused by the driver, the pictures are copied through
memory instead (slower, same result) and one line is written to the log.

## The switches

> id: hardware.switches | does: the settings that turn hardware decoding, GPU rendering and caches on or off | ui: settings.playback.hardwareDecoding, settings.playback.graphicsDevice, settings.memory.*, export.gpuRendering | cmd: app.settings.playback, app.settings.memory | resolve: Decode using hardware acceleration | premiere: Hardware decoding, GPU acceleration | when: testing a fallback, a slow machine | see: hardware.slow

**[FilmCraft] [Fork]**

### Preferences

| Where | Label | Id | What it does |
|---|---|---|---|
| Preferences ▸ Playback | **Hardware decoding**: Auto / Off | {ui:settings.playback.hardwareDecoding} | Auto (default) uses the video engine for the streams it can take, FilmCraft's own decoder for the rest. Off decodes everything in software (slow at 6.2K; useful only to rule out a driver problem). Clips already open keep their decoder until reopened. |
| Preferences ▸ Playback | **Graphics device**: Auto / one of the computer's GPUs | {ui:settings.playback.graphicsDevice} | which graphics chip draws the interface and the monitors (see [Linux: which GPU draws FilmCraft](linux.md#which-gpu-draws-filmcraft)). Takes effect at the next start. |
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

> id: hardware.slow | does: steps to find out why playback or export is slow | ui: program.resolution | resolve: render cache, proxy media | premiere: playback resolution, proxies | when: dropped frames, slow export | see: proxies.what, render-cache.why

**[Fork]**

1. Check the clip: a 4:2:2 clip with the amber badge is decoded in software (above).
2. Check the log (or the status bar) for a line saying a path was switched off, such as
   "switched to software compositing" or "GPU acceleration is unavailable; preview renders on
   the CPU". Those mean a fallback is in use.
3. Lower the Program monitor's playback resolution and see whether playback becomes smooth.
4. Make sure Preferences ▸ Playback ▸ Hardware decoding is **Auto**.

## If you know Resolve

> id: hardware.resolve | does: how these switches map to Resolve's hardware preferences | resolve: Decode H.264/H.265 using hardware acceleration | when: you come from Resolve | see: hardware.switches

**[Resolve-style]**

Resolve's "Decode H.264/H.265 using hardware acceleration" preference corresponds to
Hardware decoding: Auto. Resolve on Linux with this AMD card decodes 4:2:2 H.265 in software;
FilmCraft uses the Intel video engine for it instead, and proxies (like Resolve's optimized
media) remain the fallback.

## If you know Premiere

> id: hardware.premiere | does: how these switches map to Premiere's hardware decoding and GPU rendering | premiere: hardware-accelerated decoding, Mercury GPU | when: you come from Premiere | see: hardware.switches

**[FilmCraft]**

Hardware decoding and GPU Rendering correspond to Premiere's hardware-accelerated decoding and
Mercury Playback Engine GPU acceleration; the menu labels follow upstream FilmCraft.
