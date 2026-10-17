//! Export settings beyond the basics: the Video / Audio / Multiplexer / Effects / Metadata
//! sections of Export mode, as plain serde data (every field has a default so older settings and
//! presets keep loading), plus the derived values (output size, rate, bitrate) and the estimated
//! file size.

use filmcraft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

use crate::{ExportSettings, Format};

/// How the sequence picture is fitted into a different output frame size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scaling {
    /// Whole picture visible, black bars where the aspect ratios differ.
    #[default]
    ScaleToFit,
    /// Frame filled, the picture cropped where the aspect ratios differ.
    ScaleToFill,
    /// Frame filled, the picture distorted.
    StretchToFill,
}

/// Field order of the encoded video. FilmCraft's encoders write progressive frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldOrder {
    #[default]
    Progressive,
    UpperFirst,
    LowerFirst,
}

impl FieldOrder {
    pub fn label(self) -> &'static str {
        match self {
            FieldOrder::Progressive => "Progressive",
            FieldOrder::UpperFirst => "Upper First",
            FieldOrder::LowerFirst => "Lower First",
        }
    }
}

/// H.264 profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum H264Profile {
    Baseline,
    Main,
    #[default]
    High,
}

impl H264Profile {
    pub fn label(self) -> &'static str {
        match self {
            H264Profile::Baseline => "Baseline",
            H264Profile::Main => "Main",
            H264Profile::High => "High",
        }
    }
}

/// Export ▸ Hardware encoding: whether H.264 may be encoded by the system's hardware video encoder
/// (VideoToolbox on macOS, NVENC on NVIDIA GPUs on Windows) instead of FilmCraft's own encoder. Off
/// by default: hardware encoders make different streams, and exports are otherwise byte-identical
/// from run to run and machine to machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HardwareEncoding {
    #[default]
    Off,
    /// Use the hardware encoder when the system has one and it takes the settings; otherwise the
    /// software encoder.
    Auto,
}

/// Whether the picture of an export may be composited on the GPU (`filmcraft-gpu`'s off-screen
/// compositor) instead of the CPU reference renderer. Auto = use the GPU when the app registered
/// a GPU frame renderer and the machine has an adapter; the CPU result is the fallback either
/// way. On an effects-heavy edit it exports 2.4× (1080p) to 3.2× (4K) faster, and a single plain
/// clip takes the same time (`docs/performance.md`). The GPU matches the CPU within the
/// compositor's parity tolerance, not bit for bit, so an export's bytes can depend on the
/// machine's GPU. Off = the CPU reference renderer, byte-reproducible everywhere; Off is the
/// default (opt-in per export) until GPU export has been measured on Windows and macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuRendering {
    Auto,
    #[default]
    Off,
}

/// Bitrate encoding of bitrate-driven codecs (H.264).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BitrateMode {
    /// Constant bitrate.
    Cbr,
    /// Variable bitrate, one pass.
    #[default]
    Vbr1Pass,
    /// Variable bitrate, two passes (the picture is rendered and analysed first).
    Vbr2Pass,
    /// Constant quality: every frame is coded at [`ExportSettings::crf`] and the bitrate follows the
    /// picture (H.264, built-in encoder only; H.265 through the fork's encoders: x265 CRF or
    /// VA-API constant QP).
    Crf,
}

impl BitrateMode {
    pub fn label(self) -> &'static str {
        match self {
            BitrateMode::Cbr => "CBR",
            BitrateMode::Vbr1Pass => "VBR, 1 pass",
            BitrateMode::Vbr2Pass => "VBR, 2 pass",
            BitrateMode::Crf => "CRF (constant quality)",
        }
    }
}

/// The HEVC encoder an [`crate::Format::Hevc`] export runs (through the system ffmpeg).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HevcEncoder {
    /// VA-API hardware encoding when the GPU takes the size and profile, else libx265.
    #[default]
    Auto,
    /// VA-API hardware encoding only (`hevc_vaapi`).
    #[serde(alias = "hevc_vaapi", alias = "hardware")]
    Vaapi,
    /// Software encoding only (`libx265`).
    #[serde(alias = "libx265", alias = "software")]
    X265,
}

