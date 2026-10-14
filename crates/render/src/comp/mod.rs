//! Comp evaluation (fork C1, ADR 0007): a validated comp graph becomes a [`Program`] of passes on
//! comp-sized textures, which the CPU runs here ([`execute_cpu`], the oracle and export path) and
//! the GPU runs with the same passes (C1 step 7). Every pass is existing maths: blend modes
//! ([`crate::blend`]), the Motion matrix ([`Affine::motion`]), effect ops ([`FxOp`]: keys, blur,
//! masks, one grade node through [`crate::grade::compile_fx`]). The one new picture is the
//! gradient generator.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock, PoisonError};

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::comp::{Comp, CompKind, CompNode, MediaInSource};
use filmcraft_project::dag::{DagError, NodeId, Port};
use filmcraft_project::grade::Grade;
use filmcraft_project::{EffectInstance, ParamValue, Project, find_effect};
use filmcraft_time::Tick;

use crate::blend::{self, Blend};
use crate::effects::{FxCtx, dec};
use crate::gpufx::{self, FxOp};
use crate::image::Image;

/// Most comp-sized textures a program may hold at once.
pub const MAX_LIVE_TEXTURES: u16 = 8;
/// Longest side of a generated gradient; larger comps upscale it (a gradient upscales exactly).
pub const GRADIENT_MAX_SIDE: usize = 2048;

/// A texture slot of a program (comp-sized, premultiplied linear light).
pub type Tex = u16;

/// Why a comp can't be evaluated (it then renders transparent).
#[derive(Debug, Clone, PartialEq)]
pub enum CompError {
    Graph(DagError),
    TooManyTextures,
    Empty,
}

impl std::fmt::Display for CompError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompError::Graph(e) => e.fmt(f),
            CompError::TooManyTextures => write!(f, "the comp needs more than {MAX_LIVE_TEXTURES} pictures at once"),
            CompError::Empty => write!(f, "the comp is empty"),
        }
    }
}

impl std::error::Error for CompError {}

impl From<DagError> for CompError {
    fn from(e: DagError) -> Self {
        CompError::Graph(e)
    }
}

/// One step of a program.
#[derive(Clone, Debug)]
pub enum Pass {
    /// `dst` = a picture (Media In, Text, Gradient), already at the program size.
    Image { dst: Tex, image: Arc<Image> },
    /// `dst` = these frame-plan layers drawn on transparency (a Media In reading a track: its
    /// decoded or hardware frames, effects and Motion as on the timeline; C1 step 7d).
    Layers { dst: Tex, layers: Arc<[crate::plan::PlanLayer]> },
    /// `dst` = one premultiplied colour everywhere (Solid; transparent for a missing input).
    Fill { dst: Tex, color: [f32; 4] },
    /// `dst` = `src` (before an in-place pass on a picture another node still needs).
    Copy { src: Tex, dst: Tex },
    /// Effect ops on `tex` in place (Key, Mask, Blur, Color).
    Fx { tex: Tex, ops: Vec<FxOp> },
    /// `dst` = `src` drawn through `xform` (source pixels → destination pixels) on transparency.
    Draw { src: Tex, dst: Tex, xform: Affine, hq: bool },
    /// `src` composited over `dst` (Merge).
    Over { src: Tex, dst: Tex, mode: Blend, opacity: f32 },
}

/// A comp ready to run: passes on `textures` slots of `size`, the result in `out`.
#[derive(Clone, Debug)]
pub struct Program {
    pub size: (usize, usize),
    /// Peak number of slots used at once.
    pub textures: u16,
    pub passes: Vec<Pass>,
    pub out: Tex,
}

impl Program {
    /// The memory its pictures hold (the CPU images it carries; slots are allocated when it runs).
    pub fn byte_size(&self) -> usize {
        let size = |p: &Pass| match p {
            Pass::Image { image, .. } => image.px.len().saturating_mul(4),
            Pass::Layers { layers, .. } => layers.iter().map(|l| l.frame.byte_size()).fold(0, usize::saturating_add),
            _ => 0,
        };
        self.passes.iter().map(size).fold(0, usize::saturating_add)
    }
}

