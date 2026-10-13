//! Zero-copy output for upstream's VA-API front ends (fork Y1): an [`Accel`] over the fork's libva
//! layer ([`crate::va`]) whose decoded pictures stay on the GPU. Each frame it hands out is a
//! [`VaPicture`] holding its surface ([`PixelData::Gpu`]: the compositor imports the surface's
//! DMA-BUF, the CPU paths read it back on demand), so a surface is free for the next picture only
//! once the DPB *and* every frame have let go of it: [`ZeroCopyAccel::pick`] skips the held ones
//! (`Arc::strong_count` above one) and adds a surface from the pool instead, up to what the GOP
//! cache's GPU-memory budget can hold of them ([`ZeroCopyAccel::max_slots`]: the cache is what
//! holds frames, so it runs out first). Surface indexes never change, so the front end's DPB
//! references stay valid as the set grows.
//!
//! Safe code: every libva call it needs exists in `va.rs`. Refused (so the read-back decoder
//! takes the stream): `FILMCRAFT_NO_GPU_IMPORT`, a display without the profile, a surface whose
//! DMA-BUF cannot be exported.

use std::sync::Arc;

use filmcraft_frame::{PixelData, VideoFrame};

use crate::biplanar::Geometry;
use crate::va::{Config, Context, Display, Surface, SurfacePool};
use crate::va_hevc::VaPicture;
use crate::va_sys as sys;
use crate::vaapi::{Accel, ffi};

/// Most surfaces an accelerator ever grows to, whatever the budget (a guard; the budget-derived
/// bound, [`ZeroCopyAccel::max_slots`], is the one reached in practice). Beyond it a picture has
/// no surface and the hybrid decoder falls back.
pub const MAX_SLOTS: usize = 4096;

/// Largest picture taken (VCN tops out at 8192×4352; libva at 16K).
const MAX_DIM: u32 = 16384;

/// Render-target format and surface fourcc of 4:2:0 pictures of `bits`.
fn format(bits: u32) -> Result<(u32, u32), String> {
    match bits {
        8 => Ok((sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12)),
        10 => Ok((sys::VA_RT_FORMAT_YUV420_10, sys::VA_FOURCC_P010)),
        _ => Err(format!("{bits}-bit pictures are not decoded zero-copy")),
    }
}

/// Bytes per row of the luma and the interleaved chroma plane of a `w`-wide NV12 / P010 image.
fn row_bytes(fourcc: u32, w: u32) -> Result<[usize; 2], String> {
    let w = w as usize;
    let cw = w.div_ceil(2);
    let per = if fourcc == sys::VA_FOURCC_P010 { 2 } else { 1 };
    Ok([w.checked_mul(per).ok_or("row size overflows")?, cw.checked_mul(2 * per).ok_or("row size overflows")?])
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
    context: Context,
    display: Arc<Display>,
    /// `slots`' ids ([`Accel::surfaces`]).
    ids: Vec<ffi::VASurfaceID>,
    /// When each slot was last decoded or filled into ([`ZeroCopyAccel::pick`] reuses the one
    /// written longest ago).
    used: Vec<u64>,
    seq: u64,
    /// The surfaces made at the start (the DPB, the picture being decoded, a spare).
    initial: usize,
    geometry: Geometry,
}

impl ZeroCopyAccel {
    /// A decoder on the process's display ([`Display::get`]) for the first of `profiles` it
    /// has, with `count` surfaces (coded `size`) to start with; output pictures are cut to
    /// `geometry`. The reasons it declines are the read-back decoder's cue to take the stream.
    pub fn new(profiles: &[ffi::VAProfile], size: (u32, u32), count: usize, geometry: Geometry) -> Result<Self, String> {
        if std::env::var_os("FILMCRAFT_NO_GPU_IMPORT").is_some() {
            return Err("zero-copy display is off (FILMCRAFT_NO_GPU_IMPORT)".into());
        }
        let (rt_format, fourcc) = format(geometry.bits)?;
        if size.0 == 0 || size.1 == 0 || size.0 > MAX_DIM || size.1 > MAX_DIM {
            return Err(format!("picture size {}x{}", size.0, size.1));
        }
        let display = Display::get()?;
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
        let context = Context::new(&config, size.0, size.1)?;
        // zero-copy needs the surfaces' DMA-BUFs: a driver that cannot export them (or exports
        // something the compositor cannot import) is found out now, not at the first frame
        slots.first().ok_or("no surface")?.export().map_err(|e| format!("DMA-BUF export: {e}"))?;
        Ok(Self { used: vec![0; slots.len()], slots, pool, context, display, ids, seq: 0, initial: count, geometry })
    }

    /// Bytes of one surface (NV12 / P010 at the coded size).
    fn surface_bytes(&self) -> usize {
        let (w, h) = self.pool.size;
        let px = (w as usize).saturating_mul(h as usize);
        px.saturating_mul(3).saturating_add(1) / 2 * if self.pool.fourcc == sys::VA_FOURCC_P010 { 2 } else { 1 }
    }

    /// Most surfaces this accelerator grows to: the decoder's own plus as many as the GOP cache's
    /// GPU-memory budget holds of them (frames in the cache are what keeps surfaces; it evicts
    /// within that budget, so the set never needs more), at most [`MAX_SLOTS`].
    pub fn max_slots(&self) -> usize {
        let (_, budget) = filmcraft_codecs::gop::vram_usage();
        (budget / self.surface_bytes().max(1)).saturating_add(self.initial).min(MAX_SLOTS)
    }

    /// The driver's description.
    pub fn vendor(&self) -> &str {
        &self.display.vendor
    }

