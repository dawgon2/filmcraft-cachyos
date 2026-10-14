//! Comp evaluation: each node against the primitive it reuses, scheduling, and the graph rules.

use super::*;
use filmcraft_project::comp::{Comp, CompKind, CompNode, MediaInSource};
use filmcraft_project::dag::{Edge, Port};
use filmcraft_project::{ItemId, Param, ParamValue};
use filmcraft_time::FrameRate;

const W: usize = 32;
const H: usize = 18;

fn ctx() -> CompCtx<'static> {
    CompCtx { t: Tick(0), size: (W, H), project: None, high_quality: false }
}

/// A comp of `W`×`H` holding `nodes` (Media Out is node 1), wired by `(from, to, port)`.
fn comp(nodes: Vec<CompNode>, edges: &[(NodeId, NodeId, u8)]) -> Comp {
    let mut c = Comp::new(W as u32, H as u32, FrameRate::FPS_24, Tick(1000));
    c.nodes.extend(nodes);
    c.edges = edges.iter().map(|&(f, t, p)| Edge::new(Port::new(f, 0), Port::new(t, p))).collect();
    c
}

fn node(id: NodeId, kind: CompKind, params: &[(&str, ParamValue)]) -> CompNode {
    let mut n = CompNode::new(id, kind);
    for (k, v) in params {
        n.params.insert((*k).to_string(), Param::new(v.clone()));
    }
    n
}

fn solid(id: NodeId, c: [f32; 4]) -> CompNode {
    node(id, CompKind::Solid, &[("color", ParamValue::Color(c))])
}

fn run(c: &Comp) -> Image {
    execute_cpu(&compile(c, ctx(), &NoSources).unwrap())
}

fn max_diff(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.w, a.h), (b.w, b.h));
    a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

/// The picture a Solid of sRGB colour `c` draws (linear, premultiplied).
fn solid_image(c: [f32; 4]) -> Image {
    let [r, g, b] = dec([c[0], c[1], c[2]]);
    Image::filled(W, H, [r * c[3], g * c[3], b * c[3], c[3]])
}

#[test]
fn solid_draws_its_colour_in_linear_light() {
    let c = comp(vec![solid(2, [1.0, 0.5, 0.0, 0.5])], &[(2, 1, 0)]);
    assert_eq!(max_diff(&run(&c), &solid_image([1.0, 0.5, 0.0, 0.5])), 0.0);
}

#[test]
fn merge_is_the_blend_composite() {
    let (bg, fg) = ([0.2, 0.4, 0.6, 1.0], [0.9, 0.1, 0.3, 0.7]);
    let merge = node(4, CompKind::Merge, &[("opacity", ParamValue::Float(60.0)), ("blend", ParamValue::Choice(Blend::Screen.index()))]);
    let c = comp(vec![solid(2, bg), solid(3, fg), merge], &[(2, 4, 0), (3, 4, 1), (4, 1, 0)]);
    let mut want = solid_image(bg);
    blend::composite(&mut want, &solid_image(fg), 0.6, Blend::Screen);
    assert_eq!(max_diff(&run(&c), &want), 0.0);
}

#[test]
fn merge_without_foreground_passes_the_background() {
    let c = comp(vec![solid(2, [0.3, 0.3, 0.3, 1.0]), node(4, CompKind::Merge, &[])], &[(2, 4, 0), (4, 1, 0)]);
    assert_eq!(max_diff(&run(&c), &solid_image([0.3, 0.3, 0.3, 1.0])), 0.0);
}

#[test]
fn transform_is_the_motion_matrix_draw() {
    let g = node(2, CompKind::Gradient, &[]);
    let tr = node(
        3,
        CompKind::Transform,
        &[("scale", ParamValue::Float(50.0)), ("rotation", ParamValue::Float(30.0)), ("position", ParamValue::Vec2(Vec2::new(10.0, 6.0)))],
    );
    let c = comp(vec![g.clone(), tr], &[(2, 3, 0), (3, 1, 0)]);
    let src = gradient(&g, Tick(0), W, H);
    let m = Affine::motion(Vec2::new(10.0, 6.0), Vec2::new(0.5, 0.5), 30.0, Vec2::new(W as f64 / 2.0, H as f64 / 2.0));
    assert_eq!(max_diff(&run(&c), &src.transformed(W, H, &m)), 0.0);
}

