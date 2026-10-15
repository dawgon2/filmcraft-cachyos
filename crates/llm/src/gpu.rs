//! GPU (wgpu) kernels for inference (AI3.3b): the CPU path in [`crate::kquant`] stays the
//! reference and the fallback. Step 1: Q4_K and Q6_K matrix × f32 vector products whose weights
//! live on the GPU block by block (Q4_K 152 bytes: the GGUF block with its scales unpacked;
//! Q6_K 212 bytes: the GGUF block with its scale widened to f32; no dequantised copy).
//!
//! Every failure (no adapter, a build without a wgpu backend, a validation or out-of-memory
//! error, a lost device) is an [`LlmError`], never a panic: callers use the CPU instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wgpu::util::DeviceExt;

use crate::LlmError;
use crate::kquant::{BlockQ4K, BlockQ6K, QK_K};
use crate::quant::err;

/// u32 words per Q4_K block on the GPU (152 bytes: the GGUF block is 144, its 6-bit scales / mins are
/// stored unpacked here, as in [`BlockQ4K`]).
const Q4K_WORDS: usize = 38;
/// u32 words per Q6_K block on the GPU (212 bytes: ql, qh, the 16 i8 scales, d as f32).
const Q6K_WORDS: usize = 53;
/// Workgroups along x before rows wrap to y (under every adapter's 65 535 per dimension).
const STRIDE_X: u32 = 32_768;

/// A wgpu device with the inference pipelines.
pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    failed: Arc<AtomicBool>,
    q4k: wgpu::ComputePipeline,
    q6k: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    max_binding: u64,
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Q4K,
    Q6K,
}

/// A K-quant matrix (`rows` × `blocks · 256`) uploaded to the GPU.
pub struct GpuMatrix {
    buf: wgpu::Buffer,
    rows: u32,
    blocks: u32,
    kind: Kind,
}

impl GpuMatrix {
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn cols(&self) -> usize {
        (self.blocks as usize).saturating_mul(QK_K)
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
        let name = adapter.get_info().name;
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
        let (q4k, q6k) = (pipeline("matvec_q4k"), pipeline("matvec_q6k"));
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let ctx = Self { device, queue, failed, q4k, q6k, layout, max_binding, name };
        ctx.check()?;
        Ok(ctx)
    }

    /// The adapter's name, for logs.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn check(&self) -> Result<(), LlmError> {
        if self.failed.load(Ordering::SeqCst) { Err(err("the GPU reported an error")) } else { Ok(()) }
    }

    /// Uploads `rows` rows of Q4_K blocks (`weights.len()` must be a multiple of `rows`).
    pub fn upload_q4k(&self, weights: &[BlockQ4K], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::Q4K, Q4K_WORDS, pack_q4k)
    }

    /// Uploads `rows` rows of Q6_K blocks (`weights.len()` must be a multiple of `rows`).
    pub fn upload_q6k(&self, weights: &[BlockQ6K], rows: usize) -> Result<GpuMatrix, LlmError> {
        self.upload(weights, rows, Kind::Q6K, Q6K_WORDS, pack_q6k)
    }

    fn upload<B>(&self, weights: &[B], rows: usize, kind: Kind, per_block: usize, pack: fn(&B, &mut Vec<u32>)) -> Result<GpuMatrix, LlmError> {
        if rows == 0 || weights.is_empty() || !weights.len().is_multiple_of(rows) {
            return Err(err(format!("{} {kind:?} blocks are not {rows} whole rows", weights.len())));
        }
        let blocks = weights.len() / rows;
        let n_words = weights.len().checked_mul(per_block).ok_or_else(|| err("K-quant matrix too large"))?;
        let bytes = n_words.saturating_mul(4);
        // the shader indexes words with u32
        if bytes as u64 > self.max_binding || u32::try_from(n_words).is_err() {
            return Err(err(format!("{kind:?} matrix of {bytes} bytes is over the GPU's binding limit")));
        }
        let mut words = Vec::new();
        words.try_reserve_exact(n_words).map_err(|_| err("out of memory packing a K-quant matrix"))?;
        for b in weights {
            pack(b, &mut words);
        }
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-kquant"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.check()?;
        let rows = u32::try_from(rows).map_err(|_| err("too many rows"))?;
        let blocks = u32::try_from(blocks).map_err(|_| err("too many blocks per row"))?;
        Ok(GpuMatrix { buf, rows, blocks, kind })
    }

    /// `out[r] = m[r] · x`, with `x.len() == m.cols()`.
    pub fn matvec(&self, m: &GpuMatrix, x: &[f32]) -> Result<Vec<f32>, LlmError> {
        if x.len() != m.cols() {
            return Err(err(format!("vector of {} values for a matrix of {} columns", x.len(), m.cols())));
        }
        let out_bytes = u64::from(m.rows) * 4;
        let params: Vec<u8> = [m.rows, m.blocks, STRIDE_X, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-q4k-params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let xbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("llm-x"),
            contents: bytemuck::cast_slice(x),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let ybuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llm-y"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llm-y-read"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llm-matvec"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: m.buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: xbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: ybuf.as_entire_binding() },
            ],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("llm-matvec") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("llm-matvec"), timestamp_writes: None });
            pass.set_pipeline(match m.kind {
                Kind::Q4K => &self.q4k,
                Kind::Q6K => &self.q6k,
            });
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(m.rows.min(STRIDE_X), m.rows.div_ceil(STRIDE_X), 1);
        }
        enc.copy_buffer_to_buffer(&ybuf, 0, &read, 0, out_bytes);
        self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| err(format!("GPU wait: {e}")))?;
        self.check()?;
        rx.recv().map_err(|_| err("GPU read-back lost"))?.map_err(|e| err(format!("GPU read-back: {e}")))?;
        let data = read.slice(..).get_mapped_range().map_err(|e| err(format!("GPU read-back: {e}")))?;
        let out: Vec<f32> = data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        drop(data);
        read.unmap();
        Ok(out)
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
        // 70 001 rows: past STRIDE_X, so the second dispatch dimension is used too
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
        let m = ctx.upload_q4k(&w, 2).unwrap();
        assert!(ctx.matvec(&m, &[0.0; 256]).is_err());
        assert!(ctx.matvec(&m, &[0.0; 513]).is_err());
        assert_eq!(ctx.matvec(&m, &[0.0; 512]).unwrap(), vec![0.0, 0.0]);
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
