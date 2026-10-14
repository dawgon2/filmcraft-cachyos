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

## Review of the unsafe / FFI code (FORK-PLAN §1a.5, 2026-10-08)

Three review passes over every `unsafe` block and FFI boundary (`crates/platform/src/va.rs`,
`va_hevc.rs`, `va_sys.rs` loading, `crates/gpu/src/interop.rs`), after the `phase-2` tag (the
instruction arrived late; the tag was not moved). Each fix has a test.

**Fixed.**
- *Round 1:* parameter buffers are typed (`Context::param_buffer` over a sealed set of VA
  structs) instead of reading padded structs as `&[u8]` (UB); a `Buffer` borrows its `Context`
  (destroyed first, never submitted to another context); `Context` is `!Sync`; slice data offsets
  checked against the NAL; read-back rectangle and row arithmetic checked; spare surfaces reused
  oldest first; Vulkan import requires Vulkan 1.1 and checks the modifier, plane count, sampled
  feature, max extent, importability and the layer fourccs. An fd of −1 panicked in
  `BorrowedFd::borrow_raw` before (`va_interop::hostile_dmabuf_descriptions_are_refused`).
- *Round 2:* a `DmaBuf` owns its buffer objects (`Arc<[OwnedFd]>`; planes name an object index),
  so no description can carry a closed or foreign fd; only single-memory-plane layers are
  imported; the buffer size comes from the kernel (seek to the end of the DMA-BUF), not libva's
  report. HEVC parameters the driver trusts are refused when hostile: more than 15 references,
  list entries past the filled slots, tile counts beyond VA's arrays or the picture, explicit tile
  sizes that don't fit (`va_hevc::tests::hostile_tile_sizes_are_refused`; u32 overflow panicked in
  debug builds before).
- *Round 3:* `Context::decode` refuses a target surface of another display or smaller than the
  context (the driver would write past it; `va::tests::a_context_refuses_a_smaller_target_surface`);
  `Surface::read` converts coordinates to libva's `int` with a check instead of `as`.

**Checked and kept.** libva is loaded once into a never-unloaded static with every symbol
required (`--dynamic-link-require-all`); function pointers never outlive the library. Every
libva object (display, config, context, buffer, surface, image, mapping) is RAII and destroyed
once, on error paths too; exported fds are owned immediately after `vaExportSurfaceHandle`.
`Display: Send + Sync` relies on libva / Mesa serialising per display; contexts are not shared.
Pictures are synced (`vaSyncSurface`) before they are handed out, so sampling follows the
decoder's write. Vulkan takes the dup'd fd only on success (closed by us otherwise); image and
memory are destroyed on every failure path; wgpu-hal owns them afterwards (`Dedicated`).

**Residual risks (accepted, documented).**
- The imported image starts in `UNDEFINED` and wgpu moves it to the sampled layout; Vulkan allows
  that transition to discard contents. It keeps them for the uncompressed single-plane modifiers
  RADV and Intel export (tested every frame: in place == read back); compressed modifiers are
  refused. The strictly correct form is an acquire from `VK_QUEUE_FAMILY_FOREIGN_EXT`, which wgpu
  does not expose.
- A surface goes back to the decoder's pool when the last frame holding it drops; GPU work already
  submitted may still sample it while the decoder overwrites it (oldest-first reuse makes this
  unlikely). This can only show a wrong picture, never touch freed memory: Vulkan holds its own
  reference on the DMA-BUF.
- The bindings are generated from the installed libva headers; a libva with a different ABI for
  the same soname would be a libva bug.

## Encoding (fork Phase 5.2b, 2026-10-09)

HEVC export encodes in process through the same runtime-loaded libva (VA-API primary, §1a.4):
`va_sys.rs` now also carries `va_enc_hevc.h` and the encode buffer types (regenerated with
`crates/platform/gen-va-sys.sh`, which reproduces the decode-only file exactly when the encode
types are left out). FilmCraft writes VPS / SPS / PPS and slice headers itself
(`filmcraft_hevc::write`, checked by parsing them back) and gives them to the driver as packed
headers; the driver writes the slice data. All `unsafe` stays in `va.rs`:

- `Context::encode_hevc` fills the surface ids and the coded buffer into the picture and slice
  parameters itself, after checking that source, reconstruction and reference surfaces belong to
  the display and hold a whole picture, and that the coded buffer is this context's; picture and
  slice parameter buffers are not `ParamBuffer`s, so no other path can hand the driver ids.
- `CodedBuffer` owns its buffer (not borrowed from the context, so an encoder keeps several in
  flight; libva buffers do not depend on their context once made); `read` walks at most 64
  segments, bounds each by the buffer's size and refuses overflowed or unaligned segments.
