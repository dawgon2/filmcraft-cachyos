//! Reading the `safetensors` format (Hugging Face's published layout: an 8-byte little-endian
//! header length, a JSON header naming each tensor's dtype, shape and byte range, then the raw
//! data), for models released only as safetensors (AI-PLAN AI4 item 2: Qwen3-VL 4B). Only the
//! header and the sharded index are parsed here; every number in them is treated as hostile and
//! checked against the file before any tensor is read.

use std::collections::BTreeMap;

use crate::LlmError;

/// Largest header accepted (the 4B model's headers are under 100 kB).
pub const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;
/// Most tensors one file may list.
pub const MAX_TENSORS: usize = 100_000;
/// Most dimensions a tensor may have.
pub const MAX_DIMS: usize = 8;

/// The element types the format names (those a model we use can contain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    F32,
    F16,
    Bf16,
    I64,
    I32,
    I8,
    U8,
    Bool,
}

impl Dtype {
    fn parse(s: &str) -> Option<Dtype> {
        Some(match s {
            "F32" => Dtype::F32,
            "F16" => Dtype::F16,
            "BF16" => Dtype::Bf16,
            "I64" => Dtype::I64,
            "I32" => Dtype::I32,
            "I8" => Dtype::I8,
            "U8" => Dtype::U8,
            "BOOL" => Dtype::Bool,
            _ => return None,
        })
    }

    /// Bytes per element.
    pub fn size(self) -> u64 {
        match self {
            Dtype::F32 | Dtype::I32 => 4,
            Dtype::F16 | Dtype::Bf16 => 2,
            Dtype::I64 => 8,
            Dtype::I8 | Dtype::U8 | Dtype::Bool => 1,
        }
    }
}

/// One tensor's entry: where its bytes are in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorInfo {
    pub dtype: Dtype,
    pub shape: Vec<u64>,
    /// Absolute byte offset of the first element in the file.
    pub start: u64,
    /// Byte length (elements × dtype size).
    pub len: u64,
}

impl TensorInfo {
    /// Number of elements.
    pub fn elements(&self) -> u64 {
        self.len / self.dtype.size()
    }
}

/// A parsed header: tensors by name, and where the data section starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub tensors: BTreeMap<String, TensorInfo>,
    /// Absolute offset of the data section (8 + header length).
    pub data_start: u64,
}

fn bad(file: &str, reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: file.to_string(), reason: reason.into() }
}

/// The header length from the file's first 8 bytes, checked against the file size.
pub fn header_len(first8: &[u8], file_len: u64, file: &str) -> Result<u64, LlmError> {
    let bytes: [u8; 8] = first8.get(..8).and_then(|b| b.try_into().ok()).ok_or_else(|| bad(file, "shorter than 8 bytes"))?;
    let n = u64::from_le_bytes(bytes);
    if n == 0 || n > MAX_HEADER_BYTES {
        return Err(bad(file, format!("header length {n} is out of range")));
    }
    if n.checked_add(8).is_none_or(|end| end > file_len) {
        return Err(bad(file, format!("header length {n} runs past the end of the file")));
    }
    Ok(n)
}

