//! Comp programs on the GPU (fork C1 step 7b) against the CPU oracle
//! ([`filmcraft_render::comp::execute_cpu`] through the frame plan). Run on each GPU with
//! `MESA_VK_DEVICE_SELECT` (AMD and Intel).

use super::*;
use filmcraft_geom::Affine;
use filmcraft_render::Blend;
use filmcraft_render::comp::{Pass, Program};
use filmcraft_render::gpufx::FxOp;
use filmcraft_render::image::Image;
use filmcraft_render::plan::{LayerFx, execute_cpu};

const W: usize = 96;
const H: usize = 54;

fn compositor() -> Option<GpuCompositor> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let desc = wgpu::DeviceDescriptor { required_features: adapter.features() & wanted_features(), ..Default::default() };
    let (d, q) = pollster::block_on(adapter.request_device(&desc)).ok()?;
    eprintln!("adapter: {}", adapter.get_info().name);
    Some(GpuCompositor::new(&d, &q))
}

/// A premultiplied picture with colour ramps, a bright square and alpha falling off to the right.
fn picture() -> Arc<Image> {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let a = 1.0 - 0.6 * x as f32 / W as f32;
            let bright = (30..60).contains(&x) && (15..40).contains(&y);
            let (r, g, b) = if bright { (0.9, 0.85, 0.8) } else { (x as f32 / W as f32, y as f32 / H as f32, 0.3) };
            let i = (y * W + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[r * a, g * a, b * a, a]);
        }
    }
    Arc::new(img)
}

/// The comp as one layer, on the GPU (linear accumulator) and the CPU; the 99th percentile and
/// the mean of the per-channel difference.
fn compare(c: &mut GpuCompositor, p: Program) -> (f32, f32) {
    let plan = FramePlan::Layers { width: W, height: H, layers: vec![PlanLayer::comp(Arc::new(p), Affine::IDENTITY, 1.0, Blend::Normal)], output: None };
    let cpu = execute_cpu(&plan);
    let (gw, gh, gpu) = c.render_export_prepared(&plan, None).expect("export read-back");
    assert_eq!((gw as usize, gh as usize), (W, H));
    let mut d: Vec<f32> = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).collect();
    d.sort_by(f32::total_cmp);
    let p99 = d.get(d.len() * 99 / 100).copied().unwrap_or(f32::MAX);
    let mean = d.iter().sum::<f32>() / d.len().max(1) as f32;
    (p99, mean)
}

fn program(textures: u16, passes: Vec<Pass>, out: u16) -> Program {
    Program { size: (W, H), textures, passes, out }
}

#[test]
fn every_pass_kind_matches_the_cpu() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let img = || Pass::Image { dst: 0, image: picture() };
    let blue = Pass::Fill { dst: 1, color: [0.0, 0.1, 0.6, 1.0] };
    let turn = Affine::motion(filmcraft_geom::Vec2::new(48.0, 27.0), filmcraft_geom::Vec2::new(0.7, 0.7), 15.0, filmcraft_geom::Vec2::new(48.0, 27.0));
    let cases: Vec<(&str, Program)> = vec![
        ("image", program(1, vec![img()], 0)),
        ("fill", program(2, vec![blue.clone()], 1)),
        ("copy", program(2, vec![img(), Pass::Copy { src: 0, dst: 1 }], 1)),
        ("draw", program(2, vec![img(), Pass::Draw { src: 0, dst: 1, xform: Affine::translate(10.0, 6.0), hq: false }], 1)),
        ("draw in place", program(1, vec![img(), Pass::Draw { src: 0, dst: 0, xform: Affine::scale(0.5, 0.5), hq: false }], 0)),
        ("over", program(2, vec![img(), blue.clone(), Pass::Over { src: 0, dst: 1, mode: Blend::Normal, opacity: 0.8 }], 1)),
        ("over multiply", program(2, vec![img(), blue.clone(), Pass::Over { src: 0, dst: 1, mode: Blend::Multiply, opacity: 1.0 }], 1)),
        ("over itself", program(1, vec![img(), Pass::Over { src: 0, dst: 0, mode: Blend::Normal, opacity: 0.5 }], 0)),
        ("fx key", program(1, vec![img(), Pass::Fx { tex: 0, ops: vec![FxOp::LumaKey { th: 0.6, cut: 0.2 }] }], 0)),
        ("fx blur", program(1, vec![img(), Pass::Fx { tex: 0, ops: vec![FxOp::Gaussian { rx: vec![2], ry: vec![2], repeat: false }] }], 0)),
        ("missing input", program(2, vec![Pass::Over { src: 0, dst: 1, mode: Blend::Normal, opacity: 1.0 }], 1)),
        (
            "plan layers",
            program(
                1,
                vec![Pass::Layers {
                    dst: 0,
                    layers: vec![PlanLayer::new(
                        Arc::new(VideoFrame::rgba_f32(W as u32, H as u32, picture().px.clone())),
                        Affine::translate(6.0, 3.0),
                        0.9,
                        Blend::Normal,
                    )]
                    .into(),
                }],
                0,
            ),
        ),
    ];
    for (name, p) in cases {
        let (p99, mean) = compare(&mut c, p);
        eprintln!("{name}: p99 {p99:.5} mean {mean:.6}");
        assert!(p99 <= 4e-3 && mean <= 1e-3, "{name}: p99 {p99}, mean {mean}");
    }
    // a turned picture: its antialiased edges differ (as for timeline layers), so the bulk is
    // compared (the 90th percentile) and the mean stays small
    let (p90, mean) = {
        let p = program(2, vec![img(), Pass::Draw { src: 0, dst: 1, xform: turn, hq: false }], 1);
        let plan = FramePlan::Layers { width: W, height: H, layers: vec![PlanLayer::comp(Arc::new(p), Affine::IDENTITY, 1.0, Blend::Normal)], output: None };
        let cpu = execute_cpu(&plan);
        let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
        let mut d: Vec<f32> = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).collect();
        d.sort_by(f32::total_cmp);
        (d.get(d.len() * 9 / 10).copied().unwrap_or(f32::MAX), d.iter().sum::<f32>() / d.len().max(1) as f32)
    };
    eprintln!("turned draw: p90 {p90:.5} mean {mean:.6}");
    assert!(p90 <= 4e-3 && mean <= 5e-3, "turned draw: p90 {p90}, mean {mean}");
    // the passes ran on the GPU (slots exist), not through the CPU fallback
    assert!(c.comp_sets.iter().flatten().any(|s| s.target.is_some()));
}

