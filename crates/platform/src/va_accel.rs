//! Zero-copy output for upstream's VA-API front ends (fork Y1): an [`Accel`] over the fork's libva
//! layer ([`crate::va`]) whose decoded pictures stay on the GPU. Each frame it hands out is a
//! [`VaPicture`] holding its surface ([`PixelData::Gpu`]: the compositor imports the surface's
//! DMA-BUF, the CPU paths read it back on demand), so a surface is free for the next picture only
//! once the DPB *and* every frame have let go of it: [`ZeroCopyAccel::pick`] skips the held ones
//! (`Arc::strong_count` above one) and adds a surface from the pool instead. What the GOP cache's
//! GPU-memory budget can hold of them ([`ZeroCopyAccel::max_slots`]) is a soft bound: frames are
//! also held outside the cache (an export's lanes and encoder queue), so past it the set keeps
//! growing while the driver allocates, up to [`MAX_SLOTS`], as the fork's HEVC decoder did (U0.2.3b:
//! a hard stop at the budget failed every export picture over to the read-back decoder, 34 → 8 fps
//! for 4K). Surface indexes never change, so the front end's DPB references stay valid as the set
//! grows.
//!
//! HEVC Main 4:2:2 10 (fork U0.2.3, ported from the fork's 6.1b decoder `va_hevc`) decodes on the
//! display whose driver has that profile (the thread's decode node when it has it, else
//! [`Display::for_hevc_422`]: Intel's media engine here; AMD's VCN has none) into Y210 surfaces,
//! with VA's range-extension buffers. Each picture is then copied by that engine's video
//! processor into a linear surface ([`LinearCopy`]) and handed out as a GPU frame
//! (`GpuFormat::Y210`), which the compositing GPU imports as a DMA-BUF (Intel → AMD hand-off,
//! FORK-PLAN §1a.30); without the hand-off (`FILMCRAFT_NO_GPU_IMPORT`, no video processor, a
//! failed copy or export: logged once) pictures are read back to `Yuv16Bi` 4:2:2 frames.
//!
//! Every frame keeps the decoding context alive (iHD's `vaSyncSurface` reads it). Grey stand-ins
//! for missing references are written once per surface and reused while nothing else has been
//! written into it ([`Accel::pick_gray`]: 78 MB of CPU writes per stand-in at 6.2K 10-bit).
//!
//! Safe code: every libva call it needs exists in `va.rs`. Refused (so the read-back decoder
//! takes the stream): `FILMCRAFT_NO_GPU_IMPORT` (4:2:0), a display without the profile, a
//! surface whose DMA-BUF cannot be exported (4:2:0).

use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_frame::{Chroma, PixelData, VideoFrame, pool};

use crate::biplanar::Geometry;
use crate::va::{Config, Context, Display, LinearCopy, Surface, SurfacePool};
use crate::va_sys as sys;
use crate::vaapi::{Accel, ffi};

/// Most surfaces an accelerator ever grows to, whatever the budget (a guard; in practice the
/// driver runs out of memory first). Beyond it a picture has no surface and the hybrid decoder
/// falls back.
pub const MAX_SLOTS: usize = 4096;

/// Largest picture taken (VCN tops out at 8192×4352; libva at 16K).
const MAX_DIM: u32 = 16384;

/// Render-target format and surface fourcc of pictures of `chroma_format_idc` (1: 4:2:0, 2:
/// 4:2:2) and `bits`.
fn format(chroma_format_idc: u32, bits: u32) -> Result<(u32, u32), String> {
    match (chroma_format_idc, bits) {
        (1, 8) => Ok((sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12)),
        (1, 10) => Ok((sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010)),
        (2, 10) => Ok((sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210)),
        _ => Err(format!("chroma format {chroma_format_idc} at {bits}-bit is not decoded zero-copy")),
    }
}

/// Bytes per row of the luma and the interleaved chroma plane of a `w`-wide NV12 / P010 image;
/// of the one packed plane of a Y210 image (and 0).
fn row_bytes(fourcc: u32, w: u32) -> Result<[usize; 2], String> {
    let w = w as usize;
    let cw = w.div_ceil(2);
    if fourcc == sys::VA_FOURCC_Y210 {
        return Ok([cw.checked_mul(8).ok_or("row size overflows")?, 0]);
    }
    let per = if fourcc == sys::VA_FOURCC_P010 { 2 } else { 1 };
    Ok([w.checked_mul(per).ok_or("row size overflows")?, cw.checked_mul(2 * per).ok_or("row size overflows")?])
}

/// The display a stream of `chroma_format_idc` decodes on: a two-GPU export lane's own node
/// (fork 6E.3; for 4:2:2 only when that GPU decodes it), else the process's display (4:2:0) or
/// the 4:2:2 display ([`Display::for_hevc_422`]). From `va_hevc::display_for` (fork 6.1b).
fn display_for(chroma_format_idc: u32, node: Option<&str>) -> Result<Arc<Display>, String> {
    if let Some(node) = node {
        let d = Display::for_node(node)?;
        if chroma_format_idc != 2 || d.supports(sys::VAProfile_VAProfileHEVCMain422_10, sys::VAEntrypoint_VAEntrypointVLD) {
            return Ok(d);
        }
    }
    if chroma_format_idc == 2 { Display::for_hevc_422() } else { Display::get() }
}

/// The 4:2:2 hand-off state of one stream (fork 6.1b, moved here in U0.2.3).
#[derive(Default)]
struct Handoff {
    /// The copier into linear surfaces (made on the first picture, remade when the size changes).
    copier: Option<LinearCopy>,
    /// Pictures are read back instead (`FILMCRAFT_NO_GPU_IMPORT`, the hand-off failed for this
    /// stream, or [`ZeroCopyAccel::set_handoff`]).
    off: bool,
    /// Test hook: the next hand-off fails as a refused copy / export would.
    fail_next: bool,
    /// The copies are scaled by this (U0.2.3b d, [`filmcraft_codecs::hw::handoff_scale`]):
    /// 1.0 = the visible size.
    scale: f32,
}

/// The size a `w`×`h` picture is handed off at, scaled by `s` (even, at least 2, at most the
/// picture's size).
fn scaled_size((w, h): (u32, u32), s: f32) -> (u32, u32) {
    if !(s > 0.0 && s < 1.0) {
        return (w, h);
    }
    let one = |v: u32| ((((v as f32 * s) / 2.0).round() as u32).saturating_mul(2)).clamp(2.min(v), v);
    (one(w), one(h))
}

