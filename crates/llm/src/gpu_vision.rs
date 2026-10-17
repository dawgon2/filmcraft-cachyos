//! The Qwen3-VL vision encoder on the GPU (AI4.2): [`crate::vision::VisionEncoder`]'s weights
//! uploaded once as f32, one picture encoded by a chain of wgpu compute kernels
//! (`gpu_vision.wgsl`): the patch embedding as a tiled matmul, the learned positions added, then
//! per block LayerNorm → qkv matmul → 2-D rotary → bidirectional attention (online softmax over
//! key blocks, so memory stays O(queries × block) whatever the token count) → proj (added into
//! the residual) → LayerNorm → MLP (GELU fused into the first matmul) → residual; the 2 × 2
//! mergers (DeepStack taps and the final one) reuse the same LayerNorm / matmul kernels with the
//! exact-erf GELU. Output and arithmetic order match the CPU encoder to ~1e-5 relative.
//!
//! Weights stay f32 (about 1.7 GB for the 4B model's 0.42 B encoder parameters): the GPUs this
//! targets have 12 GB, the kernels are compute-bound at the batch sizes a picture gives (64 to a
//! few thousand tokens), so f16 weights would not make them faster, and f32 keeps the CPU parity
//! exact (f16 rounding of every weight would cost ~5e-4 relative per layer, compounding over 24
//! blocks against the ≤ 1e-3 target). `shader-f16` is also not available on every adapter.
//!
//! Every failure (no adapter, too little workgroup memory, a head too wide for the attention
//! kernel, out of GPU memory, a lost device) is an [`LlmError`]: callers use the CPU encoder.

use crate::LlmError;
use crate::gpu::GpuContext;
use crate::quant::err;
use crate::vision::{self, MAX_PATCHES, VisionEncoder, VisionOutput};
use crate::vision_config::VisionConfig;

/// Workgroups along x before a 1-D kernel's threads wrap to y (under every adapter's 65 535).
const STRIDE_X: u32 = 32_768;
const NORM_EPS: f32 = 1e-6;
/// Tokens and outputs per workgroup of the `linear` kernel (BM / BN in the shader).
const TILE_TOKENS: u32 = 64;
const TILE_OUT: u32 = 128;
/// Queries and keys per block of the `attention` kernel (QB / KB in the shader).
const QB: u32 = 64;
const KB: u32 = 32;
/// Narrowest and widest head the attention kernel stages. Workgroup memory is
/// (QB · HD + 2 · KB · HD + QB · KB + 3 · QB) f32: 41.7 KiB at 64, under the 64 KiB AMD and Intel
/// report; a 128-wide head would need 72.8 KiB. Under 16 the kernel's row mapping would index past
/// its arrays.
const MIN_HEAD_DIM: usize = 16;
const MAX_HEAD_DIM: usize = 64;
/// Most patches one picture may have on the GPU: attention grows with the square of the patch
/// count and each block is one submission under the device's wait limit (a discrete card does
/// 16 384 patches, ≈ 4 M pixels, in well under a second per block; an integrated one gets far
/// fewer: review 2026-10-16).
const MAX_GPU_PATCHES: usize = 16_384;
const MAX_GPU_PATCHES_INTEGRATED: usize = 4_096;
/// Threads per workgroup of every kernel.
const THREADS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kernel {
    /// params `[n, inp, out, act, acc]`, x, w, b, y.
    Linear,
    /// params `[dim, eps, stride_x, rows]`, x, w, b, y.
    LayerNorm,
    /// params `[n, h, hd, stride_x]`, cos, sin, qkv (in place).
    Rotary,
    /// params `[n, h, heads, scale]`, qkv, out.
    Attention,
    /// params `[n, stride_x]`, x (in place), y.
    Add,
}

impl Kernel {
    const ALL: [Kernel; 5] = [Kernel::Linear, Kernel::LayerNorm, Kernel::Rotary, Kernel::Attention, Kernel::Add];

