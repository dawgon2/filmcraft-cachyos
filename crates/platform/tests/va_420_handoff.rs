//! 4:2:0 HEVC decoded on another GPU's media engine and handed to the compositing GPU (fork
//! U0.2.3b d, `hw::set_thread_decode_handoff`, diagnostic `FILMCRAFT_DECODE_NODE`): as the 4:2:2 hand-off (`va_422_handoff`), the
//! decoding GPU copies each picture into a linear NV12 / P010 surface (over memory the compositor
//! can import), exports it as a DMA-BUF, and the compositing GPU draws it. Its samples are the
//! reference decoder's (read back by mapping the linear copy) and it renders exactly as the same
//! picture read back and uploaded. Skips (with SKIPPED) without a second render node whose driver
//! decodes HEVC, a GPU device with DMA-BUF import, or the fixtures.
#![cfg(target_os = "linux")]

use filmcraft_frame::{GpuFormat, PixelData, VideoFrame};
use filmcraft_platform::va::Display;
use filmcraft_platform::vaapi::decode_annexb_with;

/// A render node other than the process's VA-API device with a VA-API driver.
fn other_decode_node() -> Option<String> {
    let primary = Display::get().ok()?.node.clone();
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|e| e.path().to_string_lossy().to_string())
        .filter(|p| p.starts_with("/dev/dri/renderD") && *p != primary)
        .collect();
    nodes.sort();
    nodes.into_iter().find(|n| Display::for_node(n).is_ok())
}

/// The compositing device (the high-performance adapter: AMD here) with DMA-BUF import, made as
/// the app makes its own (as `va_422_handoff`).
fn gpu() -> Option<filmcraft_gpu::GpuCompositor> {
    let instance = wgpu::Instance::default();
    let opts = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = pollster::block_on(instance.request_adapter(&opts)).ok()?;
    let desc = wgpu::DeviceDescriptor {
        required_features: adapter.features() & filmcraft_gpu::wanted_features(),
        required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() },
        ..Default::default()
    };
    let Some(Ok((dev, q))) = filmcraft_gpu::interop::create_device(&adapter, &desc) else { return None };
    eprintln!("compositing on {}", adapter.get_info().name);
    Some(filmcraft_gpu::GpuCompositor::new(&dev, &q))
}

/// `frame` drawn by the compositor (its effect stage, no effects): linear RGBA f32.
fn render(c: &mut filmcraft_gpu::GpuCompositor, frame: &VideoFrame) -> Vec<f32> {
    let fx = filmcraft_render::plan::LayerFx { size: (frame.width, frame.height), decimation: 1, ops: Vec::new() };
    c.effect_image(frame, &fx).expect("rendered").2
}

/// Every picture's Y, U, V planes back to back (8- or 16-bit little-endian samples: the `.yuv`
/// layout).
fn planar_bytes(frames: &[VideoFrame]) -> Vec<u8> {
    let mut got = Vec::new();
    for f in frames {
        match &f.to_planar().expect("planar").data {
            PixelData::Yuv8 { planes, .. } => planes.iter().for_each(|p| got.extend_from_slice(p)),
            PixelData::Yuv16 { planes, .. } => planes.iter().for_each(|p| p.iter().for_each(|v| got.extend_from_slice(&v.to_le_bytes()))),
            other => panic!("planar 4:2:0, not {other:?}"),
        }
    }
    got
}

#[test]
fn pictures_decoded_on_another_gpu_match_the_reference_and_render_as_read_back() {
    let Some(node) = other_decode_node() else {
        eprintln!("SKIPPED: no second render node with a VA-API driver");
        return;
    };
    filmcraft_codecs::hw::set_thread_decode_handoff(Some(node.clone()));
    eprintln!("decoding on {node}");
    let Some(mut c) = gpu() else {
        eprintln!("SKIPPED: no GPU device with DMA-BUF import");
        return;
    };
    let dir = filmcraft_testkit::fixtures::fixtures_root().join("hevc");
    let (mut checked, mut drawn) = (0, 0);
    for name in ["bframes", "intra_main", "intra_main10", "hd_1080p"] {
        let path = dir.join(format!("{name}.hevc"));
        let (Ok(data), Ok(reference)) = (std::fs::read(&path), std::fs::read(path.with_extension("yuv"))) else { continue };
        let frames = match decode_annexb_with(&data, true) {
            Ok(f) => f,
            Err(e) if e.contains("no decoder for the stream's profile") => {
                eprintln!("SKIPPED {name}: {e}");
                continue;
            }
            Err(e) => panic!("{name}: {e}"),
        };
        assert!(!frames.is_empty(), "{name}: no pictures");
        for f in &frames {
            let PixelData::Gpu(s) = &f.data else { panic!("{name}: a GPU frame") };
            assert!(matches!(s.format(), GpuFormat::Nv12 | GpuFormat::P010), "{name}: 4:2:0");
            let d = s.dmabuf().expect("exported");
            assert_eq!(d.modifier, 0, "{name}: a linear copy");
        }
        assert!(planar_bytes(&frames) == reference, "{name}: the read-back differs from the reference");
        for f in frames.iter().take(2) {
            let PixelData::Gpu(s) = &f.data else { continue };
            let read = VideoFrame { data: s.download(f.width, f.height).expect("read back"), ..f.clone() };
            let (in_place, uploaded) = (render(&mut c, f), render(&mut c, &read));
            assert_eq!(in_place.len(), uploaded.len());
            let worst = in_place.iter().zip(&uploaded).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(worst < 1e-6, "{name}: in place differs from read back by {worst}");
            drawn += 1;
        }
        checked += 1;
        eprintln!("{name}: {} pictures bit-exact, first 2 drawn from the linear copy as read back", frames.len());
    }
    if checked == 0 {
        eprintln!("SKIPPED: no 4:2:0 fixtures in {} (cargo xtask fixtures hevc)", dir.display());
        return;
    }
    assert!(c.gpu_frames_in_place >= drawn, "drawn in place: {} of {drawn} ({} read back)", c.gpu_frames_in_place, c.gpu_frames_read_back);
}