/// The hardware side of one stream, pictures left on the GPU (see the module docs).
pub struct ZeroCopyAccel {
    // (field order is drop order: the surfaces go before the context they were decoded in, see
    // `Drop`)
    /// The surfaces, by index; a frame handed out holds a clone of its entry.
    slots: Vec<Arc<Surface>>,
    /// The accelerator's own pool (not the process's shared one): a surface decoded in this
    /// context is synced and reused by this context only. Intel's driver (iHD) looks a surface's
    /// pending decode up through the context it was decoded in, so a spare that came back to a
    /// shared pool from a decoder since dropped crashes the next `vaSyncSurface` on it (the
    /// pool's `get`, a frame's read-back).
    pool: Arc<SurfacePool>,
    /// The decoding context, shared with every frame handed out (U0.2.2): iHD's `vaSyncSurface`
    /// reads the context a surface was decoded in, so a frame read back after its decoder was
    /// dropped (the GOP cache keeps frames) used freed driver memory and crashed under load
    /// (seen with several decoders on several threads). The context now goes with the last
    /// frame. Only this accelerator decodes through it (the lock is never contended); frames
    /// only keep it alive.
    context: Arc<Mutex<Context>>,
    display: Arc<Display>,
    /// `slots`' ids ([`Accel::surfaces`]).
    ids: Vec<ffi::VASurfaceID>,
    /// When each slot was last decoded or filled into ([`ZeroCopyAccel::pick`] reuses the one
    /// written longest ago).
    used: Vec<u64>,
    /// Slots that are mid-grey and unwritten since (U0.2.3): a stand-in for a missing reference
    /// reuses one instead of writing the grey again.
    gray: Vec<bool>,
    /// Stand-ins asked for ([`Accel::fill`] with `None`) and grey images written for them
    /// (tests: stand-ins reuse their surfaces).
    gray_fills: u64,
    gray_writes: u64,
    seq: u64,
    /// The surfaces made at the start (the DPB, the picture being decoded, a spare).
    initial: usize,
    geometry: Geometry,
    /// 4:2:2 (Y210 surfaces): frames are linear copies handed to the compositing GPU, or read
    /// back (U0.2.3).
    handoff: Option<Handoff>,
    /// Pictures are read back to memory instead of handed out on the GPU
    /// ([`ZeroCopyAccel::new_read_back`]).
    cpu: bool,
}

impl ZeroCopyAccel {
    /// A decoder for the first of `profiles` the display has, with `count` surfaces (coded
    /// `size`, `chroma_format_idc` 1 or 2 at `geometry.bits`) to start with; output pictures are
    /// cut to `geometry`. 4:2:0 decodes on the process's display ([`Display::get`]; on a two-GPU
    /// export lane, the thread's decode node), 4:2:2 on the display that has Main 4:2:2 10
    /// ([`display_for`]). The reasons it declines are the read-back decoder's cue to take the
    /// stream.
    pub fn new(profiles: &[ffi::VAProfile], size: (u32, u32), count: usize, geometry: Geometry, chroma_format_idc: u32) -> Result<Self, String> {
        Self::with(profiles, size, count, geometry, chroma_format_idc, false)
    }

    /// [`ZeroCopyAccel::new`] whose pictures are read back to memory (U0.2.4: the read-back
    /// decoder, for a compositor that cannot import the surfaces; replaces upstream's `Session`).
    /// 4:2:0 only.
    pub fn new_read_back(profiles: &[ffi::VAProfile], size: (u32, u32), count: usize, geometry: Geometry) -> Result<Self, String> {
        Self::with(profiles, size, count, geometry, 1, true)
    }