#[test]
fn blur_is_the_gaussian_blur_effect() {
    let g = node(2, CompKind::Gradient, &[("shape", ParamValue::Choice(1))]);
    let blur = node(3, CompKind::Blur, &[("blurriness", ParamValue::Float(6.0))]);
    let c = comp(vec![g.clone(), blur], &[(2, 3, 0), (3, 1, 0)]);
    let mut want = gradient(&g, Tick(0), W, H);
    let mut e = find_effect("gaussian_blur").unwrap().instance();
    e.params.insert("blurriness".into(), Param::new(ParamValue::Float(6.0)));
    let cx = FxCtx { t: Tick(0), px_scale: 1.0, seconds: 0.0, timecode: "", clip_name: "", project: None, env: None, working: Default::default() };
    gpufx::apply_ops(&FxOp::eval_all(&e, &cx, W, H).unwrap(), &mut want);
    let got = run(&c);
    assert_eq!(max_diff(&got, &want), 0.0);
    assert!(max_diff(&got, &gradient(&g, Tick(0), W, H)) > 1e-4, "the blur changed the picture");
}

#[test]
fn color_node_is_the_grade_compiler() {
    let mut gn = filmcraft_project::grade::GradeNode::new(1);
    gn.params.insert("saturation".into(), Param::new(ParamValue::Float(0.0)));
    let g = node(2, CompKind::Gradient, &[("color_b", ParamValue::Color([1.0, 0.2, 0.1, 1.0]))]);
    let c = comp(vec![g.clone(), node(3, CompKind::Color { node: gn.clone() }, &[])], &[(2, 3, 0), (3, 1, 0)]);
    let mut want = gradient(&g, Tick(0), W, H);
    let grade = Grade { nodes: vec![gn], ..Default::default() };
    let ops = crate::grade::compile_fx(&grade, Tick(0), c.color.working, Default::default(), W, H);
    assert!(!ops.is_empty());
    gpufx::apply_ops(&ops, &mut want);
    assert_eq!(max_diff(&run(&c), &want), 0.0);
}

#[test]
fn gradient_runs_from_start_to_end_colour() {
    let g = node(2, CompKind::Gradient, &[]);
    let img = gradient(&g, Tick(0), W, H);
    let px = |x: usize| img.px.get((H / 2 * W + x) * 4).copied().unwrap();
    assert!(px(0) < 0.05 && px(W - 1) > 0.9, "{} {}", px(0), px(W - 1));
    assert!((1..W).all(|x| px(x) >= px(x - 1)), "monotonic");
    // a large comp generates small and upscales
    let big = gradient(&g, Tick(0), 4000, 10);
    assert_eq!((big.w, big.h), (4000, 10));
}

struct Flat(Tick);

impl CompSources for Flat {
    fn media_in(&self, source: &MediaInSource, t: Tick, w: usize, h: usize) -> Option<Image> {
        // a picture whose red channel says which item and when it was asked for
        match source {
            MediaInSource::Item { item } if t == self.0 => Some(Image::filled(w, h, [item.0 as f32 / 10.0, 0.0, 0.0, 1.0])),
            _ => None,
        }
    }
}

#[test]
fn media_in_reads_its_source_at_comp_time_plus_offset() {
    let m = CompNode::new(2, CompKind::MediaIn { source: MediaInSource::Item { item: ItemId(3) }, offset: Tick(5) });
    let c = comp(vec![m], &[(2, 1, 0)]);
    let at = |t| execute_cpu(&compile(&c, CompCtx { t: Tick(t), ..ctx() }, &Flat(Tick(15))).unwrap());
    assert_eq!(max_diff(&at(10), &Image::filled(W, H, [0.3, 0.0, 0.0, 1.0])), 0.0);
    // nothing there at another time: transparent
    assert_eq!(max_diff(&at(11), &Image::new(W, H)), 0.0);
}

#[test]
fn bypass_and_missing_inputs() {
    // a bypassed Blur passes its input; a bypassed source is transparent
    let mut blur = node(3, CompKind::Blur, &[("blurriness", ParamValue::Float(9.0))]);
    blur.bypass = true;
    let g = node(2, CompKind::Gradient, &[]);
    let c = comp(vec![g.clone(), blur], &[(2, 3, 0), (3, 1, 0)]);
    assert_eq!(max_diff(&run(&c), &gradient(&g, Tick(0), W, H)), 0.0);
    let mut s = solid(2, [1.0; 4]);
    s.bypass = true;
    assert_eq!(max_diff(&run(&comp(vec![s], &[(2, 1, 0)])), &Image::new(W, H)), 0.0);
    // an unconnected Media Out and an effect node without input are transparent
    assert_eq!(max_diff(&run(&comp(vec![], &[])), &Image::new(W, H)), 0.0);
    let c = comp(vec![node(3, CompKind::Blur, &[])], &[(3, 1, 0)]);
    assert_eq!(max_diff(&run(&c), &Image::new(W, H)), 0.0);
}

