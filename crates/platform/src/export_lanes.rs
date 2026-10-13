//! The GPUs of a two-GPU HEVC export (fork 6E.3): this machine's side of
//! [`filmcraft_export::lanes::LaneDevices`]. The primary lane is the process's VA-API displays
//! (AMD here); the second is another render node whose driver encodes HEVC (Intel here).

use std::sync::Arc;

use filmcraft_export::lanes::{LaneDevices, LaneEncoders};
use filmcraft_export::{ExportError, ExportSettings, GpuFrames, Result, VideoEncoder};
use filmcraft_time::FrameRate;

use crate::hevc_export::{HevcEncoder, LaneTarget};
use crate::va::Display;
use crate::va_sys as sys;

/// Register the two-GPU export devices (once; harmless where there is one GPU: `second_node`
/// then finds none and every export stays single).
pub fn register() {
    filmcraft_export::lanes::register_lane_devices(Box::new(VaLanes));
}

/// A render node other than the process's encode device whose VA-API driver encodes HEVC (Main
/// or Main 10): `FILMCRAFT_EXPORT_SECOND_DEVICE` when set (tests), else the first such node.
pub fn second_encode_node() -> Option<String> {
    let primary = Display::for_encode().ok()?.node.clone();
    let candidates = match std::env::var("FILMCRAFT_EXPORT_SECOND_DEVICE").ok().filter(|v| !v.is_empty()) {
        Some(n) => vec![n],
        None => crate::ffmpeg::render_nodes(),
    };
    candidates.into_iter().filter(|n| *n != primary).find(|n| {
        Display::for_node(n).is_ok_and(|d| {
            d.supports(sys::VAProfile_VAProfileHEVCMain, sys::VAEntrypoint_VAEntrypointEncSlice)
                || d.supports(sys::VAProfile_VAProfileHEVCMain10, sys::VAEntrypoint_VAEntrypointEncSlice)
        })
    })
}

fn display(node: Option<&str>) -> std::result::Result<Arc<Display>, String> {
    match node {
        Some(n) => Display::for_node(n),
        None => Display::for_encode(),
    }
}

struct VaLanes;

impl LaneDevices for VaLanes {
    fn second_node(&self) -> Option<String> {
        second_encode_node()
    }

    fn name(&self, node: Option<&str>) -> String {
        let node = match node {
            Some(n) => n.to_string(),
            None => match Display::for_encode() {
                Ok(d) => d.node.clone(),
                Err(_) => return "GPU".into(),
            },
        };
        match crate::gpu_export::pci_ids(&node).map(|(v, _)| v) {
            Some(0x1002) => "AMD".into(),
            Some(0x8086) => "Intel".into(),
            Some(0x10de) => "NVIDIA".into(),
            _ => "GPU".into(),
        }
    }

    fn frames_on(&self, node: &str) -> Option<Box<dyn GpuFrames>> {
        crate::gpu_export::frames_on(node)
    }

    fn bind_thread(&self, node: Option<&str>) {
        filmcraft_codecs::hw::set_thread_decode_node(node.map(str::to_string));
    }

    fn decodes_422(&self, node: Option<&str>) -> bool {
        let d = match node {
            Some(n) => Display::for_node(n),
            None => Display::get(),
        };
        d.is_ok_and(|d| d.supports(sys::VAProfile_VAProfileHEVCMain422_10, sys::VAEntrypoint_VAEntrypointVLD))
    }

    fn encoders(&self, w: u32, h: u32, rate: FrameRate, settings: &ExportSettings, nodes: &[Option<String>]) -> Option<Arc<dyn LaneEncoders>> {
        if settings.hevc_chroma_422 || nodes.iter().any(|n| display(n.as_deref()).is_err()) {
            return None;
        }
        let stream = HevcEncoder::lane_stream(w, h, rate, settings, nodes);
        Some(Arc::new(VaLaneEncoders { w, h, rate, settings: settings.clone(), nodes: nodes.to_vec(), stream }))
    }
}

struct VaLaneEncoders {
    w: u32,
    h: u32,
    rate: FrameRate,
    settings: ExportSettings,
    nodes: Vec<Option<String>>,
    /// The stream every lane can write (`None`: the drivers share none, so in band).
    stream: Option<filmcraft_hevc::write::StreamParams>,
}

impl LaneEncoders for VaLaneEncoders {
    fn shared_stream(&self) -> bool {
        self.stream.is_some()
    }

    fn make(&self, lane: usize, in_band: bool) -> Result<Box<dyn VideoEncoder>> {
        let node = self.nodes.get(lane).ok_or_else(|| ExportError::Encode(format!("no lane {lane}")))?.clone();
        // in band, each driver writes its own stream (its own parameter sets in every IDR)
        let in_band = in_band || self.stream.is_none();
        let target = LaneTarget { node, stream: if in_band { None } else { self.stream.clone() }, in_band };
        HevcEncoder::for_lane(self.w, self.h, self.rate, &self.settings, target).map(|e| Box::new(e) as Box<dyn VideoEncoder>)
    }
}
