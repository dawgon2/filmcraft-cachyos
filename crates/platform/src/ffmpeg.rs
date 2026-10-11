//! The system `ffmpeg` as an external helper process (Linux): VA-API hardware decoding
//! ([`crate::vaapi_pipe`]) and HEVC encoding ([`crate::hevc_export`]).
//!
//! ffmpeg is never linked: it runs as a child process fed through pipes. Everything here is
//! optional: when ffmpeg is missing, has no VA-API support, or the GPU cannot open a VA-API
//! device, [`ffmpeg`] / [`vaapi_device`] say so and callers fall back to FilmCraft's own codecs.
//!
//! - `FILMCRAFT_FFMPEG` names the ffmpeg binary (default: `ffmpeg` on `PATH`).
//! - `FILMCRAFT_VAAPI_DEVICE` names the DRM render node (default: the first one that opens,
//!   preferring a discrete GPU — AMD, then NVIDIA — over an integrated Intel one, so a machine with
//!   both decodes on the discrete card).

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Bytes of ffmpeg's stderr kept for error messages.
const STDERR_KEEP: usize = 4096;

/// The ffmpeg binary, when it runs (`ffmpeg -version`). Probed once per process.
pub fn ffmpeg() -> Option<&'static PathBuf> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| {
        let bin = std::env::var_os("FILMCRAFT_FFMPEG").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("ffmpeg"));
        let ok = Command::new(&bin).args(["-hide_banner", "-version"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        match ok {
            Ok(s) if s.success() => Some(bin),
            other => {
                log::info!("ffmpeg not usable ({}): {other:?}", bin.display());
                None
            }
        }
    })
    .as_ref()
}

/// PCI vendor ids of DRM render nodes.
const VENDOR_AMD: u32 = 0x1002;
const VENDOR_NVIDIA: u32 = 0x10de;
const VENDOR_INTEL: u32 = 0x8086;

/// The PCI vendor of a render node (`/sys/class/drm/renderD128/device/vendor`).
fn vendor(node: &str) -> Option<u32> {
    let name = std::path::Path::new(node).file_name()?.to_str()?;
    let s = std::fs::read_to_string(format!("/sys/class/drm/{name}/device/vendor")).ok()?;
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// Render nodes in preference order: discrete GPUs first (AMD, NVIDIA, others), Intel last.
pub(crate) fn render_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("renderD"))
                .map(|n| format!("/dev/dri/{n}"))
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    let rank = |n: &String| match vendor(n) {
        Some(VENDOR_AMD) => 0,
        Some(VENDOR_NVIDIA) => 1,
        Some(VENDOR_INTEL) => 3,
        _ => 2,
    };
    nodes.sort_by_key(rank);
    nodes
}

