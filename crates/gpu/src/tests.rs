use super::*;
use filmcraft_frame::Chroma;
use filmcraft_geom::{Affine, Vec2};
use filmcraft_render::Blend;
use filmcraft_render::plan::execute_cpu;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    device_with(wanted_features())
}

/// A device with whichever of `features` the adapter has.
fn device_with(features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let desc = wgpu::DeviceDescriptor { required_features: adapter.features() & features, ..Default::default() };
    pollster::block_on(adapter.request_device(&desc)).ok()
}

/// The upload paths to test: 16-bit normalized planes (when the adapter has them) and half floats.
fn devices() -> Vec<(&'static str, wgpu::Device, wgpu::Queue)> {
    let mut v = Vec::new();
    if let Some((d, q)) = device_with(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        && d.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
    {
        v.push(("unorm16", d, q));
    }
    if let Some((d, q)) = device_with(wgpu::Features::empty()) {
        v.push(("f16", d, q));
    }
    v
}

/// A 10-bit 4:2:0 or 4:2:2 picture, luma with ramps and fine detail, chroma flat (the GPU
/// interpolates chroma and the CPU repeats it, so detailed chroma would not compare exactly), as planar
/// [`PixelData::Yuv16`] and as the same samples MSB-aligned in [`PixelData::Yuv16Bi`] (P010 / P210).
fn yuv10_pair(w: u32, h: u32, chroma: Chroma) -> (Arc<VideoFrame>, Arc<VideoFrame>) {
    let (sx, sy) = chroma.shifts();
    let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));
    let y: Vec<u16> = (0..w * h).map(|i| (64 + ((i % w) * 876 / w + (i / w) * 3 + (i % 7) * 11) % 877) as u16).collect();
    let u: Vec<u16> = vec![420; (cw * ch) as usize];
    let v: Vec<u16> = vec![610; (cw * ch) as usize];
    let uv: Vec<u16> = u.iter().zip(&v).flat_map(|(a, b)| [a << 6, b << 6]).collect();
    let mk = |data| Arc::new(VideoFrame { width: w, height: h, data, color: filmcraft_color::ColorInfo::REC709, par: (1, 1), pts: Default::default() });
    let bi = mk(PixelData::Yuv16Bi { y: Arc::new(y.iter().map(|c| c << 6).collect()), uv: Arc::new(uv), chroma, bits: 10 });
    let planar = mk(PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma, bits: 10, alpha: None });
    (planar, bi)
}

/// A P010 / P210 frame draws exactly as the same samples in planar form, on both upload paths
/// (16-bit normalized and half floats give the same result), and close to the CPU conversion
/// (`to_linear_f32`, through `to_planar`).
#[test]
fn biplanar_and_16bit_uploads_match_the_cpu() {
    let mut by_path: Vec<(&str, Chroma, Vec<f32>)> = Vec::new();
    for (path, dev, q) in devices() {
        for chroma in [Chroma::C420, Chroma::C422] {
            let (w, h) = (66u32, 38u32);
            let (planar, bi) = yuv10_pair(w, h, chroma);
            let want = planar.to_linear_f32();
            assert_eq!(bi.to_linear_f32(), want, "CPU: to_planar differs");
            let mut c = GpuCompositor::new(&dev, &q);
            let fx = filmcraft_render::plan::LayerFx { size: (w, h), decimation: 1, ops: Vec::new() };
            let (_, _, from_planar) = c.effect_image(&planar, &fx).expect("effect image");
            let (_, _, from_bi) = c.effect_image(&bi, &fx).expect("effect image");
            let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
            assert!(diff(&from_planar, &from_bi) < 1e-6, "{path} {chroma:?}: biplanar differs from planar");
            let worst = diff(&want, &from_bi);
            assert!(worst < 1e-3, "{path} {chroma:?}: max diff from the CPU {worst}");
            if let Some((p, _, other)) = by_path.iter().find(|(p, c, _)| *p != path && *c == chroma) {
                assert!(diff(other, &from_bi) < 1e-5, "{path} vs {p} {chroma:?}: upload paths differ");
            }
            by_path.push((path, chroma, from_bi));
            // the prepared (worker-converted) upload draws the same as the inline one
            let plan = FramePlan::Layers {
                output: None,
                width: w as usize,
                height: h as usize,
                layers: vec![PlanLayer { frame: bi.clone(), matrix: Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None }],
            };
            c.composite(&plan);
            let inline = c.read_output().expect("readback");
            let mut b = GpuCompositor::new(&dev, &q);
            b.composite_prepared(&plan, Some(&prepare(&plan)));
            assert!(inline == b.read_output().expect("readback"), "{path}: prepared upload differs");
        }
    }
}

