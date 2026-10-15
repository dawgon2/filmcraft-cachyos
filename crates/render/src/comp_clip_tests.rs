//! Comp clips (fork C1) on the timeline: the graph renders at comp time through the CPU renderer
//! and the frame plan, with Media In items and the tracks below the comp clip as sources.

use super::*;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::comp::{Comp, CompKind, CompNode, MediaInSource};
use filmcraft_project::dag::{Edge, NodeId, Port};
use filmcraft_project::{ClipId, Label, MediaClip, MediaRef, Param, ParamValue, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, TimeRange};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const W: u32 = 64;
const H: u32 = 36;

struct Rig {
    p: Project,
    map: SourceMap,
}

impl Rig {
    fn new() -> Rig {
        Rig { p: Project::new("comps"), map: SourceMap::default() }
    }

    fn matte(&mut self, color: [f32; 4]) -> ItemId {
        let g = GeneratorSource::new(Generator::ColorMatte { color }, W, H, FrameRate::FPS_24, Tick(10 * TICKS_PER_SECOND));
        let info = g.info().clone();
        let kind = ItemKind::Media(MediaClip {
            media: MediaRef::Generator(g.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        });
        let id = self.p.add_item("matte", Label::Iris, kind, None);
        self.map.0.insert(id, Arc::new(g) as SharedSource);
        id
    }

    fn comp(&mut self, nodes: Vec<CompNode>, edges: &[(NodeId, NodeId, u8)]) -> ItemId {
        let mut c = Comp::new(W, H, FrameRate::FPS_24, Tick(10 * TICKS_PER_SECOND));
        c.nodes.extend(nodes);
        c.edges = edges.iter().map(|&(f, t, p)| Edge::new(Port::new(f, 0), Port::new(t, p))).collect();
        self.p.add_item("comp", Label::Iris, ItemKind::Comp(Arc::new(c)), None)
    }

    fn seq(&mut self) -> ItemId {
        self.p.new_sequence("seq", SequenceSettings { width: W, height: H, frame_rate: FrameRate::FPS_24, ..Default::default() }, 2, 1, None)
    }

    fn put(&mut self, seq: ItemId, track: usize, item: ItemId) -> ClipId {
        let rate = FrameRate::FPS_24;
        let ti = self.p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(48)), rate).unwrap();
        let id = ti.id;
        self.p.sequence_mut(seq).unwrap().video_tracks[track].items.push(ti);
        id
    }

    /// The frame through the CPU renderer and through the frame plan.
    fn frames(&self, seq: ItemId) -> [Image; 2] {
        let t = Tick(1000);
        [
            render_sequence(&self.p, seq, t, RenderOptions::default(), &self.map),
            plan::execute_cpu(&plan::plan_frame(&self.p, seq, t, RenderOptions::default(), &self.map)),
        ]
    }
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.02)
}

fn node(id: NodeId, kind: CompKind, params: &[(&str, ParamValue)]) -> CompNode {
    let mut n = CompNode::new(id, kind);
    for (k, v) in params {
        n.params.insert((*k).to_string(), Param::new(v.clone()));
    }
    n
}

fn media_in(id: NodeId, source: MediaInSource) -> CompNode {
    CompNode::new(id, CompKind::MediaIn { source, offset: Tick::ZERO })
}

/// Media In (`source`) at 50 % over a blue Solid.
fn over_blue(r: &mut Rig, source: MediaInSource) -> ItemId {
    r.comp(
        vec![
            media_in(2, source),
            node(3, CompKind::Transform, &[("scale", ParamValue::Float(50.0))]),
            node(4, CompKind::Solid, &[("color", ParamValue::Color([0.0, 0.0, 1.0, 1.0]))]),
            node(5, CompKind::Merge, &[]),
        ],
        &[(2, 3, 0), (4, 5, 0), (3, 5, 1), (5, 1, 0)],
    )
}

#[test]
fn a_comp_clip_shows_its_graph() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let comp = over_blue(&mut r, MediaInSource::Item { item: red });
    let seq = r.seq();
    r.put(seq, 0, comp);
    for img in r.frames(seq) {
        assert_eq!((img.w, img.h), (W as usize, H as usize));
        assert!(close(img.get(32, 18), RED), "{:?}", img.get(32, 18));
        assert!(close(img.get(2, 2), [0.0, 0.0, 1.0, 1.0]), "{:?}", img.get(2, 2));
    }
}