    /// Surfaces the accelerator has (grows while frames hold them; tests).
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    fn slot(&self, index: usize) -> Result<&Arc<Surface>, String> {
        self.slots.get(index).ok_or_else(|| "surface out of range".to_string())
    }

    fn touch(&mut self, index: usize) {
        self.seq = self.seq.wrapping_add(1);
        if let Some(u) = self.used.get_mut(index) {
            *u = self.seq;
        }
    }

    /// Every sample of the surface mid-grey (128 in NV12, 512 in P010), as the software decoders
    /// make the frames they never decoded.
    fn write_gray(&self, target: &Surface) -> Result<(), String> {
        let ten = self.pool.fourcc == sys::VA_FOURCC_P010;
        target.write(|bytes, _, _| {
            if ten {
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
}

impl Drop for ZeroCopyAccel {
    /// Wait for every slot's decode while the context still exists (Intel's driver looks pending
    /// work up through it; see [`ZeroCopyAccel::pool`]). Output pictures were synced when read;
    /// this covers the reference pictures a decoder dropped mid-stream (a seek, an evicted
    /// clip) never output. Then the fields drop in order: the slots back to the pool, the pool
    /// (its spares destroyed; surfaces frames still hold keep it until they are dropped), the
    /// context.
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
        let ctx = &self.context;
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
        self.touch(target);
        Ok(())
    }

    fn decode_hevc(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVC,
        iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVC, Vec<u8>)],
    ) -> Result<(), String> {
        let ctx = &self.context;
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
        self.touch(target);
        Ok(())
    }

    fn read(&mut self, index: usize) -> Result<VideoFrame, String> {
        let s = self.slot(index)?.clone();
        // the decode has finished when the frame is handed out
        s.sync()?;
        let (cx, cy, cw, ch) = self.geometry.crop;
        let pic = VaPicture::new(s, (cx, cy), self.geometry.bits > 8);
        Ok(VideoFrame {
            width: cw,
            height: ch,
            data: PixelData::Gpu(Arc::new(pic)),
            color: self.geometry.color,
            par: self.geometry.par,
            pts: filmcraft_time::Tick::ZERO,
        })
    }

    fn fill(&mut self, index: usize, from: Option<usize>) -> Result<(), String> {
        let target = self.slot(index)?;
        // a surface a frame still shows is never written (the same rule as `pick`)
        if Arc::strong_count(target) > 1 {
            return Err(format!("surface {index} is still held by a frame"));
        }
        let target = target.clone();
        // nothing may still be decoding into it while the image is copied in
        target.sync()?;
        match from {
            None => self.write_gray(&target)?,
            Some(f) => {
                let src = self.slot(f)?.clone();
                self.copy(&src, &target)?;
            }
        }
        self.touch(index);
        Ok(())
    }

    /// The free candidate no frame holds that was written longest ago (its last frame has had
    /// the longest for GPU work that sampled it to finish), else a surface added from the pool
    /// (index = the new last), else `None` at [`ZeroCopyAccel::max_slots`].
    fn pick(&mut self, candidates: &[usize]) -> Option<usize> {
        let free = candidates.iter().copied().filter(|&i| self.slots.get(i).is_some_and(|s| Arc::strong_count(s) == 1));
        if let Some(i) = free.min_by_key(|&i| self.used.get(i).copied().unwrap_or(0)) {
            return Some(i);
        }
        let max = self.max_slots();
        if self.slots.len() >= max {
            log::warn!("VA-API zero-copy: all {max} surfaces are held by frames or references");
            return None;
        }
        match self.pool.get() {
            Ok(s) => {
                self.ids.push(s.id);
                self.slots.push(Arc::new(s));
                self.used.push(0);
                Some(self.slots.len() - 1)
            }
            Err(e) => {
                log::warn!("VA-API zero-copy: no surface for the next picture: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_of_the_image_planes() {
        assert_eq!(row_bytes(sys::VA_FOURCC_NV12, 640).unwrap(), [640, 640]);
        assert_eq!(row_bytes(sys::VA_FOURCC_NV12, 641).unwrap(), [641, 642]);
        assert_eq!(row_bytes(sys::VA_FOURCC_P010, 640).unwrap(), [1280, 1280]);
        assert!(format(12).is_err());
    }

    /// The surface set is bounded by the GOP cache's GPU budget (what holds frames), never below
    /// the decoder's own surfaces, never above the guard. Skipped without VA-API.
    #[test]
    fn the_surface_set_is_bounded_by_the_cache_budget() {
        let profiles = [ffi::VAProfileH264High, ffi::VAProfileH264Main];
        let g = Geometry { crop: (0, 0, 64, 64), bits: 8, color: Default::default(), par: (1, 1) };
        let Ok(a) = ZeroCopyAccel::new(&profiles, (64, 64), 5, g) else { return };
        assert_eq!(a.surface_bytes(), 64 * 64 * 3 / 2);
        let (_, budget) = filmcraft_codecs::gop::vram_usage();
        assert_eq!(a.max_slots(), (budget / (64 * 64 * 3 / 2) + 5).min(MAX_SLOTS));
        assert!(a.max_slots() >= 5);
    }

    /// Frames hold their surfaces: a held slot is never picked, the set grows instead, and a
    /// released slot is reused (the one written longest ago first). Skipped without VA-API or an
    /// H.264 decoder.
    #[test]
    fn picks_only_surfaces_no_frame_holds() {
        let profiles = [ffi::VAProfileH264High, ffi::VAProfileH264Main];
        let g = Geometry { crop: (0, 0, 64, 64), bits: 8, color: Default::default(), par: (1, 1) };
        let mut a = match ZeroCopyAccel::new(&profiles, (64, 64), 3, g) {
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
}