    fn with(profiles: &[ffi::VAProfile], size: (u32, u32), count: usize, geometry: Geometry, chroma_format_idc: u32, cpu: bool) -> Result<Self, String> {
        let packed = chroma_format_idc == 2;
        if cpu && packed {
            return Err("HEVC 4:2:2 is not decoded by the read-back decoder".into());
        }
        let no_import = std::env::var_os("FILMCRAFT_NO_GPU_IMPORT").is_some();
        if no_import && !packed && !cpu {
            return Err("zero-copy display is off (FILMCRAFT_NO_GPU_IMPORT)".into());
        }
        let (rt_format, fourcc) = format(chroma_format_idc, geometry.bits)?;
        if size.0 == 0 || size.1 == 0 || size.0 > MAX_DIM || size.1 > MAX_DIM {
            return Err(format!("picture size {}x{}", size.0, size.1));
        }
        // a lane of a two-GPU export (fork 6E.3) decodes on its own GPU, as the fork's HEVC
        // decoder did (U0.2.2); 4:2:2 on a display that has the profile (U0.2.3)
        let display = display_for(chroma_format_idc, filmcraft_codecs::hw::decode_node().as_deref())?;
        let vld = sys::VAEntrypoint_VAEntrypointVLD;
        let profile = profiles
            .iter()
            .copied()
            .find(|&p| display.supports(p, vld))
            .ok_or_else(|| format!("the VA-API driver on {} has no decoder for the stream's profile", display.node))?;
        let config = Arc::new(Config::new(&display, profile, vld, rt_format)?);
        let pool = SurfacePool::new(&display, rt_format, fourcc, size.0, size.1);
        let count = count.clamp(1, 64);
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            slots.push(Arc::new(pool.get()?));
        }
        let ids: Vec<ffi::VASurfaceID> = slots.iter().map(|s| s.id).collect();
        let context = Arc::new(Mutex::new(Context::new(&config, size.0, size.1)?));
        if packed {
            log::info!("HEVC 4:2:2: decoding on {} ({})", display.node, display.vendor);
        } else if !cpu {
            // zero-copy needs the surfaces' DMA-BUFs: a driver that cannot export them (or exports
            // something the compositor cannot import) is found out now, not at the first frame
            // (4:2:2 hands out linear copies instead, made and checked per picture)
            slots.first().ok_or("no surface")?.export().map_err(|e| format!("DMA-BUF export: {e}"))?;
        }
        // diagnostic (U0.2.3b d, `FILMCRAFT_DECODE_NODE`): 4:2:0 decoded on another GPU's media
        // engine is handed to the compositing GPU the same way
        let handoff_420 = !packed && !cpu && filmcraft_codecs::hw::decode_hands_off();
        if handoff_420 {
            log::info!("HEVC / H.264 4:2:0: decoding on {} ({}), handing pictures to the compositing GPU", display.node, display.vendor);
        }
        let scale = filmcraft_codecs::hw::handoff_scale();
        let handoff = (packed || handoff_420).then(|| Handoff { off: no_import, scale, ..Handoff::default() });
        Ok(Self {
            used: vec![0; slots.len()],
            gray: vec![false; slots.len()],
            gray_fills: 0,
            gray_writes: 0,
            slots,
            pool,
            context,
            display,
            ids,
            seq: 0,
            initial: count,
            geometry,
            handoff,
            cpu,
        })
    }

    /// Bytes of one surface (NV12 / P010 / Y210 at the coded size).
    fn surface_bytes(&self) -> usize {
        let (w, h) = self.pool.size;
        let px = (w as usize).saturating_mul(h as usize);
        match self.pool.fourcc {
            sys::VA_FOURCC_Y210 => px.saturating_mul(4),
            sys::VA_FOURCC_P010 => px.saturating_mul(3).saturating_add(1) / 2 * 2,
            _ => px.saturating_mul(3).saturating_add(1) / 2,
        }
    }

    /// The surfaces the GOP cache's GPU-memory budget accounts for: the decoder's own plus as many
    /// as the budget holds of them (at most [`MAX_SLOTS`]). A soft bound since U0.2.3b: frames
    /// held outside the cache (an export's queues) let the set grow past it ([`Self::pick`]).
    pub fn max_slots(&self) -> usize {
        let (_, budget) = filmcraft_codecs::gop::vram_usage();
        (budget / self.surface_bytes().max(1)).saturating_add(self.initial).min(MAX_SLOTS)
    }

    /// The driver's description.
    pub fn vendor(&self) -> &str {
        &self.display.vendor
    }

    /// Whether the driver decodes pictures predicted from grey stand-ins for missing references
    /// (surfaces written from memory, never decoded into). Intel's iHD does not: it fails each
    /// such HEVC picture with "internal decoding error" (fork U0.2.3 hardware run), so the front
    /// end refuses those pictures before the driver sees them (Intel's engine is not handed
    /// pictures known to fail, 6.1b). Mesa (AMD's VCN) decodes them.
    pub fn decodes_from_stand_ins(&self) -> bool {
        !self.display.vendor.contains("Intel")
    }

    /// The render node the accelerator decodes on.
    pub fn node(&self) -> &str {
        &self.display.node
    }

    /// Surfaces the accelerator has (grows while frames hold them; tests).
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Stand-ins for missing references made so far, and the grey images written for them
    /// (a stand-in reuses a surface still grey from an earlier one; tests).
    pub fn gray_stats(&self) -> (u64, u64) {
        (self.gray_fills, self.gray_writes)
    }

    /// 4:2:2: hand pictures to the compositing GPU (`on`, the default unless
    /// `FILMCRAFT_NO_GPU_IMPORT` is set) or read them back to memory. Nothing for 4:2:0.
    pub fn set_handoff(&mut self, on: bool) {
        if let Some(h) = self.handoff.as_mut() {
            h.off = !on;
            if !on {
                h.copier = None;
            }
        }
    }

    /// Test hook: the next 4:2:2 hand-off fails (as a refused copy or export); the stream reads
    /// pictures back from then on.
    pub fn fail_next_handoff(&mut self) {
        if let Some(h) = self.handoff.as_mut() {
            h.fail_next = true;
        }
    }

    fn slot(&self, index: usize) -> Result<&Arc<Surface>, String> {
        self.slots.get(index).ok_or_else(|| "surface out of range".to_string())
    }

    /// Slot `index` was written now (decoded into, or filled with `gray`).
    fn touch(&mut self, index: usize, gray: bool) {
        self.seq = self.seq.wrapping_add(1);
        if let Some(u) = self.used.get_mut(index) {
            *u = self.seq;
        }
        if let Some(g) = self.gray.get_mut(index) {
            *g = gray;
        }
    }

    /// Slot `index` is about to be written with something other than grey: a failed write must
    /// not leave it marked grey, or [`Accel::pick_gray`] would hand out whatever the driver left
    /// in it as a stand-in.
    fn not_gray(&mut self, index: usize) {
        if let Some(g) = self.gray.get_mut(index) {
            *g = false;
        }
    }

    /// Every sample of the surface mid-grey (128 in NV12, 512 in P010 / Y210), as the software
    /// decoders make the frames they never decoded.
    fn write_gray(&mut self, target: &Surface) -> Result<(), String> {
        let wide = self.pool.fourcc != sys::VA_FOURCC_NV12;
        self.gray_writes = self.gray_writes.saturating_add(1);
        target.write(|bytes, _, _| {
            if wide {
                // 16-bit little-endian, MSB-aligned: 0x8000 is code 512 of 10 bits
                for p in bytes.as_chunks_mut::<2>().0 {
                    *p = 0x8000u16.to_le_bytes();
                }
            } else {
                bytes.fill(0x80);
            }
            Ok(())
        })
    }

    /// `target` a copy of `from` (through memory: frame_num gap stand-ins are rare, a seek into
    /// an open GOP).
    fn copy(&self, from: &Surface, target: &Surface) -> Result<(), String> {
        let (w, h) = self.pool.size;
        let rows = row_bytes(self.pool.fourcc, w)?;
        let mut planes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
        from.read((0, 0, w, h), |plane, _, row| {
            if let Some(p) = planes.get_mut(plane) {
                p.extend_from_slice(row);
            }
        })?;
        target.write(|bytes, offsets, pitches| {
            for (plane, data) in planes.iter().enumerate() {
                let (Some(&len), Some(&off), Some(&pitch)) = (rows.get(plane), offsets.get(plane), pitches.get(plane)) else { continue };
                if len == 0 {
                    continue;
                }
                for (r, src) in data.chunks_exact(len).enumerate() {
                    let start = r.checked_mul(pitch).and_then(|o| o.checked_add(off)).ok_or("image offset overflows")?;
                    let dst = start.checked_add(len).and_then(|end| bytes.get_mut(start..end)).ok_or("the image is smaller than the surface")?;
                    dst.copy_from_slice(src);
                }
            }
            Ok(())
        })
    }

    /// A decoded 4:2:2 picture (or 4:2:0 decoded on another GPU, `FILMCRAFT_DECODE_NODE`) copied
    /// into a linear surface of its format and exported (fork 6.1b hand-off), or
    /// `None` (read it back instead): the hand-off is off, or failed now (then off for the
    /// stream).
    fn hand_off(&mut self, decoded: &Arc<Surface>) -> Option<VideoFrame> {
        let (cx, cy, cw, ch) = self.geometry.crop;
        let display = self.display.clone();
        let (rt_format, fourcc) = (self.pool.rt_format, self.pool.fourcc);
        let ten_bit = self.geometry.bits > 8;
        let h = self.handoff.as_mut()?;
        // the compositor refused such a picture this session (logged there): no copy at all
        if h.off || (fourcc == sys::VA_FOURCC_Y210 && filmcraft_frame::packed_422_refused()) {
            return None;
        }
        let out = scaled_size((cw, ch), h.scale);
        let made = if std::mem::take(&mut h.fail_next) {
            Err("test hook".to_string())
        } else {
            if h.copier.as_ref().is_none_or(|c| c.size != out) {
                h.copier = None;
                match LinearCopy::new(&display, rt_format, fourcc, out.0, out.1, handoff_alloc()) {
                    Ok(c) => h.copier = Some(c),
                    Err(e) => {
                        h.off = true;
                        handoff_failed(&e);
                        return None;
                    }
                }
            }
            h.copier.as_ref().ok_or_else(|| "no copier".to_string()).and_then(|c| c.copy_scaled(decoded, (cx, cy, cw, ch), out))
        };
        match made {
            Ok(linear) => {
                let keep = keep_decoded(&linear);
                if keep && out != (cw, ch) {
                    // read back from the decoded surface would be at full size: hand off unscaled
                    log::info!("VA-API: handed-off pictures cannot be mapped; hand-off copies stay unscaled");
                    h.scale = 1.0;
                    h.copier = None;
                    return None;
                }
                let read_from = keep.then(|| (decoded.clone(), (cx, cy)));
                let context: Arc<dyn std::any::Any + Send + Sync> = self.context.clone();
                let pic = VaPicture { surface: Arc::new(linear), origin: (0, 0), ten_bit, read_from, visible: Some(out), _context: Some(context) };
                let mut f = self.frame(PixelData::Gpu(Arc::new(pic)));
                (f.width, f.height) = out;
                Some(f)
            }
            Err(e) => {
                h.copier = None;
                h.off = true;
                handoff_failed(&e);
                None
            }
        }
    }

    fn frame(&self, data: PixelData) -> VideoFrame {
        let (_, _, cw, ch) = self.geometry.crop;
        VideoFrame { width: cw, height: ch, data, color: self.geometry.color, par: self.geometry.par, pts: filmcraft_time::Tick::ZERO }
    }
}