/// Comp A's shape (C1 acceptance): Media In → Key → Merge over a background → Color → Out, with a
/// Transform on the foreground; slots reused by liveness.
#[test]
fn a_keyed_merge_with_a_grade_matches_the_cpu() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let p = program(
        2,
        vec![
            Pass::Image { dst: 0, image: picture() },
            Pass::Fx { tex: 0, ops: vec![FxOp::LumaKey { th: 0.5, cut: 0.25 }] },
            Pass::Draw { src: 0, dst: 0, xform: Affine::translate(8.0, -4.0), hq: false },
            Pass::Fill { dst: 1, color: [0.2, 0.05, 0.05, 1.0] },
            Pass::Over { src: 0, dst: 1, mode: Blend::Normal, opacity: 1.0 },
            Pass::Fx { tex: 1, ops: vec![FxOp::BrightnessContrast { br: 10.0, co: 20.0 }] },
        ],
        1,
    );
    let (p99, mean) = compare(&mut c, p);
    assert!(p99 <= 4e-3 && mean <= 1e-3, "p99 {p99}, mean {mean}");
}

/// Hostile sizes: a comp wider than any texture can't run on the GPU; it renders through the CPU
/// fallback instead of failing (and a later normal comp still runs on the GPU).
#[test]
fn a_comp_too_wide_for_a_texture_falls_back_to_the_cpu() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    // an op the GPU effect stage can't run: the whole comp runs on the CPU
    let cpu_only = FxOp::Gamma { g: f32::NAN };
    assert!(!cpu_only.gpu_ok());
    let p = program(1, vec![Pass::Image { dst: 0, image: picture() }, Pass::Fx { tex: 0, ops: vec![cpu_only] }], 0);
    assert!(c.run_comp(&p, 0).is_none());
    let wide = Program { size: (100_000, 2), textures: 1, passes: vec![Pass::Fill { dst: 0, color: [0.5, 0.0, 0.0, 0.5] }], out: 0 };
    assert!(c.run_comp(&wide, 0).is_none());
    let plan = FramePlan::Layers {
        width: W,
        height: H,
        layers: vec![PlanLayer::comp(Arc::new(wide), Affine::scale(W as f64 / 100_000.0, H as f64 / 2.0), 1.0, Blend::Normal)],
        output: None,
    };
    let (_, _, px) = c.render_export_prepared(&plan, None).expect("read-back");
    let mid = (H / 2 * W + W / 2) * 4;
    assert!((px[mid] - 0.5).abs() < 0.02 && (px[mid + 3] - 0.5).abs() < 0.02, "{:?}", &px[mid..mid + 4]);
    let (p99, _) = compare(&mut c, program(1, vec![Pass::Image { dst: 0, image: picture() }], 0));
    assert!(p99 <= 4e-3);
    // a picture of another size than the comp: the CPU keeps it as it is, so the CPU runs the comp
    // (the GPU drew it transparent)
    let mut small = Image::new(W / 2, H / 2);
    small.px.iter_mut().for_each(|v| *v = 0.5);
    let odd = program(1, vec![Pass::Image { dst: 0, image: Arc::new(small) }], 0);
    assert!(c.run_comp(&odd, 0).is_none());
    let (p99, mean) = compare(&mut c, odd);
    assert!(p99 <= 4e-3 && mean <= 1e-3, "p99 {p99}, mean {mean}");
}