impl HevcEncoder {
    pub const ALL: [HevcEncoder; 3] = [HevcEncoder::Auto, HevcEncoder::Vaapi, HevcEncoder::X265];
    pub fn label(self) -> &'static str {
        match self {
            HevcEncoder::Auto => "Auto (hardware if available)",
            HevcEncoder::Vaapi => "Hardware (VA-API)",
            HevcEncoder::X265 => "Software (x265)",
        }
    }
    pub fn from_name(s: &str) -> Option<HevcEncoder> {
        Some(match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "auto" => HevcEncoder::Auto,
            "vaapi" | "hevcvaapi" | "hardware" | "hw" | "gpu" => HevcEncoder::Vaapi,
            "x265" | "libx265" | "software" | "sw" | "cpu" => HevcEncoder::X265,
            _ => return None,
        })
    }
}

/// How many GPUs encode one HEVC export (fork 6E.3, `docs/fork/DESIGN-6E3.md` §2.6): the range is
/// cut into chunks that the GPUs encode at once and that are joined in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportEngines {
    /// The primary GPU, plus the second one when the export qualifies (HEVC through VA-API, 4:2:0,
    /// a second encoder, enough memory) and has no effect stage on its frames (FORK-PLAN §1a.63:
    /// the second GPU joins only where it measurably helps at full precision), else one.
    #[default]
    Auto,
    /// The primary GPU only (as before 6E.3).
    Single,
    /// Every GPU that can encode; falls back to one, with the reason in the log, when the export
    /// does not qualify.
    Dual,
}

impl ExportEngines {
    pub const ALL: [ExportEngines; 3] = [ExportEngines::Auto, ExportEngines::Single, ExportEngines::Dual];
    pub fn label(self) -> &'static str {
        match self {
            ExportEngines::Auto => "Auto",
            ExportEngines::Single => "Primary GPU only",
            ExportEngines::Dual => "All GPUs",
        }
    }
    pub fn from_name(s: &str) -> Option<ExportEngines> {
        Some(match s.to_ascii_lowercase().replace([' ', '-', '_', '+'], "").as_str() {
            "auto" | "default" => ExportEngines::Auto,
            "single" | "one" | "primary" | "primarygpuonly" | "amdonly" => ExportEngines::Single,
            "dual" | "two" | "both" | "all" | "allgpus" | "amdintel" => ExportEngines::Dual,
            _ => return None,
        })
    }
}

/// Speed / quality trade-off of the hardware HEVC encoder (fork 6E.2): the VA-API quality level
/// in process, `-compression_level` through ffmpeg's `hevc_vaapi`. libx265 has its own preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HwEncodeSpeed {
    /// The driver's own default (nothing is sent).
    #[default]
    Default,
    /// The fastest level the driver offers.
    Speed,
    /// The middle of the driver's range.
    Balanced,
    /// The best quality the driver offers (level 1).
    Quality,
}

impl HwEncodeSpeed {
    pub const ALL: [HwEncodeSpeed; 4] = [HwEncodeSpeed::Default, HwEncodeSpeed::Speed, HwEncodeSpeed::Balanced, HwEncodeSpeed::Quality];
    pub fn label(self) -> &'static str {
        match self {
            HwEncodeSpeed::Default => "Driver default",
            HwEncodeSpeed::Speed => "Speed",
            HwEncodeSpeed::Balanced => "Balanced",
            HwEncodeSpeed::Quality => "Quality",
        }
    }
    pub fn from_name(s: &str) -> Option<HwEncodeSpeed> {
        Some(match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "default" | "driverdefault" | "auto" => HwEncodeSpeed::Default,
            "speed" | "fast" | "fastest" => HwEncodeSpeed::Speed,
            "balanced" | "medium" => HwEncodeSpeed::Balanced,
            "quality" | "best" | "slow" => HwEncodeSpeed::Quality,
            _ => return None,
        })
    }
    /// The quality level for a driver offering `range` levels (1 = best, `range` = fastest;
    /// `VAConfigAttribEncQualityRange`); `None`: send nothing (driver default, or one level only).
    pub fn level(self, range: u32) -> Option<u32> {
        if range <= 1 {
            return None;
        }
        match self {
            HwEncodeSpeed::Default => None,
            HwEncodeSpeed::Speed => Some(range),
            HwEncodeSpeed::Balanced => Some(range.div_ceil(2).max(1)),
            HwEncodeSpeed::Quality => Some(1),
        }
    }
}