impl Drop for ZeroCopyAccel {
    /// Wait for every slot's decode while the context still exists (Intel's driver looks pending
    /// work up through it; see [`ZeroCopyAccel::pool`]). Output pictures were synced when read;
    /// this covers the reference pictures a decoder dropped mid-stream (a seek, an evicted
    /// clip) never output. Then the fields drop in order: the slots back to the pool, the pool
    /// (its spares destroyed; surfaces frames still hold keep it until they are dropped), the
    /// context (destroyed once no frame holds it either, after the frames' surfaces).
    fn drop(&mut self) {
        for s in &self.slots {
            if let Err(e) = s.sync() {
                log::debug!("VA-API zero-copy: syncing a surface on drop: {e}");
            }
        }
    }
}

impl Accel for ZeroCopyAccel {
    fn surfaces(&self) -> &[ffi::VASurfaceID] {
        &self.ids
    }

    fn decode_h264(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferH264,
        iq: &ffi::VAIQMatrixBufferH264,
        slices: &[(ffi::VASliceParameterBufferH264, Vec<u8>)],
    ) -> Result<(), String> {
        // no longer grey from here on, even if the submission fails part-way (U0.2.3 review)
        self.not_gray(target);
        let ctx = self.context.lock().unwrap_or_else(PoisonError::into_inner);
        let mut buffers = Vec::with_capacity(2 + 2 * slices.len());
        buffers.push(ctx.param_buffer(pic)?);
        buffers.push(ctx.param_buffer(iq)?);
        for (sp, data) in slices {
            buffers.push(ctx.param_buffer(sp)?);
            buffers.push(ctx.slice_data(data)?);
        }
        let s = self.slot(target)?;
        ctx.decode(s, &buffers)?;
        drop(buffers);
        drop(ctx);
        self.touch(target, false);
        Ok(())
    }

