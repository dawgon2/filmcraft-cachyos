//! Comp programs on the GPU (fork C1 step 7b, ADR 0007): the passes of a
//! [`filmcraft_render::comp::Program`] run on comp-sized slot textures with the renderer's own
//! layer draw and effect stage. A slot takes part in a draw as an ordinary linear RGBA layer: its
//! texture is registered in the upload cache under a placeholder frame (no pixels), so the layer
//! and effect paths sample it without an upload. The CPU [`execute_cpu`] is the oracle.
//!
//! [`execute_cpu`]: filmcraft_render::comp::execute_cpu

use std::sync::Arc;

use filmcraft_frame::{PixelData, VideoFrame};
use filmcraft_geom::Affine;
use filmcraft_render::Blend;
use filmcraft_render::comp::{Pass, Program};
use filmcraft_render::gpufx::FxOp;
use filmcraft_render::plan::{FramePlan, LayerFx, PlanLayer};

use crate::{GpuCompositor, Uploaded};

/// Most GPU memory one comp's slot textures may take (bytes); above it the comp runs on the CPU.
/// Provisional: 2 GB holds a 6.2K comp's three live pictures and the scratch slot four times over.
pub const COMP_VRAM_BUDGET: u64 = 2 << 30;

/// Most comp layers one frame runs on the GPU (the rest run on the CPU).
const MAX_COMP_SETS: usize = 8;

/// Comp pictures whose frames are remembered (a few static nodes per comp, a few comps).
const COMP_IMAGES_KEPT: usize = 8;

/// Bytes per slot texel (`Rgba16Float`).
const SLOT_TEXEL_BYTES: u64 = 8;

/// A comp-sized texture and the placeholder frame it is drawn through.
pub(crate) struct Slot {
    pub(crate) target: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    frame: Arc<VideoFrame>,
}

impl Slot {
    fn new(w: u32, h: u32) -> Slot {
        let frame = VideoFrame { width: w, height: h, data: PixelData::RgbaF32(Arc::new(Vec::new())), ..VideoFrame::rgba_f32(1, 1, vec![0.0; 4]) };
        Slot { target: None, frame: Arc::new(frame) }
    }
}

impl GpuCompositor {
    /// Runs `p` on the GPU; its result is a placeholder frame registered in the upload cache
    /// (draw it as a layer right away). None when the comp doesn't fit [`COMP_VRAM_BUDGET`] or the
    /// device's texture size, or has no passes (the caller then runs it on the CPU).
    pub(crate) fn run_comp(&mut self, p: &Program, set: usize) -> Option<Arc<VideoFrame>> {
        // each comp layer of a frame has its own slots: a second comp must not overwrite the
        // first one's result before the frame is drawn
        if set >= MAX_COMP_SETS {
            return None;
        }
        while self.comp_sets.len() <= set {
            self.comp_sets.push(Vec::new());
        }
        std::mem::swap(&mut self.comp_slots, &mut self.comp_sets[set]);
        let out = self.run_comp_in_slots(p);
        std::mem::swap(&mut self.comp_slots, &mut self.comp_sets[set]);
        out
    }