fn yuv_frame(w: u32, h: u32) -> Arc<VideoFrame> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let y: Vec<u8> = (0..w * h).map(|i| (16 + ((i % w) * 219 / w)) as u8).collect();
    let u: Vec<u8> = (0..cw * ch).map(|i| (64 + (i / cw) * 128 / ch) as u8).collect();
    let v: Vec<u8> = (0..cw * ch).map(|i| (200 - (i % cw) * 100 / cw) as u8).collect();
    Arc::new(VideoFrame {
        width: w,
        height: h,
        data: PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None },
        color: filmcraft_color::ColorInfo::REC709,
        par: (1, 1),
        pts: Default::default(),
    })
}

#[test]
fn gpu_matches_cpu_plan() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let pixels = filmcraft_media::generators::render(
        &filmcraft_media::Generator::Demo(filmcraft_media::DemoScene::OceanSunset),
        320,
        180,
        1.0,
        24,
        filmcraft_time::FrameRate::FPS_24,
    );
    let rgba = Arc::new(VideoFrame::rgba8(320, 180, pixels));
    let (w, h) = (320usize, 180usize);
    let plan = FramePlan::Layers {
        output: None,
        width: w,
        height: h,
        layers: vec![
            PlanLayer { frame: yuv_frame(640, 360), matrix: Affine::scale(0.5, 0.5), opacity: 1.0, blend: Blend::Normal, fx: None },
            PlanLayer {
                frame: rgba,
                matrix: Affine::motion(Vec2::new(200.0, 100.0), Vec2::new(0.4, 0.4), 12.0, Vec2::new(160.0, 90.0)),
                opacity: 0.7,
                blend: Blend::Normal,
                fx: None,
            },
        ],
    };
    let cpu = execute_cpu(&plan).over_black_rgba8();
    c.composite(&plan);
    let (gw, gh, gpu) = c.read_output().expect("readback");
    assert_eq!((gw as usize, gh as usize), (w, h));
    // compare away from antialiased edges: mean abs error and 99th percentile
    let mut diffs: Vec<u32> =
        cpu.chunks(4).zip(gpu.chunks(4)).map(|(a, b)| (0..3).map(|k| (a[k] as i32 - b[k] as i32).unsigned_abs()).max().unwrap_or(0)).collect();
    diffs.sort_unstable();
    let p99 = diffs[diffs.len() * 99 / 100];
    let mean = diffs.iter().sum::<u32>() as f64 / diffs.len() as f64;
    assert!(p99 <= 6 && mean < 1.5, "p99 {p99}, mean {mean}");
    // cache: compositing the same plan again uploads nothing
    let before = c.uploaded_bytes;
    c.composite(&plan);
    assert_eq!(c.uploaded_bytes, before);
}

