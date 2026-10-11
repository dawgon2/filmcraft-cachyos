# ADR 0004: The grade model: per-clip serial colour nodes (fork)

- **Status:** accepted for the fork (2026-10-11, session 9). Design set by the owner in
  [`GRADE-PLAN.md`](../../GRADE-PLAN.md) §2 (approved 2026-10-11); this records how it is built.
- **Builds on:** [ADR 0003](0003-gpu-color-ops.md) (composable colour operations, DaVinci Wide
  Gamut / DaVinci Intermediate working space). ADR 0003 reserved the place between a layer's
  input transform and its effect stack for a node grader; this ADR fills it.

## Context

The fork needs a Resolve-style colour page (GRADE-PLAN G0–G2): a list of correction nodes per
clip, each with primaries (lift / gamma / gain / offset wheels, contrast / pivot, saturation,
temperature / tint), curves and later a qualifier, windows and a LUT. Everything those nodes
compute already exists as `filmcraft_color::ops::ColorOp`s and `FxOp`s that run on the CPU and in
one fused GPU pass (ADR 0003). What is missing is a data model for the nodes, a place in the
render where they run, and commands to edit them.

Effects (`TrackItem.effects: Vec<EffectInstance>`) are the obvious container, but an effect's
parameters are one flat keyframable map with a fixed schema per effect id. A node list (add,
insert, reorder nodes, each with its own corrections, and later a graph) doesn't fit it, and
mixing grade nodes into the effect stack would let a user drag a node below a Gaussian Blur,
which a colour page never shows.

## Decision

### 1. A grade is its own per-clip object

`TrackItem.grade: Option<Grade>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`:
old projects load unchanged (no grade), and a project without grades saves byte-for-byte as
before. `None` and an empty or bypassed grade render identically (nothing runs).

### 2. Serial nodes now, a graph later

```text
Grade     { nodes: Vec<GradeNode>, bypass: bool }
GradeNode { id: u64, label: String, enabled: bool, params: BTreeMap<String, Param> }
```

- **Ids, not indices.** Each node has a stable `id`, unique within its grade (a new node takes
  `max(id) + 1`). Commands address nodes by id. Parallel / layer nodes (GRADE-PLAN §6) later add
  an `inputs: Vec<u64>` list with `#[serde(default)]` (empty = the previous node), so the serial
  list becomes a DAG without a project-format break.
- **Parameters are the existing keyframable `Param`** (GRADE-PLAN §2.6), keyed by name, so node
  parameters animate, interpolate and serialise exactly like effect parameters, and
  `grade.setParam` can mirror `effects.setParam`. A missing key means the identity default; the
  node schema (`filmcraft_project::grade::NODE_PARAMS`) lists every key with its default and
  range. G0 / G1 keys:

  | key | value | identity | maps to |
  |---|---|---|---|
  | `lift`, `gamma`, `gain`, `offset` | `Color([r, g, b, master])` | lift 0, gamma 0, gain 0, offset 0 (wheel offsets) | `ColorOp::Lggo` |
  | `contrast`, `pivot` | `Float` | 1.0, 0.336 (DI 18 % grey) | `ColorOp::Matrix` around the pivot |
  | `saturation` | `Float` | 1.0 | `ColorOp::Saturation` |
  | `temperature`, `tint` | `Float` | 0, 0 | `ColorOp::Matrix` (white-balance gains) |
  | `curve.master`, `curve.r`, `curve.g`, `curve.b` | `Curve` | the straight line | `ColorOp::Curve` |

  The wheels store offsets around identity (all zero = no change) because that is how the wheel
  widget and a "reset" behave; the compiler turns them into `Lggo`'s absolute lift / gamma /
  gain / offset. The `master` component is the wheel's luma ring and adds to all three channels.
  G2 adds qualifier, window and LUT keys (and `windows: Vec<Mask>` reusing `mask.rs`) the same
  way, as `#[serde(default)]` fields or new keys.
- **Hostile input.** Every value from a project file is clamped to its schema range when the grade
  is compiled; non-finite values fall back to the identity; a grade compiles at most
  `MAX_NODES` (64) nodes and ignores the rest (the render crate doesn't log; the G1 node strip shows
  the count). A chain that still comes out non-finite is dropped whole. Unknown keys are kept (round-trip)
  and ignored.

### 3. Where it runs

```text
decode → input transform (log decode + gamut → working space)
       → grade nodes   (on the DaVinci Intermediate signal, like Lumetri)
       → effect stack  (Lumetri and every other effect, in order)
       → composite → output transform
```

The grade sees the same signal Lumetri sees (ADR 0003 "Working space": grading operations see
the DI signal when the sequence works in DWG / DI, and the working space's own signal
otherwise), so a node and Lumetri with the same numbers do the same thing.

### 4. No new grading maths

`filmcraft_render::grade::compile(&Grade, clip_time) -> Vec<ColorOp>` evaluates each enabled
node's parameters at the clip time and emits existing `ColorOp`s; the chain is prepended to the
layer's colour stages, so on the GPU it joins the same fused pass as Lumetri (one read and one
write of the working texture), and on the CPU it runs through `ops::apply_chain`. Identity
parameters emit nothing, so an identity grade is bit-exact with no grade at all (tested on the CPU
and on the GPU). Only film grain (G1) and noise reduction (G2) add new GPU kernels, each with a
CPU oracle and parity tests like every `FxOp`.

### 5. Everything is a command

`grade.*` commands (G0: none needed for rendering; G1: `addNode`, `insertNode`, `removeNode`,
`moveNode`, `toggleNode`, `labelNode`, `setParam`, `resetNode`, `bypass`, `copy`, `paste`,
`append`, `stills.grab` / `apply` / `delete`, `compare.set`) edit through `Session::edit`, so
each is one undo step, and the CLI, control channel and MCP reach them by id. Every widget of the
colour page registers an automation id. G0 also adds the missing `effects.move` (reorder the
effect stack).

## Consequences

- Lumetri stays exactly as it is; the node grader and Lumetri can be used together (grade first,
  then Lumetri in the effect stack).
- Copy / paste attributes (`clip_ops.rs`) learns about `grade` in G1 (grade-specific copy /
  paste / append).
- The render cache (FORK-PLAN Phase 6) keys a layer by its compiled grade as well as its effects.
- Adjustment layers take a grade too: it runs on the composite below before the layer's effects,
  like a clip's grade before its effect stack (CPU only, as adjustment layers are today).
- An identity grade (an empty node, as opening the Color page on a clip creates) keeps every render
  shortcut the clip had without one (`grade::changes_picture`): titles stay vector-sharp, nested
  sequences keep drawing their own layers.
- A grade is per clip (per `TrackItem`), like Resolve's clip grade in its default mode. Grade
  groups, timeline grades and shared "remote" grades are deferred (GRADE-PLAN §6); the data model
  leaves room for them in `Sequence` without changing `Grade`.
