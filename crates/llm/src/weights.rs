//! Weight tensors read from a checked GGUF file (AI-PLAN AI1.1d): one type for every format the
//! catalogue's models use (F32, F16, Q5_0, Q8_0, Q4_K, Q6_K), with a matrix-vector product and a row lookup.
//!
//! Only call this after [`crate::gguf::check`] succeeded on the same file: it proved that every
//! tensor's bytes lie inside the file, so reading them allocates at most the file's size.

use std::io::{Read, Seek, SeekFrom};

use half::f16;

use crate::LlmError;
use crate::gguf::{GgufSummary, TensorInfo};
use crate::kquant::{self, BlockQ4K, BlockQ6K, BlockQ8K, QK_K};
use crate::quant::{self, BlockQ8_0, QK8_0, err};

/// ggml type ids used by the catalogue's models.
const F32: u32 = 0;
const F16: u32 = 1;
const Q5_0: u32 = 6;
const Q8_0: u32 = 8;
const Q4_K: u32 = 12;
const Q6_K: u32 = 14;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Data {
    F32(Vec<f32>),
    Q8_0(Vec<BlockQ8_0>),
    Q4K(Vec<BlockQ4K>),
    Q6K(Vec<BlockQ6K>),
}

/// A matrix of `rows` × `cols` (GGUF dims `[cols, rows]`), or a vector (`rows` = 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Weight {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    data: Data,
}

/// Reused buffers for the quantized activations (no caching across calls: quantizing a vector
/// costs ~1 % of a matvec, and a cache keyed on the buffer would go stale when it is rewritten).
#[derive(Default)]
pub struct Scratch {
    q8_0: Vec<BlockQ8_0>,
    q8k: Vec<BlockQ8K>,
    one_q8_0: Vec<BlockQ8_0>,
    one_q8k: Vec<BlockQ8K>,
    /// rows × t results before the transpose to t × rows.
    by_row: Vec<f32>,
}

impl Weight {
    /// Reads tensor `name` from `r` (the file `summary` came from).
    pub fn read<R: Read + Seek>(r: &mut R, summary: &GgufSummary, name: &str) -> Result<Weight, LlmError> {
        let info = summary.tensor(name).ok_or_else(|| err(format!("tensor {name} missing")))?;
        Self::read_info(r, summary, info)
    }

    fn read_info<R: Read + Seek>(r: &mut R, summary: &GgufSummary, info: &TensorInfo) -> Result<Weight, LlmError> {
        let (cols, rows) = match info.dims.as_slice() {
            [c] => (*c, 1),
            [c, r] => (*c, *r),
            d => return Err(err(format!("tensor {} has dims {d:?}", info.name))),
        };
        let cols = usize::try_from(cols).map_err(|_| err("tensor too large"))?;
        let rows = usize::try_from(rows).map_err(|_| err("tensor too large"))?;
        if cols == 0 || rows == 0 {
            return Err(err(format!("tensor {} is empty", info.name)));
        }
        let size = usize::try_from(info.size).map_err(|_| err("tensor too large"))?;
        let at = summary.data_offset.checked_add(info.offset).ok_or_else(|| err("tensor offset overflow"))?;
        r.seek(SeekFrom::Start(at)).map_err(|e| err(e.to_string()))?;
        let mut bytes = vec![0u8; size];
        r.read_exact(&mut bytes).map_err(|e| err(format!("tensor {}: {e}", info.name)))?;
        let data = match info.ty {
            F32 => Data::F32(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()),
            F16 => Data::F32(bytes.as_chunks::<2>().0.iter().map(|b| f16::from_le_bytes(*b).to_f32()).collect()),
            Q8_0 => {
                let mut v = Vec::new();
                quant::parse_q8_0(&bytes, &mut v)?;
                Data::Q8_0(v)
            }
            Q5_0 => {
                let mut v = Vec::new();
                quant::parse_q5_0_as_q8_0(&bytes, &mut v)?;
                Data::Q8_0(v)
            }
            Q4_K => {
                let mut v = Vec::new();
                kquant::parse_q4k(&bytes, &mut v)?;
                Data::Q4K(v)
            }
            Q6_K => {
                let mut v = Vec::new();
                kquant::parse_q6k(&bytes, &mut v)?;
                Data::Q6K(v)
            }
            t => return Err(err(format!("tensor {}: unsupported type {t}", info.name))),
        };
        let w = Weight { name: info.name.clone(), rows, cols, data };
        let elems = rows.checked_mul(cols).ok_or_else(|| err("tensor too large"))?;
        if w.elements() != elems {
            return Err(err(format!("tensor {}: data does not match its dims", w.name)));
        }
        if let Data::F32(v) = &w.data
            && v.iter().any(|x| !x.is_finite())
        {
            return Err(err(format!("tensor {}: non-finite value", w.name)));
        }
        Ok(w)
    }

