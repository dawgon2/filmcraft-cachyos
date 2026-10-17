//! Batched quantized matrix products for prompt prefill (AI3.3b): `n` activation vectors through
//! one Q4_K / Q6_K / Q8_0 (AI4.2; 256-column multiples) matrix in a single dispatch. A matvec per token re-reads every weight per
//! token; this tiled kernel stages a 128-row × 16/32-column weight tile in workgroup memory
//! once and reuses it across 128 tokens (see `gpu_matmul.wgsl`). Variants by adapter: AMD and
//! others stage weight bytes and prefetch the next tile; Intel stages f32 weights without
//! prefetch. Narrow matrices split K across workgroups and sum the partials in a second pass.
//!
//! SmolLM3-3B on an RX 6700 XT (synthetic weights, the 36 layers' 7 matmuls, 800 tokens):
//! ~0.76 s, ~5.9 TFLOP/s (about half the f32 peak at the 2.45 GHz it holds); Intel UHD (RKL
//! GT1) ~16 s, ~0.28 TFLOP/s. The forward pass should fuse q / k / v (and gate / up) into one
//! matrix each for wider dispatches.
//!
//! A child module of [`super`] (`gpu.rs`) so it reaches the context and matrix internals
//! without widening their visibility.

use wgpu::util::DeviceExt;

use super::{GpuContext, GpuMatrix, Kind, storage_entry};
use crate::LlmError;
use crate::quant::err;

/// Output rows per workgroup.
const TILE_ROWS: u32 = 128;
/// Tokens per workgroup.
const TILE_TOKENS: u32 = 128;
/// K columns per step when the adapter's workgroup memory allows, else the 16-column step
/// whose byte-staged variant fits wgpu's default 16 KiB.
const WIDE_STEP: u32 = 32;
const NARROW_STEP: u32 = 16;
/// Workgroups per dispatch below which K is split across workgroups (summed by a second pass),
/// so narrow matrices (attention, `ffn_down`) still fill a 40-CU GPU; 0 disables splitting.
const SPLIT_TARGET: u32 = 320;
/// Largest split scratch buffer (f32 partial sums).
const MAX_SCRATCH: u64 = 256 << 20;
/// Workgroups along x before the reduction wraps to y.
const STRIDE_X: u32 = 32_768;
/// PCI vendor id of Intel: int → float conversion is slow there and the register prefetch
/// spills, so it gets the f32-staged variant without prefetch.
const INTEL: u32 = 0x8086;

/// A compiled form of the kernel (see the constants in `gpu_matmul.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Variant {
    /// K columns per step (16 or 32).
    step: u32,
    /// Load the next step's words during the current one.
    prefetch: bool,
    /// Stage dequantised f32 weights rather than bytes + scales.
    f32_tile: bool,
    /// See [`SPLIT_TARGET`].
    split_target: u32,
}

impl Variant {
    /// Workgroup memory: activations 512 bytes per column; weights as bytes 128 per column +
    /// 64 of scales, or as f32 512 per column; 512 bytes for each array the variant leaves at
    /// its minimum. Bytes: 11.5 KiB at 16 columns, 22.5 KiB at 32; f32: 17 / 33 KiB.
    fn shared_bytes(self) -> u32 {
        let (rows, tokens) = (TILE_ROWS, TILE_TOKENS);
        let x = self.step * tokens * 4;
        if self.f32_tile { x + self.step * rows * 4 + 1024 } else { x + self.step * (rows + rows * 8 / 16) + 512 }
    }

    /// The fastest variant the device can run (`None`: none fits its workgroup memory).
    fn pick(device: &wgpu::Device) -> Option<Self> {
        let room = device.limits().max_compute_workgroup_storage_size;
        let intel = device.adapter_info().vendor == INTEL;
        let (prefetch, split_target) = if intel { (false, 0) } else { (true, SPLIT_TARGET) };
        // f32 staging at 32 columns (33 KiB) where it fits, else the bytes (22.5 / 11.5 KiB)
        [(WIDE_STEP, true), (WIDE_STEP, false), (NARROW_STEP, intel), (NARROW_STEP, false)]
            .map(|(step, f32_tile)| Self { step, prefetch, f32_tile, split_target })
            .into_iter()
            .find(|v| v.shared_bytes() <= room)
    }
}