#[test]
fn a_layer_source_reads_only_tracks_below_the_comp_clip() {
    let mut r = Rig::new();
    let green = r.matte(GREEN);
    let seq = r.seq();
    r.put(seq, 0, green);
    // V2's comp reads V1 at half size over blue
    let below = over_blue(&mut r, MediaInSource::Layer { track: 0 });
    r.put(seq, 1, below);
    for img in r.frames(seq) {
        assert!(close(img.get(32, 18), GREEN) && close(img.get(2, 2), [0.0, 0.0, 1.0, 1.0]));
    }
    // reading its own track (or above) gives nothing: blue only
    let mut r2 = Rig::new();
    let green = r2.matte(GREEN);
    let seq = r2.seq();
    r2.put(seq, 0, green);
    let own = over_blue(&mut r2, MediaInSource::Layer { track: 1 });
    r2.put(seq, 1, own);
    for img in r2.frames(seq) {
        assert!(close(img.get(32, 18), [0.0, 0.0, 1.0, 1.0]), "{:?}", img.get(32, 18));
    }
}

#[test]
fn a_comp_reading_a_comp_and_a_broken_comp() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let inner = r.comp(vec![media_in(2, MediaInSource::Item { item: red })], &[(2, 1, 0)]);
    let outer = over_blue(&mut r, MediaInSource::Item { item: inner });
    let seq = r.seq();
    r.put(seq, 0, outer);
    for img in r.frames(seq) {
        assert!(close(img.get(32, 18), RED));
    }
    // a comp with a cycle renders transparent (nothing drawn), it doesn't fail the frame
    let mut r = Rig::new();
    let broken = r.comp(vec![node(3, CompKind::Blur, &[]), node(4, CompKind::Blur, &[])], &[(3, 4, 0), (4, 3, 0), (4, 1, 0)]);
    let seq = r.seq();
    r.put(seq, 0, broken);
    for img in r.frames(seq) {
        assert!(img.get(32, 18)[3] < 0.02);
    }
}

#[test]
fn the_node_viewer_shows_one_node_s_output() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let comp = over_blue(&mut r, MediaInSource::Item { item: red });
    let seq = r.seq();
    let clip = r.put(seq, 0, comp);
    let view = |r: &Rig, node| render_comp_node(&r.p, seq, clip, node, Tick(1000), RenderOptions::default(), &r.map);
    // the Solid alone is blue everywhere; the Transform alone is red in the middle, clear outside
    let solid = view(&r, 4).expect("the Solid's output");
    assert_eq!((solid.w, solid.h), (W as usize, H as usize));
    assert!(close(solid.get(32, 18), [0.0, 0.0, 1.0, 1.0]) && close(solid.get(2, 2), [0.0, 0.0, 1.0, 1.0]));
    let scaled = view(&r, 3).expect("the Transform's output");
    assert!(close(scaled.get(32, 18), RED) && scaled.get(2, 2)[3] < 0.02, "{:?}", scaled.get(2, 2));
    // the Media Out is the clip's own picture
    assert!(close(view(&r, 1).expect("Media Out").get(2, 2), [0.0, 0.0, 1.0, 1.0]));
    // a node, clip or sequence that doesn't exist, or a clip that isn't a comp clip, gives nothing
    assert!(view(&r, 99).is_none());
    assert!(render_comp_node(&r.p, seq, ClipId(987_654), 1, Tick(1000), RenderOptions::default(), &r.map).is_none());
    assert!(render_comp_node(&r.p, ItemId(987_654), clip, 1, Tick(1000), RenderOptions::default(), &r.map).is_none());
    let plain = r.put(seq, 1, red);
    assert!(render_comp_node(&r.p, seq, plain, 1, Tick(1000), RenderOptions::default(), &r.map).is_none());
    // a hostile scale is clamped, not trusted
    for scale in [f32::NAN, -1.0, 0.0, 1e9] {
        let img = render_comp_node(&r.p, seq, clip, 4, Tick(1000), RenderOptions { scale, ..Default::default() }, &r.map).unwrap();
        assert_eq!((img.w, img.h), (W as usize, H as usize), "{scale}");
    }
}