#[test]
fn half_float_conversion() {
    for v in [0.0f32, 1.0, 0.5, 0.123, 65504.0, -2.0, 1e-5] {
        let h = f32_to_f16(v);
        // decode
        let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
        let e = ((h >> 10) & 0x1f) as i32;
        let m = (h & 0x3ff) as f32;
        let d = if e == 0 { s * m * 2f32.powi(-24) } else { s * (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
        assert!((d - v).abs() <= v.abs() * 1e-3 + 1e-6, "{v} → {d}");
    }
}

#[test]
fn prepared_upload_matches_inline_conversion() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let (w, h) = (64u32, 36u32);
    let yuv16 = Arc::new(VideoFrame {
        width: w,
        height: h,
        data: PixelData::Yuv16 {
            planes: [
                Arc::new((0..w * h).map(|i| (64 + (i * 7) % 876) as u16).collect()),
                Arc::new((0..w * h / 2).map(|i| (64 + (i * 13) % 896) as u16).collect()),
                Arc::new((0..w * h / 2).map(|i| (960 - (i * 5) % 896) as u16).collect()),
            ],
            chroma: Chroma::C422,
            bits: 10,
            alpha: None,
        },
        color: filmcraft_color::ColorInfo::REC709,
        par: (1, 1),
        pts: Default::default(),
    });
    let f32_layer: Vec<f32> = (0..w * h * 4).map(|i| ((i * 37) % 1000) as f32 / 1000.0 * if i % 4 == 3 { 1.0 } else { 0.6 }).collect();
    let rgbaf = Arc::new(VideoFrame::rgba_f32(w, h, f32_layer.clone()));
    let layers = FramePlan::Layers {
        output: None,
        width: w as usize,
        height: h as usize,
        layers: vec![
            PlanLayer { frame: yuv16, matrix: Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None },
            PlanLayer { frame: rgbaf, matrix: Affine::scale(0.5, 0.5), opacity: 0.8, blend: Blend::Normal, fx: None },
        ],
    };
    let image = FramePlan::Image(filmcraft_render::Image { w: w as usize, h: h as usize, px: f32_layer });
    for plan in [layers, image] {
        let mut a = GpuCompositor::new(&dev, &q);
        a.composite(&plan);
        let inline = a.read_output().expect("readback");
        let mut b = GpuCompositor::new(&dev, &q);
        let prep = prepare(&plan);
        assert!(prep.bytes() > 0);
        b.composite_prepared(&plan, Some(&prep));
        let prepared = b.read_output().expect("readback");
        assert!(inline == prepared, "prepared upload differs");
    }
}

/// The upload cache is keyed by pixel-buffer address, so it must keep the buffer alive: otherwise
/// a new frame allocated at a freed frame's address is drawn with the stale texture (seen as
/// whole frames from a previous composite in `crates/golden` GPU parity).
#[test]
fn upload_cache_keeps_buffers_alive() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let px = Arc::new(vec![200u8; 16 * 8 * 4]);
    let frame = Arc::new(VideoFrame { width: 16, height: 8, data: PixelData::Rgba8(px.clone()), ..(*yuv_frame(16, 8)).clone() });
    let plan = FramePlan::Layers {
        output: None,
        width: 16,
        height: 8,
        layers: vec![PlanLayer { frame, matrix: Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None }],
    };
    c.composite(&plan);
    drop(plan);
    assert!(Arc::strong_count(&px) > 1, "cached upload must own its pixel buffer");
}

/// The WGSL tetrahedral LUT matches `Lut3d::apply` on the CPU.
#[test]
fn gpu_lut_matches_cpu_tetrahedral() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let g = GpuLut::new(&dev, &q);
    for size in [2usize, 17, 33, 65] {
        let mut lut = filmcraft_color::Lut3d::from_fn(size, |c| {
            [(c[0] * c[1] + c[2] * c[2]).sin(), c[1].powf(0.45) * (1.0 - 0.3 * c[0]), (c[0] - c[2]).abs() + 0.1 * c[1]]
        });
        if size == 17 {
            lut.domain_min = [-0.1, 0.0, 0.0];
            lut.domain_max = [1.2, 1.0, 2.0];
        }
        let mut s = 99u64;
        let mut px = Vec::new();
        for _ in 0..70_000 {
            for k in 0..4 {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                px.push(if k == 3 { 0.5 } else { (s % 10_001) as f32 / 10_000.0 * 1.2 - 0.1 });
            }
        }
        let gpu = g.apply(&lut, &px).expect("gpu lut");
        let mut worst = 0f32;
        for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
            let c = lut.apply([p[0], p[1], p[2]]);
            for k in 0..3 {
                worst = worst.max((c[k] - gpu[i * 4 + k]).abs());
            }
            assert_eq!(gpu[i * 4 + 3], 0.5);
        }
        eprintln!("LUT {size}³: max |cpu − gpu| = {worst:e}");
        assert!(worst < 1e-4, "size {size}: max |cpu - gpu| = {worst}");
    }
}

fn test_masks(scale: f32) -> Vec<filmcraft_render::mask::FlatMask> {
    use filmcraft_project::{Mask, MaskMode, MaskPath, ParamValue};
    let mut a = Mask::new("a", MaskPath::ellipse(Vec2::new(150.0, 90.0), Vec2::new(90.0, 55.0)));
    a.feather.value = ParamValue::Float(24.0);
    a.expansion.value = ParamValue::Float(6.0);
    let mut b = Mask::new("b", MaskPath::polygon(&[Vec2::new(40.0, 20.0), Vec2::new(260.0, 50.0), Vec2::new(120.0, 170.0)]));
    b.mode = MaskMode::Subtract;
    b.feather.value = ParamValue::Float(0.0);
    b.opacity.value = ParamValue::Float(70.0);
    let mut c = Mask::new("c", MaskPath::rect(200.0, 100.0, 300.0, 175.0));
    c.mode = MaskMode::Difference;
    c.inverted = true;
    c.feather.value = ParamValue::Float(3.5);
    filmcraft_render::mask::prepare(&[a, b, c], filmcraft_time::Tick(0), scale)
}