/// A scaled hand-off (U0.2.3b d, `hw::set_thread_handoff_scale`): pictures come out at half
/// size and look like the reference averaged 2×2 (luma, mean difference under 3 codes).
#[test]
fn scaled_handoff_gives_smaller_pictures_of_the_same_content() {
    let Some(node) = other_decode_node() else {
        eprintln!("SKIPPED: no second render node with a VA-API driver");
        return;
    };
    filmcraft_codecs::hw::set_thread_decode_handoff(Some(node));
    filmcraft_codecs::hw::set_thread_handoff_scale(Some(0.5));
    let path = filmcraft_testkit::fixtures::fixtures_root().join("hevc").join("hd_1080p.hevc");
    let (Ok(data), Ok(reference)) = (std::fs::read(&path), std::fs::read(path.with_extension("yuv"))) else {
        eprintln!("SKIPPED: no {} (cargo xtask fixtures hevc)", path.display());
        return;
    };
    let frames = match decode_annexb_with(&data, true) {
        Ok(f) => f,
        Err(e) if e.contains("no decoder for the stream's profile") => {
            eprintln!("SKIPPED: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let (w, h) = (1920usize, 1080usize);
    let frame_bytes = w * h * 3 / 2;
    assert!(!frames.is_empty());
    for (i, f) in frames.iter().enumerate().take(3) {
        assert_eq!((f.width, f.height), (960, 540), "half size");
        let PixelData::Gpu(s) = &f.data else { panic!("a GPU frame") };
        let PixelData::Yuv8 { planes, .. } = s.download(f.width, f.height).expect("read back") else {
            eprintln!("SKIPPED: not 8-bit");
            return;
        };
        let refy = &reference[i * frame_bytes..i * frame_bytes + w * h];
        let mut diff = 0u64;
        for y in 0..540 {
            for x in 0..960 {
                let at = |dx: usize, dy: usize| u32::from(refy[(2 * y + dy) * w + 2 * x + dx]);
                let avg = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4;
                diff += u64::from(u32::from(planes[0][y * 960 + x]).abs_diff(avg));
            }
        }
        let mean = diff as f64 / (960.0 * 540.0);
        eprintln!("picture {i}: mean luma difference {mean:.2} codes");
        assert!(mean < 3.0, "picture {i}: mean luma difference {mean:.2}");
    }
}

/// Where the time of a handed-off 4K decode goes (U0.2.3b d diagnosis):
/// `FILMCRAFT_EXPORT_TRACE=1 cargo test -p filmcraft-platform --release --test va_420_handoff -- --ignored --nocapture`.
#[test]
#[ignore]
fn handed_off_4k_decode_time() {
    let Some(node) = other_decode_node() else { return };
    let Ok(data) = std::fs::read(filmcraft_testkit::fixtures::fixtures_root().join("hevc").join("bench_4k.hevc")) else { return };
    for handoff in [false, true] {
        filmcraft_codecs::hw::set_thread_decode_handoff(handoff.then(|| node.clone()));
        if !handoff {
            filmcraft_codecs::hw::set_thread_decode_node(Some(node.clone()));
        }
        let _ = filmcraft_export::trace::take();
        let t = std::time::Instant::now();
        let frames = decode_annexb_with(&data, true).expect("decoded");
        let ms = t.elapsed().as_secs_f64() * 1e3;
        eprintln!(
            "hand-off {handoff}: {} pictures in {ms:.0} ms ({:.1} ms each); {}",
            frames.len(),
            ms / frames.len().max(1) as f64,
            filmcraft_export::trace::take()
        );
    }
}