/// C1 step 7: a comp clip without effects of its own reaches the frame plan as one comp layer
/// (its program, placed by Motion) instead of a CPU-rendered canvas, and both renderers agree;
/// at 50 % Motion scale the program is compiled at half the comp size.
#[test]
fn a_plain_comp_clip_is_one_comp_layer_in_the_plan() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let comp = over_blue(&mut r, MediaInSource::Item { item: red });
    let seq = r.seq();
    let clip = r.put(seq, 0, comp);
    let t = Tick(1000);
    let comp_layers = |r: &Rig| match plan::plan_frame(&r.p, seq, t, RenderOptions::default(), &r.map) {
        plan::FramePlan::Layers { layers, .. } => layers.iter().filter_map(|l| l.comp.as_ref().map(|p| p.size)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(comp_layers(&r), vec![(W as usize, H as usize)]);
    // half size: compiled at half the comp size, drawn in the middle of a black frame
    let s = r.p.sequence_mut(seq).unwrap();
    let it = s.video_tracks[0].items.iter_mut().find(|i| i.id == clip).unwrap();
    it.effect_mut("motion").unwrap().params.get_mut("scale").unwrap().value = ParamValue::Float(50.0);
    assert_eq!(comp_layers(&r), vec![(W as usize / 2, H as usize / 2)]);
    let [cpu, planned] = r.frames(seq);
    for (x, y) in [(32, 18), (20, 12), (2, 2)] {
        assert!(close(cpu.get(x, y), planned.get(x, y)), "({x}, {y}): {:?} vs {:?}", cpu.get(x, y), planned.get(x, y));
    }
    assert!(close(planned.get(32, 18), RED), "{:?}", planned.get(32, 18));
    // (outside the half-size comp: nothing drawn, no blue)
    assert!(planned.get(2, 2)[2] < 0.02, "{:?}", planned.get(2, 2));
}

/// C1 owed: a Media In of a media item reaches the program as a frame-plan layer (the GPU draws
/// the decoded frame), not as a CPU image, and the picture is unchanged.
#[test]
fn an_item_source_compiles_to_a_plan_layer() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let comp = over_blue(&mut r, MediaInSource::Item { item: red });
    let seq = r.seq();
    r.put(seq, 0, comp);
    let t = Tick(1000);
    let plan::FramePlan::Layers { layers, .. } = plan::plan_frame(&r.p, seq, t, RenderOptions::default(), &r.map) else { panic!("layers") };
    let program = layers.iter().find_map(|l| l.comp.clone()).expect("a comp layer");
    let kinds: Vec<&str> = program
        .passes
        .iter()
        .map(|p| match p {
            comp::Pass::Layers { .. } => "layers",
            comp::Pass::Image { .. } => "image",
            _ => "other",
        })
        .collect();
    assert!(kinds.contains(&"layers") && !kinds.contains(&"image"), "{kinds:?}");
    let [cpu, planned] = r.frames(seq);
    for (x, y) in [(32, 18), (2, 2), (17, 10)] {
        assert!(close(cpu.get(x, y), planned.get(x, y)), "({x}, {y}): {:?} vs {:?}", cpu.get(x, y), planned.get(x, y));
    }
}

/// A media frame layer letterboxes into the canvas like `fit`, and an empty canvas gives none.
#[test]
fn a_media_frame_layer_fits_the_canvas() {
    let mut r = Rig::new();
    let red = r.matte(RED);
    let frame = Arc::new(filmcraft_frame::VideoFrame::rgba_f32(64, 36, vec![1.0; 64 * 36 * 4]));
    let pipe = filmcraft_color::ColorPipeline::default();
    let l = plan::media_frame_layer(&r.p, red, Arc::clone(&frame), &pipe, 32, 32).expect("a plain layer");
    let a = l.matrix.apply(filmcraft_geom::Vec2::new(0.0, 0.0));
    let b = l.matrix.apply(filmcraft_geom::Vec2::new(64.0, 36.0));
    let (x0, y0, x1, y1) = (a.x, a.y, b.x, b.y);
    assert!((x0.abs() + (x1 - 32.0).abs()) < 1e-6 && (y0 - 7.0).abs() < 1e-6 && (y1 - 25.0).abs() < 1e-6, "{x0} {y0} {x1} {y1}");
    assert!(plan::media_frame_layer(&r.p, red, Arc::clone(&frame), &pipe, 0, 32).is_none());
}

/// C1 step 7d: a Media In reading a track below reaches the program as frame-plan layers (the
/// GPU draws the decoded frames), not as a CPU image, and the picture is unchanged.
#[test]
fn a_layer_source_compiles_to_plan_layers() {
    let mut r = Rig::new();
    let green = r.matte(GREEN);
    let seq = r.seq();
    r.put(seq, 0, green);
    let below = over_blue(&mut r, MediaInSource::Layer { track: 0 });
    r.put(seq, 1, below);
    let t = Tick(1000);
    let plan::FramePlan::Layers { layers, .. } = plan::plan_frame(&r.p, seq, t, RenderOptions::default(), &r.map) else { panic!("layers") };
    let program = layers.iter().find_map(|l| l.comp.clone()).expect("a comp layer");
    let kinds: Vec<&str> = program
        .passes
        .iter()
        .map(|p| match p {
            comp::Pass::Layers { .. } => "layers",
            comp::Pass::Image { .. } => "image",
            _ => "other",
        })
        .collect();
    assert!(kinds.contains(&"layers") && !kinds.contains(&"image"), "{kinds:?}");
    let [cpu, planned] = r.frames(seq);
    for (x, y) in [(32, 18), (2, 2)] {
        assert!(close(cpu.get(x, y), planned.get(x, y)), "({x}, {y}): {:?} vs {:?}", cpu.get(x, y), planned.get(x, y));
    }
}
