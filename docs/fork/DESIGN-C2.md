# C2 design note: the grade becomes a node graph

Written 2026-10-15 (scheduled run, MINIMAL mode, ahead of C2; COMPOSITOR-PLAN §C2, ADR 0004 §2,
ADR 0007 §5). Every choice here is reversible until C2 ships a project file that uses it. Where this
note departs from ADR 0004's original plan it says so; the ADRs get a pointer to this note.

## 1. Model: edges on the existing `Grade`, serial stays implicit

```text
Grade     { nodes: Vec<GradeNode>, bypass, edges: Vec<Edge> /* new, default empty */,
            output: Option<NodeId> /* new, default None */ }
GradeNode { id, label, enabled, params, windows, lut,
            kind: NodeKind /* new, default Correction */ }
NodeKind  = Correction | ParallelMixer | LayerMixer | Splitter | Combiner | KeyMixer
```

- **Edges, not ADR 0004's `inputs: Vec<u64>`.** ADR 0007 settled one graph model (`dag::Edge` from
  an output `Port` to an input `Port`, `dag::validate` → `Order`), and keys need port numbers
  (RGB vs key), which a plain id list can't say. Same serde-default trick, so no format break.
- **`edges` empty = today's serial chain** (node *k − 1* → node *k*, the last node → output). An old
  grade never gains edges until the user branches it; the first branching edit (add parallel /
  layer / outside, or a connect on the canvas) writes the whole chain out as explicit edges in the
  same undo step. Going back to a plain chain (all mixers deleted, one path left) may drop the
  edges again, so a project that branches once and is un-branched saves as before.
- **The grade's input is a virtual node, id 0** (`grade::SOURCE`), never stored in `nodes`. Ids
  already start at 1 (`next_id`); a hostile file with a node id 0 has it renumbered at load (with
  its edges), the only migration. `output: None` = the last node in `nodes` (serial) or, for a
  graph, the one node whose RGB output feeds nothing (several: refused by validation → the grade
  renders as bypassed and the Color page says why, never a panic).
- **Ports.** Correction node: inputs 0 = RGB, 1 = key in; outputs 0 = RGB, 1 = key out (its
  qualifier × windows matte, Resolve's green triangles). Parallel / Layer mixer: inputs 0…n − 1
  RGB (n grows as nodes are attached, cap 16), output 0. Splitter: input 0, outputs 0 / 1 / 2 =
  R / G / B (each a grey picture). Combiner: inputs 0 / 1 / 2, output 0. Key mixer: inputs 0…n − 1
  keys, output 0 = key. `GradeNode` implements `dag::DagNode` through `kind` (a thin wrapper type
  supplies the virtual source).
- **Outside node is not a kind.** It is a Correction node whose key input is fed by the previous
  node's key output with `key.invert` on (Resolve's construction). New node params:
  `key.invert`, `key.gain` (0…1), `key.offset`. Mixer params: `mix.gain.<i>` (parallel, default
  1/n weighting), `layer.mode.<i>` / `layer.opacity.<i>` (blend modes as `Blend`), key mixer
  `kmix.mode.<i>` (add / subtract / intersect) and `kmix.invert.<i>`. All keyframable `Param`s.
- **Schema v19** (from v18): older builds would silently drop `edges` / `kind`, so the bump makes
  them refuse the file instead. Migration test: every serial grade fixture loads, compiles and
  renders bit-identically to v18; a v19 file with no edges is byte-identical in its grade section.

## 2. Compiling: serial fast path, branches as one graph step

