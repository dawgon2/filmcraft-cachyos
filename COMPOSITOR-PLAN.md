# COMPOSITOR-PLAN.md: a Fusion-style node compositor, simple first

Written 2026-10-11 (session 8, with the owner). **Phase C1** (a deliberately small compositor) and **C2** are
scheduled **right after Y3, before Phase 7** (owner, moved 2026-10-11, so Phase 7's soak and real-edit check cover them): FORK-PLAN §1a.6: G0 ✓ → G1 ✓ → U0 ✓ → G2 → G2b → 6 → Y1 → 6E → U0.2 → Y3 → C1 → C2 → 7 (+ Y4) → D → 8 → Y2 → H1 → H2 → H3.
**C2 (the grader's parallel / layer nodes) is scheduled (owner, 2026-10-11). C3 and C4 are approved and scheduled after H3 (owner, 2026-10-12, FORK-PLAN §1a.6 / §1a.35); C5 and later stay deferred: don't start them without the owner.** All FORK-PLAN rules apply
(never crash, `Tick` time, everything is a command, every widget has an automation id, layering).

Fusion (in DaVinci Resolve) composites with a graph of nodes instead of stacked layers, e.g.
`footage → keyer → merge over background → blur → colour → output`. Nodes can take several inputs,
which is what the G-phase grader's straight node chain can't do (GRADE-PLAN §2 keeps grades serial
and defers parallel / layer nodes, §6).

## Phase C1: simplified node compositor (≈ 25–40 h)

1. **Model:** a **Comp clip** (a new timeline item kind, `#[serde(default)]` so old projects load)
   holding a node graph: a DAG with node ids, typed input / output ports and connections; cycles
   rejected; node count, graph depth and texture memory capped (hostile project files). ADR 0007 ([`docs/adr/0007-node-graph.md`](docs/adr/0007-node-graph.md); U0 took 0005, Y3.4 0006).
2. **Nodes (only these):** Media In (a project clip or a timeline layer), Media Out, **Merge**
   (A over B with the existing blend modes and opacity), **Transform** (position / scale / rotation /
   anchor, existing motion maths), **Key** (Y3's GPU keys), **Mask** (existing mask shapes and the
   tracker's keyframes), **Blur**, **Color** (one grade node, reusing GRADE-PLAN's corrector),
   **Text** (existing text renderer), **Solid / Gradient**. Every parameter keyframable with the
   existing `Param` type.
3. **Evaluation on the GPU:** topological order compiled to passes over intermediate textures,
   reusing the existing compositor, `ColorOp` / `FxOp` passes and blend shaders; results cached per
   frame while parameters don't change; CPU path for export parity tests.
4. **UI:** a node canvas (pan / zoom, add node menu, drag to connect, select / delete, bypass a
   node), a viewer that can show any node's output, and an inspector for the selected node's
   parameters. Plain egui painter code, no graph library. Opens from the Comp clip ("Open in
   Compositor"), with a Resolve-style "Fusion" page when Workflow is Resolve (GRADE-PLAN G1.8).
5. **Commands** (`comp.*`: addNode, removeNode, connect, disconnect, setParam, bypass,
   newCompClip, convertToCompClip from selected clips), all undoable, all drivable over MCP.
6. **Grading Coach fixes (owner, 2026-10-12, FORK-PLAN §1a.40 / §1a.28):** (a) the Coach answers
   workflow / "what order" questions the guide already covers (retrieval fix + a regression
   question set built from `docs/fork/COACH-QUESTIONS.md`); (b) a written plan for the process
   assistant (model-backed answers grounded in the guide and the project, checked so it can't
   invent controls) for the owner to place. Not compositor work, but scheduled here by the owner.
   **Update (owner, 2026-10-12, §1a.47):** the fix's steps 1–2 and the node-order page ship earlier
   (step CO1 in the resolve-ui-1 + RS3 + 6.7b + Y1 group); C1 keeps steps 3–5 (question-type
   routes, heading weights; `docs/fork/coach-eval/DIAGNOSIS.md`).
7. **AI help for Fusion (owner, 2026-10-12, §1a.47):** guide pages for every comp node and the
   compositing workflow (options with trade-offs, beginner vs pro, never one prescribed way); a
   comp-graph explainer (plain language, node by node, like G2.10; template-based, local model
   optional, checked so it can't invent controls); Coach lessons per comp node type; compositing
   questions added to `docs/fork/coach-eval/questions.toml`; the process-assistant plan covers
   compositing.

**Not in C1:** 3D, particles, paint / roto brush, tracker nodes (masks use the existing tracker),
expressions, macros / groups, planar tracking, deep compositing, the grader's own parallel /
layer nodes.

**Accept:** a green-screen comp (Media In → Key → Merge over a background → Color → Media Out) plus
a tracked title (Text → Transform driven by tracker keyframes → Merge) builds through commands,
plays in real time at 1080p and 4K at half resolution (measured, synthetic), exports matching the
viewer, GPU parity on AMD and Intel, hostile-graph tests never crash; a short soak (30 min of
editing and playing comps through the control channel) passes. `cargo xtask ci`, tag `comp-1`,
build per §1a.9.

## Phase C2: parallel and layer nodes in the grader (scheduled, owner 2026-10-11; ≈ 20–35 h)

Right after C1, on C1's node DAG, so the grade graph and the compositor share one graph model,
evaluation engine and canvas.
1. **Grade model → DAG:** `Grade` keeps its node ids (ADR 0004 planned for this) and gains inputs
   per node; serial grades load unchanged (schema bump with migration test).
2. **Node types:** **Parallel mixer** (several nodes take the same input; outputs blended back,
   equal weights by default, per-input gain), **Layer mixer** (bottom-to-top stack with blend mode
   and opacity per layer; the higher layer wins where it has a key), **Splitter / Combiner**
   (per-channel R / G / B work), **key mixer** (combine keys: add / subtract / intersect / invert),
   **key inputs / outputs** between nodes (a node's qualifier / window matte used by another),
   and **outside node** (the inverse of the previous node's key).
3. **Evaluation:** GPU passes over intermediate textures, reusing C1's scheduler and the existing
   `ColorOp` chain per node; parity tests against a CPU reference graph.
4. **UI:** the Color Grading node strip becomes the C1 node canvas (drag to connect, add Parallel /
   Layer / Outside from the menu, Resolve's shortcuts for them in the Resolve keymap).
5. The node explainer (GRADE-PLAN G2.10–11) explains branches too ("the skin and background
   nodes each work on the original image; their results are blended 50 / 50").
6. **Grading Coach lessons for branches** (GRADE-PLAN G2.12's lesson engine): *When to use a
   parallel mixer* (skin and background graded from the same image) step by step; *Layer mixer
   for looks* (base grade + a look layer, blend modes, opacity); *Splitter / combiner* (fixing one
   colour channel); *Outside node* (grade everything except the subject); *Key mixer* (combining
   two selections); plus coach guide sections on choosing serial vs parallel vs layer.
**Accept:** each new lesson completes end to end through the control channel; skin + background on parallel nodes, a look on a layer node and an outside node all
render identically on CPU and GPU, round-trip through save / load, and play graded 6.2K in the
quick pass without a regression beyond noise. Tag `comp-2`.

## Phase C3: tracking, paint and roto (approved, owner 2026-10-12; ≈ 60–120 h)

Tracker node (reuses `crates/render/src/track.rs`: point / multi-point, stabilise, match move,
corner pin) and a **planar tracker** node (homography tracking of a user-drawn region, with
corner-pin output); GRADE-PLAN G4 reuses it for planar-tracked windows. **Paint** (clone / reveal
/ colour strokes, per frame or held, stroke list editable) and **roto** (Bézier / B-spline shapes,
per-point feather, keyframed, track-assisted). **Accept:** synthetic motion with a known path is
tracked within a set pixel error (point and planar); a corner-pinned insert stays locked; roto and
paint strokes survive save / load. Tag `comp-3`.

## Phase C4: expressions, macros and templates (approved, owner 2026-10-12; ≈ 30–60 h)

A small, safe expression language for parameters (time, other parameters, arithmetic, `sin` /
`noise` etc.; no I/O, bounded evaluation, parse errors shown not crashed), node groups / macros
with exposed parameters, and saved comp templates. **Accept:** hostile expressions (deep nesting,
huge loops, NaN) never crash or hang; a macro round-trips through save / load. Tag `comp-4`.

## Deferred (needs the owner): C5 and later

- **C5:** 3D (cameras, 3D text, simple lights), particles. Owner, 2026-10-12: maybe in the
  latter reaches, unsure how much use it would serve; reconsider after C4 (Blender and Send to
  Resolve cover most of it meanwhile).
- Plugin nodes come from PLUGIN-PLAN.md once that exists.