    fn name(self) -> &'static str {
        match self {
            Kernel::Linear => "linear",
            Kernel::LayerNorm => "layer_norm",
            Kernel::Rotary => "rotary",
            Kernel::Attention => "attention",
            Kernel::Add => "add",
        }
    }

    /// The kernel's WGSL: its `//// name` section of `gpu_vision.wgsl`, with the head width
    /// substituted into the attention kernel.
    fn source(self, head_dim: u32) -> Option<String> {
        let body = include_str!("gpu_vision.wgsl").split("\n//// ").skip(1).find_map(|sec| {
            let (head, body) = sec.split_once('\n')?;
            (head.trim() == self.name()).then_some(body)
        })?;
        Some(match self {
            Kernel::Attention => body.replace("const HD: u32 = 64u;", &format!("const HD: u32 = {head_dim}u;")),
            _ => body.to_string(),
        })
    }
}

struct Pipeline {
    kernel: Kernel,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

struct GpuLinear {
    w: wgpu::Buffer,
    b: wgpu::Buffer,
    inp: u32,
    out: u32,
}

struct GpuNorm {
    w: wgpu::Buffer,
    b: wgpu::Buffer,
    dim: u32,
}

struct GpuBlock {
    norm1: GpuNorm,
    norm2: GpuNorm,
    qkv: GpuLinear,
    proj: GpuLinear,
    fc1: GpuLinear,
    fc2: GpuLinear,
}

struct GpuMerger {
    post_shuffle: bool,
    norm: GpuNorm,
    fc1: GpuLinear,
    fc2: GpuLinear,
}

/// The encoder's weights on a GPU, ready to encode pictures.
pub struct GpuVisionEncoder {
    ctx: GpuContext,
    cfg: VisionConfig,
    pipelines: Vec<Pipeline>,
    max_groups: u32,
    patch: GpuLinear,
    /// The learned position table (kept on the CPU: sampling it for a grid is cheap and the
    /// CPU's code does it, so both encoders see the same positions).
    pos: Vec<f32>,
    side: usize,
    blocks: Vec<GpuBlock>,
    merger: GpuMerger,
    deepstack: Vec<GpuMerger>,
    bytes: u64,
}

impl std::fmt::Debug for GpuVisionEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuVisionEncoder")
            .field("adapter", &self.ctx.name())
            .field("bytes", &self.bytes)
            .field("blocks", &self.blocks.len())
            .finish_non_exhaustive()
    }
}

/// Uploads tensors, summing their bytes.
struct Uploader<'a> {
    ctx: &'a GpuContext,
    bytes: u64,
}

impl Uploader<'_> {
    fn buffer(&mut self, data: &[f32], label: &str) -> Result<wgpu::Buffer, LlmError> {
        let buf = self.ctx.storage_init(data, label)?;
        self.bytes = self.bytes.saturating_add(buf.size());
        Ok(buf)
    }

    fn linear(&mut self, l: &vision::Linear, label: &str) -> Result<GpuLinear, LlmError> {
        if l.inp == 0 || l.out == 0 || Some(l.w.len()) != l.inp.checked_mul(l.out) || l.b.len() != l.out {
            return Err(err(format!("vision weight {label} is {} values, not {} × {}", l.w.len(), l.out, l.inp)));
        }
        let (inp, out) = (u32::try_from(l.inp).map_err(|_| err("vision weight too wide"))?, u32::try_from(l.out).map_err(|_| err("vision weight too tall"))?);
        Ok(GpuLinear { w: self.buffer(&l.w, label)?, b: self.buffer(&l.b, label)?, inp, out })
    }

    fn norm(&mut self, n: &vision::Norm, label: &str) -> Result<GpuNorm, LlmError> {
        if n.w.is_empty() || n.w.len() != n.b.len() {
            return Err(err(format!("vision norm {label} has {} weights and {} biases", n.w.len(), n.b.len())));
        }
        let dim = u32::try_from(n.w.len()).map_err(|_| err("vision norm too wide"))?;
        Ok(GpuNorm { w: self.buffer(&n.w, label)?, b: self.buffer(&n.b, label)?, dim })
    }

    fn merger(&mut self, m: &vision::Merger, label: &str) -> Result<GpuMerger, LlmError> {
        Ok(GpuMerger { post_shuffle: m.post_shuffle, norm: self.norm(&m.norm, label)?, fc1: self.linear(&m.fc1, label)?, fc2: self.linear(&m.fc2, label)? })
    }
}