    fn decode_hevc(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVC,
        iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVC, Vec<u8>)],
    ) -> Result<(), String> {
        // no longer grey from here on, even if the submission fails part-way (U0.2.3 review)
        self.not_gray(target);
        let ctx = self.context.lock().unwrap_or_else(PoisonError::into_inner);
        let mut buffers = Vec::with_capacity(2 + 2 * slices.len());
        buffers.push(ctx.param_buffer(pic)?);
        if let Some(iq) = iq {
            buffers.push(ctx.param_buffer(iq)?);
        }
        for (sp, data) in slices {
            buffers.push(ctx.param_buffer(sp)?);
            buffers.push(ctx.slice_data(data)?);
        }
        let s = self.slot(target)?;
        ctx.decode(s, &buffers)?;
        drop(buffers);
        drop(ctx);
        self.touch(target, false);
        Ok(())
    }

    /// Range-extension profiles (Main 4:2:2 10, fork U0.2.3): base + rext in one picture- and
    /// one slice-parameter buffer each.
    fn decode_hevc_rext(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVCExtension,
        iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVCExtension, Vec<u8>)],
    ) -> Result<(), String> {
        // no longer grey from here on, even if the submission fails part-way (U0.2.3 review)
        self.not_gray(target);
        let ctx = self.context.lock().unwrap_or_else(PoisonError::into_inner);
        let mut buffers = Vec::with_capacity(2 + 2 * slices.len());
        buffers.push(ctx.param_buffer(pic)?);
        if let Some(iq) = iq {
            buffers.push(ctx.param_buffer(iq)?);
        }
        for (sp, data) in slices {
            buffers.push(ctx.param_buffer(sp)?);
            buffers.push(ctx.slice_data(data)?);
        }
        let s = self.slot(target)?;
        ctx.decode(s, &buffers)?;
        drop(buffers);
        drop(ctx);
        self.touch(target, false);
        Ok(())
    }

    fn read(&mut self, index: usize) -> Result<VideoFrame, String> {
        let s = self.slot(index)?.clone();
        // the decode has finished when the frame is handed out
        let clock = filmcraft_export::trace::start();
        s.sync()?;
        filmcraft_export::trace::add(filmcraft_export::trace::DEC_SYNC, clock);
        let (cx, cy, cw, ch) = self.geometry.crop;
        if self.cpu {
            return Ok(self.frame(read_back(&s, (cx, cy, cw, ch))?));
        }
        if self.handoff.is_some() {
            // 4:2:2 (or decoded on another GPU): a linear copy for the compositing GPU, else
            // read back to memory
            let clock = filmcraft_export::trace::start();
            let handed = self.hand_off(&s);
            filmcraft_export::trace::add(filmcraft_export::trace::DEC_COPY, clock);
            if let Some(f) = handed {
                return Ok(f);
            }
            let data = read_back(&s, (cx, cy, cw, ch))?;
            return Ok(self.frame(data));
        }
        let context: Arc<dyn std::any::Any + Send + Sync> = self.context.clone();
        let pic = VaPicture::new(s, (cx, cy), self.geometry.bits > 8, Some(context));
        Ok(self.frame(PixelData::Gpu(Arc::new(pic))))
    }

    fn fill(&mut self, index: usize, from: Option<usize>) -> Result<(), String> {
        let target = self.slot(index)?;
        // a surface a frame still shows is never written (the same rule as `pick`)
        if Arc::strong_count(target) > 1 {
            return Err(format!("surface {index} is still held by a frame"));
        }
        let target = target.clone();
        if from.is_none() {
            self.gray_fills = self.gray_fills.saturating_add(1);
        }
        match from {
            // still grey from an earlier stand-in: nothing to write (U0.2.3)
            None if self.gray.get(index).copied().unwrap_or(false) => {}
            None => {
                // nothing may still be decoding into it while the image is copied in
                self.not_gray(index);
                target.sync()?;
                self.write_gray(&target)?;
            }
            Some(f) => {
                self.not_gray(index);
                target.sync()?;
                let src = self.slot(f)?.clone();
                self.copy(&src, &target)?;
            }
        }
        self.touch(index, from.is_none());
        Ok(())
    }

    /// The free candidate no frame holds that was written longest ago (its last frame has had
    /// the longest for GPU work that sampled it to finish), else a surface added from the pool
    /// (index = the new last; past [`ZeroCopyAccel::max_slots`] too, U0.2.3b), else `None` at
    /// [`MAX_SLOTS`] or when the driver has no memory for another.
    fn pick(&mut self, candidates: &[usize]) -> Option<usize> {
        let free = candidates.iter().copied().filter(|&i| self.slots.get(i).is_some_and(|s| Arc::strong_count(s) == 1));
        if let Some(i) = free.min_by_key(|&i| self.used.get(i).copied().unwrap_or(0)) {
            return Some(i);
        }
        if self.slots.len() >= MAX_SLOTS {
            log::warn!("VA-API zero-copy: all {MAX_SLOTS} surfaces are held by frames or references");
            return None;
        }
        if self.slots.len() == self.max_slots() {
            log::debug!("VA-API zero-copy: past the GOP cache's budget ({} surfaces): frames held elsewhere", self.slots.len());
        }
        match self.pool.get() {
            Ok(s) => {
                self.ids.push(s.id);
                self.slots.push(Arc::new(s));
                self.used.push(0);
                self.gray.push(false);
                Some(self.slots.len() - 1)
            }
            Err(e) => {
                log::warn!("VA-API zero-copy: no surface for the next picture: {e}");
                None
            }
        }
    }

    /// A free candidate that is still grey (no write needed), else [`ZeroCopyAccel::pick`].
    fn pick_gray(&mut self, candidates: &[usize]) -> Option<usize> {
        let still_gray =
            candidates.iter().copied().find(|&i| self.gray.get(i).copied().unwrap_or(false) && self.slots.get(i).is_some_and(|s| Arc::strong_count(s) == 1));
        still_gray.or_else(|| self.pick(candidates))
    }
}

// ------------------------------------------------- frames, the 4:2:2 hand-off, the 4:2:2 probe
// (moved from `va_hevc` in fork U0.2.3, so the fork's decode core can go in U0.2.5)

/// Whether HEVC Main 4:2:2 10 decodes in hardware here, blocking until the display search is
/// done: the Hardware decoding setting is on, a display has the profile
/// ([`Display::for_hevc_422`]) and no 4:2:2 stream has failed in hardware this session.
pub fn hardware_422() -> bool {
    filmcraft_codecs::hw::hardware_decoding() && !filmcraft_codecs::hw::hevc_422_failed() && Display::for_hevc_422().is_ok()
}

/// [`hardware_422`]'s display part without waiting (the UI's probe, registered by
/// `filmcraft_platform::register`): `None` until the search has finished.
pub fn hardware_422_ready() -> Option<bool> {
    Display::hevc_422_ready()
}

/// Search for the 4:2:2 display on a background thread (vaInitialize on each render node), so
/// the UI's first question does not wait for it.
pub fn warm_422_probe() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("filmcraft-va-422-probe".into()).spawn(|| {
            let Some(_busy) = crate::va::Busy::enter() else { return };
            let _ = std::panic::catch_unwind(Display::for_hevc_422);
        });
        if let Err(why) = spawned {
            log::info!("VA-API 4:2:2 probe not started ahead of time: {why}");
        }
    });
}

/// Linear memory for the 4:2:2 hand-off when the VA driver does not make linear surfaces itself
/// (iHD): exportable system memory from a Vulkan device of the decoder's own (made on first use
/// and kept), which Intel's video processor writes into and the compositor imports. Any linear
/// DMA-BUF imports, whichever device made it; the device is picked by the compositor's policy
/// (the high-performance adapter) but may still be another GPU than the compositor's
/// (`MESA_VK_DEVICE_SELECT`, a different adapter order), which costs nothing more. None: no such
/// device (then the driver's own surfaces are the only try).
pub(crate) fn handoff_alloc() -> Option<crate::va::DmaBufAlloc> {
    static DEVICE: std::sync::OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();
    let (device, _) = DEVICE
        .get_or_init(|| {
            let made = std::panic::catch_unwind(filmcraft_gpu::GpuCompositor::headless_device).ok().flatten();
            made.filter(|(d, _)| filmcraft_gpu::interop::can_import(d))
        })
        .as_ref()?;
    let device = device.clone();
    Some(Arc::new(move |size| filmcraft_gpu::interop::alloc_dmabuf(&device, size)))
}

/// Whether a handed-off picture must keep its decoded surface for CPU read-back: only when its
/// linear copy (over imported memory) cannot be read by mapping its DMA-BUF (probed once per
/// session on the first copy). Otherwise the decoded surface goes back to the pool right after
/// the copy, and read-back maps the copy ([`Surface::read`]).
pub(crate) fn keep_decoded(linear: &Surface) -> bool {
    if !matches!(linear.pool().linear, Some(crate::va::LinearAlloc::Imported(_))) {
        return false;
    }
    static MAPPED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let mapped = *MAPPED.get_or_init(|| match linear.read((0, 0, 2, 1), |_, _, _| {}) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("VA-API HEVC 4:2:2: handed-off pictures cannot be mapped ({e}); they keep their decoded surfaces for reading back");
            false
        }
    });
    !mapped
}