/// Pictures a comp reads from outside itself.
pub trait CompSources {
    /// A Media In node's picture at comp time `t` (the node's offset already added), fitted into a
    /// `w`×`h` canvas (transparent outside). None: nothing there (offline, outside the clip).
    fn media_in(&self, source: &MediaInSource, t: Tick, w: usize, h: usize) -> Option<Image>;
    /// The same picture as frame-plan layers placed on a `w`×`h` canvas, when the source can give
    /// them (the GPU then draws the frames itself); None: use [`media_in`](Self::media_in).
    fn media_in_layers(&self, _source: &MediaInSource, _t: Tick, _w: usize, _h: usize) -> Option<Vec<crate::plan::PlanLayer>> {
        None
    }
    /// A text node's layer rasterised on a transparent `w`×`h` canvas (`px_scale` image pixels per
    /// comp pixel), exactly as a graphic clip draws it (layer rasters are cached by
    /// `graphic_clip`). None: nothing drawn.
    fn text(&self, layer: &EffectInstance, t: Tick, w: usize, h: usize, px_scale: f32) -> Option<Image> {
        let s = if px_scale.is_finite() && px_scale > 0.0 { px_scale } else { 1.0 };
        let canvas = (((w as f32 / s).round() as u32).max(1), ((h as f32 / s).round() as u32).max(1));
        let mut img = Image::new(w, h);
        crate::graphic_clip::render_graphic_layers(std::slice::from_ref(layer), t, canvas, &Affine::scale(f64::from(s), f64::from(s)), &mut img);
        Some(img)
    }
}

/// No outside pictures (tests, comps without Media In).
pub struct NoSources;

impl CompSources for NoSources {
    fn media_in(&self, _: &MediaInSource, _: Tick, _: usize, _: usize) -> Option<Image> {
        None
    }
}

/// Where a comp is evaluated.
#[derive(Clone, Copy)]
pub struct CompCtx<'a> {
    /// Comp time.
    pub t: Tick,
    /// The program's pixel size (the comp size × render scale; at least 1×1).
    pub size: (usize, usize),
    /// The project (LUT library of Color nodes).
    pub project: Option<&'a Project>,
    /// Maximum Render Quality for Transform draws.
    pub high_quality: bool,
}

/// Texture slots handed out by liveness: a node's slot is reused once its last consumer ran.
struct Slots {
    free: Vec<Tex>,
    next: Tex,
    peak: u16,
}

impl Slots {
    fn take(&mut self) -> Result<Tex, CompError> {
        if let Some(t) = self.free.pop() {
            return Ok(t);
        }
        if self.next >= MAX_LIVE_TEXTURES {
            return Err(CompError::TooManyTextures);
        }
        let t = self.next;
        self.next += 1;
        self.peak = self.peak.max(self.next);
        Ok(t)
    }
    fn give(&mut self, t: Tex) {
        if !self.free.contains(&t) {
            self.free.push(t);
        }
    }
}

/// Compiles `comp` at `ctx.t` into a program of `ctx.size` (validating the graph first).
pub fn compile(comp: &Comp, ctx: CompCtx, sources: &dyn CompSources) -> Result<Program, CompError> {
    compile_inner(comp, ctx, sources, &Prerendered::default())
}

/// Static-subgraph pictures a compile takes instead of their nodes: `images` stand in for their
/// node, the `skip` nodes are only read by those (so they emit nothing).
#[derive(Default)]
struct Prerendered {
    images: BTreeMap<NodeId, Arc<Image>>,
    skip: BTreeSet<NodeId>,
}

