//! A bounds-checked walk over a GGUF header (versions 2 and 3), run before the file reaches
//! candle.
//!
//! candle's GGUF reader trusts the header: string and array lengths become allocations, tensor
//! dimensions become read sizes, a zero alignment divides by zero, and model hyper-parameters size
//! tables. This walk proves, without allocating more than the file holds, that every length fits
//! in the file, that every tensor's data lies inside it, and that the hyper-parameters the
//! loaders use are in sane ranges. Format: the GGUF specification published with ggml
//! (`docs/gguf.md`): little-endian header `GGUF`, version, tensor count, metadata count,
//! metadata key/values, tensor infos, then aligned tensor data.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use crate::LlmError;

const MAGIC: u32 = 0x4655_4747; // "GGUF" little-endian
const MAX_TENSORS: u64 = 65_536;
const MAX_KV: u64 = 65_536;
const MAX_KEY: u64 = 65_536;
const MAX_STRING: u64 = 64 << 20;
/// Elements in one metadata array (vocabularies are ~150 k).
const MAX_ARRAY: u64 = 1 << 21;
/// Elements in all metadata arrays together.
const MAX_ARRAY_TOTAL: u64 = 1 << 23;
const MAX_DEPTH: u32 = 2;
const MAX_DIMS: u32 = 4;
/// Elements in one tensor: bounds a dequantized f32 copy to 2 GiB. The largest we load is
/// Qwen3-VL 4B's `token_embd.weight`, 151 936 × 2 560 ≈ 2^28.5 (Qwen2.5-1.5B's ≈ 2^27.8).
const MAX_TENSOR_ELEMS: u64 = 1 << 29;
const MAX_VOCAB: u64 = 1 << 20;
const MAX_HEAD_DIM: u64 = 512;
const DEFAULT_ALIGNMENT: u64 = 32;

/// What the walk found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GgufSummary {
    pub version: u32,
    pub architecture: Option<String>,
    pub tensors: usize,
    /// Integer-valued scalar metadata (unsigned or non-negative), by key.
    pub ints: HashMap<String, u64>,
    /// Start of the tensor data.
    pub data_offset: u64,
    /// Dimensions of `token_embd.weight` (GGUF order: embedding length first, then vocabulary).
    pub token_embd: Option<Vec<u64>>,
    /// Finite floating-point scalar metadata (`rope.freq_base`, the RMS-norm epsilon, …), by key.
    pub floats: HashMap<String, f64>,
    /// Every tensor, in file order; offsets are relative to [`Self::data_offset`] and proven to lie
    /// inside the file.
    pub tensor_infos: Vec<TensorInfo>,
}

/// One tensor's directory entry.
#[derive(Clone, Debug, PartialEq)]
pub struct TensorInfo {
    pub name: String,
    /// GGUF order: the row length (contiguous) first.
    pub dims: Vec<u64>,
    /// ggml type id (see [`ggml_type_layout`]).
    pub ty: u32,
    pub offset: u64,
    /// Bytes of data.
    pub size: u64,
}

impl GgufSummary {
    pub fn get(&self, key: &str) -> Option<u64> {
        self.ints.get(key).copied()
    }

    pub fn float(&self, key: &str) -> Option<f64> {
        self.floats.get(key).copied()
    }

    pub fn tensor(&self, name: &str) -> Option<&TensorInfo> {
        self.tensor_infos.iter().find(|t| t.name == name)
    }
}

/// `(block size in elements, bytes per block)` of a ggml tensor type, for the types candle reads.
pub fn ggml_type_layout(t: u32) -> Option<(u64, u64)> {
    Some(match t {
        0 => (1, 4),      // F32
        1 => (1, 2),      // F16
        2 => (32, 18),    // Q4_0
        3 => (32, 20),    // Q4_1
        6 => (32, 22),    // Q5_0
        7 => (32, 24),    // Q5_1
        8 => (32, 34),    // Q8_0
        9 => (32, 36),    // Q8_1
        10 => (256, 84),  // Q2_K
        11 => (256, 110), // Q3_K
        12 => (256, 144), // Q4_K
        13 => (256, 176), // Q5_K
        14 => (256, 210), // Q6_K
        15 => (256, 292), // Q8_K
        30 => (1, 2),     // BF16
        _ => return None,
    })
}

struct Rd<'a, R> {
    r: &'a mut R,
    pos: u64,
    len: u64,
    array_budget: u64,
}

