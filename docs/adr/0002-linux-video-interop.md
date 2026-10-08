# ADR 0002: In-process VA-API decoding with zero-copy GPU frames on Linux (fork)

- **Status:** accepted for the fork (2026-10-08). The owner approved in-process libva and Vulkan
  Video with `unsafe` confined as in ADR 0001 (`FORK-PLAN.md` §1, §1a.2) and set the rule that
  picks the backend (§1a.4).
- **Context documents:** `FORK-PLAN.md` Phase 2, `docs/fork-benchmarks.md`.

## Context

On Linux the fork decodes HEVC on the GPU through the system ffmpeg as a child process
(`crates/platform/src/vaapi.rs`): every 6.2K 10-bit picture (78 MB) is downloaded from VRAM,
written through a pipe, read back and uploaded to the GPU again. That path tops out at about
20 fps at 6.2K (ffmpeg's serial download + pipe write; Phase 1 measurements), while the video
engine (VCN 3.0) decodes the same clip at 43 fps with the frames left on the GPU. The goal of
Phase 2 is that decoded pictures never leave VRAM: the compositor samples them where the
decoder wrote them.

Two in-process routes were considered (FORK-PLAN Phase 2.1):

- **A. Vulkan Video** (`VK_KHR_video_decode_h265` on RADV): decode on the same Vulkan device
  wgpu renders with, into images the compositor can sample.
- **B. VA-API + DMA-BUF**: decode with libva, export each surface as DRM PRIME
  (`vaExportSurfaceHandle`) and import it into the wgpu Vulkan device
  (`VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier`).

### The comparison (owner's rule: VA-API primary unless Vulkan Video is ≥ 20 % faster and matches)

ffmpeg n9.0.1, Mesa 26.2.2, RX 6700 XT, synthetic 6240×4160 HEVC Main10 4:2:0 29.97p, 200
Mbit/s, 1798 frames, `-f null`, frames left on the GPU, 2026-10-08, load ≈ 2:

| backend | command | fps (3 runs) |
|---|---|---|
| VA-API | `-hwaccel vaapi -hwaccel_device /dev/dri/renderD129 -hwaccel_output_format vaapi` | 43.2, 43.2, 43.2 |
| Vulkan Video | `-init_hw_device vulkan=vk:0 -hwaccel vulkan -hwaccel_device vk -hwaccel_output_format vulkan` | 40.9, 40.8, 41.1 |

Correctness: `framemd5` of the first 60 pictures downloaded as P010: VA-API and Vulkan Video
match each other 60/60, and both match ffmpeg's software decoder 60/60. **Synthetic.**

Vulkan Video is 5 % *slower*, so it does not meet the 20 % condition: **VA-API (route B) is the
primary backend.** Vulkan Video becomes the backup in Phase 8.

## Decision

1. **Decoder: libva, loaded at run time.** `crates/platform/src/va/` declares the libva API with
   bindings generated once by bindgen from the system headers (libva 2.24, MIT) in its
   dynamic-loading form, committed as `sys.rs` (no bindgen or libclang needed to build), and
   opens `libva.so.2` / `libva-drm.so.2` with `libloading` (ISC) at first use. A machine without
   libva builds and runs; the backend then reports itself unavailable and the ffmpeg pipe and
   the built-in decoder take over. (`cros-libva` was not used: it links libva at build time, so
   the app would not start without it.)
2. **Bitstream parameters from FilmCraft's own HEVC parser** (`crates/hevc`: VPS/SPS/PPS, slice
   headers, short- and long-term RPS, POC, DPB). The VA-API backend fills
   `VAPictureParameterBufferHEVC`, `VAIQMatrixBufferHEVC` and `VASliceParameterBufferHEVC` from it,
   keeps the DPB (reference surfaces by POC), and outputs pictures in presentation order with the
   same bumping rules as the software decoder, so pts, reordering, seeks, flushes and RASL
   handling stay identical.
3. **Frames stay on the GPU.** A decoded picture is a VA surface held by a lease; it becomes a
   new `PixelData` variant carrying a handle (`Arc<dyn GpuSurface>`, defined in `filmcraft-frame`
   with no OS types) with its format (P010 / NV12), size and colour, and an explicit, fallible
   "download to CPU" for paths that still need pixels (CPU effects, the CPU oracle, scopes until
   Phase 4). Dropping the last reference returns the surface to the decoder's pool.
4. **Interop: DMA-BUF into the wgpu Vulkan device.** The app creates its wgpu device itself
   (wgpu-hal `vulkan::Adapter::open_with_callback`, adding `VK_KHR_external_memory_fd`,
   `VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier` and their dependencies)
   and hands it to eframe (`egui_wgpu::WgpuSetup::Existing`). The compositor imports each
   exported surface's planes as `R16Unorm` / `Rg16Unorm` (P010) or `R8Unorm` / `Rg8Unorm` (NV12)
   `VkImage`s with the surface's DRM format modifier, wraps them as wgpu textures
   (`texture_from_raw`) and samples them with the biplanar path added in Phase 1. Imports are
   cached per surface (surfaces are pooled and reused). That code needs `unsafe` with `ash`; it
   lives in **one module, `crates/gpu/src/interop.rs`**, the only place in `filmcraft-gpu` with
   `#[allow(unsafe_code)]`, under the same containment rules as ADR 0001 (every block has a
   `// SAFETY:` comment, safe public API, no panics across FFI, compiles to a no-op elsewhere).
   Without the extensions (other drivers, the web build) the compositor downloads the surface
   through the fallible CPU path instead.
5. **Backend interface.** `crates/platform` exposes the hardware decoders through a
   `HwBackend` description (name, which codecs/profiles/chroma formats/bit depths it takes, how to
   make a decoder, whether its frames are GPU-resident) and registers the available ones in a
   fixed priority list: VA-API now; Vulkan Video in Phase 8; NVIDIA NVDEC can be added later as
   another entry without touching the callers. `HybridDecoder` keeps the fallback guarantee of
   ADR 0001: primary → ffmpeg pipe → built-in software decoder, switching mid-stream when the
   primary errors, with the reason logged and counted. `perf.stats` reports the active backend.
6. **VRAM budget.** GPU-resident pictures count against a VRAM budget (Settings; default half
   the VRAM) in the GOP cache, separate from the RAM budget of Phase 1.

## Consequences

- No pipe, no download, no CPU conversion on the hot path: the decoder's CPU cost is parsing
  and submitting (≈ 1 ms per picture), and throughput is bounded by VCN.
- The wgpu device is now created by FilmCraft on Linux/Vulkan. If our device creation fails
  (missing extensions, another backend), the app falls back to eframe's default device and the
  CPU-download path; nothing else changes.
- More `unsafe` than before (libva calls in `crates/platform`, Vulkan import in
  `crates/gpu/src/interop.rs`). The parity tests (bit-exact against the software decoder after
  download), damaged-input tests and the fallback tests run on every Linux CI pass with a GPU.
- Vulkan Video remains possible as a second backend on the same interface (Phase 8), and is the
  natural one where VA-API is missing.