fn compile_inner(comp: &Comp, ctx: CompCtx, sources: &dyn CompSources, pre: &Prerendered) -> Result<Program, CompError> {
    let order = comp.validate()?;
    let (w, h) = (ctx.size.0.max(1), ctx.size.1.max(1));
    let px_scale = (w as f64 / f64::from(comp.width.max(1))) as f32;
    let nodes: BTreeMap<NodeId, &CompNode> = comp.nodes.iter().map(|n| (n.id, n)).collect();
    let last_use = order.last_use();
    let mut slots = Slots { free: Vec::new(), next: 0, peak: 0 };
    let mut result: BTreeMap<NodeId, Tex> = BTreeMap::new();
    let mut passes = Vec::new();
    let mut out = None;

    for (pos, step) in order.steps.iter().enumerate() {
        let Some(node) = nodes.get(&step.node) else { continue };
        let input = |i: usize| -> Option<Tex> { step.inputs.get(i).copied().flatten().and_then(|p: Port| result.get(&p.node).copied()) };
        // A slot that this step may overwrite: input `i`'s own slot when nothing later reads it,
        // otherwise a copy (a missing input gives a cleared slot).
        let writable = |i: usize, slots: &mut Slots, passes: &mut Vec<Pass>| -> Result<Tex, CompError> {
            let src_node = step.inputs.get(i).copied().flatten().map(|p| p.node);
            match (input(i), src_node) {
                (Some(src), Some(n)) if last_use.get(&n).copied() == Some(pos) && !reads_twice(&step.inputs, n) => Ok(src),
                (Some(src), _) => {
                    let dst = slots.take()?;
                    passes.push(Pass::Copy { src, dst });
                    Ok(dst)
                }
                (None, _) => {
                    let dst = slots.take()?;
                    passes.push(Pass::Fill { dst, color: [0.0; 4] });
                    Ok(dst)
                }
            }
        };
        let t = ctx.t;
        if pre.skip.contains(&node.id) {
            continue;
        }
        let tex = if let Some(image) = pre.images.get(&node.id) {
            let dst = slots.take()?;
            passes.push(Pass::Image { dst, image: Arc::clone(image) });
            dst
        } else if node.bypass {
            if node.kind.inputs() == 0 {
                let dst = slots.take()?;
                passes.push(Pass::Fill { dst, color: [0.0; 4] });
                dst
            } else {
                writable(0, &mut slots, &mut passes)?
            }
        } else {
            match &node.kind {
                CompKind::MediaIn { source, offset } => {
                    let dst = slots.take()?;
                    let at = Tick(t.0.saturating_add(offset.0));
                    if let Some(layers) = sources.media_in_layers(source, at, w, h) {
                        passes.push(Pass::Layers { dst, layers: layers.into() });
                    } else {
                        match sources.media_in(source, at, w, h).filter(|i| (i.w, i.h) == (w, h)) {
                            Some(image) => passes.push(Pass::Image { dst, image: Arc::new(image) }),
                            None => passes.push(Pass::Fill { dst, color: [0.0; 4] }),
                        }
                    }
                    dst
                }
                CompKind::Text { layer } => {
                    let dst = slots.take()?;
                    match sources.text(layer, t, w, h, px_scale).filter(|i| (i.w, i.h) == (w, h)) {
                        Some(image) => passes.push(Pass::Image { dst, image: Arc::new(image) }),
                        None => passes.push(Pass::Fill { dst, color: [0.0; 4] }),
                    }
                    dst
                }
                CompKind::Solid => {
                    let dst = slots.take()?;
                    passes.push(Pass::Fill { dst, color: premul(color(node, "color", t)) });
                    dst
                }
                CompKind::Gradient => {
                    let dst = slots.take()?;
                    passes.push(Pass::Image { dst, image: Arc::new(gradient(node, t, w, h)) });
                    dst
                }
                CompKind::MediaOut => writable(0, &mut slots, &mut passes)?,
                CompKind::Merge => {
                    let dst = writable(0, &mut slots, &mut passes)?;
                    if let Some(src) = input(1) {
                        let opacity = (num(node, "opacity", t, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
                        let mode = Blend::from_index(choice(node, "blend", t));
                        passes.push(Pass::Over { src, dst, mode, opacity });
                    }
                    dst
                }
                CompKind::Transform => {
                    let dst = slots.take()?;
                    match input(0) {
                        Some(src) => passes.push(Pass::Draw { src, dst, xform: transform(node, t, (w, h), px_scale), hq: ctx.high_quality }),
                        None => passes.push(Pass::Fill { dst, color: [0.0; 4] }),
                    }
                    dst
                }
                CompKind::Key { .. } | CompKind::Mask { .. } | CompKind::Blur | CompKind::Color { .. } => {
                    let tex = writable(0, &mut slots, &mut passes)?;
                    let ops = node_ops(node, &ctx, comp.color.working, w, h, px_scale);
                    if !ops.is_empty() {
                        passes.push(Pass::Fx { tex, ops });
                    }
                    tex
                }
            }
        };
        result.insert(node.id, tex);
        out = Some(tex);
        // inputs whose last consumer was this step free their slots (unless this node took it)
        for p in step.inputs.iter().flatten() {
            if last_use.get(&p.node).copied() == Some(pos)
                && let Some(&s) = result.get(&p.node)
                && s != tex
            {
                slots.give(s);
            }
        }
    }
    let out = out.ok_or(CompError::Empty)?;
    Ok(Program { size: (w, h), textures: slots.peak, passes, out })
}

/// Whether `node` feeds more than one input port of the same step (a Merge of a picture with
/// itself): its slot must not be overwritten in place then.
fn reads_twice(inputs: &[Option<Port>], node: NodeId) -> bool {
    inputs.iter().flatten().filter(|p| p.node == node).count() > 1
}

/// Runs a program on the CPU.
pub fn execute_cpu(p: &Program) -> Image {
    let (w, h) = p.size;
    let mut tex: Vec<Option<Image>> = vec![None; usize::from(p.textures)];
    let blank = || Image::new(w, h);
    for pass in &p.passes {
        match pass {
            Pass::Image { dst, image } => put(&mut tex, *dst, (**image).clone()),
            Pass::Layers { dst, layers } => {
                let mut img = blank();
                crate::plan::execute_layers(&mut img, layers);
                put(&mut tex, *dst, img);
            }
            Pass::Fill { dst, color } => put(&mut tex, *dst, if *color == [0.0; 4] { blank() } else { Image::filled(w, h, *color) }),
            Pass::Copy { src, dst } => {
                let img = get(&tex, *src).cloned().unwrap_or_else(blank);
                put(&mut tex, *dst, img);
            }
            Pass::Fx { tex: t, ops } => {
                if let Some(img) = tex.get_mut(usize::from(*t)).and_then(Option::as_mut) {
                    gpufx::apply_ops(ops, img);
                }
            }
            Pass::Draw { src, dst, xform, hq } => {
                let img = match get(&tex, *src) {
                    Some(s) if *hq => s.transformed_hq(w, h, xform),
                    Some(s) => s.transformed(w, h, xform),
                    None => blank(),
                };
                put(&mut tex, *dst, img);
            }
            Pass::Over { src, dst, mode, opacity } => {
                let Some(s) = get(&tex, *src).cloned() else { continue };
                if let Some(d) = tex.get_mut(usize::from(*dst)).and_then(Option::as_mut)
                    && (d.w, d.h) == (s.w, s.h)
                {
                    blend::composite(d, &s, *opacity, *mode);
                }
            }
        }
    }
    tex.get_mut(usize::from(p.out)).and_then(Option::take).unwrap_or_else(blank)
}

fn get(tex: &[Option<Image>], t: Tex) -> Option<&Image> {
    tex.get(usize::from(t)).and_then(Option::as_ref)
}

fn put(tex: &mut [Option<Image>], t: Tex, img: Image) {
    if let Some(slot) = tex.get_mut(usize::from(t)) {
        *slot = Some(img);
    }
}

/// Renders `comp` at `ctx` on the CPU; a comp that can't be evaluated is transparent.
pub fn render_cpu(comp: &Comp, ctx: CompCtx, sources: &dyn CompSources) -> Image {
    match compile_cached(comp, ctx, sources) {
        Ok(p) => execute_cpu(&p),
        Err(e) => {
            log::debug!("comp not rendered: {e}");
            Image::new(ctx.size.0.max(1), ctx.size.1.max(1))
        }
    }
}

/// Bytes the static-subgraph cache may hold (DESIGN-C1 §3.2).
pub const STATIC_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// The static-subgraph cache: pictures of subgraphs with no Media In / Color node and no keyframe
/// (titles, masks and blurs over solids or gradients), by everything that decides their pixels.
/// Least recently used out first.
#[derive(Default)]
struct StaticCache {
    map: HashMap<String, (Arc<Image>, u64)>,
    bytes: usize,
    clock: u64,
}

fn image_bytes(img: &Image) -> usize {
    img.w.saturating_mul(img.h).saturating_mul(16)
}

impl StaticCache {
    fn get(&mut self, key: &str) -> Option<Arc<Image>> {
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        self.map.get_mut(key).map(|(img, used)| {
            *used = clock;
            Arc::clone(img)
        })
    }
    fn put(&mut self, key: String, img: Arc<Image>, cap: usize) {
        let size = image_bytes(&img);
        if size > cap {
            return;
        }
        if let Some((old, _)) = self.map.remove(&key) {
            self.bytes = self.bytes.saturating_sub(image_bytes(&old));
        }
        while self.bytes.saturating_add(size) > cap {
            let Some(oldest) = self.map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
            if let Some((old, _)) = self.map.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(image_bytes(&old));
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(size);
        self.map.insert(key, (img, self.clock));
    }
}

fn static_cache() -> &'static Mutex<StaticCache> {
    static CACHE: OnceLock<Mutex<StaticCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Whether a node's own picture never changes with time or footage: not a Media In, not a Color
/// node (its LUTs live outside the comp) and no keyframe anywhere in it (masks, text layer).
fn static_kind(n: &CompNode) -> bool {
    !matches!(n.kind, CompKind::MediaIn { .. } | CompKind::Color { .. }) && serde_json::to_string(n).is_ok_and(|j| !j.contains("\"keyframes\""))
}

/// The static subgraphs worth caching, by their top node: each top node is static (its whole
/// subgraph is) and is read by a node that isn't, or is the output; only subgraphs holding a
/// Text, Blur, Key or Mask node (layout or a spatial pass; a lone Solid costs less than a copy).
/// Returns each top node with its subgraph (the nodes it depends on, itself included).
fn static_tops(comp: &Comp, order: &filmcraft_project::dag::Order) -> Vec<(NodeId, BTreeSet<NodeId>)> {
    let nodes: BTreeMap<NodeId, &CompNode> = comp.nodes.iter().map(|n| (n.id, n)).collect();
    let mut fixed: BTreeMap<NodeId, bool> = BTreeMap::new();
    for step in &order.steps {
        let own = nodes.get(&step.node).is_some_and(|n| static_kind(n));
        let ins = step.inputs.iter().flatten().all(|p| fixed.get(&p.node).copied().unwrap_or(false));
        fixed.insert(step.node, own && ins);
    }
    let is_fixed = |n: NodeId| fixed.get(&n).copied().unwrap_or(false);
    let mut tops = Vec::new();
    for step in &order.steps {
        let n = step.node;
        if !is_fixed(n) {
            continue;
        }
        let read_by_dynamic = order.steps.iter().any(|s| !is_fixed(s.node) && s.inputs.iter().flatten().any(|p| p.node == n));
        if !(read_by_dynamic || comp.output == Some(n)) {
            continue;
        }
        // the subgraph: n and everything it reads, through the order's input lists
        let mut sub = BTreeSet::from([n]);
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            for p in order.steps.iter().filter(|s| s.node == x).flat_map(|s| s.inputs.iter().flatten()) {
                if sub.insert(p.node) {
                    stack.push(p.node);
                }
            }
        }
        let worth = sub
            .iter()
            .filter_map(|id| nodes.get(id))
            .any(|x| !x.bypass && matches!(x.kind, CompKind::Text { .. } | CompKind::Blur | CompKind::Key { .. } | CompKind::Mask { .. }));
        if worth {
            tops.push((n, sub));
        }
    }
    tops
}

/// [`compile`] with the static-subgraph cache (DESIGN-C1 §3.2): each static subgraph worth caching
/// is rendered once per state and size into [`STATIC_CACHE_BYTES`] and comes back as one
/// picture; the rest of the graph compiles as usual. Same pixels as [`compile`].
pub fn compile_cached(comp: &Comp, ctx: CompCtx, sources: &dyn CompSources) -> Result<Program, CompError> {
    let order = comp.validate()?;
    let tops = static_tops(comp, &order);
    if tops.is_empty() {
        return compile(comp, ctx, sources);
    }
    let (w, h) = (ctx.size.0.max(1), ctx.size.1.max(1));
    let mut pre = Prerendered::default();
    for (top, sub) in &tops {
        let mut part = comp.clone();
        part.nodes.retain(|n| sub.contains(&n.id));
        part.edges.retain(|e| sub.contains(&e.from.node) && sub.contains(&e.to.node));
        part.output = Some(*top);
        let state = serde_json::to_string(&(&part.nodes, &part.edges, top, &part.color)).unwrap_or_default();
        let key = format!("{w}x{h}|{}|{state}", ctx.high_quality);
        let hit = static_cache().lock().unwrap_or_else(PoisonError::into_inner).get(&key);
        let image = match hit {
            Some(img) => img,
            None => {
                let img = Arc::new(execute_cpu(&compile(&part, ctx, sources)?));
                static_cache().lock().unwrap_or_else(PoisonError::into_inner).put(key, Arc::clone(&img), STATIC_CACHE_BYTES);
                img
            }
        };
        pre.images.insert(*top, image);
        pre.skip.extend(sub.iter().filter(|n| *n != top));
    }
    // a node in a cached subgraph that something compiled still reads is compiled too
    loop {
        let needed: Vec<NodeId> = order
            .steps
            .iter()
            .filter(|s| !pre.skip.contains(&s.node) && !pre.images.contains_key(&s.node))
            .flat_map(|s| s.inputs.iter().flatten().map(|p| p.node))
            .filter(|n| pre.skip.contains(n))
            .collect();
        if needed.is_empty() {
            break;
        }
        for n in needed {
            pre.skip.remove(&n);
        }
    }
    for (top, _) in &tops {
        pre.skip.remove(top);
    }
    compile_inner(comp, ctx, sources, &pre)
}

fn num(n: &CompNode, key: &str, t: Tick, default: f64) -> f64 {
    n.value_at(key, t).and_then(|v| v.as_f64()).filter(|v| v.is_finite()).unwrap_or(default)
}

fn choice(n: &CompNode, key: &str, t: Tick) -> u32 {
    match n.value_at(key, t) {
        Some(ParamValue::Choice(c)) => c,
        _ => 0,
    }
}

fn point(n: &CompNode, key: &str, t: Tick) -> Option<Vec2> {
    n.value_at(key, t).and_then(|v| v.as_vec2()).filter(|p| p.x.is_finite() && p.y.is_finite())
}

/// A colour parameter (sRGB-encoded, straight alpha, as every colour control) in linear light.
fn color(n: &CompNode, key: &str, t: Tick) -> [f32; 4] {
    let c = n.value_at(key, t).and_then(|v| v.as_color()).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let [r, g, b] = dec([c[0], c[1], c[2]]);
    let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 1.0 };
    [r, g, b, a]
}

fn premul(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

/// The Transform node's matrix: Motion's parameters in comp pixels (position / anchor "auto" =
/// the frame centre), scaled to the program size.
fn transform(n: &CompNode, t: Tick, (w, h): (usize, usize), px_scale: f32) -> Affine {
    let s = f64::from(px_scale);
    let centre = Vec2::new(w as f64 / 2.0, h as f64 / 2.0);
    let pos = point(n, "position", t).map_or(centre, |p| Vec2::new(p.x * s, p.y * s));
    let anchor = point(n, "anchor", t).map_or(centre, |p| Vec2::new(p.x * s, p.y * s));
    let scale = num(n, "scale", t, 100.0) / 100.0;
    let uniform = n.value_at("uniform_scale", t).and_then(|v| v.as_bool()).unwrap_or(true);
    let sx = if uniform { scale } else { num(n, "scale_width", t, 100.0) / 100.0 };
    Affine::motion(pos, Vec2::new(sx, scale), num(n, "rotation", t, 0.0), anchor)
}

/// The effect ops of a Key / Mask / Blur / Color node for a `w`×`h` image.
fn node_ops(n: &CompNode, ctx: &CompCtx, working: filmcraft_color::WorkingSpace, w: usize, h: usize, px_scale: f32) -> Vec<FxOp> {
    let cx = FxCtx { t: ctx.t, px_scale, seconds: 0.0, timecode: "", clip_name: "", project: ctx.project, env: None, working };
    let with_params = |effect: &str| -> Option<EffectInstance> {
        let mut e = find_effect(effect)?.instance();
        for (k, v) in &n.params {
            e.params.insert(k.clone(), v.clone());
        }
        Some(e)
    };
    match &n.kind {
        CompKind::Key { effect } if filmcraft_project::comp::KEY_EFFECTS.contains(&effect.as_str()) => {
            with_params(effect).and_then(|e| FxOp::eval_all(&e, &cx, w, h)).unwrap_or_default()
        }
        CompKind::Blur => with_params("gaussian_blur").and_then(|e| FxOp::eval_all(&e, &cx, w, h)).unwrap_or_default(),
        CompKind::Mask { masks } if !masks.is_empty() => {
            vec![FxOp::OpacityMask { masks: crate::mask::prepare(masks, ctx.t, px_scale).into() }]
        }
        CompKind::Color { node } => {
            let grade = Grade { nodes: vec![node.clone()], ..Default::default() };
            let gctx = crate::grade::GradeCtx { px_scale, seq_time: ctx.t, clip: 0, project: ctx.project };
            crate::grade::compile_fx(&grade, ctx.t, working, gctx, w, h)
        }
        _ => Vec::new(),
    }
}

/// The Gradient node's picture: linear (along start → end) or radial (around start, reaching end),
/// mixed in linear light, generated at most [`GRADIENT_MAX_SIDE`] on its long side and upscaled.
fn gradient(n: &CompNode, t: Tick, w: usize, h: usize) -> Image {
    let long = w.max(h).max(1);
    let k = if long > GRADIENT_MAX_SIDE { GRADIENT_MAX_SIDE as f64 / long as f64 } else { 1.0 };
    let (gw, gh) = (((w as f64 * k).round() as usize).max(1), ((h as f64 * k).round() as usize).max(1));
    let start = point(n, "start", t).unwrap_or(Vec2::new(0.0, 0.5));
    let end = point(n, "end", t).unwrap_or(Vec2::new(1.0, 0.5));
    let (a, b) = (premul(color(n, "color_a", t)), premul(color(n, "color_b", t)));
    let radial = choice(n, "shape", t) == 1;
    let (sx, sy) = (start.x * gw as f64, start.y * gh as f64);
    let (dx, dy) = (end.x * gw as f64 - sx, end.y * gh as f64 - sy);
    let len2 = dx * dx + dy * dy;
    let mut img = Image::new(gw, gh);
    for (i, px) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % gw) as f64 + 0.5 - sx, (i / gw) as f64 + 0.5 - sy);
        let f = if len2 <= f64::EPSILON {
            1.0
        } else if radial {
            ((x * x + y * y) / len2).sqrt()
        } else {
            (x * dx + y * dy) / len2
        };
        let f = f.clamp(0.0, 1.0) as f32;
        for (c, (&ca, &cb)) in px.iter_mut().zip(a.iter().zip(b.iter())) {
            *c = ca + (cb - ca) * f;
        }
    }
    if (gw, gh) == (w, h) { img } else { img.transformed(w, h, &Affine::scale(w as f64 / gw as f64, h as f64 / gh as f64)) }
}

pub(crate) mod project;

#[cfg(test)]
mod tests;