/// The buffers one picture's encode runs through.
struct Work {
    n: u32,
    /// Residual stream, `n × h`.
    x: wgpu::Buffer,
    /// A normalised copy, `n × h`.
    nrm: wgpu::Buffer,
    /// `n × 3h`.
    qkv: wgpu::Buffer,
    /// Attention output, `n × h`.
    attn: wgpu::Buffer,
    /// MLP hidden, `n × intermediate`.
    mid: wgpu::Buffer,
    /// A merger's normalised input (`n × h`) and hidden (`n / unit × unit · h`, the same size).
    mnorm: wgpu::Buffer,
    mmid: wgpu::Buffer,
    /// Merged tokens, `n / unit × out_hidden`.
    out: wgpu::Buffer,
    /// One per DeepStack tap, as `out`.
    taps: Vec<wgpu::Buffer>,
}

impl GpuVisionEncoder {
    /// Uploads `cpu`'s weights to the default high-performance adapter. An error (no GPU, too
    /// little workgroup memory, out of GPU memory, …) means: use `cpu`.
    pub fn new(cpu: &VisionEncoder) -> Result<Self, LlmError> {
        Self::with_context(GpuContext::new()?, cpu)
    }

    /// Uploads `cpu`'s weights to `ctx`'s device.
    pub fn with_context(ctx: GpuContext, cpu: &VisionEncoder) -> Result<Self, LlmError> {
        let cfg = cpu.cfg.clone();
        let hd = cfg.head_dim();
        if cfg.hidden == 0
            || cfg.heads == 0
            || !cfg.hidden.is_multiple_of(cfg.heads)
            || !hd.is_multiple_of(4)
            || !(MIN_HEAD_DIM..=MAX_HEAD_DIM).contains(&hd)
            || cfg.merge == 0
        {
            return Err(err(format!("the GPU vision encoder takes heads of {MIN_HEAD_DIM}..={MAX_HEAD_DIM} values (a multiple of 4), not {hd}")));
        }
        if cfg.hidden.saturating_mul(cfg.merge).saturating_mul(cfg.merge) > u32::MAX as usize {
            return Err(err("vision hidden size too large for the GPU"));
        }
        let device = ctx.device();
        let limits = device.limits();
        // the attention kernel's workgroup memory (the linear kernel's 12 KiB is smaller)
        let shared = (QB * hd as u32 + 2 * KB * hd as u32 + QB * KB + 3 * QB).saturating_mul(4);
        if limits.max_compute_workgroup_storage_size < shared
            || limits.max_compute_invocations_per_workgroup < THREADS
            || limits.max_compute_workgroup_size_x < THREADS
        {
            return Err(err(format!(
                "the GPU's compute limits ({} bytes of workgroup memory) are below the vision kernels' ({shared} bytes, {THREADS} threads)",
                limits.max_compute_workgroup_storage_size
            )));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut pipelines = Vec::with_capacity(Kernel::ALL.len());
        for kernel in Kernel::ALL {
            let src = kernel.source(hd as u32).ok_or_else(|| err(format!("GPU vision kernel {} missing", kernel.name())))?;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(kernel.name()), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(kernel.name()),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let layout = pipeline.get_bind_group_layout(0);
            pipelines.push(Pipeline { kernel, pipeline, layout });
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("GPU vision kernels: {e}")));
        }
        ctx.check()?;
        let mut up = Uploader { ctx: &ctx, bytes: 0 };
        let patch = up.linear(&cpu.patch, "vision-patch")?;
        let mut blocks = Vec::with_capacity(cpu.blocks.len());
        for (i, b) in cpu.blocks.iter().enumerate() {
            let l = format!("vision-block-{i}");
            blocks.push(GpuBlock {
                norm1: up.norm(&b.norm1, &l)?,
                norm2: up.norm(&b.norm2, &l)?,
                qkv: up.linear(&b.qkv, &l)?,
                proj: up.linear(&b.proj, &l)?,
                fc1: up.linear(&b.fc1, &l)?,
                fc2: up.linear(&b.fc2, &l)?,
            });
        }
        let merger = up.merger(&cpu.merger, "vision-merger")?;
        let deepstack = cpu.deepstack.iter().enumerate().map(|(i, m)| up.merger(m, &format!("vision-deepstack-{i}"))).collect::<Result<Vec<_>, _>>()?;
        let bytes = up.bytes;
        ctx.check()?;
        Ok(Self {
            max_groups: limits.max_compute_workgroups_per_dimension,
            ctx,
            cfg,
            pipelines,
            patch,
            pos: cpu.pos.clone(),
            side: cpu.side,
            blocks,
            merger,
            deepstack,
            bytes,
        })
    }

    /// The adapter's name.
    pub fn adapter(&self) -> &str {
        self.ctx.name()
    }

    /// Bytes of weights on the GPU.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Whether the adapter is a discrete GPU. An integrated one (Intel UHD) encodes no faster
    /// than the CPU's SIMD path, so callers may prefer the CPU there.
    pub fn is_discrete(&self) -> bool {
        self.ctx.is_discrete()
    }

    /// The encoder's shape.
    pub fn config(&self) -> &VisionConfig {
        &self.cfg
    }

    /// Encodes one still picture, as [`VisionEncoder::encode`] (the same `patches` and `grid`).
    pub fn encode(&self, patches: &[f32], grid: (usize, usize)) -> Result<VisionOutput, LlmError> {
        let (gh, gw) = grid;
        let m = self.cfg.merge;
        let unit = m * m;
        let n = gh
            .checked_mul(gw)
            .filter(|&n| n > 0 && n <= MAX_PATCHES)
            .ok_or_else(|| LlmError::Inference(format!("{gh} × {gw} patches is not a picture the encoder takes")))?;
        if !gh.is_multiple_of(m) || !gw.is_multiple_of(m) || Some(patches.len()) != n.checked_mul(self.patch.inp as usize) {
            return Err(LlmError::Inference("patches don't match the grid".into()));
        }
        let cap = if self.is_discrete() { MAX_GPU_PATCHES } else { MAX_GPU_PATCHES_INTEGRATED };
        if n > cap {
            return Err(err(format!("{n} patches is more than this GPU encodes in time ({cap}); use the CPU")));
        }
        let h = self.cfg.hidden;
        let n32 = u32::try_from(n).map_err(|_| err("too many patches"))?;
        let groups = n / unit;
        let work = Work {
            n: n32,
            x: self.ctx.storage(n.saturating_mul(h), "vision-x")?,
            nrm: self.ctx.storage(n.saturating_mul(h), "vision-norm")?,
            qkv: self.ctx.storage(n.saturating_mul(3).saturating_mul(h), "vision-qkv")?,
            attn: self.ctx.storage(n.saturating_mul(h), "vision-attn")?,
            mid: self.ctx.storage(n.saturating_mul(self.cfg.intermediate), "vision-mlp")?,
            mnorm: self.ctx.storage(n.saturating_mul(h), "vision-merger-norm")?,
            mmid: self.ctx.storage(n.saturating_mul(h), "vision-merger-mid")?,
            out: self.ctx.storage(groups.saturating_mul(self.cfg.out_hidden), "vision-out")?,
            taps: (0..self.deepstack.len())
                .map(|_| self.ctx.storage(groups.saturating_mul(self.cfg.out_hidden), "vision-tap"))
                .collect::<Result<Vec<_>, _>>()?,
        };
        // patch embedding plus positions, in one submission
        let input = self.ctx.storage_init(patches, "vision-patches")?;
        let pos = self.ctx.storage_init(&vision::positions(&self.pos, self.side, h, m, gh, gw), "vision-pos")?;
        let (cos, sin) = vision::rotary(self.cfg.head_dim(), m, gh, gw);
        let (cos, sin) = (self.ctx.storage_init(&cos, "vision-cos")?, self.ctx.storage_init(&sin, "vision-sin")?);
        let mut enc = self.ctx.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("vision-embed"), timestamp_writes: None });
            self.linear(&mut pass, &self.patch, &input, &work.x, n32, 0, false)?;
            self.add(&mut pass, &work.x, &pos, n.saturating_mul(h))?;
        }
        let idx = self.ctx.submit(enc)?;
        self.ctx.wait(Some(idx))?;
        // one submission per block (each waited for, so a slow adapter never trips the wait limit)
        for (i, block) in self.blocks.iter().enumerate() {
            let mut enc = self.ctx.encoder();
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("vision-block"), timestamp_writes: None });
                self.block(&mut pass, block, &work, &cos, &sin)?;
                if let Some((k, mg)) = self.cfg.deepstack.iter().position(|&l| l == i).and_then(|k| self.deepstack.get(k).map(|m| (k, m))) {
                    let tap = work.taps.get(k).ok_or_else(|| err("DeepStack tap buffer missing"))?;
                    self.merge(&mut pass, mg, &work, tap)?;
                }
            }
            let idx = self.ctx.submit(enc)?;
            self.ctx.wait(Some(idx))?;
        }
        let mut enc = self.ctx.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("vision-merger"), timestamp_writes: None });
            self.merge(&mut pass, &self.merger, &work, &work.out)?;
        }
        let out_len = groups.saturating_mul(self.cfg.out_hidden);
        let tokens = self.ctx.submit_read(enc, &work.out, out_len)?;
        let deepstack = work.taps.iter().map(|t| self.ctx.submit_read(self.ctx.encoder(), t, out_len)).collect::<Result<Vec<_>, _>>()?;
        self.ctx.check()?;
        Ok(VisionOutput { tokens, deepstack, count: groups, rows: gh / m, cols: gw / m })
    }

    /// Encodes an RGB picture, as [`VisionEncoder::encode_image`].
    pub fn encode_image(&self, rgb: &[f32], w: usize, h: usize) -> Result<VisionOutput, LlmError> {
        self.encode_image_within(rgb, w, h, crate::vision_prep::MAX_PIXELS)
    }

    /// Encodes an RGB picture resized to at most `max_pixels`, as
    /// [`VisionEncoder::encode_image_within`].
    pub fn encode_image_within(&self, rgb: &[f32], w: usize, h: usize, max_pixels: u64) -> Result<VisionOutput, LlmError> {
        let (values, grid) = vision::prepare_image(rgb, w, h, max_pixels)?;
        self.encode(&values, grid)
    }

    /// One encoder block on the residual stream `work.x`.
    fn block(&self, pass: &mut wgpu::ComputePass<'_>, b: &GpuBlock, work: &Work, cos: &wgpu::Buffer, sin: &wgpu::Buffer) -> Result<(), LlmError> {
        let n = work.n;
        let h = self.cfg.hidden as u32;
        let hd = self.cfg.head_dim() as u32;
        self.layer_norm(pass, &b.norm1, &work.x, &work.nrm, n)?;
        self.linear(pass, &b.qkv, &work.nrm, &work.qkv, n, 0, false)?;
        let threads = (n as usize).saturating_mul(h as usize);
        self.dispatch(pass, Kernel::Rotary, &[n, h, hd, STRIDE_X], &[cos, sin, &work.qkv], self.groups_1d(threads)?)?;
        let scale = 1.0 / (hd as f32).sqrt();
        let qb = n.div_ceil(QB);
        self.dispatch(pass, Kernel::Attention, &[n, h, self.cfg.heads as u32, scale.to_bits()], &[&work.qkv, &work.attn], [self.cfg.heads as u32, qb, 1])?;
        self.linear(pass, &b.proj, &work.attn, &work.x, n, 0, true)?;
        self.layer_norm(pass, &b.norm2, &work.x, &work.nrm, n)?;
        self.linear(pass, &b.fc1, &work.nrm, &work.mid, n, 1, false)?;
        self.linear(pass, &b.fc2, &work.mid, &work.x, n, 0, true)
    }

    /// A 2 × 2 merger of the residual stream into `out`.
    fn merge(&self, pass: &mut wgpu::ComputePass<'_>, m: &GpuMerger, work: &Work, out: &wgpu::Buffer) -> Result<(), LlmError> {
        let unit = (self.cfg.merge * self.cfg.merge) as u32;
        let groups = work.n / unit;
        if groups == 0 {
            return Err(err("fewer patches than one merged token"));
        }
        let rows = if m.post_shuffle { groups } else { work.n };
        self.layer_norm(pass, &m.norm, &work.x, &work.mnorm, rows)?;
        self.linear(pass, &m.fc1, &work.mnorm, &work.mmid, groups, 2, false)?;
        self.linear(pass, &m.fc2, &work.mmid, out, groups, 0, false)
    }

    /// `y = act(x · wᵀ + b)` (added into `y` when `acc`) for `n` token rows.
    fn linear(&self, pass: &mut wgpu::ComputePass<'_>, l: &GpuLinear, x: &wgpu::Buffer, y: &wgpu::Buffer, n: u32, act: u32, acc: bool) -> Result<(), LlmError> {
        let need_x = u64::from(n).saturating_mul(u64::from(l.inp)).saturating_mul(4);
        let need_y = u64::from(n).saturating_mul(u64::from(l.out)).saturating_mul(4);
        if n == 0 || x.size() < need_x || y.size() < need_y {
            return Err(err(format!("vision linear of {} × {} over {n} tokens with buffers of {} / {} bytes", l.out, l.inp, x.size(), y.size())));
        }
        self.dispatch(
            pass,
            Kernel::Linear,
            &[n, l.inp, l.out, act, u32::from(acc)],
            &[x, &l.w, &l.b, y],
            [l.out.div_ceil(TILE_OUT), n.div_ceil(TILE_TOKENS), 1],
        )
    }

    /// LayerNorm of `rows` rows of `norm.dim` values from `x` into `y`.
    fn layer_norm(&self, pass: &mut wgpu::ComputePass<'_>, norm: &GpuNorm, x: &wgpu::Buffer, y: &wgpu::Buffer, rows: u32) -> Result<(), LlmError> {
        let need = u64::from(rows).saturating_mul(u64::from(norm.dim)).saturating_mul(4);
        if rows == 0 || x.size() < need || y.size() < need {
            return Err(err("vision layer norm over buffers too small"));
        }
        let groups = [rows.min(STRIDE_X), rows.div_ceil(STRIDE_X), 1];
        self.dispatch(pass, Kernel::LayerNorm, &[norm.dim, NORM_EPS.to_bits(), STRIDE_X, rows], &[x, &norm.w, &norm.b, y], groups)
    }

    /// `x += y` over `count` values.
    fn add(&self, pass: &mut wgpu::ComputePass<'_>, x: &wgpu::Buffer, y: &wgpu::Buffer, count: usize) -> Result<(), LlmError> {
        let n = u32::try_from(count).map_err(|_| err("vision add too large"))?;
        if x.size() < u64::from(n) * 4 || y.size() < u64::from(n) * 4 {
            return Err(err("vision add over buffers too small"));
        }
        self.dispatch(pass, Kernel::Add, &[n, STRIDE_X], &[x, y], self.groups_1d(count)?)
    }

    /// Workgroup counts for `threads` threads of a 1-D kernel (wrapping to y past [`STRIDE_X`]).
    fn groups_1d(&self, threads: usize) -> Result<[u32; 3], LlmError> {
        let g = u32::try_from(threads.div_ceil(THREADS as usize).max(1)).map_err(|_| err("too many GPU threads"))?;
        Ok([g.min(STRIDE_X), g.div_ceil(STRIDE_X), 1])
    }

    /// Records one launch of `kernel` over `bufs` (bound after the `params` uniform, in order).
    fn dispatch(&self, pass: &mut wgpu::ComputePass<'_>, kernel: Kernel, params: &[u32], bufs: &[&wgpu::Buffer], groups: [u32; 3]) -> Result<(), LlmError> {
        let p = self.pipelines.iter().find(|p| p.kernel == kernel).ok_or_else(|| err(format!("GPU vision kernel {} missing", kernel.name())))?;
        if groups.iter().any(|g| *g == 0 || *g > self.max_groups) {
            return Err(err(format!("vision kernel {}: {groups:?} workgroups are over the GPU's limit", kernel.name())));
        }
        let uniform = self.ctx.uniform(params);
        let mut entries = Vec::with_capacity(bufs.len() + 1);
        entries.push(wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() });
        for (i, b) in bufs.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: i as u32 + 1, resource: b.as_entire_binding() });
        }
        let scope = self.ctx.device().push_error_scope(wgpu::ErrorFilter::Validation);
        let bg = self.ctx.device().create_bind_group(&wgpu::BindGroupDescriptor { label: Some(kernel.name()), layout: &p.layout, entries: &entries });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("vision kernel {}: {e}", kernel.name())));
        }
        pass.set_pipeline(&p.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder, or None (logged as skipped) on an adapter whose compute limits are below the
    /// kernels' (GL, Metal's 32 KiB, a software rasteriser): those fall back to the CPU in use.
    fn fits(r: Result<GpuVisionEncoder, LlmError>) -> Option<GpuVisionEncoder> {
        match r {
            Ok(g) => Some(g),
            Err(e) if e.to_string().contains("compute limits") => {
                eprintln!("SKIPPED: {e}");
                None
            }
            Err(e) => panic!("{e}"),
        }
    }

    fn tiny(hidden: usize, heads: usize, deepstack: Vec<usize>) -> VisionConfig {
        VisionConfig { depth: 3, hidden, heads, intermediate: hidden + 8, patch: 2, temporal_patch: 2, merge: 2, out_hidden: 24, positions: 9, deepstack }
    }

    fn picture(n: usize, inp: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n * inp)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    /// max |a − b| / max |b|.
    fn gap(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len());
        let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs())) / scale
    }

    /// Head widths 16 (half the threads idle in the P · V step) and 64 (the real model's, two
    /// quads per thread); grids with 8 tokens (one partial tile everywhere), 72 (past one
    /// linear tile and two key blocks, with tails) and 128 (whole tiles).
    #[test]
    fn synthetic_encoder_matches_the_cpu() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        eprintln!("adapter: {}", ctx.name());
        let mut ctx = Some(ctx);
        for (cfg, seed) in [(tiny(32, 2, vec![1]), 1u32), (tiny(128, 2, vec![0, 2]), 2)] {
            let cpu = VisionEncoder::synthetic(&cfg, seed).unwrap();
            let c = ctx.take().unwrap_or_else(|| GpuContext::new().unwrap());
            let Some(gpu) = fits(GpuVisionEncoder::with_context(c, &cpu)) else { return };
            assert!(gpu.bytes() > 0);
            for grid in [(2usize, 4usize), (6, 12), (8, 16)] {
                let n = grid.0 * grid.1;
                let patches = picture(n, cpu.patch.inp, seed + n as u32);
                let want = cpu.encode(&patches, grid).unwrap();
                let got = gpu.encode(&patches, grid).unwrap();
                assert_eq!((got.count, got.rows, got.cols), (want.count, want.rows, want.cols));
                assert_eq!(got.deepstack.len(), want.deepstack.len());
                let g = gap(&got.tokens, &want.tokens);
                let taps: Vec<f32> = got.deepstack.iter().zip(&want.deepstack).map(|(a, b)| gap(a, b)).collect();
                eprintln!("hidden {} grid {grid:?}: tokens {g:.2e}, taps {taps:?}", cfg.hidden);
                assert!(g < 1e-4, "tokens: {g}");
                assert!(taps.iter().all(|t| *t < 1e-4), "taps: {taps:?}");
            }
        }
    }

    /// GPU time of each kernel at the 4B model's shapes over 1024 and 3072 patches (a 512 × 512
    /// and a 1024 × 768 picture), one block's worth, ×10. `cargo test -p filmcraft-llm --features
    /// gpu --release --lib gpu_vision::tests::kernel_speed -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand on an idle machine"]
    fn kernel_speed() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let cfg = VisionConfig {
            depth: 1,
            hidden: 1024,
            heads: 16,
            intermediate: 4096,
            patch: 16,
            temporal_patch: 2,
            merge: 2,
            out_hidden: 2560,
            positions: 2304,
            deepstack: vec![],
        };
        let cpu = VisionEncoder::synthetic(&cfg, 7).unwrap();
        let Some(gpu) = fits(GpuVisionEncoder::with_context(ctx, &cpu)) else { return };
        let block = gpu.blocks.first().unwrap();
        for n in [1024u32, 3072] {
            let h = cfg.hidden;
            let nn = n as usize;
            let x = gpu.ctx.storage_init(&picture(nn, h, 3), "x").unwrap();
            let nrm = gpu.ctx.storage(nn * h, "nrm").unwrap();
            let qkv = gpu.ctx.storage_init(&picture(nn, 3 * h, 4), "qkv").unwrap();
            let attn = gpu.ctx.storage(nn * h, "attn").unwrap();
            let mid = gpu.ctx.storage(nn * cfg.intermediate, "mid").unwrap();
            let (cos, sin) = vision::rotary(64, 2, 32, nn / 32);
            let (cos, sin) = (gpu.ctx.storage_init(&cos, "cos").unwrap(), gpu.ctx.storage_init(&sin, "sin").unwrap());
            // 20 launches per submission; a warm-up submission (clocks ramp up), then the best
            // of 5 timed ones
            let time = |what: &str, flop: f64, rec: &dyn Fn(&mut wgpu::ComputePass<'_>) -> Result<(), LlmError>| {
                let run = || {
                    let mut enc = gpu.ctx.encoder();
                    {
                        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                        for _ in 0..20 {
                            rec(&mut pass).unwrap();
                        }
                    }
                    let idx = gpu.ctx.submit(enc).unwrap();
                    let t = std::time::Instant::now();
                    gpu.ctx.wait(Some(idx)).unwrap();
                    t.elapsed().as_secs_f64() * 1e3 / 20.0
                };
                run();
                let ms = (0..5).map(|_| run()).fold(f64::MAX, f64::min);
                eprintln!("{n} patches, {what}: {ms:.2} ms ({:.2} TFLOP/s)", flop / ms / 1e9);
            };
            let nf = f64::from(n);
            time("layer_norm", 0.0, &|p| gpu.layer_norm(p, &block.norm1, &x, &nrm, n));
            time("qkv linear 1024→3072", 2.0 * nf * 1024.0 * 3072.0, &|p| gpu.linear(p, &block.qkv, &nrm, &qkv, n, 0, false));
            time("rotary", 0.0, &|p| gpu.dispatch(p, Kernel::Rotary, &[n, 1024, 64, STRIDE_X], &[&cos, &sin, &qkv], gpu.groups_1d(nn * h).unwrap()));
            time("attention", 4.0 * nf * nf * 1024.0, &|p| {
                gpu.dispatch(p, Kernel::Attention, &[n, 1024, 16, 0.125f32.to_bits()], &[&qkv, &attn], [16, n.div_ceil(QB), 1])
            });
            time("proj linear 1024→1024 (+residual)", 2.0 * nf * 1024.0 * 1024.0, &|p| gpu.linear(p, &block.proj, &attn, &x, n, 0, true));
            time("fc1 linear 1024→4096 (GELU)", 2.0 * nf * 1024.0 * 4096.0, &|p| gpu.linear(p, &block.fc1, &nrm, &mid, n, 1, false));
            time("fc2 linear 4096→1024 (+residual)", 2.0 * nf * 4096.0 * 1024.0, &|p| gpu.linear(p, &block.fc2, &mid, &x, n, 0, true));
            let patches = picture(nn, cpu.patch.inp, 5);
            let t = std::time::Instant::now();
            let _ = gpu.encode(&patches, (32, nn / 32)).unwrap();
            eprintln!("{n} patches, whole encode (1 block + merger): {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        }
    }

    #[test]
    fn hostile_shapes_are_errors() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        // a head wider than the attention kernel stages
        let wide = VisionEncoder::synthetic(&tiny(512, 2, vec![]), 3).unwrap();
        let Err(e) = GpuVisionEncoder::with_context(ctx, &wide) else { panic!("a 256-wide head was accepted") };
        assert!(e.to_string().contains("heads"), "{e}");
        // a head not a multiple of 4
        let odd = VisionEncoder::synthetic(&tiny(6, 1, vec![]), 4).unwrap();
        assert!(GpuVisionEncoder::with_context(GpuContext::new().unwrap(), &odd).is_err());
        let cfg = tiny(32, 2, vec![]);
        let cpu = VisionEncoder::synthetic(&cfg, 5).unwrap();
        let Some(gpu) = fits(GpuVisionEncoder::with_context(GpuContext::new().unwrap(), &cpu)) else { return };
        let patches = picture(8, cpu.patch.inp, 9);
        assert!(gpu.encode(&patches, (2, 4)).is_ok());
        assert!(gpu.encode(&patches, (2, 3)).is_err(), "grid not a multiple of the merge size");
        assert!(gpu.encode(&patches, (4, 4)).is_err(), "too few patches");
        assert!(gpu.encode(&patches[..7], (2, 4)).is_err(), "truncated patches");
        assert!(gpu.encode(&patches, (0, 4)).is_err(), "empty grid");
        assert!(gpu.encode(&[], (0, 0)).is_err(), "nothing");
        assert!(gpu.encode_image(&[0.5; 3], 1, 0).is_err(), "an empty picture");
        assert!(gpu.encode_image(&[0.5; 5], 1, 1).is_err(), "a picture with the wrong value count");
    }
}