    /// The loaded values (for the GPU upload).
    #[cfg(feature = "gpu")]
    pub(crate) fn data(&self) -> &Data {
        &self.data
    }

    fn elements(&self) -> usize {
        match &self.data {
            Data::F32(v) => v.len(),
            Data::Q8_0(v) => v.len().saturating_mul(QK8_0),
            Data::Q4K(v) => v.len().saturating_mul(QK_K),
            Data::Q6K(v) => v.len().saturating_mul(QK_K),
        }
    }

    /// The tensor as one f32 vector (norm weights, biases); `rows` must be 1.
    pub fn vector(&self) -> Result<Vec<f32>, LlmError> {
        if self.rows != 1 {
            return Err(err(format!("tensor {} is a matrix, not a vector", self.name)));
        }
        self.row(0)
    }

    /// Row `i` dequantized (embedding lookup).
    pub fn row(&self, i: usize) -> Result<Vec<f32>, LlmError> {
        if i >= self.rows {
            return Err(err(format!("row {i} of {} ({} rows)", self.name, self.rows)));
        }
        let mut out = Vec::with_capacity(self.cols);
        let span = |block: usize| {
            let per = self.cols / block;
            i.checked_mul(per).and_then(|a| Some(a..a.checked_add(per)?)).ok_or_else(|| err("row out of range"))
        };
        match &self.data {
            Data::F32(v) => out.extend_from_slice(v.get(span(1)?).ok_or_else(|| err("row out of range"))?),
            Data::Q8_0(v) => quant::dequantize_q8_0(v.get(span(QK8_0)?).ok_or_else(|| err("row out of range"))?, &mut out),
            Data::Q4K(v) => kquant::dequantize_q4k(v.get(span(QK_K)?).ok_or_else(|| err("row out of range"))?, &mut out),
            Data::Q6K(v) => kquant::dequantize_q6k(v.get(span(QK_K)?).ok_or_else(|| err("row out of range"))?, &mut out),
        }
        Ok(out)
    }

    /// `out = self · x` (`x.len()` = cols, `out.len()` = rows).
    pub fn matvec(&self, x: &[f32], out: &mut [f32], scratch: &mut Scratch) -> Result<(), LlmError> {
        if x.len() != self.cols || out.len() != self.rows {
            return Err(err(format!("{}: {}×{} times {} into {}", self.name, self.rows, self.cols, x.len(), out.len())));
        }
        match &self.data {
            Data::F32(w) => {
                for (o, row) in out.iter_mut().zip(w.chunks_exact(self.cols)) {
                    *o = row.iter().zip(x).map(|(a, b)| a * b).sum();
                }
                Ok(())
            }
            Data::Q8_0(w) => {
                quant::quantize_q8_0(x, &mut scratch.q8_0)?;
                quant::matvec_q8_0(w, &scratch.q8_0, out)
            }
            Data::Q4K(w) => {
                kquant::quantize_q8k(x, &mut scratch.q8k)?;
                kquant::matvec_q4k(w, &scratch.q8k, out)
            }
            Data::Q6K(w) => {
                kquant::quantize_q8k(x, &mut scratch.q8k)?;
                kquant::matvec_q6k(w, &scratch.q8k, out)
            }
        }
    }

