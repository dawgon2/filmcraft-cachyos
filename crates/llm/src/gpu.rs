//! GPU (wgpu) kernels for inference (AI3.3b): the CPU path in [`crate::kquant`] / [`crate::quant`]
//! stays the reference and the fallback. Matrix × f32 vector products whose weights live on the GPU
//! block by block (Q4_K 152 bytes: the GGUF block with its scales unpacked; Q6_K 212 bytes: the
//! GGUF block with its scale widened to f32; Q8_0 36 bytes: f32 scale + 32 quants; F32 as is; no
//! dequantised copy), plus the element kernels of a transformer layer (`gpu_ops.wgsl`).
//!
//! Two ways to run them: [`GpuContext::matvec`] (slice in, `Vec` out; one round trip, for tests
//! and one-offs) and the crate-internal recording API ([`GpuContext::record_matvec`],
//! [`Dispatch`]) that writes into caller-owned buffers inside the caller's command encoder, so a
//! whole token runs as one submission ([`crate::gpu_forward`]).
//!
//! Every failure (no adapter, a build without a wgpu backend, a validation or out-of-memory
//! error, a lost device) is an [`LlmError`], never a panic: callers use the CPU instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wgpu::util::DeviceExt;

use crate::LlmError;
use crate::kquant::{BlockQ4K, BlockQ6K, QK_K};
use crate::quant::{BlockQ8_0, QK8_0, err};

#[path = "gpu_matmul.rs"]
mod matmul;

/// u32 words per Q4_K block on the GPU (152 bytes: the GGUF block is 144, its 6-bit scales / mins are
/// stored unpacked here, as in [`BlockQ4K`]).
const Q4K_WORDS: usize = 38;
/// u32 words per Q6_K block on the GPU (212 bytes: ql, qh, the 16 i8 scales, d as f32).
const Q6K_WORDS: usize = 53;
/// u32 words per Q8_0 block on the GPU (36 bytes: d as f32, the 32 i8 quants).
const Q8_0_WORDS: usize = 9;
/// Output rows per workgroup of the matvec kernels (`ROWS` in `gpu_matvec.wgsl`).
const MATVEC_ROWS: u32 = 4;
/// Workgroups along x before the grid wraps to y (under every adapter's 65 535 per dimension).
const STRIDE_X: u32 = 32_768;
/// Longest wait for a submission. A token of a 3 B model takes ~40 ms on a discrete GPU and
/// ~1.5 s on a weak integrated one; past this the GPU is treated as hung and the context fails.
const GPU_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A wgpu device with the inference pipelines.
pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    failed: Arc<AtomicBool>,
    /// Test hook: every wait times out ([`Self::force_timeouts`]).
    #[cfg(test)]
    force_timeout: AtomicBool,
    q4k: wgpu::ComputePipeline,
    q6k: wgpu::ComputePipeline,
    q8_0: wgpu::ComputePipeline,
    f32: wgpu::ComputePipeline,
    /// F32 matrices whose width is not a multiple of 4 (`gpu_matvec_f32.wgsl`).
    f32_scalar: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The element kernels' pipelines and their (automatic) bind group layouts, in [`Op::ALL`] order.
    ops: Vec<(Op, wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
    max_binding: u64,
    max_groups: u32,
    name: String,
    discrete: bool,
    matmul: matmul::MatmulPipelines,
    /// The read-back buffer [`Self::submit_read`] reuses (a token's logits every step); taken
    /// while in use and put back only after a clean read, so a failed map never leaves it mapped.
    read_buf: std::sync::Mutex<Option<wgpu::Buffer>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Q4K,
    Q6K,
    Q8_0,
    F32,
}

/// A matrix (`rows` × `cols`) uploaded to the GPU, with its kernel's parameters.
pub struct GpuMatrix {
    buf: wgpu::Buffer,
    params: wgpu::Buffer,
    rows: u32,
    cols: u32,
    kind: Kind,
}

impl GpuMatrix {
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn cols(&self) -> usize {
        self.cols as usize
    }
    /// Bytes on the GPU.
    pub fn bytes(&self) -> u64 {
        self.buf.size()
    }
}

/// The element kernels of `gpu_ops.wgsl`; buffers bind in the order its sections list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// params `[n, eps]`, x, weight, y; one workgroup per token (`n` values each).
    RmsNorm,
    /// params `[n_heads, head_dim, interleaved, sin_at]`, step, cos / sin table, vector (in place);
    /// workgroups along y are consecutive tokens from the step's position.
    Rope,
    /// params `[kv_dim, max_pos]`, step, k, v, k cache, v cache; y as for `Rope`.
    KvStore,
    /// params `[n_head, n_kv_head, head_dim, max_pos, kv_dim, sqrt(head_dim)]`, step, q, k cache,
    /// v cache, scores (`n_head × max_pos`), out.
    Attention,
    /// params `[n]`, g (in place), u.
    SiluMul,
    /// params `[n, period]`, x (in place), y (`y[i % period]` when period > 0: a bias added to
    /// every token).
    Add,
    /// Causal attention of a batch of new tokens (prefill): params as `Attention`, then `q_stride`
    /// (values per token in q / out); step, q, k cache, v cache, out. Workgroups: (head, token).
    AttentionPrefill,
    /// `AttentionPrefill` for 8 tokens per workgroup (head dims ≤ 256, a multiple of 4):
    /// workgroups (head, ⌈tokens / 8⌉).
    AttentionPrefill8,
    /// Qwen3's q / k norm: params `[head_dim, eps, n_heads]`, weight (`head_dim`), vector (in
    /// place; `n_heads × head_dim` per token); workgroups (head, token).
    HeadNorm,
}

impl Op {
    const ALL: [Op; 9] = [Op::RmsNorm, Op::Rope, Op::KvStore, Op::Attention, Op::SiluMul, Op::Add, Op::AttentionPrefill, Op::AttentionPrefill8, Op::HeadNorm];

    fn name(self) -> &'static str {
        match self {
            Op::RmsNorm => "rms_norm",
            Op::Rope => "rope",
            Op::KvStore => "kv_store",
            Op::Attention => "attention",
            Op::SiluMul => "silu_mul",
            Op::Add => "add",
            Op::AttentionPrefill => "attention_prefill",
            Op::AttentionPrefill8 => "attention_prefill8",
            Op::HeadNorm => "head_norm",
        }
    }

    /// The op's WGSL module: its `//// name` section of `gpu_ops.wgsl`.
    fn source(self) -> Option<&'static str> {
        include_str!("gpu_ops.wgsl").split("\n//// ").skip(1).find_map(|sec| {
            let (head, body) = sec.split_once('\n')?;
            (head.trim() == self.name()).then_some(body)
        })
    }
}

/// Which pipeline a [`Dispatch`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipe {
    Mat(Kind),
    MatF32Scalar,
    Op(Op),
}

/// One recorded kernel launch: its pipeline, its bind group (caller-owned buffers) and its
/// workgroup counts. Built once and replayed every token.
pub(crate) struct Dispatch {
    pipe: Pipe,
    bg: wgpu::BindGroup,
    groups: [u32; 3],
}