    fn run_comp_in_slots(&mut self, p: &Program) -> Option<Arc<VideoFrame>> {
        let w = u32::try_from(p.size.0).ok()?.max(1);
        let h = u32::try_from(p.size.1).ok()?.max(1);
        if w.max(h) > self.device.limits().max_texture_dimension_2d || p.passes.is_empty() {
            return None;
        }
        // effect ops the GPU stage can't run (or no stage on this device): the CPU runs the comp,
        // as it does a timeline layer's effects
        let fx_ok = |ops: &[FxOp]| self.fx.is_some() && ops.iter().all(FxOp::gpu_ok);
        if p.passes.iter().any(|pass| matches!(pass, Pass::Fx { ops, .. } if !fx_ok(ops))) {
            return None;
        }
        // a comp clip inside a Media In track would need these slots while they are in use
        if p.passes.iter().any(|pass| matches!(pass, Pass::Layers { layers, .. } if layers.iter().any(|l| l.comp.is_some()))) {
            return None;
        }
        // the program's slots and one scratch slot
        let n = usize::from(p.textures).saturating_add(1);
        let bytes = (n as u64).saturating_mul(u64::from(w)).saturating_mul(u64::from(h)).saturating_mul(SLOT_TEXEL_BYTES);
        if bytes > COMP_VRAM_BUDGET {
            log::warn!("a comp of {w}x{h} with {n} pictures doesn't fit the GPU budget; it renders on the CPU");
            return None;
        }
        // reuse the slots when the size matches (a slot whose texture was never written is empty)
        if self.comp_slots.len() != n || self.comp_slots.first().is_some_and(|s| (s.frame.width, s.frame.height) != (w, h)) {
            self.comp_slots = (0..n).map(|_| Slot::new(w, h)).collect();
        }
        let mut written = vec![false; n];
        let scratch = n - 1;
        for pass in &p.passes {
            match pass {
                Pass::Image { dst, image } => {
                    let frame = self.image_frame(image);
                    let fits = (image.w, image.h) == p.size;
                    let layers = if fits { vec![PlanLayer::new(frame, Affine::IDENTITY, 1.0, Blend::Normal)] } else { Vec::new() };
                    self.slot_draw(usize::from(*dst), layers, true, (w, h));
                    mark(&mut written, usize::from(*dst));
                }
                Pass::Layers { dst, layers } => {
                    self.slot_draw(usize::from(*dst), layers.to_vec(), true, (w, h));
                    mark(&mut written, usize::from(*dst));
                }
                Pass::Fill { dst, color } => {
                    let layers = if *color == [0.0; 4] {
                        Vec::new()
                    } else {
                        let px = Arc::new(VideoFrame::rgba_f32(1, 1, color.to_vec()));
                        vec![PlanLayer::new(px, Affine::scale(f64::from(w), f64::from(h)), 1.0, Blend::Normal)]
                    };
                    self.slot_draw(usize::from(*dst), layers, true, (w, h));
                    mark(&mut written, usize::from(*dst));
                }
                Pass::Copy { src, dst } => self.slot_layer_pass(
                    &mut written,
                    scratch,
                    usize::from(*src),
                    usize::from(*dst),
                    Affine::IDENTITY,
                    1.0,
                    Blend::Normal,
                    true,
                    false,
                    None,
                    (w, h),
                ),
                Pass::Draw { src, dst, xform, hq } => {
                    self.slot_layer_pass(&mut written, scratch, usize::from(*src), usize::from(*dst), *xform, 1.0, Blend::Normal, true, *hq, None, (w, h));
                }
                Pass::Over { src, dst, mode, opacity } => {
                    // (CPU: nothing happens unless both pictures exist)
                    let (s, d) = (usize::from(*src), usize::from(*dst));
                    if written.get(s).copied().unwrap_or(false) && written.get(d).copied().unwrap_or(false) {
                        self.slot_layer_pass(&mut written, scratch, s, d, Affine::IDENTITY, *opacity, *mode, false, false, None, (w, h));
                    }
                }
                Pass::Fx { tex, ops } => {
                    let t = usize::from(*tex);
                    if written.get(t).copied().unwrap_or(false) {
                        let fx = LayerFx { size: (w, h), decimation: 1, ops: ops.clone() };
                        self.slot_layer_pass(&mut written, scratch, t, t, Affine::IDENTITY, 1.0, Blend::Normal, true, false, Some(fx), (w, h));
                    }
                }
            }
        }
        let out = usize::from(p.out);
        if !written.get(out).copied().unwrap_or(false) {
            // nothing reached the output: a transparent picture
            self.slot_draw(out, Vec::new(), true, (w, h));
        }
        self.register_slot(out)
    }