/// Container of H.264 exports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Multiplexer {
    #[default]
    Mp4,
    /// QuickTime.
    Mov,
}

/// Video codec of an MXF export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MxfVideoCodec {
    /// Avid DNxHR (VC-3, SMPTE ST 2019-4 mapping).
    #[default]
    Dnxhr,
    /// Apple ProRes (RDD 44 mapping).
    ProRes,
    /// H.264 long GOP (ST 381-3 byte-stream mapping).
    H264,
}

impl MxfVideoCodec {
    pub const ALL: [MxfVideoCodec; 3] = [MxfVideoCodec::Dnxhr, MxfVideoCodec::ProRes, MxfVideoCodec::H264];
    pub fn label(self) -> &'static str {
        match self {
            MxfVideoCodec::Dnxhr => "Avid DNxHR",
            MxfVideoCodec::ProRes => "Apple ProRes",
            MxfVideoCodec::H264 => "H.264",
        }
    }
    /// The export format whose encoder this codec uses.
    pub fn encoder_format(self) -> Format {
        match self {
            MxfVideoCodec::Dnxhr => Format::DnxHr,
            MxfVideoCodec::ProRes => Format::ProRes,
            MxfVideoCodec::H264 => Format::H264,
        }
    }
}

/// Audio codec. `Auto` = AAC in MP4 and AAC Audio, PCM in QuickTime / WAV / AIFF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioCodec {
    #[default]
    Auto,
    Aac,
    Pcm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub codec: AudioCodec,
    /// Output sample rate (None = the sequence's).
    pub sample_rate: Option<u32>,
    /// 1 (mono), 2 (stereo) or 6 (5.1: L, R, C, LFE, Ls, Rs).
    pub channels: u32,
    /// AAC bitrate.
    pub bitrate_kbps: u32,
    /// PCM sample size: 16 or 24.
    pub bits: u16,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings { codec: AudioCodec::Auto, sample_rate: None, channels: 2, bitrate_kbps: 320, bits: 16 }
    }
}

/// Where an overlay sits in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    #[default]
    BottomCenter,
    BottomRight,
}

impl Placement {
    pub const ALL: [Placement; 9] = [
        Placement::TopLeft,
        Placement::TopCenter,
        Placement::TopRight,
        Placement::CenterLeft,
        Placement::Center,
        Placement::CenterRight,
        Placement::BottomLeft,
        Placement::BottomCenter,
        Placement::BottomRight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Placement::TopLeft => "Top Left",
            Placement::TopCenter => "Top Center",
            Placement::TopRight => "Top Right",
            Placement::CenterLeft => "Center Left",
            Placement::Center => "Center",
            Placement::CenterRight => "Center Right",
            Placement::BottomLeft => "Bottom Left",
            Placement::BottomCenter => "Bottom Center",
            Placement::BottomRight => "Bottom Right",
        }
    }
    /// (x, y) anchor fractions: 0 = left/top, 0.5 = centre, 1 = right/bottom.
    pub fn anchor(self) -> (f32, f32) {
        let i = Placement::ALL.iter().position(|p| *p == self).unwrap_or(7);
        ((i % 3) as f32 * 0.5, (i / 3) as f32 * 0.5)
    }
}

/// Export ▸ Effects ▸ Image Overlay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageOverlay {
    pub enabled: bool,
    /// PNG / JPEG file.
    pub path: String,
    pub placement: Placement,
    /// Offset in output pixels.
    pub offset: (f32, f32),
    /// Width as a percentage of the output width (the image keeps its aspect ratio).
    pub size_percent: f32,
    /// 0–100.
    pub opacity: f32,
}