#[test]
fn gpu_mask_coverage_matches_cpu() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let g = crate::GpuMask::new(&dev, &q);
    for scale in [1.0f32, 0.5] {
        let masks = test_masks(scale);
        let (w, h) = ((320.0 * scale) as usize, (180.0 * scale) as usize);
        let cpu = filmcraft_render::mask::coverage(&masks, w, h).unwrap();
        let gpu = g.coverage(&masks, w, h).expect("gpu coverage");
        let worst = cpu.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        eprintln!("mask coverage @{scale}: max |cpu − gpu| = {worst:e}");
        assert!(worst < 2e-4, "scale {scale}: {worst}");
        assert!(cpu.iter().any(|v| *v > 0.99) && cpu.iter().any(|v| *v < 0.01) && cpu.iter().any(|v| *v > 0.2 && *v < 0.8));
    }
}

#[test]
fn gpu_masked_mix_matches_cpu() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let g = crate::GpuMask::new(&dev, &q);
    let masks = test_masks(1.0);
    let (w, h) = (320usize, 180usize);
    let original = filmcraft_render::Image { w, h, px: (0..w * h * 4).map(|i| ((i * 37) % 101) as f32 / 100.0).collect() };
    let mut effected = original.clone();
    effected.px.iter_mut().for_each(|v| *v = 1.0 - *v * 0.5);
    let gpu = g.mix(&masks, w, h, &original.px, &effected.px).expect("gpu mix");
    let cov = filmcraft_render::mask::coverage(&masks, w, h).unwrap();
    let mut cpu = effected.clone();
    filmcraft_render::mask::mix(&mut cpu, &original, &cov);
    let worst = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    eprintln!("masked mix: max |cpu − gpu| = {worst:e}");
    assert!(worst < 2e-4, "{worst}");
}

/// A 4:4:4 grey picture with an alpha plane that ramps from transparent (left) to opaque (right).
fn yuv_alpha_frame(w: u32, h: u32, bits: u32) -> Arc<VideoFrame> {
    let n = (w * h) as usize;
    let max = (1u32 << bits) - 1;
    let ramp = |i: usize| ((i % w as usize) as u32 * max / (w - 1)) as u16;
    let data = if bits == 8 {
        PixelData::Yuv8 {
            planes: [Arc::new(vec![180u8; n]), Arc::new(vec![100u8; n]), Arc::new(vec![170u8; n])],
            chroma: Chroma::C444,
            alpha: Some(Arc::new((0..n).map(|i| ramp(i) as u8).collect())),
        }
    } else {
        let s = |v: u32| (v << (bits - 8)) as u16;
        PixelData::Yuv16 {
            planes: [Arc::new(vec![s(180); n]), Arc::new(vec![s(100); n]), Arc::new(vec![s(170); n])],
            chroma: Chroma::C444,
            bits,
            alpha: Some(Arc::new((0..n).map(ramp).collect())),
        }
    };
    Arc::new(VideoFrame { width: w, height: h, data, color: filmcraft_color::ColorInfo::REC709, par: (1, 1), pts: Default::default() })
}

/// A Y'CbCr layer with an alpha plane (ProRes 4444) is drawn with its alpha: the GPU result matches
/// the CPU compositor over an opaque background. (It used to draw such a layer opaque.)
#[test]
fn gpu_draws_yuv_alpha_plane_like_the_cpu() {
    for (_, dev, q) in devices() {
        draws_yuv_alpha_plane_like_the_cpu(&dev, &q);
    }
}