#[test]
fn a_shared_input_is_not_overwritten_in_place() {
    // Solid 2 feeds a Mask-less Color node (in place) and the Merge's background
    let mut gn = filmcraft_project::grade::GradeNode::new(1);
    gn.params.insert("saturation".into(), Param::new(ParamValue::Float(0.0)));
    let col = [0.8, 0.2, 0.1, 1.0];
    let merge = node(4, CompKind::Merge, &[("opacity", ParamValue::Float(50.0))]);
    let c = comp(vec![solid(2, col), node(3, CompKind::Color { node: gn.clone() }, &[]), merge], &[(2, 3, 0), (2, 4, 0), (3, 4, 1), (4, 1, 0)]);
    let mut grey = solid_image(col);
    let grade = Grade { nodes: vec![gn], ..Default::default() };
    gpufx::apply_ops(&crate::grade::compile_fx(&grade, Tick(0), c.color.working, Default::default(), W, H), &mut grey);
    let mut want = solid_image(col);
    blend::composite(&mut want, &grey, 0.5, Blend::Normal);
    assert_eq!(max_diff(&run(&c), &want), 0.0);
    // a Merge of a picture with itself
    let c = comp(vec![solid(2, [0.5, 0.5, 0.5, 0.5]), node(4, CompKind::Merge, &[])], &[(2, 4, 0), (2, 4, 1), (4, 1, 0)]);
    let mut want = solid_image([0.5, 0.5, 0.5, 0.5]);
    blend::composite(&mut want, &solid_image([0.5, 0.5, 0.5, 0.5]), 1.0, Blend::Normal);
    assert_eq!(max_diff(&run(&c), &want), 0.0);
}

#[test]
fn liveness_keeps_texture_counts_small() {
    // acceptance comp A's shape: Media In → Key → Merge over Solid → Color → Out
    let gn = filmcraft_project::grade::GradeNode::new(1);
    let a = comp(
        vec![
            CompNode::new(2, CompKind::MediaIn { source: MediaInSource::Item { item: ItemId(1) }, offset: Tick(0) }),
            node(3, CompKind::from_key("key").unwrap(), &[]),
            solid(4, [0.0, 0.0, 1.0, 1.0]),
            node(5, CompKind::Merge, &[]),
            node(6, CompKind::Color { node: gn }, &[]),
        ],
        &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 6, 0), (6, 1, 0)],
    );
    assert!(compile(&a, ctx(), &NoSources).unwrap().textures <= 3);
    // a 62-step chain of in-place nodes and Transforms needs two
    let mut nodes = vec![solid(2, [1.0; 4])];
    let mut edges = vec![];
    for i in 3..64u64 {
        let kind = if i % 2 == 0 { CompKind::Transform } else { CompKind::Blur };
        nodes.push(node(i, kind, &[]));
        edges.push((i - 1, i, 0));
    }
    edges.push((63, 1, 0));
    assert_eq!(compile(&comp(nodes, &edges), ctx(), &NoSources).unwrap().textures, 2);
}

#[test]
fn too_many_live_pictures_and_bad_graphs_render_transparent() {
    // nine sources all merged at the end: more than MAX_LIVE_TEXTURES alive at once
    let mut nodes: Vec<CompNode> = (10..19).map(|i| solid(i, [1.0; 4])).collect();
    let mut edges = vec![];
    let mut prev = 10;
    for (k, i) in (11..19u64).enumerate() {
        let m = 30 + k as u64;
        nodes.push(node(m, CompKind::Merge, &[]));
        edges.push((prev, m, 0));
        edges.push((i, m, 1));
        prev = m;
    }
    edges.push((prev, 1, 0));
    // sources come first in id order, so all nine are alive before the first Merge
    assert_eq!(compile(&comp(nodes, &edges), ctx(), &NoSources).unwrap_err(), CompError::TooManyTextures);
    // a cycle
    let c = comp(vec![node(3, CompKind::Blur, &[]), node(4, CompKind::Blur, &[])], &[(3, 4, 0), (4, 3, 0), (4, 1, 0)]);
    assert!(matches!(compile(&c, ctx(), &NoSources), Err(CompError::Graph(_))));
    assert_eq!(max_diff(&render_cpu(&c, ctx(), &NoSources), &Image::new(W, H)), 0.0);
    // a zero-size request still gives a 1×1 picture
    let tiny = render_cpu(&c, CompCtx { size: (0, 0), ..ctx() }, &NoSources);
    assert_eq!((tiny.w, tiny.h), (1, 1));
}

