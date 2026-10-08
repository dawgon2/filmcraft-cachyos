//! Zero-copy hardware-decoded frames (Linux, VA-API + Vulkan DMA-BUF import, ADR 0002): a frame
//! decoded in process and drawn in place by the GPU compositor gives exactly the image of the same
//! picture read back to memory and uploaded. Skipped without VA-API, a Vulkan adapter with the
//! import extensions, or the fixtures (`target/fixtures/platform/hevc_main*.mp4`, made by the
//! platform tests).
#![cfg(target_os = "linux")]

use std::sync::Arc;

use eframe::wgpu;

use filmcraft_codecs::VideoDecoder;
use filmcraft_frame::{PixelData, VideoFrame};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let opts = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = pollster::block_on(instance.request_adapter(&opts)).ok()?;
    let desc = wgpu::DeviceDescriptor { required_features: adapter.features() & filmcraft_gpu::wanted_features(), ..Default::default() };
    match filmcraft_gpu::interop::create_device(&adapter, &desc)? {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("interop device: {e}");
            None
        }
    }
}

#[test]
fn imported_frames_draw_exactly_as_read_back_frames() {
    let Some((dev, q)) = device() else {
        eprintln!("no Vulkan adapter with DMA-BUF import; skipped");
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/platform");
    let mut checked = 0;
    for name in ["hevc_main10.mp4", "hevc_main.mp4"] {
        let Ok(bytes) = std::fs::read(dir.join(name)) else {
            eprintln!("{name}: fixture missing; skipped");
            continue;
        };
        let file = filmcraft_isobmff::open(bytes.clone()).expect("mp4");
        let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video).expect("video");
        let track = &file.tracks[t];
        let entry = track.entries[0].clone();
        let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(&entry).expect("hvcC").expect("info");
        let mut dec = match filmcraft_platform::va_hevc::NativeVaDecoder::new(info) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{name}: no in-process VA-API ({e}); skipped");
                return;
            }
        };
        let mut frames = Vec::new();
        for i in 0..track.samples.len().min(12) {
            let s = file.read_sample(&bytes, t, i).expect("sample");
            frames.extend(dec.decode(&s, track.samples[i].pts).expect("decode"));
        }
        frames.extend(dec.flush());
        assert!(!frames.is_empty(), "{name}: no frames");
        let mut c = filmcraft_gpu::GpuCompositor::new(&dev, &q);
        for f in &frames {
            assert!(matches!(f.frame.data, PixelData::Gpu(_)), "{name}: the decoder hands out GPU frames");
            let (w, h) = (f.frame.width, f.frame.height);
            let fx = filmcraft_render::plan::LayerFx { size: (w, h), decimation: 1, ops: Vec::new() };
            let gpu = Arc::new(f.frame.clone());
            let (_, _, in_place) = c.effect_image(&gpu, &fx).expect("draw in place");
            let PixelData::Gpu(s) = &f.frame.data else { unreachable!() };
            let read = VideoFrame { data: s.download(w, h).expect("read back"), ..f.frame.clone() };
            let (_, _, uploaded) = c.effect_image(&read, &fx).expect("draw read-back");
            let worst = in_place.iter().zip(&uploaded).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(worst < 1e-6, "{name} pts {}: in place differs from read back by {worst}", f.pts);
            checked += 1;
        }
        assert!(c.gpu_frames_in_place > 0, "{name}: frames were drawn in place, not read back ({} read back)", c.gpu_frames_read_back);
        eprintln!("{name}: {} frames identical in place and read back", frames.len());
    }
    eprintln!("{checked} frames checked");
}