    /// A comp picture as a frame, the same frame for the same picture (static nodes come from the
    /// comp's cache as the same `Arc`), so the upload cache keeps its texture across frames.
    fn image_frame(&mut self, image: &Arc<filmcraft_render::image::Image>) -> Arc<VideoFrame> {
        if let Some((_, f)) = self.comp_images.iter().find(|(i, _)| Arc::ptr_eq(i, image)) {
            return f.clone();
        }
        let f = Arc::new(VideoFrame::rgba_f32(image.w as u32, image.h as u32, image.px.clone()));
        self.comp_images.push((image.clone(), f.clone()));
        if self.comp_images.len() > COMP_IMAGES_KEPT {
            self.comp_images.remove(0);
        }
        f
    }

    /// `src` drawn into `dst` as one layer (`clear`: onto transparency, else over what `dst`
    /// holds). When they are the same slot the result goes to the scratch slot and the two swap.
    #[allow(clippy::too_many_arguments)]
    fn slot_layer_pass(
        &mut self,
        written: &mut [bool],
        scratch: usize,
        src: usize,
        dst: usize,
        m: Affine,
        opacity: f32,
        blend: Blend,
        clear: bool,
        hq: bool,
        fx: Option<LayerFx>,
        size: (u32, u32),
    ) {
        let layers = match (written.get(src).copied().unwrap_or(false), self.register_slot(src)) {
            (true, Some(frame)) => vec![PlanLayer { frame, matrix: m, opacity, blend, fx: fx.map(Arc::new), comp: None }],
            _ => Vec::new(),
        };
        let target = if src == dst { scratch } else { dst };
        if target != dst && !clear {
            // over itself: start the scratch slot from the destination
            if let Some(frame) = self.register_slot(dst) {
                self.slot_draw(target, vec![PlanLayer::new(frame, Affine::IDENTITY, 1.0, Blend::Normal)], true, size);
            }
        }
        let was = self.high_quality;
        self.high_quality = was || hq;
        self.slot_draw(target, layers, clear, size);
        self.high_quality = was;
        if target != dst {
            // the scratch slot holds the result: swap the textures (each keeps its placeholder)
            let a = self.comp_slots.get_mut(target).map(|s| s.target.take());
            let b = self.comp_slots.get_mut(dst).map(|s| s.target.take());
            if let (Some(a), Some(b)) = (a, b) {
                if let Some(s) = self.comp_slots.get_mut(target) {
                    s.target = b;
                }
                if let Some(s) = self.comp_slots.get_mut(dst) {
                    s.target = a;
                }
            }
        }
        mark(written, dst);
    }

    /// Renders `layers` into slot `i` (created on first use) through the accumulator path.
    fn slot_draw(&mut self, i: usize, layers: Vec<PlanLayer>, clear: bool, (w, h): (u32, u32)) {
        let Some(slot) = self.comp_slots.get_mut(i) else { return };
        let mut target = slot.target.take();
        std::mem::swap(&mut self.accum, &mut target);
        let plan = FramePlan::Layers { width: w as usize, height: h as usize, layers, output: None };
        self.render_plan(&plan, None, clear, false);
        std::mem::swap(&mut self.accum, &mut target);
        if let Some(slot) = self.comp_slots.get_mut(i) {
            slot.target = target;
        }
    }

    /// Registers slot `i`'s texture in the upload cache under its placeholder frame (again before
    /// every use: the cache evicts old entries). None for a slot without a texture.
    fn register_slot(&mut self, i: usize) -> Option<Arc<VideoFrame>> {
        let slot = self.comp_slots.get(i)?;
        let (_, view, (w, h)) = slot.target.as_ref()?;
        let frame = slot.frame.clone();
        let PixelData::RgbaF32(px) = &frame.data else { return None };
        let key = (Arc::as_ptr(px) as *const u8 as usize, *w, *h);
        self.clock += 1;
        let up = Uploaded {
            views: [view.clone(), self.dummy.clone(), self.dummy.clone(), self.dummy.clone()],
            kind: 1,
            code_scale: 1.0,
            texel_scale: 1.0,
            alpha_scale: 0.0,
            chroma: (*w, *h),
            last_used: self.clock,
            _pixels: frame.data.clone(),
        };
        self.uploads.insert(key, up);
        Some(frame)
    }
}

fn mark(written: &mut [bool], i: usize) {
    if let Some(w) = written.get_mut(i) {
        *w = true;
    }
}