#[test]
fn text_is_the_graphic_text_layer() {
    let CompKind::Text { mut layer } = CompKind::from_key("text").unwrap() else { panic!("text kind") };
    layer.params.insert("text".into(), Param::new(ParamValue::Text("Hi".into())));
    layer.params.insert("size".into(), Param::new(ParamValue::Float(14.0)));
    let c = comp(vec![CompNode::new(2, CompKind::Text { layer: layer.clone() })], &[(2, 1, 0)]);
    let mut want = Image::new(W, H);
    crate::graphic_clip::render_graphic_layers(std::slice::from_ref(&*layer), Tick(0), (W as u32, H as u32), &Affine::scale(1.0, 1.0), &mut want);
    let got = run(&c);
    assert_eq!(max_diff(&got, &want), 0.0);
    assert!(got.px.chunks(4).any(|p| p[3] > 0.5), "glyphs drawn");
    // at half size the same layer is drawn scaled, still within the canvas
    let half = execute_cpu(&compile(&c, CompCtx { size: (W / 2, H / 2), ..ctx() }, &NoSources).unwrap());
    assert_eq!((half.w, half.h), (W / 2, H / 2));
}

/// Acceptance comp B's shape: a title moved by keyframes (as a tracker writes them) over a
/// background: the text follows the keyframed position.
#[test]
fn a_keyframed_transform_moves_a_title_over_a_background() {
    let CompKind::Text { mut layer } = CompKind::from_key("text").unwrap() else { panic!("text kind") };
    layer.params.insert("text".into(), Param::new(ParamValue::Text("II".into())));
    layer.params.insert("size".into(), Param::new(ParamValue::Float(12.0)));
    let mut pos = Param::new(ParamValue::Vec2(Vec2::new(8.0, 9.0)));
    pos.keyframes = vec![
        filmcraft_project::Keyframe::new(Tick(0), ParamValue::Vec2(Vec2::new(8.0, 9.0))),
        filmcraft_project::Keyframe::new(Tick(1000), ParamValue::Vec2(Vec2::new(24.0, 9.0))),
    ];
    let mut tr = CompNode::new(3, CompKind::Transform);
    tr.params.insert("position".into(), pos);
    let c = comp(
        vec![CompNode::new(2, CompKind::Text { layer }), tr, solid(4, [0.0, 0.0, 0.2, 1.0]), node(5, CompKind::Merge, &[])],
        &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 1, 0)],
    );
    // the bright pixels' mean x at each time
    let centre_x = |t: i64| {
        let img = execute_cpu(&compile(&c, CompCtx { t: Tick(t), ..ctx() }, &NoSources).unwrap());
        let xs: Vec<f32> = img.px.chunks(4).enumerate().filter(|(_, p)| p[0] > 0.5).map(|(i, _)| (i % W) as f32).collect();
        assert!(!xs.is_empty(), "title drawn at {t}");
        xs.iter().sum::<f32>() / xs.len() as f32
    };
    let (a, b) = (centre_x(0), centre_x(1000));
    assert!((a - 8.0).abs() < 3.0 && (b - 24.0).abs() < 3.0, "{a} {b}");
    assert!((centre_x(500) - 16.0).abs() < 3.0);
}

fn title(id: NodeId, text: &str) -> CompNode {
    let CompKind::Text { mut layer } = CompKind::from_key("text").unwrap() else { panic!("text kind") };
    layer.params.insert("text".into(), Param::new(ParamValue::Text(text.into())));
    layer.params.insert("size".into(), Param::new(ParamValue::Float(14.0)));
    CompNode::new(id, CompKind::Text { layer })
}

fn media(id: NodeId) -> CompNode {
    CompNode::new(id, CompKind::MediaIn { source: MediaInSource::Item { item: ItemId(3) }, offset: Tick::ZERO })
}

/// The cached compile of `c` at the Flat source's time, and the uncached one.
fn both(c: &Comp) -> (Program, Image, Image) {
    let cx = CompCtx { t: Tick(15), ..ctx() };
    let cached = compile_cached(c, cx, &Flat(Tick(15))).unwrap();
    let plain = execute_cpu(&compile(c, cx, &Flat(Tick(15))).unwrap());
    let img = execute_cpu(&cached);
    (cached, img, plain)
}