impl Default for ImageOverlay {
    fn default() -> Self {
        ImageOverlay { enabled: false, path: String::new(), placement: Placement::TopRight, offset: (0.0, 0.0), size_percent: 15.0, opacity: 100.0 }
    }
}

/// Export ▸ Effects ▸ Name Overlay / Timecode Overlay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextOverlay {
    pub enabled: bool,
    /// Name overlay: the text (prefix + suffix in one). Timecode overlay: an optional prefix.
    pub text: String,
    pub placement: Placement,
    pub offset: (f32, f32),
    /// Text height as a percentage of the output height.
    pub size_percent: f32,
    /// 0–100 (the text and its backing box).
    pub opacity: f32,
}

impl Default for TextOverlay {
    fn default() -> Self {
        TextOverlay { enabled: false, text: String::new(), placement: Placement::BottomCenter, offset: (0.0, 0.0), size_percent: 5.0, opacity: 100.0 }
    }
}

/// Export ▸ Effects ▸ Video Limiter: keeps luma inside a legal range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoLimiter {
    pub enabled: bool,
    /// Luma floor and ceiling in percent of the video range (0 % = black, 100 % = white).
    pub min_percent: f32,
    pub max_percent: f32,
}

impl Default for VideoLimiter {
    fn default() -> Self {
        VideoLimiter { enabled: false, min_percent: 0.0, max_percent: 100.0 }
    }
}

/// Export ▸ Effects ▸ Loudness Normalization (ITU-R BS.1770-4 integrated loudness).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoudnessNormalization {
    pub enabled: bool,
    /// Target integrated loudness (LUFS).
    pub target_lufs: f64,
    /// True-peak ceiling (dBTP) enforced by a look-ahead limiter.
    pub true_peak_dbtp: f64,
}

impl Default for LoudnessNormalization {
    fn default() -> Self {
        LoudnessNormalization { enabled: false, target_lufs: -23.0, true_peak_dbtp: -2.0 }
    }
}

/// Export ▸ Effects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportEffects {
    pub image_overlay: ImageOverlay,
    pub name_overlay: TextOverlay,
    pub timecode_overlay: TextOverlay,
    pub video_limiter: VideoLimiter,
    pub loudness: LoudnessNormalization,
}

impl ExportEffects {
    pub fn any_video(&self) -> bool {
        self.image_overlay.enabled || self.name_overlay.enabled || self.timecode_overlay.enabled || self.video_limiter.enabled
    }
}

/// Export ▸ Metadata (written to the MP4 / QuickTime `udta`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportMetadata {
    pub title: String,
    pub creator: String,
    pub copyright: String,
    pub description: String,
    pub comment: String,
}

impl ExportMetadata {
    /// `udta` items (QuickTime `©xxx` keys) plus the encoder name.
    pub fn udta(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            [("©nam", &self.title), ("©ART", &self.creator), ("©cpy", &self.copyright), ("©des", &self.description), ("©cmt", &self.comment)]
                .into_iter()
                .filter(|(_, s)| !s.trim().is_empty())
                .map(|(k, s)| (k.to_string(), s.trim().to_string()))
                .collect();
        v.push(("©too".into(), format!("Revolution {}", env!("CARGO_PKG_VERSION"))));
        v
    }
}

/// Values derived from settings + sequence at export time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    /// Output frame size (even for chroma-subsampled codecs).
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub sample_rate: u32,
    pub channels: u32,
    /// Video target / max bitrate (kbps) for bitrate-driven codecs.
    pub target_kbps: u32,
    pub max_kbps: u32,
    /// Keyframe distance (frames).
    pub keyint: u32,
}

impl ExportSettings {
    /// The format whose video encoder runs: the MXF video codec for MXF exports, else the format.
    pub fn video_format(&self) -> Format {
        if self.format.is_mxf() { self.mxf_video_codec.encoder_format() } else { self.format }
    }