fn draws_yuv_alpha_plane_like_the_cpu(dev: &wgpu::Device, q: &wgpu::Queue) {
    let (w, h) = (320usize, 180usize);
    let pixels = filmcraft_media::generators::render(
        &filmcraft_media::Generator::Demo(filmcraft_media::DemoScene::OceanSunset),
        320,
        180,
        1.0,
        24,
        filmcraft_time::FrameRate::FPS_24,
    );
    let background = Arc::new(VideoFrame::rgba8(320, 180, pixels));
    for bits in [8, 10, 12] {
        let mut c = GpuCompositor::new(dev, q);
        let plan = FramePlan::Layers {
            output: None,
            width: w,
            height: h,
            layers: vec![
                PlanLayer { frame: background.clone(), matrix: Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None },
                PlanLayer { frame: yuv_alpha_frame(320, 180, bits), matrix: Affine::IDENTITY, opacity: 0.9, blend: Blend::Normal, fx: None },
            ],
        };
        let cpu = execute_cpu(&plan).over_black_rgba8();
        c.composite(&plan);
        let (_, _, gpu) = c.read_output().expect("readback");
        let mut diffs: Vec<u32> =
            cpu.chunks(4).zip(gpu.chunks(4)).map(|(a, b)| (0..3).map(|k| (a[k] as i32 - b[k] as i32).unsigned_abs()).max().unwrap_or(0)).collect();
        diffs.sort_unstable();
        let (p99, max) = (diffs[diffs.len() * 99 / 100], diffs[diffs.len() - 1]);
        assert!(p99 <= 3 && max <= 8, "{bits}-bit: p99 {p99}, max {max}");
        // the transparent left edge shows the background, the opaque right edge the layer
        let at = |img: &[u8], x: usize| (img[(90 * w + x) * 4], img[(90 * w + x) * 4 + 1], img[(90 * w + x) * 4 + 2]);
        let bg = execute_cpu(&FramePlan::Layers {
            output: None,
            width: w,
            height: h,
            layers: vec![PlanLayer { frame: background.clone(), matrix: Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None }],
        })
        .over_black_rgba8();
        let (l, r, b0) = (at(&gpu, 1), at(&gpu, w - 2), at(&bg, 1));
        assert!((l.0 as i32 - b0.0 as i32).abs() <= 6 && (l.1 as i32 - b0.1 as i32).abs() <= 6, "{bits}-bit: left edge {l:?} vs background {b0:?}");
        assert!(r != at(&bg, w - 2), "{bits}-bit: right edge {r:?} should differ from the background");
    }
}