/// Whether ffmpeg can open a VA-API device on `node` and upload a picture to it.
fn vaapi_opens(ff: &PathBuf, node: &str) -> bool {
    let st = Command::new(ff)
        .args(["-hide_banner", "-v", "error", "-init_hw_device"])
        .arg(format!("vaapi=va:{node}"))
        .args(["-filter_hw_device", "va", "-f", "lavfi", "-i", "nullsrc=s=64x64:d=0.04", "-vf", "format=nv12,hwupload", "-frames:v", "1", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(st, Ok(s) if s.success())
}

/// The VA-API render node ffmpeg uses, when one works. Probed once per process.
pub fn vaapi_device() -> Option<&'static str> {
    static D: OnceLock<Option<String>> = OnceLock::new();
    D.get_or_init(|| {
        let ff = ffmpeg()?;
        if let Some(node) = std::env::var_os("FILMCRAFT_VAAPI_DEVICE").and_then(|v| v.into_string().ok()).filter(|v| !v.is_empty()) {
            return vaapi_opens(ff, &node).then_some(node);
        }
        let found = render_nodes().into_iter().find(|n| vaapi_opens(ff, n));
        log::info!("VA-API device: {found:?}");
        found
    })
    .as_deref()
}

/// What the VA-API driver of [`vaapi_device`] lists (`vainfo` lines such as
/// `VAProfileHEVCMain10 : VAEntrypointVLD`), when `vainfo` is installed. `None`: unknown (no
/// `vainfo`), so callers try and fall back on failure.
pub fn vaapi_profiles() -> Option<&'static [String]> {
    static P: OnceLock<Option<Vec<String>>> = OnceLock::new();
    P.get_or_init(|| {
        let node = vaapi_device()?;
        let out = Command::new("vainfo").args(["--display", "drm", "--device", node]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("VAProfile") && l.contains("VAEntrypoint"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        Some(lines)
    })
    .as_deref()
}

/// Whether the driver supports `profile` (e.g. `VAProfileHEVCMain10`) with `entrypoint`
/// (`VAEntrypointVLD` decode, `VAEntrypointEncSlice` encode). Unknown (no `vainfo`) counts as yes.
pub fn vaapi_supports(profile: &str, entrypoint: &str) -> bool {
    match vaapi_profiles() {
        None => true,
        Some(lines) => lines.iter().any(|l| l.split(' ').next() == Some(profile) && l.contains(entrypoint)),
    }
}

/// The tail of a child's stderr, collected on a thread (so a chatty ffmpeg never blocks on a full
/// pipe).
#[derive(Clone, Default)]
pub struct StderrTail(Arc<Mutex<VecDeque<u8>>>);

impl StderrTail {
    /// Drain `r` into the tail until EOF.
    pub fn collect(r: impl Read + Send + 'static) -> Self {
        let tail = StderrTail::default();
        let t = tail.clone();
        let spawned = std::thread::Builder::new().name("ffmpeg-stderr".into()).spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut r = r;
                let mut buf = [0u8; 1024];
                while let Ok(n) = r.read(&mut buf) {
                    let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
                    let mut q = t.0.lock().unwrap_or_else(PoisonError::into_inner);
                    q.extend(chunk);
                    while q.len() > STDERR_KEEP {
                        q.pop_front();
                    }
                }
            }));
        });
        if let Err(e) = spawned {
            log::warn!("cannot start the ffmpeg stderr reader: {e}");
        }
        tail
    }

    /// The last lines ffmpeg wrote, trimmed.
    pub fn text(&self) -> String {
        let q = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes: Vec<u8> = q.iter().copied().collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
}

/// Length-prefixed NAL units (`avcC` / `hvcC` samples) as an Annex B byte stream (4-byte start
/// codes), appended to `out`. Stops at the first malformed length prefix.
pub fn to_annexb(sample: &[u8], length_size: usize, out: &mut Vec<u8>) {
    if !(1..=4).contains(&length_size) {
        return;
    }
    let mut pos = 0usize;
    while let Some(pre) = sample.get(pos..pos.saturating_add(length_size)) {
        let len = pre.iter().fold(0usize, |a, &b| (a << 8) | b as usize);
        pos += length_size;
        let Some(nal) = sample.get(pos..pos.saturating_add(len)) else { break };
        pos += len;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
}

/// Annex B byte stream → NAL units (start codes removed, trailing zero bytes of the stream kept
/// with the last unit as the spec allows).
pub fn annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 <= data.len() {
        if data.get(i..i + 3) == Some(&[0, 0, 1]) {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut end = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
        // a 4-byte start code's leading zero belongs to it, not to this unit
        while end > s && k + 1 < starts.len() && data.get(end - 1) == Some(&0) {
            end -= 1;
        }
        if let Some(n) = data.get(s..end).filter(|n| !n.is_empty()) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb_round_trip() {
        let sample = [0, 0, 0, 3, 0x40, 1, 7, 0, 0, 0, 2, 0x42, 1];
        let mut b = Vec::new();
        to_annexb(&sample, 4, &mut b);
        assert_eq!(b, [0, 0, 0, 1, 0x40, 1, 7, 0, 0, 0, 1, 0x42, 1]);
        assert_eq!(annexb_nals(&b), vec![&[0x40, 1, 7][..], &[0x42, 1][..]]);
    }

    #[test]
    fn hostile_input_is_bounded() {
        let mut b = Vec::new();
        to_annexb(&[0xff, 0xff, 0xff, 0xff, 1], 4, &mut b);
        assert!(b.is_empty());
        to_annexb(&[0, 0, 0, 1, 2], 0, &mut b);
        to_annexb(&[0, 0, 0, 1, 2], 9, &mut b);
        assert!(b.is_empty());
        assert!(annexb_nals(&[]).is_empty());
        assert!(annexb_nals(&[0, 0, 1]).is_empty());
        assert_eq!(annexb_nals(&[0, 0, 1, 5, 0, 0, 1]), vec![&[5][..]]);
    }
}