/// Log the first hand-off failure of the session (later ones are as the first).
pub(crate) fn handoff_failed(why: &str) {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::warn!("VA-API HEVC 4:2:2: handing pictures to the GPU failed ({why}); reading them back to memory instead");
    }
}

/// A decoded picture in a VA surface, as a frame's GPU pixels.
pub struct VaPicture {
    /// The memory handed to the GPU (exported): the decoded surface (4:2:0) or its linear copy
    /// (4:2:2 hand-off).
    pub(crate) surface: Arc<Surface>,
    pub(crate) origin: (u32, u32),
    pub(crate) ten_bit: bool,
    /// A linear copy's decoded surface and crop origin, read back for the CPU paths only when the
    /// copy cannot be mapped ([`keep_decoded`]; iHD's vaGetImage of a surface over imported
    /// memory returns zeros). None otherwise: the decoded surface is back in its pool after the
    /// copy, and read-back maps the copy.
    pub(crate) read_from: Option<(Arc<Surface>, (u32, u32))>,
    /// A linear copy's picture size (its surface is wider, `va::linear_width`): the size the
    /// compositor is told the memory has, so sampling clamps at the picture's edge instead of
    /// reading the unwritten padding (U0.2.3b d: the right-most column of a 352-wide picture
    /// took chroma from the padding). None: the surface's coded size.
    pub(crate) visible: Option<(u32, u32)>,
    /// What the surface was decoded in, kept until the frame is dropped (after the surface:
    /// field order) so read-back never syncs a surface whose context is gone (U0.2.2, the
    /// zero-copy accelerator; see `va_accel::ZeroCopyAccel::context`).
    pub(crate) _context: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl VaPicture {
    /// A decoded 4:2:0 picture in `surface` (NV12, or P010 when `ten_bit`), its visible picture
    /// starting at `origin`; read back from the surface itself. The fork's zero-copy accelerator
    /// for upstream's front ends ([`crate::va_accel`], Y1) makes its frames this way.
    /// `context` is kept alive with the frame (the decoding context, see [`VaPicture`]).
    pub(crate) fn new(surface: Arc<Surface>, origin: (u32, u32), ten_bit: bool, context: Option<Arc<dyn std::any::Any + Send + Sync>>) -> Self {
        Self { surface, origin, ten_bit, read_from: None, visible: None, _context: context }
    }
}

impl std::fmt::Debug for VaPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VaPicture(surface {}, {}-bit)", self.surface.id, if self.ten_bit { 10 } else { 8 })
    }
}

impl filmcraft_frame::GpuSurface for VaPicture {
    fn memory_id(&self) -> u64 {
        self.surface.mem().memory_id
    }
    fn liveness(&self) -> std::sync::Weak<()> {
        self.surface.mem().liveness()
    }
    fn format(&self) -> filmcraft_frame::GpuFormat {
        if self.surface.pool().fourcc == sys::VA_FOURCC_Y210 {
            filmcraft_frame::GpuFormat::Y210
        } else if self.ten_bit {
            filmcraft_frame::GpuFormat::P010
        } else {
            filmcraft_frame::GpuFormat::Nv12
        }
    }
    fn coded_size(&self) -> (u32, u32) {
        self.visible.unwrap_or(self.surface.pool().size)
    }
    fn crop_origin(&self) -> (u32, u32) {
        self.origin
    }
    fn dmabuf(&self) -> Option<filmcraft_frame::DmaBuf> {
        match self.surface.export() {
            Ok(e) => {
                let mut d = e.dmabuf.clone();
                if let Some((w, h)) = self.visible {
                    // never larger than the memory (the visible size fits the surface)
                    (d.width, d.height) = (w.min(d.width), h.min(d.height));
                }
                Some(d)
            }
            Err(e) => {
                log::warn!("VA-API surface export failed: {e}");
                None
            }
        }
    }
    fn download(&self, width: u32, height: u32) -> Result<PixelData, String> {
        match &self.read_from {
            Some((decoded, (x, y))) => read_back(decoded, (*x, *y, width, height)),
            None => read_back(&self.surface, (self.origin.0, self.origin.1, width, height)),
        }
    }
    fn gpu_bytes(&self) -> usize {
        let (w, h) = self.surface.pool().size;
        let px = w as usize * h as usize;
        if self.surface.pool().fourcc == sys::VA_FOURCC_Y210 {
            // packed 4:2:2, 16-bit samples: 4 bytes per pixel, plus the decoded surface it holds
            let decoded = self.read_from.as_ref().map_or(0, |(s, _)| (s.pool().size.0 as usize).saturating_mul(s.pool().size.1 as usize).saturating_mul(4));
            return px.saturating_mul(4).saturating_add(decoded);
        }
        (px * 3 / 2) * if self.ten_bit { 2 } else { 1 }
    }
}

