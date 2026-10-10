//! The H.265 / HEVC delivery presets (fork Phase 5.4) exported for real with the platform's HEVC
//! encoder registered, as the desktop app does: ffprobe (an external test oracle only) checks
//! container, codec, profile, pixel format, frame size, the colour description (primaries,
//! transfer, matrix, range) and the audio format; mpv decodes every file when installed. Skips
//! without ffprobe or an HEVC encoder.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

use filmcraft_engine::Session;
use serde_json::{Value, json};

fn tool(name: &str) -> Option<PathBuf> {
    let ok = Command::new(name).arg("-version").output().is_ok_and(|o| o.status.success());
    ok.then(|| PathBuf::from(name))
}

/// (preset, size, container, audio codec)
const PRESETS: [(&str, (u64, u64), &str, &str); 5] = [
    ("YouTube 4K HEVC", (3840, 2160), "mp4", "aac"),
    ("6.2K Master HEVC", (6240, 4160), "mov", "pcm_s24le"),
    ("Archive HEVC (x265, quality)", (1920, 1080), "mov", "pcm_s24le"),
    ("Vertical 2160×3840 HEVC", (2160, 3840), "mp4", "aac"),
    ("H.265 / HEVC 2160p 4K 10-bit", (3840, 2160), "mp4", "aac"),
];

#[test]
fn hevc_presets_export_and_ffprobe_confirms() {
    let Some(ffprobe) = tool("ffprobe") else {
        eprintln!("SKIPPED: no ffprobe");
        return;
    };
    if !filmcraft_platform::hevc_export::register() {
        eprintln!("SKIPPED: no HEVC encoder");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/export-tests").join(format!("hevc-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    for (k, (name, (w, h), container, audio)) in PRESETS.iter().enumerate() {
        let path = dir.join(format!("p{k}")).to_string_lossy().to_string();
        let params = json!({"preset": name, "path": path, "range": "custom", "startSeconds": 1.0, "endSeconds": 1.25, "wait": true});
        let r = s.execute("file.exportMedia", params).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = r["path"].as_str().unwrap().to_string();
        let o = Command::new(&ffprobe).args(["-v", "error", "-of", "json", "-show_format", "-show_streams", &out]).output().unwrap();
        assert!(o.status.success(), "{name}: {}", String::from_utf8_lossy(&o.stderr));
        let j: Value = serde_json::from_slice(&o.stdout).unwrap();
        let streams = j["streams"].as_array().unwrap();
        let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap_or_else(|| panic!("{name}: no video"));
        let a = streams.iter().find(|s| s["codec_type"] == "audio").unwrap_or_else(|| panic!("{name}: no audio"));
        assert!(j["format"]["format_name"].as_str().unwrap().contains(container), "{name}: {}", j["format"]["format_name"]);
        assert_eq!(v["codec_name"], "hevc", "{name}");
        assert_eq!(v["profile"], "Main 10", "{name}");
        assert_eq!(v["pix_fmt"], "yuv420p10le", "{name}");
        assert_eq!((v["width"].as_u64().unwrap(), v["height"].as_u64().unwrap()), (*w, *h), "{name}");
        assert_eq!(v["r_frame_rate"], "24000/1001", "{name}");
        for (key, want) in [("color_primaries", "bt709"), ("color_transfer", "bt709"), ("color_space", "bt709"), ("color_range", "tv")] {
            assert_eq!(v[key], want, "{name}: {key}");
        }
        assert_eq!(a["codec_name"], *audio, "{name}");
        assert_eq!((a["sample_rate"].as_str(), a["channels"].as_u64()), (Some("48000"), Some(2)), "{name}");
        // every frame decodes in mpv (a player, not just a demuxer)
        if tool("mpv").is_some() {
            let m = Command::new("mpv").args(["--no-config", "--vo=null", "--ao=null", "--untimed", "--msg-level=all=error", &out]).output().unwrap();
            assert!(m.status.success() && m.stderr.is_empty(), "{name}: mpv: {}", String::from_utf8_lossy(&m.stderr));
        }
    }
}