    /// Whether the output can carry an alpha channel ([`ExportSettings::alpha`]): PNG, TIFF,
    /// Targa and OpenEXR sequences and ProRes 4444 / 4444 XQ QuickTime movies.
    pub fn supports_alpha(&self) -> bool {
        match self.format {
            Format::PngSequence | Format::TiffSequence | Format::TgaSequence | Format::ExrSequence => true,
            Format::ProRes => crate::prores_profile(&self.prores_profile).chroma() == filmcraft_prores::ChromaFormat::Yuv444,
            _ => false,
        }
    }

    /// Whether the exported frames keep their alpha channel instead of being flattened over black.
    pub fn keeps_alpha(&self) -> bool {
        self.alpha && self.supports_alpha()
    }

    /// Whether the output is a numbered image sequence.
    pub fn is_image_sequence(&self) -> bool {
        self.format.is_image_sequence()
    }

    /// Output size, rate, audio format and bitrate for a sequence of `seq_w`×`seq_h` at `seq_rate`.
    pub fn resolve(&self, seq_w: u32, seq_h: u32, seq_rate: FrameRate, seq_sr: u32) -> Resolved {
        let (mut w, mut h) = match self.frame_size {
            Some((w, h)) => (w.max(2), h.max(2)),
            None => (((seq_w as f32 * self.scale).round() as u32).max(2), ((seq_h as f32 * self.scale).round() as u32).max(2)),
        };
        if !self.is_image_sequence() && self.format != Format::Gif {
            w &= !1;
            h &= !1;
        }
        let rate = self.frame_rate.filter(|r| r.num > 0 && r.den > 0).unwrap_or(seq_rate);
        let fps = rate.num as f64 / rate.den as f64;
        let target = match self.adaptive_bitrate {
            Some(bpp) if bpp > 0.0 => ((w as f64 * h as f64 * fps * bpp as f64) / 1000.0).round() as u32,
            _ => self.bitrate_kbps,
        }
        .max(100);
        let max = match self.bitrate_mode {
            BitrateMode::Cbr => target,
            _ => self.max_bitrate_kbps.filter(|m| *m >= target).unwrap_or_else(|| (u64::from(target) * 3 / 2).min(u64::from(u32::MAX)) as u32),
        };
        let keyint = self.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (fps * 2.0).round().max(1.0) as u32);
        let channels = match self.audio.channels {
            0 | 1 => 1,
            6.. => 6,
            _ => 2,
        };
        Resolved { width: w, height: h, rate, sample_rate: self.audio.sample_rate.unwrap_or(seq_sr), channels, target_kbps: target, max_kbps: max, keyint }
    }

    /// The audio codec actually used.
    pub fn audio_codec(&self) -> AudioCodec {
        match (self.format, self.audio.codec) {
            (Format::Wav | Format::Aiff | Format::MxfOp1a | Format::MxfOpAtom, _) => AudioCodec::Pcm,
            (Format::Aac, _) => AudioCodec::Aac,
            (f, _) if f.is_mp4_with(self.multiplexer) => AudioCodec::Aac,
            (_, AudioCodec::Auto) if self.format.is_mp4_video() => AudioCodec::Aac,
            (_, AudioCodec::Auto) => AudioCodec::Pcm,
            (_, c) => c,
        }
    }

    /// Whether the format carries audio at all.
    pub fn has_audio(&self) -> bool {
        self.include_audio && !self.is_image_sequence() && self.format != Format::Gif
    }

    /// Whether the format carries video.
    pub fn has_video(&self) -> bool {
        !self.format.is_audio_only()
    }

    /// File extension of the output.
    pub fn extension(&self) -> &'static str {
        if self.format.is_h26x() && self.multiplexer == Multiplexer::Mov { "mov" } else { self.format.extension() }
    }

    /// Estimated output size in bytes for `duration` of a sequence (`seq_w`×`seq_h` at `seq_rate`).
    pub fn estimate_bytes(&self, seq_w: u32, seq_h: u32, seq_rate: FrameRate, seq_sr: u32, duration: Tick) -> u64 {
        let r = self.resolve(seq_w, seq_h, seq_rate, seq_sr);
        let secs = duration.seconds().max(0.0);
        let fps = r.rate.num as f64 / r.rate.den as f64;
        let px = r.width as f64 * r.height as f64;
        let video_bps = match self.video_format() {
            Format::Aac => 0.0,
            // constant quality: roughly 0.08 bits per pixel at CRF 23, halving every 6 steps
            Format::H264 | Format::Hevc if self.bitrate_mode == BitrateMode::Crf => {
                let crf = if self.crf.is_finite() { f64::from(self.crf.clamp(0.0, 51.0)) } else { f64::from(crate::DEFAULT_CRF) };
                px * fps * 0.08 * 2f64.powf((23.0 - crf) / 6.0)
            }
            Format::H264 | Format::Hevc | Format::Av1 => r.target_kbps as f64 * 1000.0,
            Format::ProRes => crate::prores_profile(&self.prores_profile).nominal_mbps_1080p30() * 1e6 * px / (1920.0 * 1080.0) * fps / 29.97,
            Format::DnxHr => {
                // nominal 1080p29.97 data rates of the DNxHR profiles (Mb/s)
                let mbps = match self.dnx_profile.to_ascii_lowercase().as_str() {
                    "lb" => 45.0,
                    "sq" => 145.0,
                    "hqx" => 220.0,
                    _ => 220.0,
                };
                mbps * 1e6 * px / (1920.0 * 1080.0) * fps / 29.97
            }
            Format::Apv => {
                let bpp = match crate::apv_profile(&self.apv_profile) {
                    filmcraft_apv::Profile::P422_12 => 3.6,
                    filmcraft_apv::Profile::P444_10 => 4.2,
                    filmcraft_apv::Profile::P444_12 | filmcraft_apv::Profile::P4444_10 | filmcraft_apv::Profile::P4444_12 => 5.0,
                    _ => 3.0,
                };
                px * fps * bpp
            }
            Format::Mjpeg | Format::JpegSequence => px * 8.0 * (0.4 + self.quality as f64 / 100.0 * 2.0) * fps / 8.0,
            Format::PngSequence => px * 4.0 * 0.45 * 8.0 * fps,
            Format::TiffSequence | Format::BmpSequence => px * if self.format == Format::BmpSequence { 3.0 } else { 4.0 } * 8.0 * fps,
            Format::TgaSequence => px * if self.keeps_alpha() { 4.0 } else { 3.0 } * 8.0 * fps,
            // 10-bit RGB in one 32-bit word per pixel
            Format::DpxSequence => px * 4.0 * 8.0 * fps,
            // two bytes per half-float channel
            Format::ExrSequence => px * if self.keeps_alpha() { 8.0 } else { 6.0 } * 8.0 * fps,
            Format::Gif => px * 0.6 * 8.0 * fps,
            Format::Wav | Format::Aiff | Format::MxfOp1a | Format::MxfOpAtom => 0.0,
        };
        let audio_bps = if self.has_audio() || self.format.is_audio_only() {
            match self.audio_codec() {
                AudioCodec::Aac => self.audio.bitrate_kbps as f64 * 1000.0,
                _ => r.sample_rate as f64 * r.channels as f64 * if self.audio.bits >= 24 { 24.0 } else { 16.0 },
            }
        } else {
            0.0
        };
        ((video_bps + audio_bps) * secs / 8.0).round() as u64
    }
}