    /// `out[i·rows + r] = self[r] · xs[i]` for `t = xs.len() / cols` input rows: each weight row is
    /// read once per batch. Bit-identical to `t` calls of [`Self::matvec`].
    pub fn matmul(&self, xs: &[f32], out: &mut [f32], scratch: &mut Scratch) -> Result<(), LlmError> {
        let t = xs.len().checked_div(self.cols).unwrap_or(0);
        if t == 0 || !xs.len().is_multiple_of(self.cols) || Some(out.len()) != t.checked_mul(self.rows) {
            return Err(err(format!("{}: {}×{} times {} values into {}", self.name, self.rows, self.cols, xs.len(), out.len())));
        }
        if t == 1 {
            return self.matvec(xs, out, scratch);
        }
        let quantize8 = |s: &mut Scratch| -> Result<(), LlmError> {
            s.q8_0.clear();
            for x in xs.chunks_exact(self.cols) {
                quant::quantize_q8_0(x, &mut s.one_q8_0)?;
                s.q8_0.extend_from_slice(&s.one_q8_0);
            }
            Ok(())
        };
        let quantizek = |s: &mut Scratch| -> Result<(), LlmError> {
            s.q8k.clear();
            for x in xs.chunks_exact(self.cols) {
                kquant::quantize_q8k(x, &mut s.one_q8k)?;
                s.q8k.extend_from_slice(&s.one_q8k);
            }
            Ok(())
        };
        scratch.by_row.clear();
        scratch.by_row.resize(out.len(), 0.0);
        match &self.data {
            Data::F32(_) => {
                for (x, o) in xs.chunks_exact(self.cols).zip(out.chunks_exact_mut(self.rows)) {
                    self.matvec(x, o, scratch)?;
                }
                return Ok(());
            }
            Data::Q8_0(w) => {
                quantize8(scratch)?;
                quant::matmul_q8_0(w, &scratch.q8_0, self.cols / QK8_0, &mut scratch.by_row)?;
            }
            Data::Q4K(w) => {
                quantizek(scratch)?;
                kquant::matmul_q4k(w, &scratch.q8k, self.cols / QK_K, &mut scratch.by_row)?;
            }
            Data::Q6K(w) => {
                quantizek(scratch)?;
                kquant::matmul_q6k(w, &scratch.q8k, self.cols / QK_K, &mut scratch.by_row)?;
            }
        }
        for (r, row) in scratch.by_row.chunks_exact(t).enumerate() {
            for (i, &v) in row.iter().enumerate() {
                if let Some(o) = out.get_mut(i * self.rows + r) {
                    *o = v;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::tests::Builder;
    use std::io::Cursor;

    fn values(n: usize, seed: u32) -> Vec<f32> {
        (0..n).map(|i| ((i as u32).wrapping_mul(2_654_435_761).wrapping_add(seed) >> 8) as f32 / (1u32 << 24) as f32 - 0.5).collect()
    }

    fn q8_0_bytes(xs: &[f32]) -> Vec<u8> {
        let mut q = Vec::new();
        quant::quantize_q8_0(xs, &mut q).unwrap();
        let mut b = Vec::new();
        for blk in &q {
            b.extend_from_slice(&f16::from_f32(blk.d).to_le_bytes());
            b.extend(blk.qs.iter().map(|v| v.to_le_bytes()[0]));
        }
        b
    }

    fn file(rows: usize, cols: usize) -> (Vec<u8>, Vec<f32>, Vec<f32>) {
        let w = values(rows * cols, 1);
        let norm = values(cols, 2);
        let bytes = Builder::default()
            .tensor_data("m.weight", &[cols as u64, rows as u64], Q8_0, &q8_0_bytes(&w))
            .tensor_data("n.weight", &[cols as u64], F32, &norm.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
            .build();
        (bytes, w, norm)
    }

    fn summary(bytes: &[u8]) -> GgufSummary {
        crate::gguf::check(&mut Cursor::new(bytes), bytes.len() as u64).unwrap()
    }

    #[test]
    fn reads_vectors_rows_and_multiplies() {
        let (rows, cols) = (5, 64);
        let (bytes, w, norm) = file(rows, cols);
        let s = summary(&bytes);
        let mut c = Cursor::new(&bytes);
        let n = Weight::read(&mut c, &s, "n.weight").unwrap();
        assert_eq!(n.vector().unwrap(), norm);
        let m = Weight::read(&mut c, &s, "m.weight").unwrap();
        assert_eq!((m.rows, m.cols), (rows, cols));
        let r2 = m.row(2).unwrap();
        for (a, b) in r2.iter().zip(&w[2 * cols..3 * cols]) {
            assert!((a - b).abs() < 0.01);
        }
        let x = values(cols, 3);
        let mut out = vec![0.0; rows];
        let mut scratch = Scratch::default();
        m.matvec(&x, &mut out, &mut scratch).unwrap();
        for (r, o) in out.iter().enumerate() {
            let want: f32 = w[r * cols..(r + 1) * cols].iter().zip(&x).map(|(a, b)| a * b).sum();
            assert!((o - want).abs() < 0.05, "row {r}: {o} vs {want}");
        }
    }

    #[test]
    fn matmul_equals_matvecs() {
        let (rows, cols, t) = (70, 64, 3);
        let (bytes, _, _) = file(rows, cols);
        let s = summary(&bytes);
        let m = Weight::read(&mut Cursor::new(&bytes), &s, "m.weight").unwrap();
        let xs = values(t * cols, 9);
        let mut scratch = Scratch::default();
        let mut out = vec![0.0; t * rows];
        m.matmul(&xs, &mut out, &mut scratch).unwrap();
        for (x, o) in xs.chunks_exact(cols).zip(out.chunks_exact(rows)) {
            let mut one = vec![0.0; rows];
            m.matvec(x, &mut one, &mut scratch).unwrap();
            assert_eq!(o, one.as_slice());
        }
        assert!(m.matmul(&xs[..cols * t - 1], &mut out, &mut scratch).is_err());
        assert!(m.matmul(&xs, &mut out[..rows], &mut scratch).is_err());
    }

    #[test]
    fn hostile_tensors_are_errors() {
        let (bytes, _, _) = file(2, 32);
        let s = summary(&bytes);
        let mut c = Cursor::new(&bytes);
        assert!(Weight::read(&mut c, &s, "missing").is_err());
        let m = Weight::read(&mut c, &s, "m.weight").unwrap();
        assert!(m.row(2).is_err());
        assert!(m.vector().is_err());
        let mut scratch = Scratch::default();
        assert!(m.matvec(&[0.0; 31], &mut [0.0; 2], &mut scratch).is_err());
        assert!(m.matvec(&[0.0; 32], &mut [0.0; 3], &mut scratch).is_err());
        // A NaN in an F32 tensor, an unsupported type and a 3-D tensor are refused.
        let nan = Builder::default().tensor_data("n", &[2], F32, &[f32::NAN.to_le_bytes(), 0f32.to_le_bytes()].concat()).build();
        assert!(Weight::read(&mut Cursor::new(&nan), &summary(&nan), "n").is_err());
        let q41 = Builder::default().tensor_data("q", &[32], 3, &[0u8; 20]).build();
        assert!(Weight::read(&mut Cursor::new(&q41), &summary(&q41), "q").is_err());
        let cube = Builder::default().tensor_data("c", &[1, 1, 1], F32, &[0u8; 4]).build();
        assert!(Weight::read(&mut Cursor::new(&cube), &summary(&cube), "c").is_err());
    }
}