/// Parses the JSON header `json` of a file `file_len` bytes long (`json` is the `header_len`
/// bytes after the first 8). Every tensor must have a known dtype, a shape whose byte size
/// matches its range, and a range inside the data section; ranges may not overlap.
pub fn parse_header(json: &[u8], file_len: u64, file: &str) -> Result<Header, LlmError> {
    let data_start = (json.len() as u64).checked_add(8).ok_or_else(|| bad(file, "header too large"))?;
    let data_len = file_len.checked_sub(data_start).ok_or_else(|| bad(file, "header runs past the end of the file"))?;
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| bad(file, format!("header is not JSON: {e}")))?;
    let map = v.as_object().ok_or_else(|| bad(file, "header is not a JSON object"))?;
    if map.len() > MAX_TENSORS.saturating_add(1) {
        return Err(bad(file, format!("header lists {} entries, more than {MAX_TENSORS}", map.len())));
    }
    let mut tensors = BTreeMap::new();
    let mut ranges: Vec<(u64, u64, &str)> = Vec::new();
    for (name, t) in map {
        if name == "__metadata__" {
            continue;
        }
        let dtype = t.get("dtype").and_then(|d| d.as_str()).and_then(Dtype::parse).ok_or_else(|| bad(file, format!("`{name}`: unknown dtype")))?;
        let dims = t.get("shape").and_then(|s| s.as_array()).ok_or_else(|| bad(file, format!("`{name}`: no shape")))?;
        if dims.len() > MAX_DIMS {
            return Err(bad(file, format!("`{name}`: {} dimensions", dims.len())));
        }
        let shape: Vec<u64> =
            dims.iter().map(|d| d.as_u64()).collect::<Option<_>>().ok_or_else(|| bad(file, format!("`{name}`: shape is not whole numbers")))?;
        let offs =
            t.get("data_offsets").and_then(|o| o.as_array()).filter(|o| o.len() == 2).ok_or_else(|| bad(file, format!("`{name}`: no data_offsets pair")))?;
        let (Some(a), Some(b)) = (offs.first().and_then(|x| x.as_u64()), offs.get(1).and_then(|x| x.as_u64())) else {
            return Err(bad(file, format!("`{name}`: data_offsets are not whole numbers")));
        };
        if a > b || b > data_len {
            return Err(bad(file, format!("`{name}`: bytes {a}..{b} are outside the data section ({data_len} bytes)")));
        }
        let elements = shape.iter().try_fold(1u64, |n, d| n.checked_mul(*d)).ok_or_else(|| bad(file, format!("`{name}`: shape overflows")))?;
        let want = elements.checked_mul(dtype.size()).ok_or_else(|| bad(file, format!("`{name}`: size overflows")))?;
        if want != b - a {
            return Err(bad(file, format!("`{name}`: shape needs {want} bytes, its range has {}", b - a)));
        }
        let start = data_start.checked_add(a).ok_or_else(|| bad(file, format!("`{name}`: offset overflows")))?;
        ranges.push((a, b, name.as_str()));
        tensors.insert(name.clone(), TensorInfo { dtype, shape, start, len: b - a });
    }
    ranges.sort_unstable();
    for w in ranges.windows(2) {
        if let [(_, end, x), (next, _, y)] = w
            && next < end
        {
            return Err(bad(file, format!("`{x}` and `{y}` overlap")));
        }
    }
    Ok(Header { tensors, data_start })
}

/// A sharded model's `model.safetensors.index.json`: which file holds each tensor. File names
/// must be plain names in the model's folder (no paths), so a hostile index can't point outside it.
pub fn parse_index(json: &[u8], file: &str) -> Result<BTreeMap<String, String>, LlmError> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| bad(file, format!("index is not JSON: {e}")))?;
    let map = v.get("weight_map").and_then(|m| m.as_object()).ok_or_else(|| bad(file, "index has no weight_map"))?;
    if map.len() > MAX_TENSORS {
        return Err(bad(file, format!("index lists {} tensors, more than {MAX_TENSORS}", map.len())));
    }
    let mut out = BTreeMap::new();
    for (name, f) in map {
        let f = f.as_str().ok_or_else(|| bad(file, format!("`{name}`: file is not a string")))?;
        // exactly one plain path component (no drive prefix like `C:x`, no `:` stream, no `..`)
        let one = {
            let mut c = std::path::Path::new(f).components();
            matches!((c.next(), c.next()), (Some(std::path::Component::Normal(_)), None))
        };
        let plain = one && f != "." && f != ".." && !f.contains(['/', '\\', '\0', ':']) && f.ends_with(".safetensors");
        if !plain {
            return Err(bad(file, format!("`{name}`: `{f}` is not a safetensors file in the model's folder")));
        }
        out.insert(name.clone(), f.to_string());
    }
    Ok(out)
}

/// Reads and checks the header of the safetensors file at `path` (only the header is read).
pub fn read_header(path: &std::path::Path) -> Result<Header, LlmError> {
    use std::io::Read;
    let name = path.display().to_string();
    let mut f = std::fs::File::open(path).map_err(|_| LlmError::NotInstalled(name.clone()))?;
    let file_len = f.metadata().map_err(|e| bad(&name, e.to_string()))?.len();
    let mut first8 = [0u8; 8];
    f.read_exact(&mut first8).map_err(|_| bad(&name, "shorter than 8 bytes"))?;
    let n = header_len(&first8, file_len, &name)?;
    let n = usize::try_from(n).map_err(|_| bad(&name, "header too large"))?;
    let mut json = vec![0u8; n];
    f.read_exact(&mut json).map_err(|e| bad(&name, e.to_string()))?;
    parse_header(&json, file_len, &name)
}