/// Two comp clips in one frame (comps A and B on V2 / V3): each keeps its own result.
#[test]
fn two_comps_in_one_frame_keep_their_own_pictures() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let red = program(1, vec![Pass::Fill { dst: 0, color: [0.8, 0.0, 0.0, 1.0] }], 0);
    let blue = program(1, vec![Pass::Fill { dst: 0, color: [0.0, 0.0, 0.6, 1.0] }], 0);
    let plan = FramePlan::Layers {
        width: W,
        height: H,
        layers: vec![
            PlanLayer::comp(Arc::new(red), Affine::IDENTITY, 1.0, Blend::Normal),
            // the second comp covers only the right half
            PlanLayer::comp(Arc::new(blue), Affine::translate(W as f64 / 2.0, 0.0), 1.0, Blend::Normal),
        ],
        output: None,
    };
    let cpu = execute_cpu(&plan);
    let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
    for x in [4usize, W - 4] {
        let i = (H / 2 * W + x) * 4;
        for k in 0..4 {
            assert!((cpu.px[i + k] - gpu[i + k]).abs() < 2e-3, "x {x}: cpu {:?} gpu {:?}", &cpu.px[i..i + 4], &gpu[i..i + 4]);
        }
    }
}

/// A busy second comp (more uploads than the upload cache keeps) must not push the first comp's
/// result out of the cache before the frame draws it (it drew transparent).
#[test]
fn a_busy_later_comp_does_not_evict_an_earlier_result() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let red = program(1, vec![Pass::Fill { dst: 0, color: [0.8, 0.0, 0.0, 1.0] }], 0);
    // every fill uploads its own 1×1 colour: 40 new cache entries after the red comp's result
    let busy: Vec<Pass> = (0..40).map(|i| Pass::Fill { dst: 0, color: [0.0, 0.0, 0.3 + i as f32 / 100.0, 1.0] }).collect();
    let blue = program(1, busy, 0);
    let plan = FramePlan::Layers {
        width: W,
        height: H,
        layers: vec![
            PlanLayer::comp(Arc::new(red), Affine::IDENTITY, 1.0, Blend::Normal),
            PlanLayer::comp(Arc::new(blue), Affine::translate(W as f64 / 2.0, 0.0), 1.0, Blend::Normal),
        ],
        output: None,
    };
    let cpu = execute_cpu(&plan);
    let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
    for x in [4usize, W - 4] {
        let i = (H / 2 * W + x) * 4;
        for k in 0..4 {
            assert!((cpu.px[i + k] - gpu[i + k]).abs() < 2e-3, "x {x}: cpu {:?} gpu {:?}", &cpu.px[i..i + 4], &gpu[i..i + 4]);
        }
    }
}

/// A comp with effects and a frame layer with effects of another size share one effect-pool frame:
/// both sizes stay pooled (the comp's slot draws ended the pool frame and dropped the frame-size
/// textures, the frame then dropped the comp-size ones, so playback reallocated them every frame).
#[test]
fn comp_and_frame_effects_keep_their_pooled_textures() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    if c.fx.is_none() {
        eprintln!("no effect stage; skipping");
        return;
    }
    let blur = || vec![FxOp::Gaussian { rx: vec![2], ry: vec![2], repeat: false }];
    let comp = Program {
        size: (W / 2, H / 2),
        textures: 1,
        passes: vec![Pass::Fill { dst: 0, color: [0.2, 0.5, 0.1, 1.0] }, Pass::Fx { tex: 0, ops: blur() }],
        out: 0,
    };
    let below = PlanLayer {
        frame: Arc::new(VideoFrame::rgba_f32(W as u32, H as u32, picture().px.clone())),
        matrix: Affine::IDENTITY,
        opacity: 1.0,
        blend: Blend::Normal,
        fx: Some(LayerFx { size: (W as u32, H as u32), decimation: 1, ops: blur() }.into()),
        comp: None,
    };
    let plan =
        FramePlan::Layers { width: W, height: H, layers: vec![below, PlanLayer::comp(Arc::new(comp), Affine::IDENTITY, 1.0, Blend::Normal)], output: None };
    let cpu = execute_cpu(&plan);
    for round in 0..2 {
        let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
        let worst = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
        assert!(worst < 5e-3, "round {round}: worst difference {worst}");
        let sizes = c.fx.as_ref().map_or(0, |f| f.pooled_sizes());
        assert_eq!(sizes, 2, "round {round}: comp-size and frame-size effect textures both pooled");
    }
}