- `render::grade::compile_fx` keeps its signature. **A grade without edges, or whose graph is a
  plain chain, compiles exactly as today** (same `FxOp`s, so identity stays bit-exact, the fused
  per-pixel `ColorChain` stays one GPU pass and graded 6.2K playback can't regress).
- A real graph compiles to **one `FxOp::GradeGraph(Arc<GradeProgram>)`** in the item's effect stage,
  at the same place in the pipeline (after the input transform, before the effect stack).
  `GradeProgram` mirrors C1's `comp::Program` (passes on slot textures, `Order::last_use` for slot
  liveness, peak slots counted, `MAX_LIVE_TEXTURES` 8 → refused and rendered ungraded with a
  message rather than exhausting VRAM: a 6.2K RGBA16F slot is ~208 MB). Passes:
  - `Copy { src, dst }`, `Fx { tex, ops }` (a Correction node's own steps: today's per-node
    `compile_fx` output on its slot: `ColorChain` / `HslSecondary` / `Masked` / `Denoise`);
  - `Matte { src_rgb, dst, … }`: a node's key out (qualifier × windows, the same maths the node
    already uses internally, now written to a slot) and `KeyIn` that multiplies a node's own matte
    by an incoming key (inverted / gained), so the Outside node is existing maths;
  - `Mix { srcs, weights, dst }` (parallel: weighted sum), `Over { src, dst, mode, opacity }` with the
    layer's key as alpha (layer mixer: C1's Merge pass, reused), `Channel { src, dst, which }` /
    `Combine { r, g, b, dst }` (splitter / combiner), `KeyMix { srcs, modes, dst }`.
- Each branch works on the **grading signal** (DaVinci Intermediate in a DWG sequence): the graph
  step encodes once at its start and decodes once at its end, exactly as the serial chain does, so
  mixing happens where a colourist expects it (Resolve also mixes on the timeline signal).
- Finite check as today (`step_finite` over every pass); a non-finite graph drops to no grade.

## 3. CPU reference first, then GPU

1. `render::grade::execute_graph_cpu(&GradeProgram, &Image) -> Image` is the oracle (like
   `comp::execute_cpu`); the CPU renderer uses it directly.
2. GPU: `crates/gpu` runs `GradeProgram` inside the effect stage with pooled textures (C1's
   `fx_frame_open` pool frame, so no per-frame allocation) and the existing effect shaders for
   `Fx`; new WGSL only for `Mix`, `Channel` / `Combine`, `KeyMix`, `Matte` / `KeyIn` (small
   per-pixel shaders; a Fable / Opus leaf builder per §1a.44). `Over` reuses the layer blend.
3. Parity tests (AMD and Intel): skin + background on a parallel mixer, a look on a layer mixer
   (each blend mode), an outside node, a splitter / combiner fixing one channel, a key mixer
   intersect, each against the CPU oracle (same bounds as the existing grade parity tests), plus a
   round trip through save / load and a serial-grade bit-exact regression.

## 4. UI, commands, coach

- The Color page node strip becomes `widgets/node_canvas.rs` (C1's canvas: drag to connect, ports
  drawn as Resolve's RGB / key triangles; serial-only grades still lay out as a row). The Premiere
  workflow's Lumetri panel is unaffected (Lumetri stays a fixed serial chain, §1a.14).
- Commands (all undoable, by id): `grade.addParallel`, `grade.addLayer`, `grade.addOutside`,
  `grade.addSplitter`, `grade.addKeyMixer`, `grade.connect` / `grade.disconnect` (`from` / `to`
  ports, refused when `dag::validate` would fail), `grade.setOutput`; existing `grade.*` commands
  keep their ids and behaviour on serial grades. Resolve keymap: Alt+P parallel, Alt+L layer,
  Alt+O outside (alongside Alt+S serial): **check each against the public manual when wiring the
  keymap**.
- Explainer (G2.10–11): branches read as "the skin and background nodes each work on the same
  image; their results are blended 50 / 50". Lessons (`coach_ui` walk tests): parallel mixer, layer
  mixer for looks, splitter / combiner, outside node, key mixer; guide sections on serial vs
  parallel vs layer (provenance [Resolve-style], differences stated). New strings get es + zh-cn.

## 5. Lanes (when C2 starts, mode permitting)

1. Model + schema v19 + migration / hostile tests + `DagNode` (main runner; everything waits on it).
2. CPU `GradeProgram` compile + oracle + render tests (builder A, `crates/render`).
3. GPU passes + parity (builder B, `crates/gpu`, after 2's types land; Fable / Opus per §1a.44).
4. Commands + canvas UI + keymap (builder C, engine / ui-egui, after 1).
5. Explainer, lessons, guide pages (Sonnet, after 4).
Accept per COMPOSITOR-PLAN §C2 (lessons end to end, CPU = GPU, save / load, graded 6.2K quick pass
without regression). Tag `comp-2`.