/// The scope samples read from the composited output are the samples `filmcraft_scopes::Signal`
/// picks from the read-back frame (fork Phase 4.4).
#[test]
fn scope_samples_match_the_read_back_frame() {
    use filmcraft_render::plan::{FramePlan, PlanLayer};
    let Some((dev, q)) = super::blend_tests::device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    for (w, h) in [(1000usize, 563usize), (320, 180), (481, 271)] {
        let plan = FramePlan::Layers {
            width: w,
            height: h,
            layers: vec![PlanLayer::new(
                super::blend_tests::ramp_layer(w as u32, h as u32, 2),
                filmcraft_geom::Affine::IDENTITY,
                1.0,
                filmcraft_render::blend::Blend::Normal,
            )],
            output: None,
        };
        c.composite(&plan);
        let (fw, fh, full) = c.read_output().expect("readback");
        let r = c.request_scope_samples(480, 270).expect("scope samples");
        let mut got = None;
        for _ in 0..2000 {
            got = r.try_take();
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let got = got.expect("read back in time");
        let want = filmcraft_scopes::Signal::from_rgba8_max(fw as usize, fh as usize, &full, 480, 270);
        let ours = filmcraft_scopes::Signal::from_rgba8_max(r.w as usize, r.h as usize, &got, r.w as usize, r.h as usize);
        assert_eq!((ours.w, ours.h), (want.w, want.h), "{w}×{h}");
        assert_eq!(ours.rgb, want.rgb, "{w}×{h}");
    }
}

/// Maximum Render Quality: the compositor's Lanczos-3 minification matches
/// `Image::transformed_hq` (6.2K → 4K-like ×0.615, and ×0.3 with bilinear taps).
#[test]
fn high_quality_resample_matches_cpu() {
    use filmcraft_render::plan::{FramePlan, PlanLayer};
    let Some((dev, q)) = super::blend_tests::device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    c.set_high_quality(true);
    let (sw, sh) = (624usize, 416usize);
    let px: Vec<f32> = (0..sw * sh)
        .flat_map(|i| {
            let (x, y) = ((i % sw) as f32, (i / sw) as f32);
            let v = 0.5 + 0.3 * (x * 0.37).sin() * (y * 0.23).cos();
            [v, v * 0.8, 1.0 - v, 1.0]
        })
        .collect();
    let frame = Arc::new(VideoFrame::rgba_f32(sw as u32, sh as u32, px.clone()));
    let src = filmcraft_render::Image { w: sw, h: sh, px };
    for s in [3840.0 / 6240.0, 0.3] {
        let (w, h) = ((sw as f64 * s).round() as usize, (sh as f64 * s).round() as usize);
        let m = filmcraft_geom::Affine::scale(s, s);
        let plan = FramePlan::Layers {
            width: w,
            height: h,
            layers: vec![PlanLayer::new(frame.clone(), m, 1.0, filmcraft_render::blend::Blend::Normal)],
            output: None,
        };
        c.composite(&plan);
        let (_, _, gpu) = c.read_output().expect("readback");
        let cpu = src.transformed_hq(w, h, &m).over_black_rgba8();
        let mut d: Vec<u8> = cpu.iter().zip(&gpu).map(|(a, b)| a.abs_diff(*b)).collect();
        d.sort_unstable();
        let (p99, max) = (d[d.len() * 99 / 100], d[d.len() - 1]);
        eprintln!("×{s:.3}: 8-bit p99 {p99} max {max}");
        assert!(p99 <= 1 && max <= 3, "×{s}: p99 {p99} max {max}");
    }
}

/// Export frames as Y'CbCr from the accumulator (fork Phase 5.1) match the CPU conversion of the
/// CPU-composited picture (`filmcraft_export::rgb_to_yuv420p10le`'s maths), 10- and 8-bit, odd
/// sizes included: within 2 codes (the accumulator is half float).
#[test]
fn yuv420_from_the_accumulator_matches_cpu() {
    use filmcraft_render::plan::{FramePlan, PlanLayer, execute_cpu};
    let Some((dev, q)) = super::blend_tests::device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    for (w, h) in [(320usize, 180usize), (161, 91)] {
        let plan = FramePlan::Layers {
            width: w,
            height: h,
            layers: vec![
                PlanLayer::new(super::blend_tests::yuv_frame(640, 360), filmcraft_geom::Affine::scale(0.5, 0.5), 1.0, filmcraft_render::blend::Blend::Normal),
                PlanLayer::new(
                    super::blend_tests::ramp_layer(120, 60, 3),
                    filmcraft_geom::Affine::translate(30.0, 40.0),
                    0.8,
                    filmcraft_render::blend::Blend::Normal,
                ),
            ],
            output: None,
        };
        let cpu = execute_cpu(&plan);
        c.composite(&plan);
        for (ten, (kr, kb)) in [(true, (0.2126f32, 0.0722f32)), (false, (0.2126, 0.0722)), (true, (0.2627, 0.0593))] {
            let gpu = c.read_yuv420(kr, kb, ten).expect("yuv read-back");
            // CPU reference: over black, clamped sRGB encode, then the export's 4:2:0 maths
            let enc = |v: f32| filmcraft_color::linear_to_srgb(v.clamp(0.0, 1.0));
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            let (kg, sb, sr) = (1.0 - kr - kb, 2.0 * (1.0 - kb), 2.0 * (1.0 - kr));
            let (ys, yo, cs, co, lo, hi) = if ten { (876.0, 64.0, 896.0, 512.0, 4.0, 1019.0) } else { (219.0, 16.0, 224.0, 128.0, 1.0, 254.0) };
            let mut want = vec![0u32; w * h + 2 * cw * ch];
            let mut acc = vec![[0f32; 3]; cw * ch];
            for y in 0..h {
                for x in 0..w {
                    let p = cpu.get(x, y);
                    let (r, g, b) = (enc(p[0]), enc(p[1]), enc(p[2]));
                    let yy = kr * r + kg * g + kb * b;
                    want[y * w + x] = (yo + ys * yy).round().clamp(lo, hi) as u32;
                    let a = &mut acc[(y / 2) * cw + x / 2];
                    a[0] += (b - yy) / sb;
                    a[1] += (r - yy) / sr;
                    a[2] += 1.0;
                }
            }
            for (i, a) in acc.iter().enumerate() {
                want[w * h + i] = (co + cs * a[0] / a[2]).round().clamp(lo, hi) as u32;
                want[w * h + cw * ch + i] = (co + cs * a[1] / a[2]).round().clamp(lo, hi) as u32;
            }
            let got: Vec<u32> = if ten {
                gpu.as_chunks::<2>().0.iter().map(|b| u32::from(u16::from_le_bytes(*b))).collect()
            } else {
                gpu.iter().map(|b| u32::from(*b)).collect()
            };
            assert_eq!(got.len(), want.len(), "{w}×{h}");
            let worst = got.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            eprintln!("{w}×{h} ten {ten} kr {kr}: max {worst} codes");
            assert!(worst <= if ten { 3 } else { 1 }, "{w}×{h} ten {ten}: {worst}");
        }
    }
}
