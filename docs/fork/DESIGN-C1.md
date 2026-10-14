# Design C1: the simplified node compositor (COMPOSITOR-PLAN C1)

Written 2026-10-12 (design-only lane, no build). Implementation plan for
[`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md) Phase C1 (≈ 25–40 h, scheduled after Y3, before
C2; FORK-PLAN §1a.6 / §1a.42 order). Written so a runner can start C1 without a design pause.
Line numbers are as of `5b27f3ab`; re-check before relying on them.

Rules that bind every step: never crash (hostile project files: every count, size, depth capped;
cycles rejected; `get()` not `[i]`), `Tick` time, everything is a command, every widget has an
automation id, layering (`cargo xtask layers`), clean-room (Fusion is observed behaviour only;
our own words, icons and code), **no grading maths written twice** (§1a.14: the Color node *is* a
`GradeNode`; every pixel op is an existing `ColorOp` / `FxOp`; the only new kernel is a gradient).

**C1 item 6 (Grading Coach fixes, owner §1a.40 (b)) is not designed here:** another lane writes
it under [`docs/fork/coach-eval/`](coach-eval/) (the retrieval fix, the regression question set
from `COACH-QUESTIONS.md`, and the process-assistant plan for the owner to place). C1's step list
reserves a slot for merging that lane (§7, step 12) and nothing else.

## 0. What exists and is reused (facts found)

| Reused for | Where | Notes |
|---|---|---|
| Frame plan and two-input steps | `crates/render/src/plan.rs:35-135` (`PlanLayer`, `LayerFx { size, decimation, ops }`, `FramePlan::{Layers, Composite, Image}`, `PlanStep::{Layers, Transition { inputs: [Vec<PlanLayer>; 2], .. }, Output}`), `plan_frame` :259, `push_item` :408, `execute_cpu` :625 | a transition already renders two isolated canvases and combines them: the comp program generalises this to N intermediates |
| GPU compositor | `crates/gpu/src/lib.rs` `composite` :949, `composite_prepared` :955, `composite_steps` :1191 (transition inputs drawn to pooled canvases), `prepare_layers` :320, the effect stage (`fx.rs`, fused `colorops.wgsl` passes), pooled working textures (:1128), `FX_VRAM_BUDGET`, `render_export_prepared` :1233 | intermediate textures and per-layer op lists already exist |
| Blend modes | `crates/render/src/blend.rs` (`Blend`, `blend_rgb`, `composite`, `composite_at`), `composite.wgsl` | Merge node = A over B with these |
| Motion maths | `crates/render/src/lib.rs:383` `motion_matrix`, `filmcraft_geom::Affine`, the intrinsic `motion` effect (`project/src/effect.rs:265`: position / scale / rotation / anchor) | Transform node reuses the same parameters and matrix |
| Colour | `crates/render/src/grade.rs` `compile_fx(&Grade, t, working, ctx, w, h) -> Vec<FxOp>`, `filmcraft_project::grade::{Grade, GradeNode, NODE_PARAMS}`, `panels/grade.rs` tabs (primaries / curves / qualifier / windows / noise / LUT) | Color node holds one `GradeNode`; its inspector is the existing node tabs |
| Keys | Y3.1 GPU keys (`ultra_key`, luma key; CPU `effects.rs:533 key()` is the oracle until Y3 lands) | Key node = the key effect's `FxOp` chain; if Y3 slips, `gpu_ok` is false and the comp renders on the CPU (correct, slower) |
| Masks | `filmcraft_project::mask::{Mask, MaskShape}` (path / linear, feather, invert, keyframed, tracked), `FxOp::Masked` / `OpacityMask`, `render/src/mask.rs::prepare` | Mask node = `OpacityMask` by a `Vec<Mask>` |
| Blur | `FxOp::Gaussian` (`effects::gaussian_boxes`), `DirectionalBlur` | Blur node |
| Text | graphic layers (`project::graphic_design::LayerSpec`, `render/src/graphic_clip.rs` `text_layout`, `layer_quad`, `shape_path`; `graphics.rs`) | Text node = one text layer rendered to an RGBA image / texture as a graphic clip does |
| Solids | `filmcraft_media::generators::Generator::ColorMatte` | Solid node |
| Tracker | `crates/render/src/track.rs` (`track_step`, `fit_robust`), results stored as keyframes on masks / motion (engine `masks.rs`) | Transform driven by tracker keyframes: no new tracker code (C3 adds tracker nodes) |
| Keyframes | `filmcraft_project::{Param, ParamValue, Keyframe}` (`keyframe.rs:12-183`) | every node parameter |
| Commands / undo | `Session::edit` (`engine/src/lib.rs:875`, project-level, one undo step), `edit_sequence_as` merge keys (:920), `CommandSpec` pattern in `engine/src/grade.rs:1-40` | `comp.*` |
| Frame server | `ui-egui/src/frames.rs` `FrameKey { target: Target::{Sequence, SequencePlan, Item}, .. }` | viewer of any node's output = a new `Target::CompNode` |
| Pages / workspaces | `ui-egui/src/panels/pages.rs` `PAGES` (Media, Edit, Color, Fairlight, Deliver), workspaces presets, `state::Mode` | Resolve workflow gets a **Fusion** page |
| Format | `crates/format/src/lib.rs` `MIGRATIONS` (v16 now; 6.7b takes v17), lineage marker, `tests/fixtures.rs` | C1 bumps to the next free number |
| Guide / coach | `docs/guide/`, `coach_ask.rs` `GUIDE_PAGES`, `coach_ui` id test | new page `docs/guide/compositor.md` |

## 1. The graph model (one model for C1 and C2)

### 1.1 Generic DAG (`crates/project/src/dag.rs`, L1, no render dependency)

```rust
pub type NodeId = u64;                       // stable, unique within a graph (max + 1 on add)
pub struct Port { pub node: NodeId, pub index: u8 }
pub struct Edge { pub from: Port, pub to: Port }   // from = an output port, to = an input port

/// What a graph needs to tell about its nodes to be sorted, checked and scheduled.
pub trait DagNode {
    fn id(&self) -> NodeId;
    fn inputs(&self) -> u8;      // number of input ports (Merge 2, Media Out 1, Media In 0 …)
    fn outputs(&self) -> u8;     // 1 for every C1 node (C2's splitter: 3)
    fn bypassed(&self) -> bool;  // passes input 0 through
}

pub const MAX_NODES: usize = 256;
pub const MAX_EDGES: usize = 1024;
pub const MAX_DEPTH: usize = 64;

pub fn validate<N: DagNode>(nodes: &[N], edges: &[Edge], output: NodeId) -> Result<Order, DagError>;
// Order = Vec<NodeId> in evaluation order, restricted to nodes reachable from `output`,
// plus per-node input resolution (`Vec<Option<Port>>`, one per input port).
// DagError: Cycle(Vec<NodeId>), TooManyNodes, TooManyEdges, TooDeep, UnknownNode(NodeId),
//           BadPort(Port), DuplicateInput(Port), NoOutput.
```

Kahn's algorithm with bounded work (`MAX_NODES` × `MAX_EDGES`); duplicate node ids rejected;
an input port with two edges rejected (the canvas replaces the edge instead); dangling input
ports are allowed (a Merge with only its background connected passes it through; a node whose
input 0 is missing produces transparent black). `validate` is pure and cheap enough to run on
every edit and on load (so the UI can show "cycle" and the renderer never sees one).

**How C2 reuses it:** `Grade` gets `inputs: Vec<NodeId>` per node as ADR 0004 planned; a serial
grade is the DAG where node *k* has input `k − 1`; `GradeNode` implements `DagNode` and C2's
parallel / layer / splitter mixers are more `GradeNode` kinds. The scheduler (§3) and the canvas
widget (§4) take any `DagNode` graph, so C2 writes no sort, no cycle check, no canvas.

### 1.2 The Comp (`crates/project/src/comp.rs`)

```rust
/// A composition: a node graph rendered as one picture of `width × height` at `rate`.
/// A project item (`ItemKind::Comp(Arc<Comp>)`), so one comp can be on the timeline many times
/// and project snapshots don't deep-copy it (like `ItemKind::Sequence`).
pub struct Comp {
    pub width: u32, pub height: u32, pub rate: FrameRate, pub duration: Tick,
    #[serde(default)] pub nodes: Vec<CompNode>,
    #[serde(default)] pub edges: Vec<Edge>,
    /// The Media Out node (None: the comp is transparent).
    #[serde(default)] pub output: Option<NodeId>,
    /// Working / grading space of the Color nodes: the sequence's when the comp was made.
    #[serde(default)] pub color: ColorPipeline,
}

pub struct CompNode {
    pub id: NodeId,
    pub kind: CompKind,
    #[serde(default, skip_serializing_if = "String::is_empty")] pub label: String,
    #[serde(default, skip_serializing_if = "Not::not")] pub bypass: bool,
    /// Keyframable parameters by key (`comp::params(kind)` is the schema; missing = default).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")] pub params: BTreeMap<String, Param>,
    /// Canvas position (UI only, saved so the layout survives).
    #[serde(default)] pub pos: [f32; 2],
}

#[serde(rename_all = "camelCase", tag = "type")]
pub enum CompKind {
    /// A project clip (media / subclip / nested sequence / another comp, cycles refused at the
    /// project level by `refuse_self_nesting`) with its own in-point; or a timeline layer
    /// (`Layer { track }`: video track `track` of the sequence the comp clip sits in, rendered
    /// alone at the comp clip's time, like `FxEnv::track`).
    MediaIn { source: MediaInSource },
    MediaOut,
    Merge,                      // in 0 = background, in 1 = foreground; params blend, opacity
    Transform,                  // position, scale (x, y, uniform), rotation, anchor (Motion's keys)
    Key { effect: String },     // "ultra_key" | "luma_key": params = that effect's params
    Mask { #[serde(default)] masks: Vec<Mask> },   // params: invert, feather handled per mask
    Blur,                       // params: blurriness, direction (horizontal / vertical / both)
    Color { node: GradeNode },  // one grade node, params live in `node.params` (not here)
    Text { layer: Box<LayerSpec> },               // a graphic text layer
    Solid,                      // params: color
    Gradient,                   // params: start, end (Vec2 0..1), color_a, color_b, kind (linear / radial)
}
```

Caps (hostile files; every one tested): `width × height ≤ 16384 × 16384` and
`≤ 256 Mpx`, `nodes ≤ MAX_NODES`, `edges ≤ MAX_EDGES`, `params ≤ 64` per node, label ≤ 64
chars, text ≤ 4 096 chars, masks ≤ `MAX_WINDOWS` per Mask node, `pos` finite (else 0). A comp
that fails `validate` still **loads** (nothing is dropped), renders transparent black, and the
canvas shows the error on the output node; commands that would create a cycle are refused.

**Timeline item:** `ItemKind::Comp(Arc<Comp>)` as a project item; a comp clip is an ordinary
`TrackItem` pointing at it (so it has Motion / Opacity / effects / a grade / TNR like a nested
sequence). `duration()` / `frame_rate()` / `frame_size()` matches in `project/src/lib.rs:367-410`
and `:1737-1758` gain a `Comp` arm. `refuse_self_nesting` (`engine/src/lib.rs:879`) learns the
comp → MediaIn → comp edge, so a comp can't contain itself through a sequence either.

## 2. Node semantics (each maps to existing maths)

| node | inputs | output | implementation (CPU oracle / GPU) |
|---|---|---|---|
| Media In | 0 | the clip's picture at comp time `t` (+ `offset`), decoded at the comp's working size, input transform applied, scaled to fit (`scale_to_frame` rules), transparent outside | `base_layer` / the plan's layer draw with `LayerFx` (exactly what a clip on the timeline gets, minus Motion); a `Layer { track }` = `FxEnv::track`'s render |
| Media Out | 1 | pass-through | none |
| Merge | 2 | `blend(bg, fg, mode, opacity)` | `blend::composite` / `composite.wgsl` layer draw of fg over bg's canvas |
| Transform | 1 | the input drawn through an `Affine` (position / scale / rotation / anchor; `Image::transformed[_hq]` with Maximum Render Quality) | the same quad draw the compositor does for Motion |
| Key | 1 | the input with its alpha from the key (spill / cleanup as the effect's params) | the key effect's `FxOp`s (Y3.1), CPU `effects::key` oracle |
| Mask | 1 | alpha × mask coverage | `FxOp::OpacityMask` with `mask::prepare` |
| Blur | 1 | gaussian / directional | `FxOp::Gaussian` / `DirectionalBlur` |
| Color | 1 | `compile_fx(&Grade { nodes: vec![node] }, t, comp.color.working, ctx, w, h)` | the fused `ColorChain` pass + the node's windows / qualifier as today |
| Text | 0 | the text layer rasterised on a transparent canvas of the comp size | `graphic_clip` layout + the text rasteriser, cached per (layer, t, size) |
| Solid | 0 | a flat colour (alpha included) | `ColorMatte` |
| Gradient | 0 | linear / radial between two colours | **new**: a CPU `Image` generator (≤ 2048 px on the long side, cached per (params, size), bilinear upscale by the layer draw; a gradient upscales exactly) — no shader, no second implementation |

Bypass = input 0 through; a bypassed source node = transparent. Every parameter is a `Param`
evaluated at comp time (`param_at`, with the comp clip's `effect_time_at` mapping speed / holds
exactly as effects do), so keyframes, speed changes and frame holds behave like effects. The
tracked title of the acceptance test is a Transform whose position `Param` holds the tracker's
keyframes (pasted from a tracked mask or Motion: `comp.setParam` accepts a keyframe list).

**Where the comp renders in the clip's chain:** a comp clip's picture is the comp's Media Out,
then (as any clip) its TNR (media only: not for comps), grade, effect stack, Motion / Opacity.

## 3. Evaluation and scheduling (`crates/render/src/comp/`)

### 3.1 Program

`comp::compile(project, comp, t, opts, sources, size) -> Result<Program, CompError>`:
`validate` → for each node in order, one `Pass`:

```rust
pub struct Program { pub size: (u32, u32), pub textures: u16, pub passes: Vec<Pass>, pub out: Tex }
pub enum Pass {
    /// Draw layers (a Media In / Text / Solid / Gradient / the result of a Transform) into `dst`
    /// (cleared to transparent first): the existing layer draw, `LayerFx` ops included.
    Draw { layers: Vec<PlanLayer>, dst: Tex },
    /// Run an op list on a texture in place (Blur, Color, Key, Mask): the existing effect stage.
    Fx { tex: Tex, ops: Vec<FxOp> },
    /// Composite `src` over `dst` with a blend mode, opacity and an affine (Merge; Transform
    /// folds its affine into the following Merge or into a Draw when it can).
    Over { src: Tex, dst: Tex, mode: Blend, opacity: f32, xform: Affine, hq: bool },
}
```

Texture allocation by liveness: a node's texture is freed after its last consumer (counted from
the edges); `textures` is the peak (acceptance comps: 3). Cap: `MAX_LIVE_TEXTURES = 8` at the
comp size, and `textures × size × 16 B` must fit `FX_VRAM_BUDGET` (`gpu/src/lib.rs:1290`),
else the comp renders on the CPU (logged once, like the "grade doesn't fit the GPU program"
warning in `plan.rs:550`). Media In frames are fetched through the same `clip_frame` /
`SourceProvider` path as timeline clips (GOP cache, proxies, hardware frames with zero-copy
import), so a 6.2K Media In costs what a timeline clip costs.

### 3.2 CPU and GPU execution

- **CPU** (`comp::execute_cpu(&Program, &dyn Sources) -> Image`): `Draw` = `plan::effect_image`
  + `Image::transformed` into a cleared canvas; `Fx` = `gpufx::apply_ops`; `Over` =
  `blend::composite_at` through the affine. This is the oracle and the export-parity path.
- **GPU**: a new `PlanStep::Comp { program: Arc<Program>, opacity, blend, xform }` produced by
  `push_item` for a comp clip; `gpu/src/lib.rs::composite_steps` gets one arm:
  `Draw` → `prepare_layers` + the layer draw into a pooled canvas; `Fx` → the effect stage on
  that canvas (fused `colorops.wgsl` for `ColorChain`s, `fx.wgsl` for spatial ops, as per layer
  today); `Over` → the blend draw with the quad transform; the program's `out` canvas is then
  drawn onto the frame's canvas like a transition result. All textures come from the existing
  pool (no new allocation path). Nested comps (a Media In pointing at another comp) compile
  recursively up to `MAX_NEST_DEPTH` (`lib.rs`), the same bound nested sequences use.
- **Caching:** (1) frame-level, as today (frame cache by revision + render cache: a comp clip's
  segment hash includes the comp through `hash_source`, which must gain a `Comp` arm hashing
  the comp's JSON); (2) **static subgraph cache**: a node with no Media In / Layer ancestor and no
  keyframed parameter in its subgraph (titles, solids, gradients, masks over them) is rendered
  once per (node state hash, size) into a small bounded LRU (`CompStaticCache`, 256 MB, CPU
  images; the GPU uploads them as plain layers). Per-node caching of animated subgraphs is C4
  material (expressions make it worthwhile); not in C1.
- **Export parity:** export and render-cache parts go through `plan_frame` / `render_sequence`
  as today; `is_cpu_frame` stays truthful (a comp with a CPU-only node makes the frame a CPU
  frame).

### 3.3 Costs (estimated, measure in the acceptance run)

Acceptance comp A (6.2K Media In → Key → Merge over a 4K background → Color → Out) at 1080p
sequence: ≈ 3 draws + 2 fx passes + 1 over ≈ 4–6 ms GPU on the RX 6700 XT; at 4K half ≈ the
same per pixel count. Comp B (Text → Transform → Merge): 1 cached static draw + 1 over.

## 4. UI (`crates/ui-egui`)

### 4.1 Node canvas widget (`widgets/node_canvas.rs`, generic, no project types)

Input: `CanvasGraph { nodes: Vec<NodeView { id, title, kind_label, pos, inputs: Vec<&str>,
outputs: Vec<&str>, bypassed, selected, error: Option<String>, is_output }>, edges, pan, zoom }`;
output: `Vec<CanvasEvent>` (`Select(ids)`, `Move(id, pos)` while dragging → `MoveEnd`,
`Connect(from, to)`, `Disconnect(edge)`, `AddRequested(kind, pos)` from the right-click menu,
`Bypass(id)`, `Delete(ids)`, `SetOutput(id)`, `View(id)`). Plain `egui::Painter` (rounded
rectangles, port circles, cubic edges, grid), pan with middle / space-drag, zoom with the wheel
around the pointer, box select, keyboard Delete / Ctrl+B (bypass) / 1–9 (view node *n*) in the
Resolve keymap (Fusion-style), standard names in the Premiere keymap. Every element registers an
automation id: `comp.canvas`, `comp.node.<id>`, `comp.port.<id>.in.<n>` / `.out.<n>`,
`comp.edge.<from>-<to>`, `comp.add.<kind>` (menu items), `comp.view.<id>`. State
(`ui.panels.comp`: pan, zoom, selection, viewed node) is serde so the control channel can read
and set it. **C2 feeds the grade's node list to the same widget** (the Color page node strip
becomes this canvas in C2).

### 4.2 Compositor panel (`panels/comp.rs`)

Three parts in one dockable panel **Compositor** (Window ▸ Compositor; opens on "Open in
Compositor" from a comp clip's context menu / `comp.open`): the canvas (top), the **inspector**
of the selected node (bottom-left: parameters as Effect Controls rows; for a Color node the
existing `panels/grade.rs` tab widgets for one node, so the grader's UI is not written twice;
for Text the graphic text controls; for Mask the mask list with the existing mask editor on the
viewer), and the **viewer** target (bottom-right: a combo "Show: Media Out / <node>" that sets
the Program monitor to `Target::CompNode(comp, node)` while the panel has focus). The timeline
shows a comp clip with a distinct colour label and a node-count badge.

- **Premiere workflow:** the panel is available in every workspace (Window menu), plus a
  **Compositing** workspace preset (Compositor large, Program monitor, Effect Controls,
  Project). `menus.rs` adds Window ▸ Compositor and Clip ▸ Compositor ▸ {New Comp Clip, Convert
  to Comp Clip, Open in Compositor} (from `CommandSpec.menu`).
- **Resolve workflow:** `pages.rs` `PAGES` gains `("fusion", "Fusion", Some("Compositing"))`
  between Edit and Color (Resolve's order), mapped to the Compositing workspace; the page shows
  the comp of the clip under the playhead (a plain clip offers "Convert to Comp Clip" in the
  page's empty state). RS2b's terminology overlay gives the Resolve words (Media Pool, Inspector,
  Timeline) for free.
- Screenshots in both workflows go to the acceptance notes (never commit the Resolve
  references).

## 5. Commands (`crates/engine/src/comp.rs`; all undoable through `Session::edit`, `journal: true`)

`comp` addresses the comp **project item** (default: the comp the Compositor panel shows, else
the selected comp clip's item); `node` by id. Refusals are `BadParams` with a reason.

| id | label | params | effect |
|---|---|---|---|
| `comp.new` | New Comp | `{"name":str?,"width":n?,"height":n?,"rate":fps?,"duration":ticks?}` | a comp project item with Media Out (defaults: the active sequence's settings) |
| `comp.newClip` | New Comp Clip | `{"comp":id?,"track":n?,"time":ticks?}` | places a comp on the timeline (overwrite at the playhead) |
| `comp.convertToCompClip` | Convert to Comp Clip | `{"clips":[id]?}` | selected video clips (same track or stacked) → one comp (Media In per clip, bottom track first, each one's Motion folded into a Transform, Merges chained, Media Out), replaced on the lowest track by one comp clip over the covering span; refused across audio or when any clip is a comp of the same comp (self-nesting) |
| `comp.open` | Open in Compositor | `{"clip":id?}` | UI state (journalled as a view change, not undo) |
| `comp.get` | Get Comp | `{"comp":id?}` | query: the graph as JSON (+ `order`, `error`) |
| `comp.params` | List Comp Node Parameters | `{"kind":str}` | query: the schema per kind |
| `comp.addNode` | Add Node | `{"comp":id?,"kind":str,"label":str?,"pos":[x,y]?,"after":node?}` | `after` splices the node into that node's output edge (Fusion's "insert after selected") |
| `comp.removeNode` | Delete Node | `{"comp":id?,"nodes":[id]}` | reconnects in 0 → out when the node had one of each (as Fusion does) |
| `comp.connect` | Connect | `{"comp":id?,"from":node,"fromPort":n=0,"to":node,"toPort":n=0}` | replaces the edge on that input; refused on a cycle (`validate` first, nothing committed) |
| `comp.disconnect` | Disconnect | `{"comp":id?,"to":node,"toPort":n=0}` | |
| `comp.setParam` | Set Node Parameter | `{"comp":id?,"node":id,"param":str,"value":any?,"reset":str?,"time":ticks?,"keyframes":[…]?}` | mirrors `grade.setParam` (Color nodes: the grade keys, same clamping); merge key per (comp, node, param) so a drag is one undo step |
| `comp.toggleAnimation` | Toggle Node Parameter Animation | `{"comp":id?,"node":id,"param":str,"time":ticks?}` | |
| `comp.bypass` | Bypass Node | `{"comp":id?,"node":id,"on":bool?}` | |
| `comp.labelNode` | Label Node | `{"comp":id?,"node":id,"label":str}` | ≤ 64 chars |
| `comp.moveNode` | Move Node (canvas) | `{"comp":id?,"node":id,"pos":[x,y]}` | merged undo (one step per drag) |
| `comp.setOutput` | Set Media Out | `{"comp":id?,"node":id}` | |
| `comp.view` | View Node | `{"comp":id?,"node":id?}` | viewer target (UI state) |
| `comp.setSize` | Comp Settings | `{"comp":id?,"width":n?,"height":n?,"rate":fps?,"duration":ticks?}` | |

Grade-node parameters of a Color node reuse `grade::NODE_PARAMS` and its clamp; `comp.params
{"kind":"color"}` returns that schema. The MCP server exposes everything through `command_list`;
the acceptance comps are built by a `command_batch` script kept in `tools/comp-accept.json`.

## 6. Project serde, schema and migration

- New: `ItemKind::Comp(Arc<Comp>)` (an enum variant: an **older build cannot read a project that
  has one**), `TrackItem` unchanged. Schema bump to the next free number (**v18 if 6.7b's v17 is
  the only bump before C1**; check `SCHEMA_VERSION` when C1 starts) with a no-op migration
  `vN_to_vN+1` and a docs line "fork C1: comps (`ItemKind::Comp`, `Project` items). Older builds
  refuse projects that contain one (`TooNew`)". A project without comps still saves exactly as
  before. Fixture `vN-comp.fcproj` with the two acceptance comps; `fixtures.rs` loop picks it up.
- Hostile decode: `Comp` fields are `#[serde(default)]`; caps are enforced on load by
  `Comp::sanitize()` called from the format decoder's post-load pass (counts truncated, sizes
  clamped, non-finite positions zeroed, a cycle left in place and reported by `validate`).
  Fuzz test: random JSON comps (random kinds, random edges, random params, NaN, huge sizes)
  through `decode` + `validate` + `compile` + `execute_cpu` at 64×64: never panics, never
  allocates over the caps (asserted through the `textures` and image sizes).

## 7. Steps, estimates, lanes (≈ 36–44 h)

Lanes are leaf-builder worktrees (single crate, own `CARGO_TARGET_DIR`, GRADE-PLAN §7 limits: at
most two building at once while ≥ 50 GB free, the integrator merges). A lane starts only once
the step it depends on is **committed**.

| # | step | crate(s) | est. | lane | depends on |
|---|---|---|---|---|---|
| 1 | **ADR 0007** ([`docs/adr/0007-node-graph.md`](../adr/0007-node-graph.md); 0006 is user shader effects) "One node graph for compositing and grading" (context: COMPOSITOR-PLAN + §1a.14; decision: `dag.rs` trait + scheduler shared by comps and grades, `Comp` as a project item, Color node = `GradeNode`, CPU oracle / GPU parity; consequences: C2 only adds node kinds; caps). Write first, update at the end. | docs | 1 h | integrator | — |
| 2 | `project/src/dag.rs` (trait, `validate`, errors, caps) + unit tests (topo order, cycle, caps, unreachable pruned, duplicate input, dangling ports) | project | 3 h | integrator | 1 |
| 3 | `project/src/comp.rs` (`Comp`, `CompNode`, `CompKind`, params schema per kind, `sanitize`, `ItemKind::Comp` arms, `refuse_self_nesting` hook) + serde / hostile tests | project, engine (one fn) | 4 h | integrator | 2 |
| 4 | Format bump + fixture + fuzz test | format | 1.5 h | **lane T** (test author) | 3 |
| 5 | `render/src/comp/{mod,program,cpu}.rs`: `compile`, liveness allocation, `execute_cpu`, static-subgraph cache, `hash_source` Comp arm, `push_item` → `PlanStep::Comp`; golden tests per node kind against the primitives they reuse (Merge = `blend::composite`, Transform = `Image::transformed`, Color = `compile_fx` on the image, Mask = `OpacityMask`), the two acceptance comps on the CPU | render | 6 h | integrator | 3 |
| 6 | Gradient generator (+ cache) and the Text node adapter (graphic layer → image) | render | 2 h | **lane A** (render) | 3 |
| 7 | GPU: `composite_steps` Comp arm (Draw / Fx / Over on pooled canvases), VRAM check → CPU fallback, export path; parity tests AMD + Intel (`MESA_VK_DEVICE_SELECT`) per node kind and for both acceptance comps (≤ 1e-3 rel, as TNR) | gpu | 5 h | **lane B** (gpu) | 5's `Program` types committed |
| 8 | `engine/src/comp.rs` commands (§5), `convertToCompClip`, undo / redo / hostile-params tests, MCP smoke (`command_batch` builds comp A and B) | engine | 4 h | **lane C** (engine) | 3 |
| 9 | `widgets/node_canvas.rs` generic widget + a UI test with a fake graph (ids registered, connect / select / delete events, pan / zoom state serde) | ui-egui | 5 h | **lane D** (ui-egui) | 2 (only the `DagNode` shape) |
| 10 | `panels/comp.rs` (canvas + inspector + viewer), `Target::CompNode` in `frames.rs`, Window / Clip menus, Compositing workspace, Fusion page, i18n (es, zh-cn), timeline badge; UI tests in both workflows | ui-egui | 5 h | integrator | 8, 9 |
| 11 | Guide `docs/guide/compositor.md` ([Resolve-style] for the Fusion-like flow, [Fork] for the limits; the two acceptance comps as recipes), `GUIDE_PAGES`, coach ids test, `docs/architecture.md` §1 note on `PlanStep::Comp`, README page table | docs, engine | 2 h | **lane T** | 10 |
| 12 | **Merge the Coach-fixes lane** (`docs/fork/coach-eval/`, C1 item 6): review, run its regression set, nothing designed here | engine | 1–2 h | integrator | that lane's branch |
| 13 | Acceptance: bench cases `comp-1080p` / `comp-4k-half` (real-time, synthetic), 30-min soak through the control channel (build / edit / play comps, `FILMCRAFT_SOAK_MINUTES`), screenshots in both workflows, `cargo xtask ci`, tag `comp-1`, build / NOTES / publish per §1a.9 / §1a.33, contribution slot (§1a.25) | — | 3 h | integrator | all |

Critical path: 1 → 2 → 3 → 5 → 7 → 10 → 13 (≈ 27 h of integrator time); lanes A–D and T run
beside it (≈ 17 h of lane time). Reviewer (Fable) on every lane merge.

## 8. Test plan (summary; each step above names its tests)

- **Model:** dag unit tests; serde round trip; old project unchanged; a project without comps
  saves byte-identical; hostile fuzz (never panics; caps hold).
- **Evaluation:** per-node golden against the reused primitive (so a regression in a primitive
  shows in exactly one place); scheduler liveness (peak textures of the acceptance comps = 3,
  of a 64-node chain = 2); bypass / missing input / transparent rules; nested comp depth bound;
  static cache hit (second render of a title comp does no layout); comp A and comp B CPU
  goldens checked into `crates/golden` fixtures (generated, small size).
- **GPU parity:** every node kind and both acceptance comps, AMD + Intel; the comp inside a
  colour-managed (DWG / DI) sequence and in Rec. 709; VRAM-cap fallback renders identically on
  the CPU.
- **Export = viewer:** export a comp sequence (CPU and GPU) and compare with the frame server's
  picture (`max_diff` ≤ 1e-3 rel, as the TNR test does).
- **Commands:** each `comp.*` round-trips through undo / redo; cycles refused; self-nesting
  refused; `convertToCompClip` of stacked clips renders the same frame as the stack did (Motion
  folded into Transforms); hostile params refused.
- **UI:** automation ids present in both workflows; the canvas events drive commands; the
  Fusion page opens the comp under the playhead; the Program monitor shows a chosen node.
- **Soak / bench:** as in step 13; numbers logged as synthetic.

## 9. Risks and mitigations

| risk | mitigation |
|---|---|
| Y3.1 GPU keys slip → the Key node is CPU-only | `gpu_ok` false → the comp frame renders on the CPU (correct); acceptance comp A then measures CPU time; the GPU figure is re-run when Y3.1 lands (log it) |
| VRAM at 6.2K comps (one texture ≈ 416 MB at full res; comps are the sequence size, not 6.2K) | the live-texture cap + budget check with a CPU fallback; comps are evaluated at the sequence's output size × `opts.scale` (playback resolution 1/2, 1/4 applies) |
| A second grade UI for Color nodes | forbidden by design: the inspector embeds `panels/grade.rs`'s node tabs for one node; if those need a small refactor (take a `&GradeNode` + a command target instead of the clip's grade), do it in step 10 and keep the Color page byte-identical (UI test) |
| Schema collision with upstream at U0.2 / later syncs | lineage marker (`FORK_LINEAGE`); renumber on sync as `format/src/lib.rs` docs describe |
| Canvas usability (no graph library) | keep C1's canvas plain (grid, straight ports, cubic edges, box select); polish is C4 / Phase 7 work; test with the MCP `ui_*` tools and screenshots |
| `convertToCompClip` losing clip state (audio links, transitions, grades) | refuse when a selected clip is inside a transition or has a linked audio clip that isn't selected; grades / TNR / effects of the converted clips move into a Color node + the clip's effect list stays on the comp clip only when every clip had the same (else refused with a message listing what to flatten first) — keep this strict in C1, relax in C4 |
| Coach lane conflict | step 12 merges it as its own commit; it touches `coach*.rs` / `docs/guide/color/coach.md` only, no overlap with C1's files |

## 10. Open questions (provisional choices; OWNER-QUESTIONS entries if the owner wants otherwise)

- **Comp = project item** (reusable, nestable, shows in the Project panel / Media Pool) rather
  than a per-clip graph. Alternative: `TrackItem.comp` like `grade`. Chosen for reuse and because
  Fusion comps are per clip only *visually*; reversible before C2 ships (serde change only).
- **Media In of a timeline layer** (`Layer { track }`) is included (common "comp over the tracks
  below" use) but evaluates the tracks below with the same `FxEnv::track` cost model; if it
  proves slow on 6.2K, limit it to the render cache / export like TNR.
- **Gradient as a CPU-generated image** (no shader). Alternative: a WGSL generator; rejected for
  C1 (a second implementation to keep in parity for a trivial picture).
- **Static-subgraph cache only** in C1; per-node caches for animated graphs wait for C4.
- **Fusion page placement** between Edit and Color (Resolve's order) — the page bar grows to six
  entries; RS3's layouts need one more preset ("Compositing").