#[cfg(test)]
impl Dispatch {
    /// A matrix × vector launch (profiling tests).
    pub(crate) fn is_matvec(&self) -> bool {
        matches!(self.pipe, Pipe::Mat(_) | Pipe::MatF32Scalar)
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn word(s: &[u8]) -> u32 {
    s.iter().rev().fold(0u32, |acc, &v| (acc << 8) | u32::from(v))
}

/// The GPU words of one Q4_K block (see `gpu_matvec.wgsl`).
fn pack_q4k(b: &BlockQ4K, out: &mut Vec<u32>) {
    out.push(b.d.to_bits());
    out.push(b.dmin.to_bits());
    for half in [&b.scales[..4], &b.scales[4..], &b.mins[..4], &b.mins[4..]] {
        out.push(word(half));
    }
    out.extend(b.qs.as_chunks::<4>().0.iter().map(|c| word(c)));
}

/// The GPU words of one Q6_K block (see `gpu_matvec.wgsl`).
fn pack_q6k(b: &BlockQ6K, out: &mut Vec<u32>) {
    out.extend(b.ql.as_chunks::<4>().0.iter().map(|c| word(c)));
    out.extend(b.qh.as_chunks::<4>().0.iter().map(|c| word(c)));
    out.extend(b.scales.as_chunks::<4>().0.iter().map(|c| word(&c.map(|v| v.to_le_bytes()[0]))));
    out.push(b.d.to_bits());
}

/// The GPU words of one Q8_0 block (see `gpu_matvec.wgsl`).
fn pack_q8_0(b: &BlockQ8_0, out: &mut Vec<u32>) {
    out.push(b.d.to_bits());
    out.extend(b.qs.as_chunks::<4>().0.iter().map(|c| word(&c.map(|v| v.to_le_bytes()[0]))));
}

fn pack_f32(v: &f32, out: &mut Vec<u32>) {
    out.push(v.to_bits());
}

impl GpuContext {
    /// The default high-performance adapter, or an error when there is none.
    pub fn new() -> Result<Self, LlmError> {
        if wgpu::Instance::enabled_backend_features().is_empty() {
            return Err(err("this build has no GPU backend"));
        }
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
        )
        .map_err(|e| err(format!("no GPU adapter: {e}")))?;
        let limits = adapter.limits();
        let info = adapter.get_info();
        let (name, discrete) = (info.name, info.device_type == wgpu::DeviceType::DiscreteGpu);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("filmcraft-llm"),
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .map_err(|e| err(format!("GPU device: {e}")))?;
        let failed = Arc::new(AtomicBool::new(false));
        let flag = failed.clone();
        // wgpu's default handler panics; record the error and fail the call instead
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            log::warn!("LLM GPU error: {e}");
            flag.store(true, Ordering::SeqCst);
        }));
        let flag = failed.clone();
        device.set_device_lost_callback(move |reason, msg| {
            log::warn!("LLM GPU device lost ({reason:?}): {msg}");
            flag.store(true, Ordering::SeqCst);
        });
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llm-matvec"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_matvec.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llm-matvec"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llm-matvec"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader_f32 = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llm-matvec-f32"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_matvec_f32.wgsl").into()),
        });
        let pipeline_in = |module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = |entry: &str| pipeline_in(&shader, entry);
        let (q4k, q6k, q8_0, f32) = (pipeline("matvec_q4k"), pipeline("matvec_q6k"), pipeline("matvec_q8_0"), pipeline("matvec_f32"));
        let f32_scalar = pipeline_in(&shader_f32, "matvec_f32_scalar");
        let mut ops = Vec::with_capacity(Op::ALL.len());
        for op in Op::ALL {
            let src = op.source().ok_or_else(|| err(format!("GPU kernel {} missing", op.name())))?;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(op.name()), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let p = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(op.name()),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let l = p.get_bind_group_layout(0);
            ops.push((op, p, l));
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("GPU kernels: {e}")));
        }
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let max_groups = limits.max_compute_workgroups_per_dimension;
        let matmul = matmul::MatmulPipelines::new(&device);
        let ctx = Self {
            device,
            queue,
            failed,
            #[cfg(test)]
            force_timeout: AtomicBool::new(false),
            q4k,
            q6k,
            q8_0,
            f32,
            f32_scalar,
            layout,
            ops,
            max_binding,
            max_groups,
            name,
            discrete,
            matmul,
            read_buf: std::sync::Mutex::new(None),
        };
        ctx.check()?;
        Ok(ctx)
    }

    /// The adapter's name, for logs.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The device, for sibling modules that build their own pipelines (`gpu_vision`).
    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Whether the adapter is a discrete GPU (integrated ones share the CPU's memory bandwidth and
    /// are often slower than the CPU kernels for this work).
    pub fn is_discrete(&self) -> bool {
        self.discrete
    }

    /// Largest storage buffer one kernel can bind, in bytes.
    pub fn max_binding(&self) -> u64 {
        self.max_binding
    }

    /// An error once the device reported an uncaptured error or was lost: nothing it computed
    /// since can be trusted.
    pub(crate) fn check(&self) -> Result<(), LlmError> {
        if self.failed.load(Ordering::SeqCst) { Err(err("the GPU reported an error")) } else { Ok(()) }
    }

    /// Test hook: from now on every wait times out (as on a hung GPU) and fails the context.
    #[cfg(test)]
    pub(crate) fn force_timeouts(&self) {
        self.force_timeout.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail(&self, why: &str) {
        log::warn!("LLM GPU: {why}");
        self.failed.store(true, Ordering::SeqCst);
    }

    /// Uploads `rows` rows of Q4_K blocks (`weights.len()` must be a multiple of `rows`).
    pub fn upload_q4k(&self, weights: &[BlockQ4K], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::Q4K, Q4K_WORDS, QK_K, pack_q4k)
    }

    /// Uploads `rows` rows of Q6_K blocks (`weights.len()` must be a multiple of `rows`).
    pub fn upload_q6k(&self, weights: &[BlockQ6K], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::Q6K, Q6K_WORDS, QK_K, pack_q6k)
    }

    /// Uploads `rows` rows of Q8_0 blocks (`weights.len()` must be a multiple of `rows`).
    pub fn upload_q8_0(&self, weights: &[BlockQ8_0], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::Q8_0, Q8_0_WORDS, QK8_0, pack_q8_0)
    }

    /// Uploads a row-major f32 matrix of `rows` rows (`weights.len()` must be a multiple of `rows`).
    pub fn upload_f32(&self, weights: &[f32], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::F32, 1, 1, pack_f32)
    }

    /// Uploads a model weight in whatever format it was loaded.
    pub(crate) fn upload_weight(&self, w: &crate::weights::Weight) -> Result<GpuMatrix, LlmError> {
        use crate::weights::Data;
        let m = match w.data() {
            Data::F32(v) => self.upload_f32(v, w.rows),
            Data::Q8_0(v) => self.upload_q8_0(v, w.rows),
            Data::Q4K(v) => self.upload_q4k(v, w.rows),
            Data::Q6K(v) => self.upload_q6k(v, w.rows),
        }
        .map_err(|e| err(format!("{}: {e}", w.name)))?;
        if (m.rows(), m.cols()) != (w.rows, w.cols) {
            return Err(err(format!("{}: uploaded as {}×{}, expected {}×{}", w.name, m.rows(), m.cols(), w.rows, w.cols)));
        }
        Ok(m)
    }

    fn upload<B>(
        &self,
        weights: &[B],
        rows: usize,
        kind: Kind,
        per_block: usize,
        block_len: usize,
        pack: fn(&B, &mut Vec<u32>),
    ) -> Result<GpuMatrix, LlmError> {
        if rows == 0 || weights.is_empty() || !weights.len().is_multiple_of(rows) {
            return Err(err(format!("{} {kind:?} blocks are not {rows} whole rows", weights.len())));
        }
        let blocks = weights.len() / rows;
        let cols = blocks.checked_mul(block_len).ok_or_else(|| err("matrix too wide"))?;
        let n_words = weights.len().checked_mul(per_block).ok_or_else(|| err("matrix too large"))?;
        let bytes = n_words.saturating_mul(4);
        // the shader indexes words with u32
        if bytes as u64 > self.max_binding || u32::try_from(n_words).is_err() {
            return Err(err(format!("{kind:?} matrix of {bytes} bytes is over the GPU's binding limit of {}", self.max_binding)));
        }
        let mut words = Vec::new();
        words.try_reserve_exact(n_words).map_err(|_| err("out of memory packing a matrix"))?;
        for b in weights {
            pack(b, &mut words);
        }
        let rows = u32::try_from(rows).map_err(|_| err("too many rows"))?;
        let blocks = u32::try_from(blocks).map_err(|_| err("too many blocks per row"))?;
        let cols = u32::try_from(cols).map_err(|_| err("too many columns"))?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-weights"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("GPU out of memory for a {bytes}-byte matrix: {e}")));
        }
        let params = self.uniform(&[rows, blocks, STRIDE_X, cols]);
        // the upload's staging copy lives until a submission retires it; unretired, a whole
        // model's (Qwen3-VL 4B Q8_0: 4.5 GB, host-visible VRAM with resizable BAR) pushed the
        // weights and KV caches out of VRAM and doubled the time per token
        let index = self.queue.submit(std::iter::empty());
        self.wait(Some(index))?;
        Ok(GpuMatrix { buf, params, rows, cols, kind })
    }

    /// A uniform buffer holding `words` (padded to 16 bytes), writable with [`Self::write_u32`].
    pub(crate) fn uniform(&self, words: &[u32]) -> wgpu::Buffer {
        let mut padded = words.to_vec();
        padded.resize(words.len().div_ceil(4).max(1) * 4, 0);
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-params"),
            contents: bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// A zeroed storage buffer of `n` f32 (at least one), within the binding limit.
    pub(crate) fn storage(&self, n: usize, label: &str) -> Result<wgpu::Buffer, LlmError> {
        let size = (n.max(1) as u64)
            .checked_mul(4)
            .filter(|s| *s <= self.max_binding)
            .ok_or_else(|| err(format!("{label}: {n} values are over the GPU's binding limit")))?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("GPU out of memory for {label}: {e}")));
        }
        Ok(buf)
    }

    /// A storage buffer holding `data`.
    pub(crate) fn storage_init(&self, data: &[f32], label: &str) -> Result<wgpu::Buffer, LlmError> {
        let buf = self.storage(data.len(), label)?;
        self.write(&buf, 0, data)?;
        Ok(buf)
    }

    /// Queues `data` into `buf` at f32 index `at` (done before the next submission).
    pub(crate) fn write(&self, buf: &wgpu::Buffer, at: usize, data: &[f32]) -> Result<(), LlmError> {
        let off = (at as u64).saturating_mul(4);
        let len = (data.len() as u64).saturating_mul(4);
        if off.saturating_add(len) > buf.size() {
            return Err(err("GPU write past the end of a buffer"));
        }
        if !data.is_empty() {
            self.queue.write_buffer(buf, off, bytemuck::cast_slice(data));
        }
        Ok(())
    }

    /// Queues `words` into the uniform `buf` (done before the next submission).
    pub(crate) fn write_u32(&self, buf: &wgpu::Buffer, words: &[u32]) -> Result<(), LlmError> {
        if (words.len() as u64).saturating_mul(4) > buf.size() {
            return Err(err("GPU write past the end of a buffer"));
        }
        self.queue.write_buffer(buf, 0, bytemuck::cast_slice(words));
        Ok(())
    }

    /// The launch of `y = m · x` (x holds `m.cols()` f32, y receives `m.rows()` f32).
    pub(crate) fn matvec_dispatch(&self, m: &GpuMatrix, x: &wgpu::Buffer, y: &wgpu::Buffer) -> Result<Dispatch, LlmError> {
        self.matvec_batch_dispatch(m, x, y, 1)
    }

    /// The launch of `y[t · rows ..] = m · x[t · cols ..]` for `n` token-major vectors: a matvec
    /// per token in one dispatch (every token re-reads the weights; [`Self::record_matmul`] is
    /// the fast path for K-quant weights).
    pub(crate) fn matvec_batch_dispatch(&self, m: &GpuMatrix, x: &wgpu::Buffer, y: &wgpu::Buffer, n: u32) -> Result<Dispatch, LlmError> {
        let x_len = u64::from(m.cols).saturating_mul(u64::from(n));
        let y_len = u64::from(m.rows).saturating_mul(u64::from(n));
        if n == 0 || u32::try_from(x_len).is_err() || u32::try_from(y_len).is_err() || x.size() < x_len.saturating_mul(4) || y.size() < y_len.saturating_mul(4)
        {
            return Err(err(format!("matvec of {}×{} × {n} with buffers of {} and {} bytes", m.rows, m.cols, x.size(), y.size())));
        }
        // `gpu_matvec.wgsl`: MATVEC_ROWS rows per workgroup, activations as vec4
        let scalar = m.kind == Kind::F32 && !m.cols.is_multiple_of(4);
        let (pipe, wgs) = if scalar { (Pipe::MatF32Scalar, m.rows) } else { (Pipe::Mat(m.kind), m.rows.div_ceil(MATVEC_ROWS)) };
        let groups = [wgs.min(STRIDE_X), wgs.div_ceil(STRIDE_X), n];
        if groups.iter().any(|g| *g > self.max_groups) {
            return Err(err(format!("{} rows × {n} are over the GPU's workgroup limit", m.rows)));
        }
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llm-matvec"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: m.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: m.buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: x.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: y.as_entire_binding() },
            ],
        });
        Ok(Dispatch { pipe, bg, groups })
    }

    fn op(&self, op: Op) -> Option<&(Op, wgpu::ComputePipeline, wgpu::BindGroupLayout)> {
        self.ops.iter().find(|o| o.0 == op)
    }

    fn pipeline(&self, pipe: Pipe) -> Option<&wgpu::ComputePipeline> {
        match pipe {
            Pipe::Mat(Kind::Q4K) => Some(&self.q4k),
            Pipe::Mat(Kind::Q6K) => Some(&self.q6k),
            Pipe::Mat(Kind::Q8_0) => Some(&self.q8_0),
            Pipe::Mat(Kind::F32) => Some(&self.f32),
            Pipe::MatF32Scalar => Some(&self.f32_scalar),
            Pipe::Op(op) => self.op(op).map(|o| &o.1),
        }
    }

    /// Workgroup counts for `n` threads in groups of `per` (one dimension).
    pub(crate) fn groups(&self, n: usize, per: usize) -> Result<[u32; 2], LlmError> {
        let g = u32::try_from(n.div_ceil(per.max(1)).max(1))
            .ok()
            .filter(|g| *g <= self.max_groups)
            .ok_or_else(|| err(format!("{n} GPU threads are over the workgroup limit")))?;
        Ok([g, 1])
    }

    /// Workgroup counts for `n` threads in groups of `per`, wrapping to y past [`STRIDE_X`]
    /// groups (the element kernels `silu_mul` and `add` index `x + y · groups_x · per`).
    pub(crate) fn groups_2d(&self, n: usize, per: usize) -> Result<[u32; 2], LlmError> {
        let g = u32::try_from(n.div_ceil(per.max(1)).max(1)).map_err(|_| err(format!("{n} GPU threads are over the workgroup limit")))?;
        let gs = [g.min(STRIDE_X), g.div_ceil(STRIDE_X)];
        if gs.iter().any(|g| *g > self.max_groups) {
            return Err(err(format!("{n} GPU threads are over the workgroup limit")));
        }
        Ok(gs)
    }

    /// The launch of element kernel `op` over `bufs` (bound in order, see [`Op`]).
    pub(crate) fn op_dispatch(&self, op: Op, bufs: &[&wgpu::Buffer], groups: [u32; 2]) -> Result<Dispatch, LlmError> {
        let (_, _, layout) = self.op(op).ok_or_else(|| err(format!("GPU kernel {} missing", op.name())))?;
        if groups.iter().any(|g| *g == 0 || *g > self.max_groups) {
            return Err(err(format!("{}: {groups:?} workgroups", op.name())));
        }
        let groups = [groups[0], groups[1], 1];
        let entries: Vec<wgpu::BindGroupEntry<'_>> =
            bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(op.name()), layout, entries: &entries });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(err(format!("{}: {e}", op.name())));
        }
        Ok(Dispatch { pipe: Pipe::Op(op), bg, groups })
    }

    /// Runs `d` inside `pass`.
    pub(crate) fn run(&self, pass: &mut wgpu::ComputePass<'_>, d: &Dispatch) {
        let Some(p) = self.pipeline(d.pipe) else {
            self.fail("a GPU kernel is missing");
            return;
        };
        pass.set_pipeline(p);
        pass.set_bind_group(0, &d.bg, &[]);
        pass.dispatch_workgroups(d.groups[0], d.groups[1], d.groups[2]);
    }

    /// Records `ds` in order as one compute pass of `enc` (wgpu orders the dispatches' buffer
    /// accesses).
    pub(crate) fn record(&self, enc: &mut wgpu::CommandEncoder, ds: &[Dispatch]) {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("llm"), timestamp_writes: None });
        for d in ds {
            self.run(&mut pass, d);
        }
    }

    /// Records `y = m · x` into `enc`: `x` holds `m.cols()` f32, `y` receives `m.rows()` f32. A
    /// buffer too small for the matrix records nothing and fails the context (the next
    /// [`Self::check`], submission or read-back is an error).
    pub(crate) fn record_matvec(&self, enc: &mut wgpu::CommandEncoder, m: &GpuMatrix, x: &wgpu::Buffer, y: &wgpu::Buffer) {
        match self.matvec_dispatch(m, x, y) {
            Ok(d) => self.record(enc, std::slice::from_ref(&d)),
            Err(e) => self.fail(&e.to_string()),
        }
    }

    /// Records `y[t · rows ..] = m · x[t · cols ..]` for `n` token-major vectors into `enc`: the
    /// tiled matmul for Q4_K / Q6_K weights and Q8_0 weights with a multiple of 256 columns (when
    /// the adapter runs it), else a matvec per token.
    pub(crate) fn record_batch(&self, enc: &mut wgpu::CommandEncoder, m: &GpuMatrix, x: &wgpu::Buffer, y: &wgpu::Buffer, n: u32) -> Result<(), LlmError> {
        let tiled = matches!(m.kind, Kind::Q4K | Kind::Q6K) || (m.kind == Kind::Q8_0 && m.cols.is_multiple_of(QK_K as u32));
        if tiled && self.matmul.ready() {
            return self.record_matmul(enc, m, x, y, n);
        }
        let d = self.matvec_batch_dispatch(m, x, y, n)?;
        self.record(enc, std::slice::from_ref(&d));
        Ok(())
    }

    /// A new command encoder.
    pub(crate) fn encoder(&self) -> wgpu::CommandEncoder {
        self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("llm") })
    }

    /// Submits `enc` without waiting.
    pub(crate) fn submit(&self, enc: wgpu::CommandEncoder) -> Result<wgpu::SubmissionIndex, LlmError> {
        self.check()?;
        Ok(self.queue.submit([enc.finish()]))
    }

    /// Waits (at most [`GPU_WAIT`]) for submission `index` (`None`: the latest); a timeout fails
    /// the context.
    pub(crate) fn wait(&self, index: Option<wgpu::SubmissionIndex>) -> Result<(), LlmError> {
        #[cfg(test)]
        if self.force_timeout.load(Ordering::SeqCst) {
            self.fail("GPU wait: timed out (forced by a test)");
            return Err(err("GPU wait: timed out (forced by a test)"));
        }
        if let Err(e) = self.device.poll(wgpu::PollType::Wait { submission_index: index, timeout: Some(GPU_WAIT) }) {
            self.fail(&format!("GPU wait: {e}"));
            return Err(err(format!("GPU wait: {e}")));
        }
        self.check()
    }

    /// Submits `enc` plus a copy of the first `n` f32 of `src`, waits and returns them.
    pub(crate) fn submit_read(&self, mut enc: wgpu::CommandEncoder, src: &wgpu::Buffer, n: usize) -> Result<Vec<f32>, LlmError> {
        let bytes = (n as u64).saturating_mul(4);
        if bytes == 0 || bytes > src.size() {
            return Err(err(format!("read-back of {n} values from a {}-byte buffer", src.size())));
        }
        self.check()?;
        let cached = self.read_buf.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().filter(|b| b.size() >= bytes);
        let read = cached.unwrap_or_else(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llm-read"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        enc.copy_buffer_to_buffer(src, 0, &read, 0, bytes);
        self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..bytes).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.wait(None)?;
        // the poll returned, so the callback should have run; never block on it regardless
        match rx.recv_timeout(GPU_WAIT) {
            Ok(r) => r.map_err(|e| err(format!("GPU read-back: {e}")))?,
            Err(e) => {
                self.fail(&format!("GPU read-back never completed: {e}"));
                return Err(err(format!("GPU read-back never completed: {e}")));
            }
        }
        let data = read.slice(..bytes).get_mapped_range().map_err(|e| err(format!("GPU read-back: {e}")))?;
        let out: Vec<f32> = data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        drop(data);
        read.unmap();
        *self.read_buf.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(read);
        Ok(out)
    }

    /// `out[r] = m[r] · x`, with `x.len() == m.cols()` (one round trip).
    pub fn matvec(&self, m: &GpuMatrix, x: &[f32]) -> Result<Vec<f32>, LlmError> {
        if x.len() != m.cols() {
            return Err(err(format!("vector of {} values for a matrix of {} columns", x.len(), m.cols())));
        }
        let xbuf = self.storage_init(x, "llm-x")?;
        let ybuf = self.storage(m.rows(), "llm-y")?;
        let mut enc = self.encoder();
        self.record_matvec(&mut enc, m, &xbuf, &ybuf);
        self.submit_read(enc, &ybuf, m.rows())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kquant::{dequantize_q4k, dequantize_q6k};

    /// Deterministic blocks covering every scale / min / nibble value.
    fn blocks(n: usize, seed: u32) -> Vec<BlockQ4K> {
        let mut s = seed;
        let mut next = move || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 24) as u8
        };
        (0..n)
            .map(|i| BlockQ4K {
                d: 0.01 + (i % 7) as f32 * 0.003,
                dmin: 0.005 + (i % 5) as f32 * 0.002,
                scales: std::array::from_fn(|_| next() & 63),
                mins: std::array::from_fn(|_| next() & 63),
                qs: std::array::from_fn(|_| next()),
            })
            .collect()
    }

    fn next_byte(s: &mut u32) -> u8 {
        *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*s >> 24) as u8
    }

    fn blocks6(n: usize, seed: u32) -> Vec<BlockQ6K> {
        let mut s = seed;
        (0..n)
            .map(|i| BlockQ6K {
                ql: std::array::from_fn(|_| next_byte(&mut s)),
                qh: std::array::from_fn(|_| next_byte(&mut s)),
                scales: std::array::from_fn(|_| i8::from_le_bytes([next_byte(&mut s)])),
                d: 0.002 + (i % 3) as f32 * 0.001,
            })
            .collect()
    }

    fn blocks8(n: usize, seed: u32) -> Vec<BlockQ8_0> {
        let mut s = seed;
        (0..n).map(|i| BlockQ8_0 { d: 0.004 + (i % 5) as f32 * 0.001, qs: std::array::from_fn(|_| i8::from_le_bytes([next_byte(&mut s)])) }).collect()
    }

    fn assert_close(y: &[f32], deq: &[f32], x: &[f32], what: &str) {
        let cols = x.len();
        for (r, got) in y.iter().enumerate() {
            let row = &deq[r * cols..(r + 1) * cols];
            let want: f64 = row.iter().zip(x).map(|(a, b)| f64::from(*a) * f64::from(*b)).sum();
            let mag: f64 = row.iter().zip(x).map(|(a, b)| (f64::from(*a) * f64::from(*b)).abs()).sum();
            assert!((f64::from(*got) - want).abs() <= 1e-5 * mag + 1e-6, "{what} row {r} of {}: {got} vs {want}", y.len());
        }
    }

    #[test]
    fn q6k_matvec_matches_the_cpu() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        for (rows, per_row) in [(3usize, 1usize), (64, 8), (70_001, 1)] {
            let w = blocks6(rows * per_row, rows as u32);
            let cols = per_row * QK_K;
            let x: Vec<f32> = (0..cols).map(|i| ((i * 53 % 97) as f32 - 48.0) / 20.0).collect();
            let mut deq = Vec::new();
            dequantize_q6k(&w, &mut deq);
            let m = ctx.upload_q6k(&w, rows).unwrap();
            assert_eq!((m.rows(), m.cols()), (rows, cols));
            let y = ctx.matvec(&m, &x).unwrap();
            assert_eq!(y.len(), rows);
            assert_close(&y, &deq, &x, "Q6_K");
        }
    }

    #[test]
    fn q4k_matvec_matches_the_cpu() {
        let ctx = match GpuContext::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIPPED: {e}");
                return;
            }
        };
        eprintln!("adapter: {}", ctx.name());
        // 70 001 rows: many workgroups (STRIDE_X: `matvec_tiling_edges_match_the_cpu`)
        for (rows, per_row) in [(3usize, 1usize), (64, 8), (70_001, 1)] {
            let w = blocks(rows * per_row, rows as u32);
            let cols = per_row * QK_K;
            let x: Vec<f32> = (0..cols).map(|i| ((i * 37 % 101) as f32 - 50.0) / 25.0).collect();
            let mut deq = Vec::new();
            dequantize_q4k(&w, &mut deq);
            let m = ctx.upload_q4k(&w, rows).unwrap();
            assert_eq!((m.rows(), m.cols()), (rows, cols));
            let y = ctx.matvec(&m, &x).unwrap();
            assert_eq!(y.len(), rows);
            assert_close(&y, &deq, &x, "Q4_K");
        }
    }

    #[test]
    fn q8_0_and_f32_matvecs_match_the_cpu() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        // 70 001 rows: many workgroups; 65 blocks per row: more blocks than threads
        for (rows, per_row) in [(3usize, 1usize), (64, 65), (70_001, 1)] {
            let w = blocks8(rows * per_row, rows as u32);
            let cols = per_row * QK8_0;
            let x: Vec<f32> = (0..cols).map(|i| ((i * 29 % 89) as f32 - 44.0) / 30.0).collect();
            let mut deq = Vec::new();
            crate::quant::dequantize_q8_0(&w, &mut deq);
            let m = ctx.upload_q8_0(&w, rows).unwrap();
            assert_eq!((m.rows(), m.cols()), (rows, cols));
            assert_close(&ctx.matvec(&m, &x).unwrap(), &deq, &x, "Q8_0");
            // the dequantised matrix as F32
            let f = ctx.upload_f32(&deq, rows).unwrap();
            assert_eq!((f.rows(), f.cols()), (rows, cols));
            assert_close(&ctx.matvec(&f, &x).unwrap(), &deq, &x, "F32");
        }
    }

    /// The four-rows-per-workgroup kernels' edges (AI4.2): row counts that are not a multiple
    /// of 4 (the last workgroup's missing rows are not written), units per row that are not a
    /// multiple of the 64 threads, 131 077 rows (32 770 workgroups: past STRIDE_X, so the
    /// second dispatch dimension is used), F32 widths that are not a multiple of 4 (the scalar
    /// kernel), and three token-major vectors in one dispatch (workgroup z), for every kind.
    #[test]
    fn matvec_tiling_edges_match_the_cpu() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let check = |m: &GpuMatrix, deq: &[f32], what: &str| {
            let (rows, cols) = (m.rows(), m.cols());
            let x: Vec<f32> = (0..cols * 3).map(|i| ((i * 31 % 103) as f32 - 51.0) / 27.0).collect();
            // one vector
            assert_close(&ctx.matvec(m, &x[..cols]).unwrap(), deq, &x[..cols], what);
            // three at once; y starts non-zero so a missed row shows
            let xb = ctx.storage_init(&x, "x").unwrap();
            let yb = ctx.storage_init(&vec![7.0; rows * 3 + 1], "y").unwrap();
            let d = ctx.matvec_batch_dispatch(m, &xb, &yb, 3).unwrap();
            let mut enc = ctx.encoder();
            ctx.record(&mut enc, std::slice::from_ref(&d));
            let y = ctx.submit_read(enc, &yb, rows * 3 + 1).unwrap();
            for t in 0..3 {
                assert_close(&y[t * rows..(t + 1) * rows], deq, &x[t * cols..(t + 1) * cols], &format!("{what} token {t}"));
            }
            assert_eq!(y[rows * 3], 7.0, "{what}: wrote past the last row");
        };
        for (rows, per_row) in [(1usize, 1usize), (2, 3), (5, 2), (6, 5), (7, 1), (9, 9), (131_077, 1)] {
            let what = |k: &str| format!("{k} {rows}×{per_row} blocks");
            let w = blocks(rows * per_row, rows as u32 + 1);
            let mut deq = Vec::new();
            dequantize_q4k(&w, &mut deq);
            check(&ctx.upload_q4k(&w, rows).unwrap(), &deq, &what("Q4_K"));
            let w = blocks6(rows * per_row, rows as u32 + 2);
            let mut deq = Vec::new();
            dequantize_q6k(&w, &mut deq);
            check(&ctx.upload_q6k(&w, rows).unwrap(), &deq, &what("Q6_K"));
            // Q8_0 blocks are 32 wide: 3× the blocks for comparable widths
            let w = blocks8(rows * per_row * 3, rows as u32 + 3);
            let mut deq = Vec::new();
            crate::quant::dequantize_q8_0(&w, &mut deq);
            check(&ctx.upload_q8_0(&w, rows).unwrap(), &deq, &what("Q8_0"));
            if rows < 1000 {
                check(&ctx.upload_f32(&deq, rows).unwrap(), &deq, &what("F32"));
            }
        }
        // F32 of widths 1, 3, 37, 1027 (scalar kernel) and 4, 68, 300 (vec4 kernel; 300 = 75 vec4s,
        // more than one pass of the 64 threads)
        for (rows, cols) in [(3usize, 1usize), (5, 3), (7, 37), (4, 1027), (2, 4), (9, 68), (9, 300)] {
            let deq: Vec<f32> = (0..rows * cols).map(|i| ((i * 17 % 61) as f32 - 30.0) / 13.0).collect();
            check(&ctx.upload_f32(&deq, rows).unwrap(), &deq, &format!("F32 {rows}×{cols}"));
        }
    }

    /// The recording API: two products chained on the GPU (y = B · (A · x)) in one encoder,
    /// read back once.
    #[test]
    fn recorded_matvecs_chain_in_one_submission() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let (a, b) = (blocks(64 * 2, 3), blocks8(32 * 2, 4));
        let (ma, mb) = (ctx.upload_q4k(&a, 64).unwrap(), ctx.upload_q8_0(&b, 32).unwrap());
        let x: Vec<f32> = (0..512).map(|i| ((i * 37 % 101) as f32 - 50.0) / 25.0).collect();
        let xb = ctx.storage_init(&x, "x").unwrap();
        let mid = ctx.storage(64, "mid").unwrap();
        let yb = ctx.storage(32, "y").unwrap();
        let mut enc = ctx.encoder();
        ctx.record_matvec(&mut enc, &ma, &xb, &mid);
        ctx.record_matvec(&mut enc, &mb, &mid, &yb);
        let y = ctx.submit_read(enc, &yb, 32).unwrap();
        let mid_cpu = ctx.matvec(&ma, &x).unwrap();
        let want = ctx.matvec(&mb, &mid_cpu).unwrap();
        assert_eq!(y, want);
        // a buffer too small for the matrix fails the context instead of panicking
        let mut enc = ctx.encoder();
        ctx.record_matvec(&mut enc, &ma, &yb, &mid);
        assert!(ctx.submit(enc).is_err());
        assert!(ctx.check().is_err());
    }

    #[test]
    fn q4k_matvec_refuses_bad_shapes() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let w = blocks(4, 1);
        assert!(ctx.upload_q4k(&w, 0).is_err());
        assert!(ctx.upload_q4k(&w, 3).is_err());
        assert!(ctx.upload_q4k(&[], 1).is_err());
        assert!(ctx.upload_q6k(&blocks6(3, 1), 2).is_err());
        assert!(ctx.upload_q8_0(&blocks8(3, 1), 2).is_err());
        assert!(ctx.upload_f32(&[1.0; 5], 2).is_err());
        let m = ctx.upload_q4k(&w, 2).unwrap();
        assert!(ctx.matvec(&m, &[0.0; 256]).is_err());
        assert!(ctx.matvec(&m, &[0.0; 513]).is_err());
        assert_eq!(ctx.matvec(&m, &[0.0; 512]).unwrap(), vec![0.0, 0.0]);
    }

    fn vals(n: usize, seed: u32, amp: f32) -> Vec<f32> {
        let mut s = seed;
        (0..n).map(|_| (f32::from(next_byte(&mut s)) / 255.0 - 0.5) * amp).collect()
    }

    fn worst(a: &[f32], b: &[f32]) -> (f32, f32) {
        assert_eq!(a.len(), b.len());
        let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        (a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs())), scale)
    }

    fn run_op(ctx: &GpuContext, op: Op, bufs: &[&wgpu::Buffer], groups: [u32; 2], out: &wgpu::Buffer, n: usize) -> Vec<f32> {
        let d = ctx.op_dispatch(op, bufs, groups).unwrap();
        let mut enc = ctx.encoder();
        ctx.record(&mut enc, &[d]);
        ctx.submit_read(enc, out, n).unwrap()
    }

    #[test]
    fn element_kernels_match_the_cpu() {
        use crate::forward::{Rope, rms_norm, rotate, silu};
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        // RMSNorm over more values than threads
        let n = 1000;
        let (x, w) = (vals(n, 1, 6.0), vals(n, 2, 2.0));
        let (xb, wb, yb) = (ctx.storage_init(&x, "x").unwrap(), ctx.storage_init(&w, "w").unwrap(), ctx.storage(n, "y").unwrap());
        let p = ctx.uniform(&[n as u32, 1e-6f32.to_bits()]);
        let got = run_op(&ctx, Op::RmsNorm, &[&p, &xb, &wb, &yb], [1, 1], &yb, n);
        let mut want = vec![0.0; n];
        rms_norm(&x, &w, 1e-6, &mut want);
        let (d, s) = worst(&got, &want);
        assert!(d <= 1e-5 * s, "rms_norm: {d} / {s}");
        // SiLU gate and residual add (in place)
        let p = ctx.uniform(&[n as u32]);
        let got = run_op(&ctx, Op::SiluMul, &[&p, &xb, &wb], ctx.groups(n, 64).unwrap(), &xb, n);
        let want: Vec<f32> = x.iter().zip(&w).map(|(a, b)| silu(*a) * b).collect();
        let (d, s) = worst(&got, &want);
        assert!(d <= 1e-6 * s, "silu_mul: {d} / {s}");
        let got = run_op(&ctx, Op::Add, &[&p, &xb, &wb], ctx.groups(n, 64).unwrap(), &xb, n);
        let want2: Vec<f32> = got.iter().zip(&want).zip(&w).map(|((_, a), b)| a + b).collect();
        let (d, s) = worst(&got, &want2);
        assert!(d <= 1e-6 * s, "add: {d} / {s}");
        // Rope, both styles, at a position from the step uniform, on the CPU's own table
        let (heads, head_dim, max_pos, pos) = (3usize, 64usize, 16usize, 11u32);
        let half = head_dim / 2;
        let theta: Vec<f32> = (0..half).map(|i| 1.0 / 10_000f32.powf((2 * i) as f32 / head_dim as f32)).collect();
        let cos: Vec<f32> = (0..max_pos).flat_map(|p| theta.iter().map(move |t| (p as f32 * t).cos())).collect();
        let sin: Vec<f32> = (0..max_pos).flat_map(|p| theta.iter().map(move |t| (p as f32 * t).sin())).collect();
        let table = ctx.storage_init(&[cos.clone(), sin.clone()].concat(), "table").unwrap();
        let step = ctx.uniform(&[pos, pos + 1]);
        for (style, flag) in [(Rope::Halves, 0u32), (Rope::Interleaved, 1)] {
            let v = vals(heads * head_dim, 3, 4.0);
            let vb = ctx.storage_init(&v, "v").unwrap();
            let p = ctx.uniform(&[heads as u32, head_dim as u32, flag, (max_pos * half) as u32]);
            let got = run_op(&ctx, Op::Rope, &[&p, &step, &table, &vb], ctx.groups(heads * half, 64).unwrap(), &vb, v.len());
            let mut want = v.clone();
            let at = pos as usize * half;
            for h in want.chunks_exact_mut(head_dim) {
                rotate(h, &cos[at..at + half], &sin[at..at + half], style);
            }
            let (d, s) = worst(&got, &want);
            assert!(d <= 1e-6 * s, "rope {style:?}: {d} / {s}");
        }
    }

    /// The KV store, and one token's grouped-query attention over 1, 37 and 300 cached positions
    /// (300: more than the 256 threads of a workgroup).
    #[test]
    fn attention_matches_the_cpu() {
        use crate::forward::{dot, softmax};
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let (n_head, n_kv, hd, max_pos) = (4usize, 2usize, 64usize, 320usize);
        let kv_dim = n_kv * hd;
        for n_ctx in [1usize, 37, 300] {
            let kc = vals(max_pos * kv_dim, 5, 3.0);
            let vc = vals(max_pos * kv_dim, 6, 3.0);
            let q = vals(n_head * hd, 7, 3.0);
            let (kcb, vcb, qb) = (ctx.storage_init(&kc, "kc").unwrap(), ctx.storage_init(&vc, "vc").unwrap(), ctx.storage_init(&q, "q").unwrap());
            let (scores, out) = (ctx.storage(n_head * max_pos, "scores").unwrap(), ctx.storage(n_head * hd, "out").unwrap());
            let step = ctx.uniform(&[(n_ctx - 1) as u32, n_ctx as u32]);
            let sqrt_dim = (hd as f32).sqrt();
            let p = ctx.uniform(&[n_head as u32, n_kv as u32, hd as u32, max_pos as u32, kv_dim as u32, sqrt_dim.to_bits()]);
            let got = run_op(&ctx, Op::Attention, &[&p, &step, &qb, &kcb, &vcb, &scores, &out], [n_head as u32, 1], &out, n_head * hd);
            let mut want = vec![0.0f32; n_head * hd];
            for (hi, (qh, o)) in q.chunks_exact(hd).zip(want.chunks_exact_mut(hd)).enumerate() {
                let off = (hi / (n_head / n_kv)) * hd;
                let mut sc: Vec<f32> = (0..n_ctx).map(|p| dot(qh, &kc[p * kv_dim + off..p * kv_dim + off + hd]) / sqrt_dim).collect();
                softmax(&mut sc);
                for (p, w) in sc.iter().enumerate() {
                    for (oo, vv) in o.iter_mut().zip(&vc[p * kv_dim + off..p * kv_dim + off + hd]) {
                        *oo += w * vv;
                    }
                }
            }
            let (d, s) = worst(&got, &want);
            assert!(d <= 1e-5 * s, "attention over {n_ctx}: {d} / {s}");
        }
        // the store writes the step's row of both caches and nothing else
        let (k, v) = (vals(kv_dim, 8, 1.0), vals(kv_dim, 9, 1.0));
        let (kb, vb) = (ctx.storage_init(&k, "k").unwrap(), ctx.storage_init(&v, "v").unwrap());
        let (kcb, vcb) = (ctx.storage(4 * kv_dim, "kc").unwrap(), ctx.storage(4 * kv_dim, "vc").unwrap());
        let step = ctx.uniform(&[2, 3]);
        let p = ctx.uniform(&[kv_dim as u32, 4]);
        let d = ctx.op_dispatch(Op::KvStore, &[&p, &step, &kb, &vb, &kcb, &vcb], ctx.groups(kv_dim, 64).unwrap()).unwrap();
        let mut enc = ctx.encoder();
        ctx.record(&mut enc, &[d]);
        let kgot = ctx.submit_read(enc, &kcb, 4 * kv_dim).unwrap();
        let vgot = ctx.submit_read(ctx.encoder(), &vcb, 4 * kv_dim).unwrap();
        let row = |c: &[f32], r: usize| c[r * kv_dim..(r + 1) * kv_dim].to_vec();
        assert_eq!((row(&kgot, 2), row(&vgot, 2)), (k, v));
        assert!(row(&kgot, 1).iter().chain(&row(&vgot, 3)).all(|x| *x == 0.0));
        // a position past the cache writes nothing
        ctx.write_u32(&step, &[4, 5]).unwrap();
        let d = ctx.op_dispatch(Op::KvStore, &[&p, &step, &vb, &kb, &kcb, &vcb], ctx.groups(kv_dim, 64).unwrap()).unwrap();
        let mut enc = ctx.encoder();
        ctx.record(&mut enc, &[d]);
        assert_eq!(ctx.submit_read(enc, &kcb, 4 * kv_dim).unwrap(), kgot);
        // a wrong buffer count or workgroup count is an error, not a panic
        assert!(ctx.op_dispatch(Op::Add, &[&p, &kb], [1, 1]).is_err());
        assert!(ctx.op_dispatch(Op::Add, &[&p, &kb, &vb], [0, 1]).is_err());
        assert!(ctx.check().is_ok());
    }

    /// The batched element kernels past one dimension of workgroups (SmolLM3's 512 × 11 008 MLP
    /// activations wrap to y), the bias add's period, and causal prefill attention (head
    /// dimensions 64 / 128 / 512, a batch at position 0 and one continuing a cache past two
    /// 128-position tiles) against the CPU's arithmetic.
    #[test]
    fn batched_kernels_match_the_cpu() {
        use crate::forward::{dot, silu, softmax};
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let n = 3 * (STRIDE_X as usize) * 64 / 2 + 77;
        let (a, b) = (vals(n, 11, 4.0), vals(n, 12, 2.0));
        let (ab, bb) = (ctx.storage_init(&a, "a").unwrap(), ctx.storage_init(&b, "b").unwrap());
        let groups = ctx.groups_2d(n, 64).unwrap();
        assert!(groups[1] > 1, "{groups:?}");
        let p = ctx.uniform(&[n as u32]);
        let got = run_op(&ctx, Op::SiluMul, &[&p, &ab, &bb], groups, &ab, n);
        let want: Vec<f32> = a.iter().zip(&b).map(|(x, y)| silu(*x) * y).collect();
        let (d, s) = worst(&got, &want);
        assert!(d <= 1e-6 * s, "silu_mul 2-D: {d} / {s}");
        // x[i] += bias[i % 1000]
        let bias = vals(1000, 13, 1.0);
        let biasb = ctx.storage_init(&bias, "bias").unwrap();
        let p = ctx.uniform(&[n as u32, 1000]);
        let got2 = run_op(&ctx, Op::Add, &[&p, &ab, &biasb], groups, &ab, n);
        let want2: Vec<f32> = got.iter().enumerate().map(|(i, x)| x + bias[i % 1000]).collect();
        assert_eq!(got2, want2);

        for (hd, n_head, n_kv) in [(64usize, 4usize, 2usize), (128, 8, 2), (256, 4, 2), (512, 2, 1)] {
            let kv_dim = n_kv * hd;
            let max_pos = 300usize;
            for (pos, batch) in [(0usize, 7usize), (250, 37)] {
                let end = pos + batch;
                let kc = vals(max_pos * kv_dim, 21, 3.0);
                let vc = vals(max_pos * kv_dim, 22, 3.0);
                let q = vals(batch * n_head * hd, 23, 3.0);
                let (kcb, vcb, qb) = (ctx.storage_init(&kc, "kc").unwrap(), ctx.storage_init(&vc, "vc").unwrap(), ctx.storage_init(&q, "q").unwrap());
                let out = ctx.storage(batch * n_head * hd, "out").unwrap();
                let step = ctx.uniform(&[pos as u32, end as u32]);
                let sqrt_dim = (hd as f32).sqrt();
                let p = ctx.uniform(&[n_head as u32, n_kv as u32, hd as u32, max_pos as u32, kv_dim as u32, sqrt_dim.to_bits(), (n_head * hd) as u32]);
                let got = run_op(&ctx, Op::AttentionPrefill, &[&p, &step, &qb, &kcb, &vcb, &out], [n_head as u32, batch as u32], &out, batch * n_head * hd);
                let got8 = (hd <= 256).then(|| {
                    let out8 = ctx.storage(batch * n_head * hd, "out8").unwrap();
                    run_op(
                        &ctx,
                        Op::AttentionPrefill8,
                        &[&p, &step, &qb, &kcb, &vcb, &out8],
                        [n_head as u32, batch.div_ceil(8) as u32],
                        &out8,
                        batch * n_head * hd,
                    )
                });
                let mut want = vec![0.0f32; batch * n_head * hd];
                for b in 0..batch {
                    let n_ctx = pos + b + 1;
                    for h in 0..n_head {
                        let qh = &q[(b * n_head + h) * hd..][..hd];
                        let off = (h / (n_head / n_kv)) * hd;
                        let mut sc: Vec<f32> = (0..n_ctx).map(|j| dot(qh, &kc[j * kv_dim + off..][..hd]) / sqrt_dim).collect();
                        softmax(&mut sc);
                        let o = &mut want[(b * n_head + h) * hd..][..hd];
                        for (j, w) in sc.iter().enumerate() {
                            for (oo, vv) in o.iter_mut().zip(&vc[j * kv_dim + off..][..hd]) {
                                *oo += w * vv;
                            }
                        }
                    }
                }
                let (d, s) = worst(&got, &want);
                assert!(d <= 1e-5 * s, "prefill attention hd {hd}, {batch} at {pos}: {d} / {s}");
                if let Some(got8) = got8 {
                    let (d, s) = worst(&got8, &want);
                    assert!(d <= 1e-5 * s, "prefill attention (8 per group) hd {hd}, {batch} at {pos}: {d} / {s}");
                }
            }
        }
        assert!(ctx.check().is_ok());
    }

    /// GPU time of SmolLM3-3B's prefill attention (16 heads of 128, 4 KV heads) for an 800-token
    /// prompt in batches of 512 + 288, × 36 layers. `cargo test -p filmcraft-llm --features gpu
    /// --release --lib gpu::tests::prefill_attention_speed -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand on an idle machine"]
    fn prefill_attention_speed() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let (n_head, n_kv, hd, max_pos) = (16usize, 4usize, 128usize, 2048usize);
        let kv_dim = n_kv * hd;
        let kc = ctx.storage_init(&vals(max_pos * kv_dim, 1, 2.0), "kc").unwrap();
        let vc = ctx.storage_init(&vals(max_pos * kv_dim, 2, 2.0), "vc").unwrap();
        let q = ctx.storage_init(&vals(512 * n_head * hd, 3, 2.0), "q").unwrap();
        let out = ctx.storage(512 * n_head * hd, "out").unwrap();
        let p = ctx.uniform(&[n_head as u32, n_kv as u32, hd as u32, max_pos as u32, kv_dim as u32, (hd as f32).sqrt().to_bits(), (n_head * hd) as u32]);
        let run = |pos: u32, n: u32, op: Op| {
            let step = ctx.uniform(&[pos, pos + n]);
            let rows = if op == Op::AttentionPrefill8 { n.div_ceil(8) } else { n };
            let d = ctx.op_dispatch(op, &[&p, &step, &q, &kc, &vc, &out], [n_head as u32, rows]).unwrap();
            let t = std::time::Instant::now();
            for _ in 0..36 {
                let mut enc = ctx.encoder();
                ctx.record(&mut enc, std::slice::from_ref(&d));
                ctx.submit(enc).unwrap();
            }
            ctx.wait(None).unwrap();
            t.elapsed().as_secs_f64()
        };
        for op in [Op::AttentionPrefill, Op::AttentionPrefill8] {
            run(0, 64, op);
            let (a, b) = (run(0, 512, op), run(512, 288, op));
            eprintln!("{}: {op:?} x36: 512 at 0 {:.1} ms, 288 at 512 {:.1} ms", ctx.name(), a * 1e3, b * 1e3);
        }
    }

    /// Timing on a SmolLM3-sized `ffn_up` (11 008 × 2 048 Q4_K): the GPU round trip (upload x,
    /// dispatch, read back) against the CPU kernel. `cargo test -p filmcraft-llm --features gpu
    /// --release --lib gpu::tests::q4k_matvec_speed -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand on an idle machine"]
    fn q4k_matvec_speed() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let (rows, per_row) = (11_008usize, 8usize);
        let w = blocks(rows * per_row, 7);
        let x: Vec<f32> = (0..per_row * QK_K).map(|i| ((i * 37 % 101) as f32 - 50.0) / 25.0).collect();
        let m = ctx.upload_q4k(&w, rows).unwrap();
        let _ = ctx.matvec(&m, &x).unwrap();
        let n = 50;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = ctx.matvec(&m, &x).unwrap();
        }
        let gpu = t.elapsed().as_secs_f64() / f64::from(n) * 1e3;
        let mut xq = Vec::new();
        crate::kquant::quantize_q8k(&x, &mut xq).unwrap();
        let mut out = vec![0.0f32; rows];
        crate::kquant::matvec_q4k(&w, &xq, &mut out).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..n {
            crate::kquant::matvec_q4k(&w, &xq, &mut out).unwrap();
        }
        let cpu = t.elapsed().as_secs_f64() / f64::from(n) * 1e3;
        eprintln!("{}: Q4_K 11008x2048 matvec: GPU round trip {gpu:.3} ms, CPU {cpu:.3} ms", ctx.name());
    }
}