fn bad(reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: String::new(), reason: reason.into() }
}

impl<R: Read + Seek> Rd<'_, R> {
    fn need(&self, n: u64) -> Result<(), LlmError> {
        match self.pos.checked_add(n) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(bad(format!("truncated at byte {} (needs {n} more)", self.pos))),
        }
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], LlmError> {
        self.need(N as u64)?;
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        self.pos = self.pos.saturating_add(N as u64);
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32, LlmError> {
        Ok(u32::from_le_bytes(self.bytes::<4>()?))
    }

    fn u64(&mut self) -> Result<u64, LlmError> {
        Ok(u64::from_le_bytes(self.bytes::<8>()?))
    }

    fn skip(&mut self, n: u64) -> Result<(), LlmError> {
        self.need(n)?;
        let off = i64::try_from(n).map_err(|_| bad("length overflow"))?;
        self.r.seek(SeekFrom::Current(off))?;
        self.pos = self.pos.saturating_add(n);
        Ok(())
    }

    /// A string's length after checking it fits.
    fn str_len(&mut self, max: u64) -> Result<u64, LlmError> {
        let n = self.u64()?;
        if n > max {
            return Err(bad(format!("string of {n} bytes")));
        }
        self.need(n)?;
        Ok(n)
    }

    fn string(&mut self, max: u64) -> Result<String, LlmError> {
        let n = self.str_len(max)?;
        let n = usize::try_from(n).map_err(|_| bad("length overflow"))?;
        let mut v = vec![0u8; n];
        self.r.read_exact(&mut v)?;
        self.pos = self.pos.saturating_add(n as u64);
        String::from_utf8(v).map_err(|_| bad("string is not UTF-8"))
    }

    fn skip_string(&mut self) -> Result<(), LlmError> {
        let n = self.str_len(MAX_STRING)?;
        self.skip(n)
    }

    /// Walk one value of type `ty`; returns it when it is an integer scalar (or a string, kept
    /// only when `keep_string`).
    fn value(&mut self, ty: u32, depth: u32, keep_string: bool) -> Result<Value, LlmError> {
        Ok(match ty {
            0 => Value::Int(u64::from(u8::from_le_bytes(self.bytes::<1>()?))),
            1 => Value::from_signed(i64::from(i8::from_le_bytes(self.bytes::<1>()?))),
            2 => Value::Int(u64::from(u16::from_le_bytes(self.bytes::<2>()?))),
            3 => Value::from_signed(i64::from(i16::from_le_bytes(self.bytes::<2>()?))),
            4 => Value::Int(u64::from(self.u32()?)),
            5 => Value::from_signed(i64::from(i32::from_le_bytes(self.bytes::<4>()?))),
            6 => Value::from_float(f64::from(f32::from_le_bytes(self.bytes::<4>()?))),
            7 => match u8::from_le_bytes(self.bytes::<1>()?) {
                0 | 1 => Value::Other,
                b => return Err(bad(format!("bool value {b}"))),
            },
            8 => {
                if keep_string {
                    Value::Str(self.string(MAX_KEY)?)
                } else {
                    self.skip_string()?;
                    Value::Other
                }
            }
            9 => {
                if depth >= MAX_DEPTH {
                    return Err(bad("arrays nested too deeply"));
                }
                let ety = self.u32()?;
                let n = self.u64()?;
                if n > MAX_ARRAY || n > self.array_budget {
                    return Err(bad(format!("array of {n} elements")));
                }
                self.array_budget = self.array_budget.saturating_sub(n);
                match fixed_size(ety) {
                    Some(sz) => {
                        let total = n.checked_mul(sz).ok_or_else(|| bad("array size overflow"))?;
                        self.skip(total)?;
                    }
                    None if ety == 8 || ety == 9 => {
                        for _ in 0..n {
                            self.value(ety, depth.saturating_add(1), false)?;
                        }
                    }
                    None => return Err(bad(format!("array of unknown type {ety}"))),
                }
                Value::Other
            }
            10 => Value::Int(self.u64()?),
            11 => Value::from_signed(i64::from_le_bytes(self.bytes::<8>()?)),
            12 => Value::from_float(f64::from_le_bytes(self.bytes::<8>()?)),
            t => return Err(bad(format!("unknown metadata type {t}"))),
        })
    }
}

enum Value {
    Int(u64),
    Float(f64),
    Str(String),
    Other,
}