#[test]
fn a_static_title_is_cached_and_renders_the_same() {
    // Text → Transform → Merge over a Media In: the title subgraph renders once
    let c = comp(
        vec![title(2, "Cache me"), node(3, CompKind::Transform, &[("scale", ParamValue::Float(80.0))]), media(4), node(5, CompKind::Merge, &[])],
        &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 1, 0)],
    );
    let order = c.validate().unwrap();
    let tops = static_tops(&c, &order);
    assert_eq!(tops, vec![(3, BTreeSet::from([2, 3]))]);
    let (p1, img, plain) = both(&c);
    assert_eq!(max_diff(&img, &plain), 0.0);
    let image_of = |p: &Program| -> Vec<Arc<Image>> {
        p.passes.iter().filter_map(|x| if let Pass::Image { image, .. } = x { Some(Arc::clone(image)) } else { None }).collect()
    };
    // Media In + the cached title, no Draw of the title any more
    assert_eq!(image_of(&p1).len(), 2);
    assert!(!p1.passes.iter().any(|x| matches!(x, Pass::Draw { .. })));
    // a second compile reuses the same picture; another text is another entry
    let title_of = |p: &Program| image_of(p).into_iter().find(|i| i.px.chunks(4).any(|q| q[3] < 0.5)).unwrap();
    let (p2, ..) = both(&c);
    assert!(Arc::ptr_eq(&title_of(&p1), &title_of(&p2)));
    let mut other = c.clone();
    other.nodes[1] = title(2, "Other");
    let (p3, img3, plain3) = both(&other);
    assert_eq!(max_diff(&img3, &plain3), 0.0);
    assert!(!Arc::ptr_eq(&title_of(&p1), &title_of(&p3)));
}

#[test]
fn keyframes_footage_and_color_nodes_are_not_cached() {
    let mut moving = node(3, CompKind::Transform, &[]);
    let mut pos = Param::new(ParamValue::Vec2(Vec2::new(0.0, 0.0)));
    pos.keyframes.push(filmcraft_project::Keyframe::new(Tick(0), ParamValue::Vec2(Vec2::new(0.0, 0.0))));
    pos.keyframes.push(filmcraft_project::Keyframe::new(Tick(100), ParamValue::Vec2(Vec2::new(10.0, 5.0))));
    moving.params.insert("position".into(), pos);
    let c = comp(vec![title(2, "Moving"), moving, media(4), node(5, CompKind::Merge, &[])], &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 1, 0)]);
    // only the title itself (read by the keyframed Transform) is static
    assert_eq!(static_tops(&c, &c.validate().unwrap()), vec![(2, BTreeSet::from([2]))]);
    let (_, img, plain) = both(&c);
    assert_eq!(max_diff(&img, &plain), 0.0);
    // a lone Solid over footage is not worth a cache entry; a Color node never is
    let c = comp(vec![solid(2, [1.0, 0.0, 0.0, 1.0]), media(3), node(4, CompKind::Merge, &[])], &[(3, 4, 0), (2, 4, 1), (4, 1, 0)]);
    assert!(static_tops(&c, &c.validate().unwrap()).is_empty());
    let c = comp(vec![title(2, "Graded"), CompNode::new(3, CompKind::from_key("color").unwrap())], &[(2, 3, 0), (3, 1, 0)]);
    assert_eq!(static_tops(&c, &c.validate().unwrap()), vec![(2, BTreeSet::from([2]))]);
}

#[test]
fn a_cached_node_still_read_elsewhere_is_still_compiled() {
    // Solid → Mask (cached with its Solid) → Merge over footage, and the same Solid → a second Merge
    let mask = CompNode::new(3, CompKind::Mask { masks: Vec::new() });
    let c = comp(
        vec![solid(2, [0.0, 1.0, 0.0, 0.5]), mask, media(4), node(5, CompKind::Merge, &[]), node(6, CompKind::Merge, &[])],
        &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 6, 0), (2, 6, 1), (6, 1, 0)],
    );
    let (_, img, plain) = both(&c);
    assert_eq!(max_diff(&img, &plain), 0.0);
    assert!(img.px.chunks(4).any(|q| q[1] > 0.1), "the solid is drawn");
}

#[test]
fn the_static_cache_stays_within_its_bytes() {
    let mut cache = StaticCache::default();
    let img = |n: usize| Arc::new(Image::new(n, n));
    let cap = image_bytes(&img(10)) * 2;
    cache.put("a".into(), img(10), cap);
    cache.put("b".into(), img(10), cap);
    assert!(cache.get("a").is_some());
    cache.put("c".into(), img(10), cap);
    assert!(cache.get("b").is_none(), "least recently used out first");
    assert!(cache.get("a").is_some() && cache.get("c").is_some());
    cache.put("huge".into(), img(100), cap);
    assert!(cache.get("huge").is_none());
    assert!(cache.bytes <= cap);
    cache.put("a".into(), img(10), cap);
    assert_eq!(cache.bytes, cap);
}