/// Reads tensor `t` from the file at `path` as f32 (F32, F16 or BF16 data; other dtypes are an error).
/// At most `max_elements` are read, so a hostile header can't make it allocate without bound.
pub fn read_f32(path: &std::path::Path, t: &TensorInfo, max_elements: u64) -> Result<Vec<f32>, LlmError> {
    use std::io::{Read, Seek, SeekFrom};
    let name = path.display().to_string();
    let n = t.elements();
    if !matches!(t.dtype, Dtype::F32 | Dtype::F16 | Dtype::Bf16) {
        return Err(bad(&name, format!("{:?} tensors can't be read as f32 yet", t.dtype)));
    }
    if n > max_elements {
        return Err(bad(&name, format!("tensor of {n} elements is larger than the {max_elements} allowed")));
    }
    let len = usize::try_from(t.len).map_err(|_| bad(&name, "tensor too large"))?;
    let mut f = std::fs::File::open(path).map_err(|_| LlmError::NotInstalled(name.clone()))?;
    let file_len = f.metadata().map_err(|e| bad(&name, e.to_string()))?.len();
    if t.start.checked_add(t.len).is_none_or(|end| end > file_len) {
        return Err(bad(&name, "tensor runs past the end of the file"));
    }
    f.seek(SeekFrom::Start(t.start)).map_err(|e| bad(&name, e.to_string()))?;
    let mut raw = vec![0u8; len];
    f.read_exact(&mut raw).map_err(|e| bad(&name, e.to_string()))?;
    to_f32(&raw, t.dtype).ok_or_else(|| bad(&name, format!("{:?} tensors can't be read as f32 yet", t.dtype)))
}

/// Little-endian `raw` of `dtype` as f32 (F32, F16 and BF16; None for the others).
pub fn to_f32(raw: &[u8], dtype: Dtype) -> Option<Vec<f32>> {
    match dtype {
        Dtype::F32 => Some(raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()),
        // bf16 is the top half of an f32
        Dtype::Bf16 => Some(raw.as_chunks::<2>().0.iter().map(|b| f32::from_bits(u32::from(u16::from_le_bytes(*b)) << 16)).collect()),
        Dtype::F16 => Some(raw.as_chunks::<2>().0.iter().map(|b| f16_to_f32(u16::from_le_bytes(*b))).collect()),
        _ => None,
    }
}