/// The matmul pipelines, or why the adapter cannot run them.
pub(super) struct MatmulPipelines {
    inner: Result<Pipelines, String>,
}

struct Pipelines {
    q4k: wgpu::ComputePipeline,
    q6k: wgpu::ComputePipeline,
    q8_0: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    max_groups: u32,
    split_target: u32,
    /// The split partial sums, reused (grown) across calls; every use is a matmul pass and its
    /// reduction in one encoder, which wgpu orders against the next use.
    scratch: std::sync::Mutex<Option<wgpu::Buffer>>,
}

impl MatmulPipelines {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        match Variant::pick(device) {
            Some(v) => Self::with(device, v),
            None => Self {
                inner: Err(format!("the GPU's {} bytes of workgroup memory are below the matmul kernel's", device.limits().max_compute_workgroup_storage_size)),
            },
        }
    }

    /// Whether the adapter runs the matmul kernels.
    pub(super) fn ready(&self) -> bool {
        self.inner.is_ok()
    }

    fn with(device: &wgpu::Device, v: Variant) -> Self {
        let limits = device.limits();
        if limits.max_compute_workgroup_storage_size < v.shared_bytes()
            || limits.max_compute_invocations_per_workgroup < 256
            || limits.max_compute_workgroup_size_x < 256
        {
            return Self { inner: Err(format!("the GPU's compute limits are below the matmul kernel's ({} bytes shared, 256 threads)", v.shared_bytes())) };
        }
        let mut source = include_str!("gpu_matmul.wgsl")
            .replace("const BK: u32 = 16u;", &format!("const BK: u32 = {}u;", v.step))
            .replace("const PREFETCH: bool = true;", &format!("const PREFETCH: bool = {};", v.prefetch));
        if v.f32_tile {
            source = source
                .replace("const F32_TILE: bool = false;", "const F32_TILE: bool = true;")
                .replace("const WQ_K: u32 = BK / 4u;", "const WQ_K: u32 = 1u;")
                .replace("const WD_G: u32 = NG * 2u;", "const WD_G: u32 = 1u;")
                .replace("const WS_K: u32 = 1u;", "const WS_K: u32 = BK;");
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("llm-matmul"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llm-matmul"),
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
            label: Some("llm-matmul"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let (q4k, q6k, q8_0, reduce) = (pipeline("matmul_q4k"), pipeline("matmul_q6k"), pipeline("matmul_q8_0"), pipeline("reduce"));
        Self {
            inner: Ok(Pipelines {
                q4k,
                q6k,
                q8_0,
                reduce,
                layout,
                max_groups: limits.max_compute_workgroups_per_dimension,
                split_target: v.split_target,
                scratch: std::sync::Mutex::new(None),
            }),
        }
    }
}

impl Pipelines {
    /// A scratch buffer of at least `bytes`.
    fn scratch(&self, device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
        let mut slot = self.scratch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(b) = slot.as_ref().filter(|b| b.size() >= bytes) {
            return b.clone();
        }
        let b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llm-matmul-split"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        *slot = Some(b.clone());
        b
    }
}

/// How many K splits (each a run of whole blocks) a `groups`-workgroup dispatch gets, and the
/// blocks per split: enough to reach `target` workgroups, at most one per block, and the
/// partial sums (`splits · out_values` f32) within the scratch cap and u32 indexing.
fn split(groups: u32, blocks: u32, out_values: u64, target: u32, max_binding: u64) -> (u32, u32) {
    if groups == 0 || groups >= target || blocks < 2 {
        return (1, blocks);
    }
    let cap = (MAX_SCRATCH.min(max_binding) / 4).min(u64::from(u32::MAX)) / out_values.max(1);
    let want = target.div_ceil(groups).min(blocks).min(u32::try_from(cap).unwrap_or(u32::MAX));
    if want < 2 {
        return (1, blocks);
    }
    let per = blocks.div_ceil(want);
    (blocks.div_ceil(per), per)
}

/// Bytes of `count` f32 values, if that fits the binding limit and u32 indexing in the shader.
fn f32_bytes(count: u64, max_binding: u64, what: &str) -> Result<u64, LlmError> {
    let bytes = count.checked_mul(4).ok_or_else(|| err(format!("{what} too large")))?;
    if count == 0 || bytes > max_binding || u32::try_from(count).is_err() {
        return Err(err(format!("{what} of {count} values does not fit the GPU's limits")));
    }
    Ok(bytes)
}

impl GpuContext {
    /// Records `y[t · rows + r] = m[r] · x[t · cols ..][..cols]` for `n` token-major activation
    /// vectors. `x` must hold at least `n · m.cols()` f32, `y` at least `n · m.rows()`.
    /// Nothing runs until the encoder is submitted; shape errors are returned here.
    pub(crate) fn record_matmul(&self, enc: &mut wgpu::CommandEncoder, m: &GpuMatrix, x: &wgpu::Buffer, y: &wgpu::Buffer, n: u32) -> Result<(), LlmError> {
        let pl = self.matmul.inner.as_ref().map_err(|e| err(e.clone()))?;
        let pipeline = match m.kind {
            Kind::Q4K => &pl.q4k,
            Kind::Q6K => &pl.q6k,
            Kind::Q8_0 if m.cols.is_multiple_of(crate::kquant::QK_K as u32) => &pl.q8_0,
            Kind::Q8_0 | Kind::F32 => return Err(err(format!("no batched GPU matmul for {:?} weights of {} columns", m.kind, m.cols))),
        };
        let blocks = m.cols / crate::kquant::QK_K as u32;
        let cols = u64::from(blocks).saturating_mul(crate::kquant::QK_K as u64);
        let x_bytes = f32_bytes(u64::from(n).saturating_mul(cols), self.max_binding, "matmul input")?;
        let y_bytes = f32_bytes(u64::from(n).saturating_mul(u64::from(m.rows)), self.max_binding, "matmul output")?;
        if x.size() < x_bytes || y.size() < y_bytes {
            return Err(err(format!("matmul buffers of {} / {} bytes, need {x_bytes} / {y_bytes}", x.size(), y.size())));
        }
        let (gx, gy) = (m.rows.div_ceil(TILE_ROWS), n.div_ceil(TILE_TOKENS));
        if gx > pl.max_groups || gy > pl.max_groups {
            return Err(err(format!("matmul of {} rows × {n} tokens is over the GPU's dispatch limit", m.rows)));
        }
        let cols = u32::try_from(cols).map_err(|_| err("matmul too wide"))?;
        let out_values = u64::from(n) * u64::from(m.rows);
        let (splits, per) = split(gx.saturating_mul(gy), blocks, out_values, pl.split_target, self.max_binding);
        if splits > pl.max_groups {
            return Err(err(format!("matmul split of {splits} is over the GPU's dispatch limit")));
        }
        // with splits, the matmul writes partial sums to scratch and `reduce` adds them into y
        let scratch = (splits > 1).then(|| pl.scratch(&self.device, out_values * u64::from(splits) * 4));
        let params: Vec<u8> = [m.rows, blocks, n, cols, per, splits, STRIDE_X, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-matmul-params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = |w: &wgpu::Buffer, out: &wgpu::Buffer| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("llm-matmul"),
                layout: &pl.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: w.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: x.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: out.as_entire_binding() },
                ],
            })
        };
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("llm-matmul"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind(&m.buf, scratch.as_ref().unwrap_or(y)), &[]);
        pass.dispatch_workgroups(gx, gy, splits);
        if let Some(scratch) = &scratch {
            // the reduction reads the partials through the weights binding
            let groups = u32::try_from(out_values.div_ceil(256)).map_err(|_| err("matmul output too large"))?;
            pass.set_pipeline(&pl.reduce);
            pass.set_bind_group(0, &bind(scratch, y), &[]);
            pass.dispatch_workgroups(groups.min(STRIDE_X), groups.div_ceil(STRIDE_X), 1);
        }
        Ok(())
    }

    /// `out[t · rows + r] = m[r] · x[t · cols ..][..cols]` for `n` token-major vectors
    /// (`x.len() == n · m.cols()`); uploads `x` and reads the result back.
    pub fn matmul(&self, m: &GpuMatrix, x: &[f32], n: usize) -> Result<Vec<f32>, LlmError> {
        if n == 0 || Some(x.len()) != n.checked_mul(m.cols()) {
            return Err(err(format!("{} values are not {n} vectors of {} columns", x.len(), m.cols())));
        }
        let n32 = u32::try_from(n).map_err(|_| err("too many matmul vectors"))?;
        let out_len = n.checked_mul(m.rows()).ok_or_else(|| err("matmul output too large"))?;
        f32_bytes(out_len as u64, self.max_binding, "matmul output")?;
        f32_bytes(x.len() as u64, self.max_binding, "matmul input")?;
        let xbuf = self.storage_init(x, "llm-matmul-x")?;
        let ybuf = self.storage(out_len, "llm-matmul-y")?;
        let mut enc = self.encoder();
        self.record_matmul(&mut enc, m, &xbuf, &ybuf, n32)?;
        // bounded wait: a hung GPU fails the context instead of blocking
        self.submit_read(enc, &ybuf, out_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kquant::{BlockQ4K, BlockQ6K, QK_K, dequantize_q4k, dequantize_q6k};

    fn next_byte(s: &mut u32) -> u8 {
        *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*s >> 24) as u8
    }

    fn blocks4(n: usize, seed: u32) -> Vec<BlockQ4K> {
        let mut s = seed;
        (0..n)
            .map(|i| BlockQ4K {
                d: 0.01 + (i % 7) as f32 * 0.003,
                dmin: 0.005 + (i % 5) as f32 * 0.002,
                scales: std::array::from_fn(|_| next_byte(&mut s) & 63),
                mins: std::array::from_fn(|_| next_byte(&mut s) & 63),
                qs: std::array::from_fn(|_| next_byte(&mut s)),
            })
            .collect()
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

    fn activations(n: usize, cols: usize) -> Vec<f32> {
        (0..n * cols).map(|i| ((i * 37 % 101) as f32 - 50.0) / 25.0 + (i / cols) as f32 * 0.01).collect()
    }

    /// Every output against the f64 product of the CPU dequantisation (1e-5 of Σ|w·x|).
    fn assert_close(y: &[f32], deq: &[f32], x: &[f32], rows: usize, n: usize, what: &str) {
        let cols = deq.len() / rows;
        assert_eq!(y.len(), rows * n, "{what}");
        for t in 0..n {
            let xt = &x[t * cols..(t + 1) * cols];
            for r in 0..rows {
                let row = &deq[r * cols..(r + 1) * cols];
                let want: f64 = row.iter().zip(xt).map(|(a, b)| f64::from(*a) * f64::from(*b)).sum();
                let mag: f64 = row.iter().zip(xt).map(|(a, b)| (f64::from(*a) * f64::from(*b)).abs()).sum();
                let got = y[t * rows + r];
                assert!((f64::from(got) - want).abs() <= 1e-5 * mag + 1e-6, "{what} {rows}x{cols} n={n}: token {t} row {r}: {got} vs {want}");
            }
        }
    }

    fn shapes() -> Vec<(usize, usize, usize)> {
        // (rows, blocks per row, n): partial tiles both ways, n = 1, several K blocks
        vec![(3, 1, 1), (130, 2, 3), (64, 8, 64), (200, 3, 65), (257, 2, 300), (128, 1, 128), (130, 7, 5)]
    }

    /// The variants under test: bytes / f32 staging, 16 / 32 columns per step, with and without
    /// prefetch, no split, the default split and a small split target that gives uneven splits
    /// (7 blocks over 2 workgroups → 3 + 3 + 1).
    fn variants() -> Vec<Variant> {
        let v = |step, prefetch, f32_tile, split_target| Variant { step, prefetch, f32_tile, split_target };
        vec![
            v(NARROW_STEP, true, false, SPLIT_TARGET),
            v(NARROW_STEP, false, false, 0),
            v(WIDE_STEP, true, false, 6),
            v(WIDE_STEP, false, false, SPLIT_TARGET),
            v(NARROW_STEP, false, true, 0),
            v(WIDE_STEP, true, true, SPLIT_TARGET),
        ]
    }

    /// A context per variant the adapter can run, and the default pick.
    fn contexts() -> Vec<(GpuContext, String)> {
        let mut out = Vec::new();
        for v in variants().into_iter().map(Some).chain([None]) {
            match GpuContext::new() {
                Ok(mut ctx) => {
                    let Some(v) = v.or_else(|| Variant::pick(&ctx.device)) else { continue };
                    ctx.matmul = MatmulPipelines::with(&ctx.device, v);
                    if ctx.matmul.inner.is_ok() {
                        let what = format!("{v:?}");
                        eprintln!("adapter: {}, {what}", ctx.name());
                        out.push((ctx, what));
                    }
                }
                Err(e) => eprintln!("SKIPPED: {e}"),
            }
        }
        out
    }

    #[test]
    fn splits_cover_the_blocks() {
        assert_eq!(split(602, 8, 1, SPLIT_TARGET, u64::MAX), (1, 8));
        assert_eq!(split(112, 43, 1, SPLIT_TARGET, u64::MAX), (3, 15));
        assert_eq!(split(28, 8, 1, SPLIT_TARGET, u64::MAX), (8, 1));
        assert_eq!(split(2, 7, 1, 6, u64::MAX), (3, 3));
        assert_eq!(split(1, 1, 1, SPLIT_TARGET, u64::MAX), (1, 1));
        assert_eq!(split(1, 8, 1, 0, u64::MAX), (1, 8));
        // the scratch cap: 2^25 outputs leave room for 2 splits of 256 MiB
        assert_eq!(split(1, 8, 1 << 25, SPLIT_TARGET, u64::MAX), (2, 4));
        assert_eq!(split(1, 8, 1 << 27, SPLIT_TARGET, u64::MAX), (1, 8));
        for (groups, blocks) in [(1u32, 43u32), (5, 7), (100, 100), (3, 2)] {
            let (n, per) = split(groups, blocks, 10, SPLIT_TARGET, u64::MAX);
            assert!(n * per >= blocks && (n - 1) * per < blocks, "{groups} {blocks}: {n} x {per}");
        }
    }

    #[test]
    fn q4k_matmul_matches_the_cpu() {
        for (ctx, what) in contexts() {
            for (rows, per_row, n) in shapes() {
                let w = blocks4(rows * per_row, rows as u32 + n as u32);
                let cols = per_row * QK_K;
                let x = activations(n, cols);
                let mut deq = Vec::new();
                dequantize_q4k(&w, &mut deq);
                let m = ctx.upload_q4k(&w, rows).unwrap();
                let y = ctx.matmul(&m, &x, n).unwrap();
                assert_close(&y, &deq, &x, rows, n, &format!("Q4_K {what}"));
            }
        }
    }

    #[test]
    fn q6k_matmul_matches_the_cpu() {
        for (ctx, what) in contexts() {
            for (rows, per_row, n) in shapes() {
                let w = blocks6(rows * per_row, rows as u32 + n as u32);
                let cols = per_row * QK_K;
                let x = activations(n, cols);
                let mut deq = Vec::new();
                dequantize_q6k(&w, &mut deq);
                let m = ctx.upload_q6k(&w, rows).unwrap();
                let y = ctx.matmul(&m, &x, n).unwrap();
                assert_close(&y, &deq, &x, rows, n, &format!("Q6_K {what}"));
            }
        }
    }

    fn blocks8(n: usize, seed: u32) -> Vec<crate::quant::BlockQ8_0> {
        let mut s = seed;
        (0..n)
            .map(|i| crate::quant::BlockQ8_0 { d: 0.004 + (i % 5) as f32 * 0.001, qs: std::array::from_fn(|_| i8::from_le_bytes([next_byte(&mut s)])) })
            .collect()
    }

    /// Q8_0 (AI4.2: Qwen3-VL's Q8_0 and mixed GGUFs) through the tiled kernel on every variant:
    /// 8 blocks of 32 per 256-column step, the signed byte offset by 128 against d / 128·d.
    #[test]
    fn q8_0_matmul_matches_the_cpu() {
        for (ctx, what) in contexts() {
            for (rows, per_row, n) in shapes() {
                let w = blocks8(rows * per_row * 8, rows as u32 + n as u32);
                let cols = per_row * QK_K;
                let x = activations(n, cols);
                let mut deq = Vec::new();
                crate::quant::dequantize_q8_0(&w, &mut deq);
                let m = ctx.upload_q8_0(&w, rows).unwrap();
                let y = ctx.matmul(&m, &x, n).unwrap();
                assert_close(&y, &deq, &x, rows, n, &format!("Q8_0 {what}"));
            }
            // a Q8_0 matrix whose columns are not a multiple of 256 has no tiled matmul
            let m = ctx.upload_q8_0(&blocks8(3 * 3, 1), 3).unwrap();
            assert!(ctx.matmul(&m, &activations(2, 96), 2).is_err());
        }
    }

    /// 70 001 rows (547 row tiles, more than STRIDE_X matvec rows) with two token tiles.
    #[test]
    fn matmul_many_rows() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let (rows, n) = (70_001usize, 130usize);
        let w = blocks4(rows, 9);
        let x = activations(n, QK_K);
        let mut deq = Vec::new();
        dequantize_q4k(&w, &mut deq);
        let m = ctx.upload_q4k(&w, rows).unwrap();
        let y = ctx.matmul(&m, &x, n).unwrap();
        assert_close(&y, &deq, &x, rows, n, "Q4_K");
        // and it agrees with the matvec, token by token
        for t in [0, 129] {
            let v = ctx.matvec(&m, &x[t * QK_K..(t + 1) * QK_K]).unwrap();
            for (r, (a, b)) in v.iter().zip(&y[t * rows..(t + 1) * rows]).enumerate() {
                assert!((a - b).abs() <= 1e-3 * a.abs().max(1.0), "token {t} row {r}: {a} vs {b}");
            }
        }
    }

    /// SmolLM3-3B's MLP shapes at a full prefill batch: `ffn_up` 11 008 × 2 048 Q4_K and
    /// `ffn_down` 2 048 × 11 008 Q6_K (43 blocks per row: on a split adapter an uneven K split,
    /// e.g. 5 × 9 blocks with 7 in the last), n = 512, each with the adapter's default variant.
    /// Checked on 400 sampled outputs against the f64 product of the CPU dequantisation.
    #[test]
    fn matmul_at_smollm3_shapes() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let n = 512usize;
        for (rows, per_row, q6) in [(11_008usize, 8usize, false), (2_048, 43, true)] {
            let cols = per_row * QK_K;
            let (m, deq) = if q6 {
                let w = blocks6(rows * per_row, 31);
                let mut deq = Vec::new();
                dequantize_q6k(&w, &mut deq);
                (ctx.upload_q6k(&w, rows).unwrap(), deq)
            } else {
                let w = blocks4(rows * per_row, 37);
                let mut deq = Vec::new();
                dequantize_q4k(&w, &mut deq);
                (ctx.upload_q4k(&w, rows).unwrap(), deq)
            };
            let blocks = per_row as u32;
            let groups = (rows as u32).div_ceil(TILE_ROWS) * (n as u32).div_ceil(TILE_TOKENS);
            let (splits, per) = split(groups, blocks, (n * rows) as u64, SPLIT_TARGET, ctx.max_binding());
            eprintln!("{}: {rows}x{cols} n={n}: {splits} K split(s) of {per} blocks (with the default split target)", ctx.name());
            let x = activations(n, cols);
            let y = ctx.matmul(&m, &x, n).unwrap();
            assert_eq!(y.len(), rows * n);
            let mut s = 12_345u32;
            for i in 0..400 {
                // the corners, then pseudo-random outputs
                let (t, r) = match i {
                    0 => (0, 0),
                    1 => (n - 1, rows - 1),
                    _ => (next_byte(&mut s) as usize * 2 % n, (next_byte(&mut s) as usize * 256 + next_byte(&mut s) as usize) % rows),
                };
                let (row, xt) = (&deq[r * cols..(r + 1) * cols], &x[t * cols..(t + 1) * cols]);
                let want: f64 = row.iter().zip(xt).map(|(a, b)| f64::from(*a) * f64::from(*b)).sum();
                let mag: f64 = row.iter().zip(xt).map(|(a, b)| (f64::from(*a) * f64::from(*b)).abs()).sum();
                let got = f64::from(y[t * rows + r]);
                assert!((got - want).abs() <= 1e-5 * mag + 1e-6, "{rows}x{cols}: token {t} row {r}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn matmul_refuses_bad_shapes() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        let m = ctx.upload_q4k(&blocks4(4, 1), 2).unwrap();
        assert!(ctx.matmul(&m, &[], 0).is_err());
        assert!(ctx.matmul(&m, &[0.0; 512], 0).is_err());
        assert!(ctx.matmul(&m, &[0.0; 512], 2).is_err());
        assert!(ctx.matmul(&m, &[0.0; 1023], 2).is_err());
        assert!(ctx.matmul(&m, &[0.0; 1024], usize::MAX).is_err());
        assert_eq!(ctx.matmul(&m, &[0.0; 1024], 2).unwrap(), vec![0.0; 4]);
        // record: buffers smaller than the shape, n = 0, n past u32 indexing
        let buf =
            |size: u64| ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let (x, y) = (buf(4 * 512 * 3), buf(4 * 2 * 3));
        let mut enc = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        assert!(ctx.record_matmul(&mut enc, &m, &x, &y, 4).is_err());
        assert!(ctx.record_matmul(&mut enc, &m, &x, &y, 0).is_err());
        assert!(ctx.record_matmul(&mut enc, &m, &x, &y, u32::MAX).is_err());
        assert!(ctx.record_matmul(&mut enc, &m, &x, &y, 3).is_ok());
        ctx.queue.submit([enc.finish()]);
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        ctx.check().unwrap();
    }

    /// GPU time of SmolLM3-3B's prefill matmuls (synthetic weights): per layer q, o 2048 × 2048,
    /// k, v 512 × 2048 (Q4_K), gate, up 11 008 × 2048 (Q4_K), down 2048 × 11 008 (Q6_K), 36
    /// layers submitted one by one and waited for once (nothing uploaded or read back), plus
    /// one 11 008 × 2048 Q4_K matrix alone. The adapter's default variant; FC_MATMUL_ALL=1 for
    /// every variant, FC_MATMUL_QUICK=1 for the single matrix only. `cargo test -p
    /// filmcraft-llm --features gpu --release --lib gpu::matmul::tests::matmul_speed --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand on an idle machine"]
    fn matmul_speed() {
        let all = std::env::var_os("FC_MATMUL_ALL").is_some();
        let list = contexts();
        let last = list.len().saturating_sub(1);
        for (i, (ctx, what)) in list.into_iter().enumerate() {
            if all || i == last {
                eprintln!("{what}:");
                speed(&ctx);
            }
        }
    }

    fn speed(ctx: &GpuContext) {
        let up = ctx.upload_q4k(&blocks4(11_008 * 8, 1), 11_008).unwrap();
        let gate = ctx.upload_q4k(&blocks4(11_008 * 8, 2), 11_008).unwrap();
        let q = ctx.upload_q4k(&blocks4(2048 * 8, 3), 2048).unwrap();
        let o = ctx.upload_q4k(&blocks4(2048 * 8, 4), 2048).unwrap();
        let k = ctx.upload_q4k(&blocks4(512 * 8, 5), 512).unwrap();
        let v = ctx.upload_q4k(&blocks4(512 * 8, 6), 512).unwrap();
        let down = ctx.upload_q6k(&blocks6(2048 * 43, 7), 2048).unwrap();
        let wait = || {
            ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            ctx.check().unwrap();
        };
        for n in [512u32, 800] {
            let nn = n as usize;
            let buf = |len: usize| {
                ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&activations(nn, len / nn)),
                    usage: wgpu::BufferUsages::STORAGE,
                })
            };
            let (x, h) = (buf(nn * 2048), buf(nn * 11_008));
            let out = |len: usize| {
                ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: (len * 4) as u64,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                })
            };
            let (ya, yb) = (out(nn * 11_008), out(nn * 11_008));
            // one submit per repetition (a layer): a single multi-second submit trips the
            // Intel driver's hang check and loses the device
            let run = |ms: &[(&GpuMatrix, &wgpu::Buffer, &wgpu::Buffer)], reps: usize| {
                let cmds: Vec<_> = (0..reps)
                    .map(|_| {
                        let mut enc = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                        for (m, x, y) in ms {
                            ctx.record_matmul(&mut enc, m, x, y, n).unwrap();
                        }
                        enc.finish()
                    })
                    .collect();
                let t = std::time::Instant::now();
                for c in cmds {
                    ctx.queue.submit([c]);
                }
                wait();
                t.elapsed().as_secs_f64()
            };
            let single = [(&up, &x, &ya)];
            run(&single, 2);
            let reps = 20;
            // best of three (the clock ramps up during the first)
            let s = (0..3).map(|_| run(&single, reps) / reps as f64).fold(f64::INFINITY, f64::min);
            let flop = 2.0 * 11_008.0 * 2048.0 * f64::from(n);
            eprintln!("{}: Q4_K 11008x2048 n={n}: {:.3} ms, {:.0} GFLOP/s", ctx.name(), s * 1e3, flop / s / 1e9);
            if std::env::var_os("FC_MATMUL_QUICK").is_some() {
                continue;
            }
            // a slow (integrated) GPU gets one round and no breakdown
            let rounds = if s > 0.02 { 1 } else { 3 };
            let layer = [(&q, &x, &ya), (&k, &x, &yb), (&v, &x, &yb), (&o, &x, &yb), (&gate, &x, &ya), (&up, &x, &yb), (&down, &h, &ya)];
            run(&layer, 1);
            let s = (0..rounds).map(|_| run(&layer, 36)).fold(f64::INFINITY, f64::min);
            let flop = 2.0 * f64::from(n) * 36.0 * (2.0 * 2048.0 * 2048.0 + 2.0 * 512.0 * 2048.0 + 3.0 * 11_008.0 * 2048.0);
            eprintln!("{}: 36 layers x 7 matmuls, n={n}: {:.1} ms, {:.0} GFLOP/s, {:.0} tokens/s", ctx.name(), s * 1e3, flop / s / 1e9, f64::from(n) / s);
            if rounds == 1 {
                continue;
            }
            let parts: Vec<String> = ["q", "k", "v", "o", "gate", "up", "down"]
                .iter()
                .zip(&layer)
                .map(|(name, mxy)| {
                    let s = (0..2).map(|_| run(std::slice::from_ref(mxy), 36)).fold(f64::INFINITY, f64::min);
                    format!("{name} {:.1}", s * 1e3)
                })
                .collect();
            eprintln!("  x36 each (ms): {}", parts.join(", "));
        }
    }
}