/// The comp budget covers every slot set held: a second comp that would go above it runs on the
/// CPU (each comp used to have the whole budget, so eight comps could hold eight budgets) and the
/// picture is still right; sets a frame no longer uses and, after a comp-free stretch, all comp
/// textures are let go.
#[test]
fn comps_share_one_budget_and_their_textures_are_let_go() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let fill = |color: [f32; 4]| Arc::new(program(1, vec![Pass::Fill { dst: 0, color }], 0));
    // one comp's two slots (its picture and the scratch, counted whether drawn or not) fit; with
    // the first comp's drawn picture held, the second's don't (2.5 slots)
    c.comp_vram_budget = 5 * (W * H) as u64 * 4;
    let plan = FramePlan::Layers {
        width: W,
        height: H,
        layers: vec![
            PlanLayer::comp(fill([0.8, 0.0, 0.0, 1.0]), Affine::IDENTITY, 1.0, Blend::Normal),
            PlanLayer::comp(fill([0.0, 0.0, 0.6, 1.0]), Affine::translate(W as f64 / 2.0, 0.0), 1.0, Blend::Normal),
        ],
        output: None,
    };
    let cpu = execute_cpu(&plan);
    let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
    let worst = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
    assert!(worst < 2e-3, "worst difference {worst}");
    let held = |c: &GpuCompositor| c.comp_sets.iter().map(|s| s.iter().filter(|x| x.target.is_some()).count()).collect::<Vec<_>>();
    assert_eq!(held(&c), vec![1, 0], "the second comp ran on the CPU");
    // a frame with one comp keeps one set
    c.comp_vram_budget = comp::COMP_VRAM_BUDGET;
    let one = FramePlan::Layers {
        width: W,
        height: H,
        layers: vec![PlanLayer::comp(fill([0.1, 0.6, 0.1, 1.0]), Affine::IDENTITY, 1.0, Blend::Normal)],
        output: None,
    };
    c.render_export_prepared(&one, None).expect("read-back");
    assert_eq!(c.comp_sets.len(), 1);
    // comp-free frames: kept for a while, then let go
    let plain = FramePlan::Layers { width: W, height: H, layers: Vec::new(), output: None };
    for _ in 1..comp::COMP_IDLE_FRAMES {
        c.render_export_prepared(&plain, None).expect("read-back");
    }
    assert_eq!(c.comp_sets.len(), 1, "kept until the idle stretch ends");
    c.render_export_prepared(&plain, None).expect("read-back");
    assert!(c.comp_sets.is_empty() && c.comp_slots.is_empty() && c.comp_output.is_none(), "comp textures let go");
}

/// A comp smaller than the frame draws into its own output / backdrop textures: the frame's keep
/// their size (they were reallocated twice per frame when comp and frame sizes alternated) and the
/// picture still matches the CPU.
#[test]
fn a_comp_of_another_size_leaves_the_frames_targets_alone() {
    let Some(mut c) = compositor() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let (cw, ch) = (W / 2, H / 2);
    let comp = Program { size: (cw, ch), textures: 1, passes: vec![Pass::Fill { dst: 0, color: [0.2, 0.5, 0.1, 1.0] }], out: 0 };
    let pic = picture();
    let below = PlanLayer {
        frame: Arc::new(VideoFrame::rgba_f32(W as u32, H as u32, pic.px.clone())),
        matrix: Affine::IDENTITY,
        opacity: 1.0,
        blend: Blend::Normal,
        fx: None,
        comp: None,
    };
    let plan = FramePlan::Layers {
        width: W,
        height: H,
        // a blend-mode layer reads the destination: the frame's backdrop is used
        layers: vec![below, PlanLayer::comp(Arc::new(comp), Affine::translate(8.0, 4.0), 0.8, Blend::Screen)],
        output: None,
    };
    let cpu = execute_cpu(&plan);
    for round in 0..3 {
        let made = TARGETS_MADE.with(std::cell::Cell::get);
        let (_, _, gpu) = c.render_export_prepared(&plan, None).expect("read-back");
        if round > 0 {
            assert_eq!(TARGETS_MADE.with(std::cell::Cell::get), made, "round {round} created render targets again");
        }
        let worst = cpu.px.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
        assert!(worst < 3e-3, "worst difference {worst}");
        let size = |s: &Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>| s.as_ref().map(|t| t.2);
        assert_eq!(size(&c.output), Some((W as u32, H as u32)), "the frame's output");
        assert_eq!(size(&c.backdrop), Some((W as u32, H as u32)), "the frame's backdrop");
    }
}