/// IEEE binary16 bits as f32 (exact: every half value is an f32 value).
fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let man = u32::from(h & 0x3ff);
    let bits = match (exp, man) {
        (0, 0) => sign,
        // subnormal: man × 2^-24
        (0, m) => return if sign != 0 { -(m as f32) } else { m as f32 } * f32::from_bits(0x3380_0000),
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file: header JSON + `data` bytes, as the format lays it out.
    fn file(json: &str, data: usize) -> Vec<u8> {
        let mut v = (json.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(json.as_bytes());
        v.extend(std::iter::repeat_n(0u8, data));
        v
    }

    fn parse(bytes: &[u8]) -> Result<Header, LlmError> {
        let n = header_len(bytes, bytes.len() as u64, "t")?;
        let json = bytes.get(8..8 + n as usize).unwrap();
        parse_header(json, bytes.len() as u64, "t")
    }

    #[test]
    fn reads_tensors_and_their_absolute_ranges() {
        let json = r#"{"__metadata__":{"format":"pt"},"a":{"dtype":"BF16","shape":[2,3],"data_offsets":[0,12]},"b":{"dtype":"F32","shape":[],"data_offsets":[12,16]}}"#;
        let f = file(json, 16);
        let h = parse(&f).unwrap();
        assert_eq!(h.data_start, 8 + json.len() as u64);
        let a = &h.tensors["a"];
        assert_eq!((a.dtype, a.shape.clone(), a.start, a.len, a.elements()), (Dtype::Bf16, vec![2, 3], h.data_start, 12, 6));
        assert_eq!(h.tensors["b"].elements(), 1, "a scalar has one element");
    }

    #[test]
    fn hostile_headers_are_errors_not_panics() {
        let cases = [
            (r#"{"a":{"dtype":"Q4","shape":[1],"data_offsets":[0,1]}}"#, 1, "unknown dtype"),
            (r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,1]}}"#, 1, "size mismatch"),
            (r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,9]}}"#, 1, "past the data"),
            (r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[1,0]}}"#, 1, "reversed range"),
            (r#"{"a":{"dtype":"F32","shape":[18446744073709551615,2],"data_offsets":[0,8]}}"#, 8, "shape overflow"),
            (r#"{"a":{"dtype":"U8","shape":[-1],"data_offsets":[0,1]}}"#, 1, "negative dim"),
            (r#"{"a":{"dtype":"U8","shape":[1,1,1,1,1,1,1,1,1],"data_offsets":[0,1]}}"#, 1, "too many dims"),
            (r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,2]},"b":{"dtype":"U8","shape":[2],"data_offsets":[1,3]}}"#, 3, "overlap"),
            (r#"{"a":{"dtype":"U8","shape":[1]}}"#, 1, "no offsets"),
            (r#"[1,2]"#, 0, "not an object"),
            (r#"{"a":"#, 0, "not JSON"),
        ];
        for (json, data, why) in cases {
            assert!(parse(&file(json, data)).is_err(), "{why}");
        }
        // header lengths that lie
        assert!(header_len(&[1, 2, 3], 3, "t").is_err(), "short file");
        assert!(header_len(&0u64.to_le_bytes(), 100, "t").is_err(), "empty header");
        assert!(header_len(&u64::MAX.to_le_bytes(), 100, "t").is_err(), "huge header");
        assert!(header_len(&50u64.to_le_bytes(), 20, "t").is_err(), "past the end");
    }

    #[test]
    fn the_index_names_plain_files_in_the_model_folder() {
        let ok = br#"{"metadata":{},"weight_map":{"a.weight":"model-00001-of-00002.safetensors"}}"#;
        assert_eq!(parse_index(ok, "i").unwrap()["a.weight"], "model-00001-of-00002.safetensors");
        for f in ["../x.safetensors", "/etc/passwd", "sub\\x.safetensors", "x.bin", "", "..", "C:evil.safetensors", "x.safetensors:stream.safetensors"] {
            let j = format!(r#"{{"weight_map":{{"a":"{}"}}}}"#, f.replace('\\', "\\\\"));
            assert!(parse_index(j.as_bytes(), "i").is_err(), "{f}");
        }
        assert!(parse_index(br#"{"weight_map":{"a":3}}"#, "i").is_err());
        assert!(parse_index(br#"{}"#, "i").is_err());
    }

    #[test]
    fn bf16_and_f32_read_back_and_limits_hold() {
        // 1.5 and -2.0 in bf16 (top halves of their f32 bits), then an f32 0.25
        let mut data: Vec<u8> = Vec::new();
        for v in [1.5f32, -2.0] {
            data.extend_from_slice(&((v.to_bits() >> 16) as u16).to_le_bytes());
        }
        data.extend_from_slice(&0.25f32.to_le_bytes());
        let json = r#"{"b":{"dtype":"BF16","shape":[2],"data_offsets":[0,4]},"f":{"dtype":"F32","shape":[1],"data_offsets":[4,8]},"i":{"dtype":"I8","shape":[0],"data_offsets":[8,8]}}"#;
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        bytes.extend_from_slice(&data);
        let dir = std::env::temp_dir().join(format!("fc-safetensors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.safetensors");
        std::fs::write(&path, &bytes).unwrap();
        let h = read_header(&path).unwrap();
        assert_eq!(read_f32(&path, &h.tensors["b"], 10).unwrap(), vec![1.5, -2.0]);
        assert_eq!(read_f32(&path, &h.tensors["f"], 10).unwrap(), vec![0.25]);
        assert!(read_f32(&path, &h.tensors["b"], 1).is_err(), "over the element limit");
        assert!(read_f32(&path, &h.tensors["i"], 10).is_err(), "unsupported dtype");
        // f16: 1.0, -2.5, the smallest subnormal, infinity
        let f16 = to_f32(&[0x00, 0x3c, 0x00, 0xc1, 0x01, 0x00, 0x00, 0x7c], Dtype::F16).unwrap();
        assert_eq!(f16[..2], [1.0, -2.5]);
        assert_eq!(f16[2], 2f32.powi(-24));
        assert!(f16[3].is_infinite());
        // a header entry that points past a truncated file
        let mut lying = h.tensors["f"].clone();
        lying.start += 100;
        assert!(read_f32(&path, &lying, 10).is_err());
        assert!(read_f32(&dir.join("missing.safetensors"), &h.tensors["f"], 10).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The real Qwen3-VL 4B files (AI4 item 2), when present: every tensor in the index is in its
    /// file's header, and the vision tower is there.
    /// `FILMCRAFT_QWEN3VL_DIR=<folder> cargo test -p filmcraft-llm --lib qwen3vl -- --ignored`
    #[test]
    #[ignore]
    fn qwen3vl_files_parse() {
        let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
        let index = parse_index(&std::fs::read(dir.join("model.safetensors.index.json")).unwrap(), "index").unwrap();
        let mut headers = BTreeMap::new();
        for f in index.values() {
            if !headers.contains_key(f) {
                headers.insert(f.clone(), read_header(&dir.join(f)).unwrap());
            }
        }
        for (name, f) in &index {
            assert!(headers[f].tensors.contains_key(name), "{name} not in {f}");
        }
        // a vision weight reads back as finite numbers
        let (name, f) = index.iter().find(|(k, _)| k.contains("visual") && k.ends_with("weight")).unwrap();
        let w = read_f32(&dir.join(f), &headers[f].tensors[name], 1 << 26).unwrap();
        assert!(!w.is_empty() && w.iter().all(|v| v.is_finite()), "{name}");
        let vision = index.keys().filter(|k| k.contains("visual")).count();
        eprintln!("{} tensors, {vision} in the vision tower, {} files", index.len(), headers.len());
        assert!(vision > 0);
    }
}