- `Surface::write` maps a `vaCreateImage` image (checked to be at least the surface's size) and
  copies it in with `vaPutImage`; the caller fills the mapping through bounds-checked slices.
- Misc parameters are `repr(C)` pairs of the `type` word and the payload (`Misc<T>`), a sealed
  `ParamBuffer` like the decode parameters.

`va_enc.rs` (safe code) keeps three pictures in flight; the export falls back to ffmpeg
`hevc_vaapi` → `libx265` (without B-frames) mid-stream, re-encoding the frames the driver had not
finished, from an IDR picture.

## Zero-copy encode input (fork Phase 5.2c, 2026-10-10)

The export renders into the encoder's own input surfaces: `Surface::export_writable` exports a
surface with `VA_EXPORT_SURFACE_READ_WRITE` (cached per surface; a writable export replaces a
read-only one), and `interop::Importer::import_target` imports its luma and chroma layers as
UNORM `COLOR_ATTACHMENT` images (the modifier must list `COLOR_ATTACHMENT` for the format; the
same image-format and external-memory queries as the sampled import). wgpu renders the samples;
the encoder thread waits for that submission on the CPU (`VaPicture::ready`) before
`vaBeginPicture`, and every frame dropped unencoded waits too, so neither the encoder surface nor
the decoder surfaces its plan samples are reused while the GPU still uses them.

**Residual risk (write direction).** The rendered image is never released to
`VK_QUEUE_FAMILY_FOREIGN_EXT` nor moved to a layout the video engine is promised to read. That
the encoder sees the writes after the CPU wait is observed behaviour of RADV and ANV for
uncompressed modifiers (layouts are the memory layout itself; caches are flushed at the end of a
submission), not a Vulkan guarantee. Compressed modifiers, including single-plane ones (Intel
CCS, AMD DCC, AMD GFX12), are refused (`compressed_modifier`), for imports in both directions.
Byte-exact parity tests on AMD (RADV + radeonsi) and Intel (ANV + iHD) guard the behaviour; a
driver that cannot render into the surfaces makes the export read frames back instead.

## Review of the encode FFI (fork Phase 5 acceptance, 2026-10-10)

An independent review (a separate agent, not the author) of every `unsafe` block and FFI boundary
added since `phase-4` (`va.rs` encode, `interop.rs` targets, and the callers they rely on):
nothing critical, no reachable memory-safety bug. Fixed with tests where testable:

- `CodedBuffer::read` takes `&mut self` (the type is `Sync`: two threads could map one buffer).
- `VaPicture` and `VaHevcEncoder` wait for outstanding GPU / VA work on drop (cancel, failure);
  a lost GPU submission stays lost (a never-written surface is never encoded).
- `vaMaxNum*` counts above the array cap are refused instead of clamped (libva writes the
  driver's real count); more than 64 coded segments is an error, not a truncated picture;
  segments are read unaligned.
- `encode_hevc` checks the surfaces' render-target format against the config and forces one
  reference per list (`num_ref_idx_*_minus1 = 0`); sequence parameters are no longer a
  `ParamBuffer` but go through `Context::sequence_buffer`, which checks the declared picture size
  against the context.
- `fill_semiplanar` refuses a driver chroma offset beyond the image (was a panic).
- Imports of freed memory are dropped on every call (an export's surfaces stayed pinned in VRAM
  until the next import); `import_target` refuses read-only exports; `Misc<T>` sizes are checked
  at compile time.
- Found while re-testing on Intel: the mid-stream fallback to ffmpeg `hevc_vaapi` did not turn
  B-frames off (`-bf 0`; Intel's encoder reorders), so the stream's order broke.

Accepted: `va_sys.rs` has no bindgen layout tests (generated from the system headers by
`gen-va-sys.sh` for this ABI; the structs passed by size are the generated ones).

## Update: one VA-API stack (fork U0.2, 2026-10-13)

The fork's own in-process HEVC decoder (`va_hevc.rs`, with `crates/hevc/src/accel.rs` driving it)
is gone. Every in-process H.264 / HEVC decode now runs through upstream's front ends
(`vaapi/h264.rs`, `vaapi/hevc.rs`) on the fork's display layer (`crate::va::Display`). Pictures are
either left on the GPU and imported as described above (`va_accel::ZeroCopyAccel`), handed to the
compositing GPU as linear copies (4:2:2, or decoded on the second GPU, optionally scaled to the size
drawn), or read back (`ZeroCopyAccel::new_read_back`). Upstream's own libva loader, `Display` and
`Session` (`vaapi/va.rs`) are removed, so the process opens one VA display per render node. The
hostile-input checks of the old decoder (tile sizes, entry points, zero-data slices) live in the
front end (`vaapi/tests.rs`).