impl Value {
    fn from_float(v: f64) -> Self {
        if v.is_finite() { Value::Float(v) } else { Value::Other }
    }

    fn from_signed(v: i64) -> Self {
        u64::try_from(v).map_or(Value::Other, Value::Int)
    }
}

fn fixed_size(ty: u32) -> Option<u64> {
    match ty {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

/// Walk the GGUF in `r` (whose length is `len` bytes) from its start. On success, every tensor's
/// data lies inside the file and every length in the header is bounded by the file; the reader is
/// left somewhere inside it (rewind before reusing it).
pub fn check<R: Read + Seek>(r: &mut R, len: u64) -> Result<GgufSummary, LlmError> {
    r.seek(SeekFrom::Start(0))?;
    let mut rd = Rd { r, pos: 0, len, array_budget: MAX_ARRAY_TOTAL };
    if rd.u32()? != MAGIC {
        return Err(bad("not a GGUF file"));
    }
    let version = rd.u32()?;
    if !(2..=3).contains(&version) {
        return Err(bad(format!("unsupported GGUF version {version}")));
    }
    let tensors = rd.u64()?;
    let kvs = rd.u64()?;
    if tensors > MAX_TENSORS || kvs > MAX_KV {
        return Err(bad(format!("{tensors} tensors, {kvs} metadata entries")));
    }
    let mut out = GgufSummary { version, ..Default::default() };
    // candle honours `general.alignment` only as u8/u16/u32 or a non-negative i8/i16/i32
    // (gguf_file.rs), else it uses 32; mirror that (the last entry wins, as in its map).
    let mut alignment = DEFAULT_ALIGNMENT;
    for _ in 0..kvs {
        let key = rd.string(MAX_KEY)?;
        let ty = rd.u32()?;
        let keep = key == "general.architecture";
        let value = rd.value(ty, 0, keep)?;
        if key == "general.alignment" {
            alignment = match (&value, ty) {
                (Value::Int(v), 0..=5) => *v,
                _ => DEFAULT_ALIGNMENT,
            };
        }
        match value {
            Value::Int(v) => {
                out.ints.insert(key, v);
            }
            Value::Float(v) => {
                out.floats.insert(key, v);
            }
            Value::Str(s) if keep => out.architecture = Some(s),
            _ => {}
        }
    }
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut total = 0u64;
    for _ in 0..tensors {
        let name = rd.string(MAX_KEY)?;
        let n_dims = rd.u32()?;
        if n_dims == 0 || n_dims > MAX_DIMS {
            return Err(bad(format!("tensor with {n_dims} dimensions")));
        }
        let mut elems = 1u64;
        let mut dims = Vec::new();
        for _ in 0..n_dims {
            let d = rd.u64()?;
            dims.push(d);
            elems = elems.checked_mul(d).filter(|e| *e <= MAX_TENSOR_ELEMS).ok_or_else(|| bad("tensor too large"))?;
        }
        let first = dims.first().copied().unwrap_or(0);
        let ty = rd.u32()?;
        let (block, bytes) = ggml_type_layout(ty).ok_or_else(|| bad(format!("unsupported tensor type {ty}")))?;
        if !first.is_multiple_of(block) {
            return Err(bad("tensor row is not a whole number of blocks"));
        }
        let size = (elems / block).checked_mul(bytes).ok_or_else(|| bad("tensor size overflow"))?;
        let offset = rd.u64()?;
        let end = offset.checked_add(size).ok_or_else(|| bad("tensor offset overflow"))?;
        total = total.saturating_add(size);
        if size > 0 {
            ranges.push((offset, end));
        }
        if name == "token_embd.weight" {
            out.token_embd = Some(dims.clone());
        }
        out.tensor_infos.push(TensorInfo { name, dims, ty, offset, size });
    }
    if alignment == 0 || alignment > 65_536 {
        return Err(bad(format!("alignment {alignment}")));
    }
    let data_offset = rd.pos.div_ceil(alignment).checked_mul(alignment).ok_or_else(|| bad("alignment overflow"))?;
    let room = len.checked_sub(data_offset).ok_or_else(|| bad("tensor data starts past the end of the file"))?;
    if total > room {
        return Err(bad("tensor data is larger than the file"));
    }
    // Every range inside the file and none overlapping, so candle reads at most the file, once.
    ranges.sort_unstable();
    let mut prev_end = 0u64;
    for (start, end) in ranges {
        if end > room {
            return Err(bad("tensor data runs past the end of the file"));
        }
        if start < prev_end {
            return Err(bad("tensors overlap"));
        }
        prev_end = end;
    }
    out.tensors = usize::try_from(tensors).map_err(|_| bad("too many tensors"))?;
    out.data_offset = data_offset;
    Ok(out)
}

/// Check the hyper-parameters candle's `qwen2` / `llama` loaders size tables with.
pub fn check_hyper(s: &GgufSummary, arch: &str) -> Result<(), LlmError> {
    if s.architecture.as_deref() != Some(arch) {
        return Err(bad(format!("architecture {:?}, expected {arch}", s.architecture)));
    }
    let need = |k: &str, lo: u64, hi: u64| -> Result<u64, LlmError> {
        let key = format!("{arch}.{k}");
        match s.get(&key) {
            Some(v) if (lo..=hi).contains(&v) => Ok(v),
            Some(v) => Err(bad(format!("{key} = {v}"))),
            None => Err(bad(format!("{key} missing"))),
        }
    };
    let heads = need("attention.head_count", 1, 1024)?;
    need("attention.head_count_kv", 1, heads)?;
    let embd = need("embedding_length", 1, 65_536)?;
    if !embd.is_multiple_of(heads) {
        return Err(bad("embedding_length is not a multiple of head_count"));
    }
    // The rope tables are context × head_dim / 2 (cos and sin), so bound head_dim; rope needs it even.
    let head_dim = embd / heads;
    if head_dim > MAX_HEAD_DIM || !head_dim.is_multiple_of(2) {
        return Err(bad(format!("head dimension {head_dim}")));
    }
    // candle dequantizes the token embeddings to f32: they must be embedding × vocabulary.
    match s.token_embd.as_deref() {
        Some([e, vocab]) if *e == embd && (1..=MAX_VOCAB).contains(vocab) => {}
        other => return Err(bad(format!("token_embd.weight is {other:?}, expected [{embd}, vocabulary]"))),
    }
    need("block_count", 1, 512)?;
    match arch {
        "qwen2" => {
            need("context_length", 1, 131_072)?;
        }
        _ => {
            let rope = need("rope.dimension_count", 2, MAX_HEAD_DIM)?;
            if !rope.is_multiple_of(2) {
                return Err(bad(format!("rope dimension {rope}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// A tiny well-formed GGUF v3 builder for tests.
    #[derive(Default)]
    pub(crate) struct Builder {
        kv: Vec<u8>,
        n_kv: u64,
        ti: Vec<u8>,
        n_t: u64,
        data: u64,
        blobs: Vec<(u64, Vec<u8>)>,
    }

    impl Builder {
        fn s(buf: &mut Vec<u8>, s: &str) {
            buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        pub(crate) fn str(mut self, k: &str, v: &str) -> Self {
            Self::s(&mut self.kv, k);
            self.kv.extend_from_slice(&8u32.to_le_bytes());
            Self::s(&mut self.kv, v);
            self.n_kv += 1;
            self
        }
        pub(crate) fn u32(mut self, k: &str, v: u32) -> Self {
            Self::s(&mut self.kv, k);
            self.kv.extend_from_slice(&4u32.to_le_bytes());
            self.kv.extend_from_slice(&v.to_le_bytes());
            self.n_kv += 1;
            self
        }
        pub(crate) fn f32(mut self, k: &str, v: f32) -> Self {
            Self::s(&mut self.kv, k);
            self.kv.extend_from_slice(&6u32.to_le_bytes());
            self.kv.extend_from_slice(&v.to_le_bytes());
            self.n_kv += 1;
            self
        }
        pub(crate) fn raw_kv(mut self, bytes: &[u8]) -> Self {
            self.kv.extend_from_slice(bytes);
            self.n_kv += 1;
            self
        }
        /// An F32 tensor with `dims`.
        pub(crate) fn tensor(mut self, name: &str, dims: &[u64]) -> Self {
            Self::s(&mut self.ti, name);
            self.ti.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for d in dims {
                self.ti.extend_from_slice(&d.to_le_bytes());
            }
            self.ti.extend_from_slice(&0u32.to_le_bytes());
            self.ti.extend_from_slice(&self.data.to_le_bytes());
            self.data += dims.iter().product::<u64>() * 4;
            self.data = self.data.div_ceil(32) * 32;
            self.n_t += 1;
            self
        }
        /// A tensor of type `ty` whose data is `bytes` (padded to 32).
        #[cfg_attr(not(feature = "infer"), allow(dead_code))]
        pub(crate) fn tensor_data(mut self, name: &str, dims: &[u64], ty: u32, bytes: &[u8]) -> Self {
            Self::s(&mut self.ti, name);
            self.ti.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for d in dims {
                self.ti.extend_from_slice(&d.to_le_bytes());
            }
            self.ti.extend_from_slice(&ty.to_le_bytes());
            self.ti.extend_from_slice(&self.data.to_le_bytes());
            self.blobs.push((self.data, bytes.to_vec()));
            self.data += bytes.len() as u64;
            self.data = self.data.div_ceil(32) * 32;
            self.n_t += 1;
            self
        }
        /// A tensor info with a chosen type and offset that adds no data.
        pub(crate) fn tensor_raw(mut self, name: &str, dims: &[u64], ty: u32, offset: u64) -> Self {
            Self::s(&mut self.ti, name);
            self.ti.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for d in dims {
                self.ti.extend_from_slice(&d.to_le_bytes());
            }
            self.ti.extend_from_slice(&ty.to_le_bytes());
            self.ti.extend_from_slice(&offset.to_le_bytes());
            self.n_t += 1;
            self
        }
        pub(crate) fn build(self) -> Vec<u8> {
            let mut b = Vec::new();
            b.extend_from_slice(b"GGUF");
            b.extend_from_slice(&3u32.to_le_bytes());
            b.extend_from_slice(&self.n_t.to_le_bytes());
            b.extend_from_slice(&self.n_kv.to_le_bytes());
            b.extend_from_slice(&self.kv);
            b.extend_from_slice(&self.ti);
            let pad = (b.len() as u64).div_ceil(32) * 32;
            b.resize(pad as usize, 0);
            b.resize((pad + self.data) as usize, 0);
            for (at, blob) in &self.blobs {
                let at = (pad + at) as usize;
                b[at..at + blob.len()].copy_from_slice(blob);
            }
            b
        }
    }

    fn run(bytes: &[u8]) -> Result<GgufSummary, LlmError> {
        let mut c = Cursor::new(bytes);
        check(&mut c, bytes.len() as u64)
    }

    pub(crate) fn tiny_qwen() -> Builder {
        Builder::default()
            .str("general.architecture", "qwen2")
            .u32("qwen2.attention.head_count", 2)
            .u32("qwen2.attention.head_count_kv", 1)
            .u32("qwen2.embedding_length", 8)
            .u32("qwen2.context_length", 64)
            .u32("qwen2.block_count", 1)
            .f32("qwen2.attention.layer_norm_rms_epsilon", 1e-6)
            .tensor("token_embd.weight", &[8, 16])
            .tensor("output_norm.weight", &[8])
    }

    #[test]
    fn well_formed_header_passes() {
        let b = tiny_qwen().build();
        let s = run(&b).unwrap();
        assert_eq!(s.version, 3);
        assert_eq!(s.architecture.as_deref(), Some("qwen2"));
        assert_eq!(s.tensors, 2);
        assert_eq!(s.get("qwen2.context_length"), Some(64));
        assert_eq!(s.data_offset % 32, 0);
        check_hyper(&s, "qwen2").unwrap();
        assert!(check_hyper(&s, "llama").is_err());
    }

    #[test]
    fn every_truncation_is_an_error() {
        let b = tiny_qwen().build();
        for n in 0..b.len() {
            assert!(run(&b[..n]).is_err(), "prefix {n} accepted");
        }
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(run(b"").is_err());
        assert!(run(b"hello world, not a model").is_err());
        let mut x = 0x1234_5678u32;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        assert!(run(&noise).is_err());
        let mut g = b"GGUF".to_vec();
        g.extend_from_slice(&noise);
        assert!(run(&g).is_err());
    }

    #[test]
    fn hostile_lengths_are_refused_without_allocating() {
        // A key claiming u64::MAX bytes.
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes());
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(run(&b).is_err());
        // Huge tensor / kv counts.
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(run(&b).is_err());
        // An array of 2^40 u8.
        let mut kv = Vec::new();
        kv.extend_from_slice(&1u64.to_le_bytes());
        kv.push(b'a');
        kv.extend_from_slice(&9u32.to_le_bytes());
        kv.extend_from_slice(&0u32.to_le_bytes());
        kv.extend_from_slice(&(1u64 << 40).to_le_bytes());
        assert!(run(&tiny_qwen().raw_kv(&kv).build()).is_err());
        // A tensor larger than the file, and one whose element count overflows.
        assert!(run(&tiny_qwen().tensor_raw("x", &[1 << 16, 1 << 10], 0, 0).build()).is_err());
        assert!(run(&tiny_qwen().tensor_raw("x", &[4], 0, u64::MAX - 2).build()).is_err());
        assert!(run(&tiny_qwen().tensor_raw("x", &[u64::MAX, 2], 0, 0).build()).is_err());
        assert!(run(&tiny_qwen().tensor_raw("x", &[0], 0, 0).build()).is_ok());
        assert!(run(&tiny_qwen().tensor_raw("x", &[4], 99, 0).build()).is_err());
        assert!(run(&tiny_qwen().tensor_raw("x", &[33], 2, 0).build()).is_err());
        assert!(run(&tiny_qwen().tensor_raw("x", &[1, 1, 1, 1, 1], 0, 0).build()).is_err());
        // Zero alignment would divide by zero in candle.
        assert!(run(&tiny_qwen().u32("general.alignment", 0).build()).is_err());
        // Hyper-parameters that would size huge tables.
        let s = run(&tiny_qwen().u32("qwen2.block_count", u32::MAX).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
        let s = run(&tiny_qwen().u32("qwen2.context_length", u32::MAX).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
        let s = run(&tiny_qwen().u32("qwen2.attention.head_count", 0).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
        // head_dim = 8 / 8 = 1 is odd; 65536 / 2 is too wide.
        let s = run(&tiny_qwen().u32("qwen2.attention.head_count", 8).u32("qwen2.attention.head_count_kv", 8).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
        let s = run(&tiny_qwen().u32("qwen2.embedding_length", 65_536).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
        // token_embd.weight must be [embedding_length, vocabulary].
        let s = run(&tiny_qwen().u32("qwen2.embedding_length", 16).build()).unwrap();
        assert!(check_hyper(&s, "qwen2").is_err());
    }

    #[test]
    fn overlapping_tensors_are_refused() {
        assert!(run(&tiny_qwen().tensor_raw("x", &[8], 0, 0).build()).is_err());
        let two = Builder::default().tensor_raw("a", &[4], 0, 0).tensor_raw("b", &[4], 0, 0);
        let mut bytes = two.build();
        bytes.resize(bytes.len() + 64, 0);
        assert!(matches!(run(&bytes), Err(LlmError::BadModel { reason, .. }) if reason.contains("overlap")));
        // Adjacent is fine.
        let ok = Builder::default().tensor_raw("a", &[4], 0, 0).tensor_raw("b", &[4], 0, 16);
        let mut bytes = ok.build();
        bytes.resize(bytes.len() + 64, 0);
        assert!(run(&bytes).is_ok());
    }

    #[test]
    fn alignment_follows_candle() {
        let mut kv = Vec::new();
        kv.extend_from_slice(&17u64.to_le_bytes());
        kv.extend_from_slice(b"general.alignment");
        kv.extend_from_slice(&10u32.to_le_bytes()); // u64: candle ignores it and uses 32
        kv.extend_from_slice(&0u64.to_le_bytes());
        let s = run(&tiny_qwen().raw_kv(&kv).build()).unwrap();
        assert_eq!(s.data_offset % 32, 0);
        let mut bytes = tiny_qwen().u32("general.alignment", 64).build();
        bytes.resize(bytes.len() + 64, 0);
        let s = run(&bytes).unwrap();
        assert_eq!(s.data_offset % 64, 0);
    }

    #[test]
    fn deeply_nested_arrays_are_refused() {
        let mut kv = Vec::new();
        kv.extend_from_slice(&1u64.to_le_bytes());
        kv.push(b'n');
        kv.extend_from_slice(&9u32.to_le_bytes());
        for _ in 0..4 {
            kv.extend_from_slice(&9u32.to_le_bytes());
            kv.extend_from_slice(&1u64.to_le_bytes());
        }
        kv.extend_from_slice(&0u32.to_le_bytes());
        kv.extend_from_slice(&0u64.to_le_bytes());
        assert!(run(&tiny_qwen().raw_kv(&kv).build()).is_err());
    }
}