/// The Export-mode Summary panel: one line each for the output video, audio and format.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub format: String,
    pub video: String,
    pub audio: String,
    pub estimated_bytes: u64,
    pub estimated_size: String,
}

impl ExportSettings {
    /// Summary lines for a sequence of `seq_w`×`seq_h` at `seq_rate` / `seq_sr` exporting `duration`.
    pub fn summary(&self, seq_w: u32, seq_h: u32, seq_rate: FrameRate, seq_sr: u32, duration: Tick) -> Summary {
        let r = self.resolve(seq_w, seq_h, seq_rate, seq_sr);
        let mbps = |k: u32| format!("{:.2} Mbps", k as f64 / 1000.0);
        let video = if !self.has_video() {
            "No video".to_string()
        } else {
            let par = self.pixel_aspect.map(|(n, d)| format!("{:.2}", n.max(1) as f64 / d.max(1) as f64)).unwrap_or_else(|| "1.0".into());
            let mut v = format!("{}x{} ({par}), {} fps, {}", r.width, r.height, r.rate.label(), self.field_order.label());
            match self.video_format() {
                Format::H264 => {
                    v += &format!(
                        ", {} {}, {}",
                        self.h264_profile.label(),
                        self.h264_level.map(|l| format!("L{}.{}", l / 10, l % 10)).unwrap_or_else(|| "auto level".into()),
                        self.bitrate_mode.label()
                    );
                    v += &match self.bitrate_mode {
                        BitrateMode::Cbr => format!(", {}", mbps(r.target_kbps)),
                        BitrateMode::Crf => format!(" {}", self.crf),
                        _ => format!(", Target {}, Max {}", mbps(r.target_kbps), mbps(r.max_kbps)),
                    };
                    v += &format!(", keyframe every {} frames", r.keyint);
                }
                Format::Hevc => {
                    // the fork's encoders (VA-API in process, ffmpeg VA-API / x265) take Main or Main 10
                    v += if self.hevc_bit_depth == 10 { ", HEVC Main 10" } else { ", HEVC Main" };
                    v += &match self.bitrate_mode {
                        BitrateMode::Crf => format!(", constant quality {}", if self.crf.is_finite() { self.crf.clamp(0.0, 51.0) } else { crate::DEFAULT_CRF }),
                        BitrateMode::Cbr => format!(", CBR, {}", mbps(r.target_kbps)),
                        _ => format!(", VBR, Target {}, Max {}", mbps(r.target_kbps), mbps(r.max_kbps)),
                    };
                    v += &format!(", keyframe every {} frames, {}", r.keyint, self.hevc_encoder.label());
                }
                Format::Av1 => {
                    v += &format!(", AV1 Main (hardware encoder), {}", self.bitrate_mode.label());
                    v += &match self.bitrate_mode {
                        BitrateMode::Cbr => format!(", {}", mbps(r.target_kbps)),
                        _ => format!(", Target {}, Max {}", mbps(r.target_kbps), mbps(r.max_kbps)),
                    };
                    v += &format!(", keyframe every {} frames", r.keyint);
                }
                Format::ProRes => {
                    use filmcraft_prores::Profile;
                    v += match crate::prores_profile(&self.prores_profile) {
                        Profile::Proxy => ", ProRes 422 Proxy",
                        Profile::Lt => ", ProRes 422 LT",
                        Profile::Standard => ", ProRes 422",
                        Profile::Hq => ", ProRes 422 HQ",
                        Profile::P4444 => ", ProRes 4444",
                        Profile::P4444Xq => ", ProRes 4444 XQ",
                    }
                }
                Format::DnxHr => v += &format!(", DNxHR {}", if self.dnx_profile.is_empty() { "HQ".into() } else { self.dnx_profile.to_ascii_uppercase() }),
                Format::Apv => v += &format!(", {}", crate::apv_profile(&self.apv_profile).name()),
                Format::Mjpeg | Format::JpegSequence => v += &format!(", quality {}", self.quality),
                Format::DpxSequence => v += ", 10-bit RGB",
                Format::ExrSequence => v += ", 16-bit half float, linear",
                _ => {}
            }
            v
        };
        let audio = if self.has_audio() || self.format.is_audio_only() {
            let ch = match r.channels {
                1 => "Mono",
                6 => "5.1",
                _ => "Stereo",
            };
            match self.audio_codec() {
                AudioCodec::Aac => format!("AAC, {} kbps, {} Hz, {ch}", self.audio.bitrate_kbps, r.sample_rate),
                _ => format!("Uncompressed {}-bit PCM, {} Hz, {ch}", if self.audio.bits >= 24 { 24 } else { 16 }, r.sample_rate),
            }
        } else {
            "No audio".to_string()
        };
        let container = match self.format {
            Format::H264 | Format::Hevc if self.multiplexer == Multiplexer::Mov => "QuickTime",
            Format::H264 | Format::Hevc | Format::Av1 => "MP4",
            Format::ProRes | Format::DnxHr | Format::Apv | Format::Mjpeg => "QuickTime",
            Format::MxfOp1a | Format::MxfOpAtom => self.mxf_video_codec.label(),
            f if f.is_image_sequence() => "Image sequence",
            _ => "",
        };
        let format = if container.is_empty() { self.format.label().to_string() } else { format!("{} ({container})", self.format.label()) };
        let estimated_bytes = self.estimate_bytes(seq_w, seq_h, seq_rate, seq_sr, duration);
        Summary { format, video, audio, estimated_bytes, estimated_size: format_bytes(estimated_bytes) }
    }
}

