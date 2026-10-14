# ADR 0007: One node graph for compositing and grading (fork)

- **Status:** accepted for the fork (2026-10-14, C1 step 1; written in a MINIMAL-usage run without a
  separate designer: **provisional**, open to the owner's change like any §1a choice). Scope set by
  [`COMPOSITOR-PLAN.md`](../../COMPOSITOR-PLAN.md) Phase C1 / C2; full design in
  [`docs/fork/DESIGN-C1.md`](../fork/DESIGN-C1.md). Updated at the end of C1 with what shipped.
- **Builds on:** ADR 0003 (GPU colour ops: every pixel op is a `ColorOp` / `FxOp`), ADR 0004 (the
  grade model: `Grade` / `GradeNode` with stable node ids, planned to gain inputs), ADR 0006 (the
  user-effect registry a later "Shader" node can reuse). **Leads to:** C2 (the grader's parallel /
  layer / splitter-combiner / key-mixer / outside nodes), C3 / C4 (more node kinds, per-node caches).

## Context

The owner's §1a.14 rule: grading is built from small, composable, individually tested operations,
and "no grading maths is written twice". C1 adds a simplified node compositor (Fusion-like, in our
own words and code); C2 then turns the serial grade into a node graph. If each feature brought its
own graph type, sort, cycle check, scheduler and canvas, C2 would duplicate C1. A node graph is also
hostile input: project files can hold cycles, huge counts, dangling ports and absurd sizes.

## Decision

1. **One generic DAG** in `crates/project/src/dag.rs` (L1, no render dependency): `NodeId = u64`,
   `Port { node, index }`, `Edge { from, to }`, a `DagNode` trait (`id`, `inputs`, `outputs`,
   `bypassed`) and a pure `validate(nodes, edges, output) -> Result<Order, DagError>` (Kahn's
   algorithm with bounded work). `Order` is the evaluation order restricted to nodes reachable from
   the output, plus each node's resolved input ports. Errors: cycle, too many nodes / edges, too
   deep, unknown node, bad port, duplicate input, duplicate node id, no output. Caps:
   `MAX_NODES = 256`, `MAX_EDGES = 1024`, `MAX_DEPTH = 64`. Dangling inputs are allowed (a missing
   input 0 is transparent black; a Merge with only its background passes it through). `validate`
   is cheap enough to run on every edit and on load.
2. **The comp is a project item** (`ItemKind::Comp(Arc<Comp>)`, `crates/project/src/comp.rs`): a
   graph of `CompNode`s (Media In / Out, Merge, Transform, Key, Mask, Blur, Color, Text, Solid,
   Gradient) rendered at the comp's size; a comp clip is an ordinary `TrackItem` pointing at it, so
   it gets Motion / Opacity / effects / a grade like a nested sequence. Self-nesting is refused
   through the same check nested sequences use. Provisional: chosen over a per-clip graph for reuse
   (one comp on the timeline many times) and cheap snapshots; reversible before C2 ships (serde).
3. **Every node is existing maths.** The Color node holds one `GradeNode` and compiles through
   `render::grade::compile_fx`; Merge is `blend`; Transform is the Motion matrix; Key / Mask / Blur
   are the existing `FxOp`s. The only new pixel code is a CPU gradient generator (an upscaled
   gradient is exact, so no shader twin is needed).
4. **One program, two executors.** `render::comp::compile` turns a validated comp into a `Program`
   of `Draw` / `Fx` / `Over` passes with liveness-based texture allocation (cap 8 live textures and
   the GPU effect VRAM budget, else the frame renders on the CPU). `execute_cpu` is the oracle and
   the export-parity path; the GPU runs the same program through `composite_steps` with pooled
   canvases. Parity is tested per node kind on AMD and Intel.
5. **C2 adds node kinds, not machinery.** `Grade` gains per-node inputs (ADR 0004's plan);
   `GradeNode` implements `DagNode`, a serial grade is the chain *k − 1 → k*, and the scheduler and
   the generic canvas widget (`widgets/node_canvas.rs`, no project types) take any `DagNode` graph.
6. **Hostile files load, never crash.** `Comp` fields are `#[serde(default)]`; `Comp::sanitize`
   clamps sizes (≤ 16384 per side, ≤ 256 Mpx), truncates counts, zeroes non-finite positions and
   leaves a cycle in place for `validate` to report. A comp that fails validation renders
   transparent and shows the error on its output node; commands that would create a cycle are
   refused before anything is committed.
7. **Everything is a command** (`comp.*` in `crates/engine/src/comp.rs`, undoable through
   `Session::edit`), and the canvas registers automation ids for every node, port and edge.

## Consequences

- A project with a comp cannot be opened by an older build (an enum variant; schema bump with a
  no-op migration). Projects without comps save exactly as before.
- C2 writes no sort, no cycle check, no canvas; grades migrate unchanged (serial = a chain).
- The caps bound the work of a hostile graph; the fuzz test (format) holds them.
- Fusion-like behaviour is clean-room: observed behaviour and the public manual only, our own
  words, icons and code.

## Alternatives considered

- **A graph type per feature (comp graph, grade graph):** rejected; duplicates sort, validation,
  scheduler and canvas, against §1a.14.
- **A third-party graph crate (e.g. petgraph):** rejected for the core; the needed subset (Kahn's
  sort with caps and port checks) is small, and owning it keeps the caps and errors exact.
- **A WGSL gradient generator:** rejected for C1 (a second implementation to keep in parity).

## As built (2026-10-14, C1 step 7)

- The GPU path is a field on the plan layer (`PlanLayer::comp: Option<Arc<Program>>`), not a
  `PlanStep::Comp`: the frame stays a `FramePlan::Layers`, so GPU export and the frame server keep
  their layer path, and a comp clip is placed by Motion like any clip.
- `gpu::comp::run_comp` runs `Program` passes on `Rgba16Float` slots by registering each slot in
  the renderer's upload cache under a placeholder frame and reusing the layer draw / effect stage
  (no new shaders). Parity with `execute_cpu` on AMD and Intel: p99 ≤ 7·10⁻⁴ per pass kind.
- Media In nodes that read a track compile to `Pass::Layers` (that track's plan layers), keeping
  hardware frames on the GPU; Item sources still arrive as CPU pictures (`Pass::Image`).