/// Read the rectangle (x, y, w, h) of a surface back to memory, by the surface's format:
/// P010 → `Yuv16Bi` 4:2:0, Y210 → `Yuv16Bi` 4:2:2 (both MSB-aligned 10-bit), NV12 → planar `Yuv8`.
pub(crate) fn read_back(surface: &Surface, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> Result<PixelData, String> {
    // before allocating: the rectangle must lie in the surface
    let (sw, sh) = surface.pool().size;
    if cw == 0 || ch == 0 || cx.checked_add(cw).is_none_or(|r| r > sw) || cy.checked_add(ch).is_none_or(|b| b > sh) {
        return Err(format!("rectangle {cw}x{ch} at ({cx}, {cy}) outside the {sw}x{sh} surface"));
    }
    let (w, h) = (cw as usize, ch as usize);
    let (ccw, cch) = (w.div_ceil(2), h.div_ceil(2));
    let fourcc = surface.pool().fourcc;
    Ok(if fourcc == sys::VA_FOURCC_Y210 {
        // one packed plane, per two pixels Y0 Cb Y1 Cr (16-bit little-endian, MSB-aligned), split
        // into a luma plane and a Cb/Cr-interleaved plane at half width, full height (P210)
        let mut y = pool::take_u16_filled(w * h);
        let mut uv = pool::take_u16_filled(ccw * 2 * h);
        surface.read((cx, cy, cw, ch), |_, r, row| {
            let (Some(dy), Some(duv)) = (y.get_mut(r * w..(r + 1) * w), uv.get_mut(r * ccw * 2..(r + 1) * ccw * 2)) else { return };
            for (i, q) in row.as_chunks::<8>().0.iter().enumerate() {
                let s = |k: usize| u16::from_le_bytes([q[2 * k], q[2 * k + 1]]);
                if let Some(d) = dy.get_mut(2 * i) {
                    *d = s(0);
                }
                if let Some(d) = dy.get_mut(2 * i + 1) {
                    *d = s(2);
                }
                if let Some(d) = duv.get_mut(2 * i..2 * i + 2) {
                    d[0] = s(1);
                    d[1] = s(3);
                }
            }
        })?;
        PixelData::Yuv16Bi { y: Arc::new(y), uv: Arc::new(uv), chroma: Chroma::C422, bits: 10 }
    } else if fourcc == sys::VA_FOURCC_P010 {
        let mut y = pool::take_u16_filled(w * h);
        let mut uv = pool::take_u16_filled(ccw * cch * 2);
        surface.read((cx, cy, cw, ch), |plane, r, row| {
            let dst = if plane == 0 { y.get_mut(r * w..(r + 1) * w) } else { uv.get_mut(r * ccw * 2..(r + 1) * ccw * 2) };
            if let Some(dst) = dst {
                for (d, b) in dst.iter_mut().zip(row.as_chunks::<2>().0) {
                    *d = u16::from_le_bytes(*b);
                }
            }
        })?;
        PixelData::Yuv16Bi { y: Arc::new(y), uv: Arc::new(uv), chroma: Chroma::C420, bits: 10 }
    } else {
        let mut y = pool::take_u8(w * h);
        y.resize(w * h, 0);
        let (mut u, mut v) = (pool::take_u8(ccw * cch), pool::take_u8(ccw * cch));
        u.resize(ccw * cch, 128);
        v.resize(ccw * cch, 128);
        surface.read((cx, cy, cw, ch), |plane, r, row| {
            if plane == 0 {
                if let (Some(dst), Some(src)) = (y.get_mut(r * w..(r + 1) * w), row.get(..w)) {
                    dst.copy_from_slice(src);
                }
            } else if let (Some(du), Some(dv)) = (u.get_mut(r * ccw..(r + 1) * ccw), v.get_mut(r * ccw..(r + 1) * ccw)) {
                for ((a, b), p) in du.iter_mut().zip(dv.iter_mut()).zip(row.as_chunks::<2>().0) {
                    *a = p[0];
                    *b = p[1];
                }
            }
        })?;
        PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_of_the_image_planes() {
        assert_eq!(row_bytes(sys::VA_FOURCC_NV12, 640).unwrap(), [640, 640]);
        assert_eq!(row_bytes(sys::VA_FOURCC_NV12, 641).unwrap(), [641, 642]);
        assert_eq!(row_bytes(sys::VA_FOURCC_P010, 640).unwrap(), [1280, 1280]);
        assert_eq!(row_bytes(sys::VA_FOURCC_Y210, 641).unwrap(), [642 * 4, 0]);
        assert!(format(1, 12).is_err());
        assert!(format(2, 8).is_err());
        assert!(format(3, 10).is_err());
        assert_eq!(format(2, 10).unwrap(), (sys::VA_RT_FORMAT_YUV422_10, sys::VA_FOURCC_Y210));
    }

    /// The surface set is bounded by the GOP cache's GPU budget (what holds frames), never below
    /// the decoder's own surfaces, never above the guard. Skipped without VA-API.
    #[test]
    fn the_surface_set_is_bounded_by_the_cache_budget() {
        let profiles = [ffi::VAProfileH264High, ffi::VAProfileH264Main];
        let g = Geometry { crop: (0, 0, 64, 64), bits: 8, color: Default::default(), par: (1, 1) };
        let Ok(a) = ZeroCopyAccel::new(&profiles, (64, 64), 5, g, 1) else { return };
        assert_eq!(a.surface_bytes(), 64 * 64 * 3 / 2);
        let (_, budget) = filmcraft_codecs::gop::vram_usage();
        assert_eq!(a.max_slots(), (budget / (64 * 64 * 3 / 2) + 5).min(MAX_SLOTS));
        assert!(a.max_slots() >= 5);
    }

    /// U0.2.3b: frames held outside the GOP cache (an export's lanes and encoder queue) push the
    /// set past the cache's budget; it keeps growing instead of failing the picture over to the
    /// read-back decoder (export 34 → 8 fps at 4K). 8K surfaces keep the budget small (2 GiB of
    /// 53 MB each). Skipped without VA-API HEVC at 8192×4320.
    #[test]
    fn the_set_grows_past_the_cache_budget_while_frames_are_held() {
        let profiles = [ffi::VAProfileHEVCMain];
        let g = Geometry { crop: (0, 0, 8192, 4320), bits: 8, color: Default::default(), par: (1, 1) };
        let mut a = match ZeroCopyAccel::new(&profiles, (8192, 4320), 2, g, 1) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("no 8K zero-copy HEVC accelerator: {e}; skipped");
                return;
            }
        };
        let max = a.max_slots();
        if max > 64 {
            eprintln!("a GOP budget of {max} 8K surfaces; skipped");
            return;
        }
        let mut held = Vec::new();
        while a.slot_count() < max + 2 {
            let all: Vec<usize> = (0..a.slot_count()).collect();
            let Some(i) = a.pick(&all) else {
                // the budget is no reason to stop; only the driver's memory below it is
                assert!(a.slot_count() < max, "stopped at the budget's {max} surfaces");
                eprintln!("the driver ran out of memory at {} surfaces; skipped", a.slot_count());
                return;
            };
            held.push(a.read(i).expect("frame"));
        }
        assert!(a.slot_count() > max, "grew past the budget's {max} surfaces");
    }

    /// Frames hold their surfaces: a held slot is never picked, the set grows instead, and a
    /// released slot is reused (the one written longest ago first). Skipped without VA-API or an
    /// H.264 decoder.
    #[test]
    fn picks_only_surfaces_no_frame_holds() {
        let profiles = [ffi::VAProfileH264High, ffi::VAProfileH264Main];
        let g = Geometry { crop: (0, 0, 64, 64), bits: 8, color: Default::default(), par: (1, 1) };
        let mut a = match ZeroCopyAccel::new(&profiles, (64, 64), 3, g, 1) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("no zero-copy H.264 accelerator: {e}; skipped");
                return;
            }
        };
        assert_eq!(a.slot_count(), 3);
        // the first picture: the first candidate (nothing written yet); mark the slots written
        // in order 0, 1, 2 (fills, as a frame_num gap would)
        for i in 0..3 {
            a.fill(i, None).expect("fill");
        }
        assert_eq!(a.pick(&[0, 1, 2]), Some(0), "written longest ago");
        let f0 = a.read(0).expect("frame");
        let f1 = a.read(1).expect("frame");
        assert_eq!(a.pick(&[0, 1, 2]), Some(2), "0 and 1 are in frames");
        let f2 = a.read(2).expect("frame");
        assert_eq!(a.pick(&[0, 1, 2]), Some(3), "all held: a surface was added");
        assert_eq!(a.slot_count(), 4);
        assert_eq!(a.surfaces().len(), 4);
        drop(f1);
        assert_eq!(a.pick(&[0, 1, 2]), Some(1), "released");
        assert_eq!(a.pick(&[0, 2]), Some(4), "still held: added again");
        assert_eq!(a.pick(&[0, 1, 2, 3, 4]), Some(3), "never written: before the released one");
        drop((f0, f2));
        assert_eq!(a.pick(&[0, 1, 2]), Some(0), "written longest ago");
        // the frames read back what was filled in
        let f = a.read(0).expect("frame");
        let PixelData::Gpu(s) = &f.data else { panic!("a GPU frame") };
        let PixelData::Yuv8 { planes, .. } = s.download(64, 64).expect("download") else { panic!("NV12 reads back as Yuv8") };
        assert!(planes.iter().all(|p| p.iter().all(|&v| v == 0x80)), "mid-grey");
    }

    #[test]
    fn scaled_sizes_are_even_and_bounded() {
        assert_eq!(scaled_size((6240, 4160), 0.519_230_8), (3240, 2160));
        assert_eq!(scaled_size((1920, 1080), 0.5), (960, 540));
        for s in [1.0, 0.0, -1.0, f32::NAN, f32::INFINITY, 2.0] {
            assert_eq!(scaled_size((1921, 1081), s), (1921, 1081), "{s}: unscaled");
        }
        assert_eq!(scaled_size((64, 64), 1e-6), (2, 2), "at least 2");
        assert_eq!(scaled_size((1, 1), 0.5), (1, 1), "never larger than the picture");
    }

    /// A scaled hand-off copy (U0.2.3b d) lands at the output size with the source's values
    /// (a flat picture stays flat), and refuses an output larger than its copier. Skipped
    /// without VA-API video processing on renderD128.
    #[test]
    fn scaled_handoff_copy_keeps_values() {
        let Ok(display) = Display::for_node("/dev/dri/renderD128") else { return };
        let fmt = (sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010);
        let Ok(copier) = LinearCopy::new(&display, fmt.0, fmt.1, 320, 180, handoff_alloc()) else {
            eprintln!("no video processor; skipped");
            return;
        };
        let src = SurfacePool::new(&display, fmt.0, fmt.1, 640, 360).get().expect("surface");
        // luma code 300, chroma 700 (10-bit MSB-aligned)
        src.write(|bytes, offsets, _| {
            for (i, p) in bytes.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                *p = if 2 * i < offsets[1] { 300u16 << 6 } else { 700u16 << 6 }.to_le_bytes();
            }
            Ok(())
        })
        .expect("written");
        let out = copier.copy_scaled(&src, (0, 0, 640, 360), (320, 180)).expect("scaled");
        let PixelData::Yuv16Bi { y, uv, .. } = read_back(&out, (0, 0, 320, 180)).expect("read") else { panic!("P010 reads back as Yuv16Bi") };
        assert_eq!((y.len(), uv.len()), (320 * 180, 320 * 90));
        assert!(y.iter().all(|&v| (v >> 6).abs_diff(300) <= 1), "luma kept");
        assert!(uv.iter().all(|&v| (v >> 6).abs_diff(700) <= 1), "chroma kept");
        assert!(copier.copy_scaled(&src, (0, 0, 640, 360), (321, 180)).is_err(), "larger than the copier");
        assert!(copier.copy_scaled(&src, (0, 0, 640, 360), (0, 180)).is_err(), "empty output");
    }

    /// Time of one 4K P010 hand-off copy on a render node (`FILMCRAFT_TIME_NODE`, default
    /// renderD128), and which memory it lands in (U0.2.3b d diagnosis):
    /// `cargo test -p filmcraft-platform --release --lib -- --ignored --nocapture handoff_copy_time`.
    #[test]
    #[ignore]
    fn handoff_copy_time() {
        let node = std::env::var("FILMCRAFT_TIME_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        let Ok(display) = Display::for_node(&node) else { return };
        let (w, h) = (3840, 2160);
        let src = SurfacePool::new(&display, sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010, w, h).get().expect("surface");
        let copier = LinearCopy::new(&display, sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010, w, h, handoff_alloc()).expect("copier");
        // copies dropped at once (a spare surface each time), then held (as a GOP cache holds
        // frames: every copy needs new memory)
        for hold in [false, true] {
            let mut times = Vec::new();
            let mut held = Vec::new();
            for _ in 0..20 {
                let t = std::time::Instant::now();
                let out = copier.copy(&src, (0, 0, w, h)).expect("copied");
                times.push(t.elapsed().as_secs_f64() * 1e3);
                if hold {
                    held.push(out);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "{node}: imported {}, held {hold}: 4K P010 copy median {:.1} ms (min {:.1}, max {:.1})",
                copier.imported(),
                times[10],
                times[0],
                times[19]
            );
        }
    }

    /// Time of one 6.2K P010 hand-off copy, plain and scaled to 4K (U0.2.3b d (1): does a
    /// scaling copy cut the hand-off cost?):
    /// `cargo test -p filmcraft-platform --release --lib -- --ignored --nocapture handoff_scaled_copy_time`
    /// (alone: it holds 40 6.2K surfaces).
    #[test]
    #[ignore]
    fn handoff_scaled_copy_time() {
        let node = std::env::var("FILMCRAFT_TIME_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        let Ok(display) = Display::for_node(&node) else { return };
        let (w, h) = (6240, 4160);
        let fmt = (sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010);
        let src = SurfacePool::new(&display, fmt.0, fmt.1, w, h).get().expect("surface");
        // 6240×4160 fit into 3840×2160 keeps 3:2: 3240×2160
        for out in [(w, h), (3240, 2160)] {
            let copier = LinearCopy::new(&display, fmt.0, fmt.1, out.0, out.1, handoff_alloc()).expect("copier");
            let mut times = Vec::new();
            let mut held = Vec::new();
            for _ in 0..20 {
                let t = std::time::Instant::now();
                held.push(copier.copy_scaled(&src, (0, 0, w, h), out).expect("copied"));
                times.push(t.elapsed().as_secs_f64() * 1e3);
            }
            times.sort_by(f64::total_cmp);
            eprintln!("{node}: 6.2K P010 to {}x{} held copy median {:.1} ms (min {:.1}, max {:.1})", out.0, out.1, times[10], times[0], times[19]);
        }
    }
}