/// A time left, rounded up to the second: `45 s`, `2:05`, `1:02:05`.
pub fn format_eta(d: std::time::Duration) -> String {
    let s = d.as_secs().saturating_add(u64::from(d.subsec_nanos() > 0));
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60),
    }
}

/// `1.2 GB`, `350 MB`, `12 KB`.
pub fn format_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

#[cfg(test)]
mod hw_speed_tests {
    use super::HwEncodeSpeed;

    #[test]
    fn levels_stay_inside_the_driver_range() {
        // one level (or none reported): nothing is sent
        for r in [0, 1] {
            assert!(HwEncodeSpeed::ALL.iter().all(|s| s.level(r).is_none()));
        }
        for r in [2u32, 3, 7, u32::MAX] {
            assert_eq!(HwEncodeSpeed::Default.level(r), None);
            assert_eq!(HwEncodeSpeed::Quality.level(r), Some(1));
            assert_eq!(HwEncodeSpeed::Speed.level(r), Some(r));
            let b = HwEncodeSpeed::Balanced.level(r).unwrap_or(0);
            assert!((1..=r).contains(&b), "{r}: {b}");
        }
        assert_eq!(HwEncodeSpeed::Balanced.level(7), Some(4));
    }

    #[test]
    fn names_parse_and_round_trip() {
        assert_eq!(HwEncodeSpeed::from_name("Speed"), Some(HwEncodeSpeed::Speed));
        assert_eq!(HwEncodeSpeed::from_name("driver default"), Some(HwEncodeSpeed::Default));
        assert_eq!(HwEncodeSpeed::from_name("nope"), None);
        for s in HwEncodeSpeed::ALL {
            let j = serde_json::to_string(&s).unwrap_or_default();
            assert_eq!(serde_json::from_str::<HwEncodeSpeed>(&j).ok(), Some(s));
        }
        // settings written before 6E.2 read as the driver default
        let old: crate::ExportSettings = serde_json::from_str("{}").unwrap_or_default();
        assert_eq!(old.hw_encode_speed, HwEncodeSpeed::Default);
    }
}

#[cfg(test)]
mod export_engines_tests {
    use super::ExportEngines;

    #[test]
    fn names_round_trip_and_old_settings_read_as_auto() {
        for e in ExportEngines::ALL {
            assert_eq!(ExportEngines::from_name(e.label()), Some(e), "{e:?}");
            let json = serde_json::to_string(&e).unwrap_or_default();
            assert_eq!(serde_json::from_str::<ExportEngines>(&json).ok(), Some(e));
        }
        assert_eq!(ExportEngines::from_name("AMD + Intel"), Some(ExportEngines::Dual));
        assert_eq!(ExportEngines::from_name("amd-only"), Some(ExportEngines::Single));
        assert_eq!(ExportEngines::from_name(""), None);
        assert_eq!(ExportEngines::from_name("three"), None);
        // settings written before 6E.3
        let old: crate::ExportSettings = serde_json::from_str("{}").unwrap_or_default();
        assert_eq!(old.export_engines, ExportEngines::Auto);
    }
}
