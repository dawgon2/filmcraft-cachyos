# Design 6.7b: temporal noise reduction per clip (owner decision Q9 (c))

Written 2026-10-12 (design-only lane, no build). Implements FORK-PLAN §1a.40 (a) and
OWNER-QUESTIONS Q9: temporal noise reduction (TNR) stops being a grade-node option and becomes a
separate **clip-level "Temporal NR"** applied before the grade, like Resolve's per-clip NR. Same
four parameters; saved projects migrate. Scheduled right after RS3, before Y1, accepted in the
first group (§1a.41: `resolve-ui-1` + RS3 + 6.7b + Y1, tag `phase-6-7b`).

Line numbers are as of `5b27f3ab`; re-check before relying on them. Read with
[`PHASE6-DESIGN.md`](PHASE6-DESIGN.md) §6.7 (the kernel and the neighbour rules, unchanged here).

## 0. What exists today (facts found, don't re-research)

| Where | What |
|---|---|
| `crates/project/src/grade.rs:125-132` | `NODE_PARAMS` has `tnr.luma` (0, 0–1), `tnr.chroma` (0, 0–1), `tnr.frames` (1, 1–2), `tnr.motion` (0.5, 0–1); missing key = identity; `GradeNode::float_at` clamps (NaN / mistyped → identity). Test `temporal_nr_keys_default_to_identity_and_clamp`. |
| `crates/render/src/grade.rs:76-150` | `changes_picture(project, item, t, temporal)`, `is_media`, `temporal_node(grade, t)` (**first active node only**, provisional Q9), `temporal(n, t) -> Option<FxOp::TemporalDenoise>`, `uses_temporal(item, t)`, `ever_temporal(project, item)` (clip start / end / every tnr keyframe), `temporal_at(project, seq, t)`, `strip_temporal(ops)`. |
| `crates/render/src/grade.rs:214-235` | `compile_fx` pushes the temporal op **before everything** (whole picture, no window / qualifier), then the nodes. |
| `crates/render/src/grade.rs:53-65` | `item_fx(project, item, t, seq_t, working, px_scale, w, h)`: the one entry every render path calls (CPU `lib.rs:525`, GPU `plan.rs:543`, adjustment layers `lib.rs:251` followed by `strip_temporal`). |
| `crates/render/src/temporal.rs` | neighbours (`neighbour_times`, `clip_bounds`, holds / speed / reverse / transitions), `FlowCache` (LRU by media identity + frames + size), `attach_frames` (GPU: `NeighbourPicture::Frame`, the GPU replays the ops before the temporal op) and `attach_images` (CPU: `base_layer`). Both start with `wanted(ops, project, item, opts)` and `strip_temporal` otherwise. |
| `crates/render/src/gpufx.rs:218-330` | `FxOp::TemporalDenoise { luma, chroma, radius, motion, inputs }`, `TemporalInputs`, `apply_ops_with(ops, img, outer, temporal)` (replays the ops before a temporal op on its source-frame neighbours; also inside `Masked`). |
| `crates/render/src/tnr.rs`, `flow.rs`, `gpu/src/fx.rs` / `fx.wgsl`, `gpu/tests/tnr_render.rs` | the kernel, flow, GPU pass and parity test. **Untouched by 6.7b.** |
| `crates/render/src/lib.rs:87-96` | `RenderOptions.temporal` (default false; export / render cache true; paused Program frame true). |
| `crates/render/src/preview.rs:309-323, 486-492` | `hash_item` hashes the **whole `TrackItem` JSON** (minus id / start / label / markers / link / group) → any new clip field is in the segment hash automatically; `ever_temporal` adds `TEMPORAL_NR_COST_MS` so the segment is "needs rendering". `HASH_VERSION = "filmcraft-preview-v3"`. |
| `crates/export/src/pipeline.rs:172-174, 538-660` | export renders with `temporal: true`; test `export_renders_temporal_nr_like_the_paused_program_frame` builds a grade with `tnr.*` on a node. |
| `crates/ui-egui/src/frames.rs:66-76, 1284-1295` | `FrameKey.temporal`, `temporal_frame(playing, pointer_down, uses)`, `job_temporal`. |
| `crates/ui-egui/src/panels/monitor.rs:210-250, 514-516` | the paused Program frame asks for a temporal render when `grade::temporal_at` says a clip uses it; the non-temporal frame is shown first. |
| `crates/ui-egui/src/panels/grade.rs:556-575, 798` | Noise tab: `float_rows` for `tnr.*` under a "Temporal" label, the "first active node only" note; `tnr.frames` steps by 1. Automation ids `grade.param.tnr.*` (+ `.reset`). |
| `crates/render/src/grade_match.rs:102-114` | "same grade?" comparison treats `tnr.frames` / `tnr.motion` as live only when `tnr.luma` / `tnr.chroma` are present. |
| `crates/engine/src/keyboard.rs:600-640` | `file.exportFrame` renders the sequence frame with default options (`temporal: false`): **Export Frame has no TNR** (owed item in HANDOFF). |
| `crates/engine/src/clip_ops.rs:88, 1042-1069` | Copy / Paste Attributes has a `grade` flag. |
| `crates/ui-egui/examples/bench/xm5.rs:268-286` | bench `export-tnr`, `cache-tnr` put `tnr.*` on a node (`add_tnr_node`). |
| `crates/ui-egui/src/i18n/{es,zh-cn}.tsv` | 8 rows each for the Temporal strings. |
| `crates/format/src/lib.rs:77-82, 430-433` | `MIGRATIONS` table, `SCHEMA_VERSION = len + 1` (**v16** now), `v15_to_v16` no-op; fork lineage marker; tests in `crates/format/tests/fixtures.rs` with hand-made fixtures per schema. `commands.rs:3896-3991`: an upgraded project keeps `<name> (schema vN backup).fcproj` on the first save and toasts. |
| `docs/guide/color/noise.md` | "Temporal noise reduction" section written for the node-level v1 (first node, Noise tab). |
| `docs/guide/README.md` | page table says noise.md covers "temporal on the first node (6.7)". |

## 1. Decisions

### 1.1 Model: `TrackItem.temporal_nr`

```rust
// crates/project/src/tnr.rs (new, L1, serde; re-exported from lib.rs)
/// Clip-level temporal noise reduction (fork 6.7b, Q9 (c)): applied to a media / subclip
/// item's picture after the input transform and before its grade and effects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalNr {
    /// Off keeps the values (the Inspector's switch); default on.
    #[serde(default = "on", skip_serializing_if = "Clone::clone")]
    pub enabled: bool,
    /// Keyed by `TNR_PARAMS` (`luma`, `chroma`, `frames`, `motion`); a missing key is the
    /// identity; unknown keys are kept (round trip) and ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Param>,
}

/// The schema: the same four controls as the v1 node keys, without the `tnr.` prefix.
pub const TNR_PARAMS: [NodeParam; 4] = [
    NodeParam { key: "luma",   label: "Temporal NR luma",   kind: Float { identity: 0.0, min: 0.0, max: 1.0 } },
    NodeParam { key: "chroma", label: "Temporal NR chroma", kind: Float { identity: 0.0, min: 0.0, max: 1.0 } },
    NodeParam { key: "frames", label: "Temporal NR frames", kind: Float { identity: 1.0, min: 1.0, max: 2.0 } },
    NodeParam { key: "motion", label: "Temporal NR motion", kind: Float { identity: 0.5, min: 0.0, max: 1.0 } },
];

impl TemporalNr {
    pub fn float_at(&self, key: &str, t: Tick) -> f32;   // schema clamp, NaN / mistyped → identity
    pub fn is_identity_at(&self, t: Tick) -> bool;       // luma == 0 && chroma == 0 (or !enabled)
    pub fn is_identity_ever(&self) -> bool;              // static values and every keyframe
}
```

- `TrackItem` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub temporal_nr:
  Option<TemporalNr>` right after `grade` (`crates/project/src/lib.rs:596-599`). `None`, `Some`
  with identity values and `enabled: false` render identically (nothing runs), and a project
  without TNR saves byte-for-byte as before (the field is skipped).
- `Param` is the existing keyframable type, so the values animate exactly as the node keys did
  and migrated keyframes move unchanged.
- `NodeParam` / `NodeParamKind` / the clamp helper move from `grade.rs` into a small shared
  `param_schema.rs` (or stay in `grade.rs` and are `pub use`d): no second clamp implementation.
- `tnr.*` keys are **removed from `NODE_PARAMS`**. `grade.setParam tnr.luma` is refused with
  "temporal NR is a clip setting since schema v17: use `tnr.set`" (a clear error, not silent).
  Unknown keys in a node's map still round-trip (ADR 0004), so a file edited by hand with
  `tnr.*` on a node after v17 loads, keeps the keys and ignores them.

### 1.2 Project schema v17 and the migration

`crates/format/src/lib.rs`: append `v16_to_v17` to `MIGRATIONS` (`SCHEMA_VERSION` becomes 17);
module docs get a **v17** line ("fork 6.7b: clip-level temporal NR, `TrackItem::temporal_nr`;
node `tnr.*` keys moved to the clip"). The step is the first fork migration that **changes
data**, so it is written defensively on `serde_json::Value` (never on the typed model: a file
with fields this build can't type must still migrate):

1. Walk every project item whose `kind` is `Sequence` (`items` map, nested sequences are
   inline), every `video_tracks[*].items[*]` (and nothing else: audio tracks have no grades).
2. For a clip with `grade.nodes`, find **the first node with `enabled != false`** (serde default
   true): the node whose `tnr.*` rendered in v1 (`temporal_node`: first *active* node). Take its
   `tnr.luma` / `tnr.chroma` / `tnr.frames` / `tnr.motion` `Param` values as they are
   (keyframes included), strip the `tnr.` prefix and write
   `temporal_nr: { enabled, params }` when at least one of the four keys is present.
   `enabled` = `!grade.bypass` (a bypassed grade rendered no TNR in v1; after migration the clip
   setting is independent of the grade, so it starts off to keep the picture). Keys with
   non-finite / mistyped values are copied as they are (the clamp handles them at render time,
   exactly as before).
3. Remove every `tnr.*` key from **every** node of the grade (disabled and later nodes included).
   Their values were ignored in v1 ("these values are ignored here"), so dropping them keeps the
   render bit-exact; the migration counts them and the loader's toast says
   "Temporal NR moved from the grade to the clip on N clips; M ignored node settings dropped".
   (The schema backup file keeps the original, `commands.rs:3920`.)
4. **Which node wins if several carry `tnr.*`:** the first enabled node, always, because that is
   what rendered. A grade whose first enabled node has no `tnr.*` but a later node does gets
   **no** clip TNR (that TNR never rendered); the count in the toast tells the user.
5. A clip that already has `temporal_nr` (a file hand-edited or written by a newer build and
   downgraded) keeps it; node keys are then dropped without overwriting it.
6. Nested sequences are project items too, so step 1 covers them; multicam source sequences
   likewise. Subclips: the clip (`TrackItem`) carries the grade, so nothing special.

Fork lineage (`FORK_LINEAGE`, `fork_shape`): v17 files carry `"lineage": "fork/hevc-vaapi"` as
every fork file does. **Risk for U0.2:** if upstream ships its own v17 before the next sync, the
lineage marker already refuses the other lineage's v17 (`OtherLineage`); the sync then renumbers
(the fork's v17 becomes v18 after upstream's v17, with a `fork_shape` rule "has `temporal_nr`").
Record this in UPSTREAM-SYNC.md when it happens; nothing to do now.

### 1.3 Compile: the op comes from the clip, not the grade

- `render::grade::item_fx` (the single entry) prepends the clip op:
  `if let Some(op) = crate::temporal_nr::op_at(item, t) { out.insert(0, op) }` where
  `temporal_nr::op_at(item, t) -> Option<FxOp::TemporalDenoise>` reads `item.temporal_nr`
  (enabled, both strengths clamped, `frames` rounded to 1..=MAX_RADIUS, `inputs: None`). The op
  keeps its exact position (after the input transform, before every grade op and effect), so the
  rendered picture is **bit-exact** with v1's first-node TNR: same kernel, same inputs, same place.
- `compile_fx` loses its `temporal_node` block (and `temporal_node` / `temporal` / `uses_temporal`
  / `ever_temporal` / `temporal_at` move to `crates/render/src/temporal_nr.rs`, reading the clip
  field; `grade.rs` keeps `pub use` shims so `monitor.rs`, `preview.rs`, `plan.rs`, `lib.rs`
  compile unchanged, then the shims go in the same step).
- `changes_picture(project, item, t, temporal)` → `temporal && is_media(project, item) &&
  temporal_nr::uses_at(item, t)`; the grade part is unchanged. Adjustment layers keep
  `strip_temporal` (the field is ignored on non-media items; the UI hides it there).
- **Cheaper neighbours.** v1 already placed the op first, so the per-frame cost is unchanged
  (flows + ≤ 4 neighbour draws). What gets simpler: the "op before the temporal op" replay in
  `apply_ops_with` / the GPU stage now only ever replays the layer's **input transform** (one
  fused `ColorChain`), never grade ops; `TemporalDenoise` can no longer sit inside `Masked`.
  Cleanup (optional, step 4): drop the `Masked`-with-temporal branch of `apply_ops_with` and
  `gpu_ok`'s `TemporalDenoise inside Masked` rule, keep `before` = the input ops only. Do it only
  if the parity tests stay green; otherwise leave the general code (it is correct, just general).
  Option (b) of Q9 (re-grading neighbours) is never needed: with the clip setting the neighbours
  are always plain decoded clip frames after the input transform.

### 1.4 Cache keys and invalidation

- **Segment hash (render cache, previews):** `hash_item` serialises the whole clip, so
  `temporal_nr` is hashed with no code change; editing any TNR value changes the segment hash
  (existing test `segment hash changes on a tnr edit` is re-pointed at the clip field).
  `ever_temporal` → `temporal_nr::ever(item)` keeps adding `TEMPORAL_NR_COST_MS`.
- **`HASH_VERSION` stays v3.** A migrated project serialises differently (keys moved), so its
  segments get new hashes and the old parts become stale → evicted by the index's stale-hash
  rule and re-rendered once. Bit-exact output means stale parts were never *wrong*; a version
  bump would throw away every cached part of every project for nothing. Test: a v16 file's
  segment hash after migration ≠ before (documented), and two v17 projects with equal TNR hash
  equal.
- **Flow cache** (`FlowCache`: media identity + source frames + size): unchanged; it never keyed
  on the grade.
- **Frame server** (`FrameKey.temporal`, `job_temporal`, `monitor.rs` double request): unchanged;
  `temporal_at` now reads the clip field. Revision bump on every `tnr.*` command invalidates the
  monitor cache as any edit does.
- **Grade stills / "same grade?"** (`grade_match.rs:102-114`): the `tnr.*` lines go; TNR is no
  longer part of a grade, so two clips with the same grade and different TNR are "same grade"
  (as in Resolve). Copy / paste / append grade no longer carry TNR; Paste Attributes gains
  `temporalNr: bool = true` (§1.6).

### 1.5 Export, Export Frame, proxies

- **Export / render cache:** `pipeline.rs` keeps `temporal: true`; nothing changes except the
  test fixture (clip field instead of node keys).
- **Export Frame** (`file.exportFrame`, owed item): the sequence branch renders with
  `RenderOptions { temporal: true, captions: true, .. }`, so the still matches the export (one
  frame, ±2 neighbours, ~0.3–0.6 s at 6.2K; acceptable for a still). The source-monitor branch
  (`render_item`) stays without TNR (a source frame has no clip). Guide "Limits" line removed.
- **Proxies:** neighbours are fetched through the same source the current frame came from
  (`clip_frame` / `base_layer`), so with `media.proxies = on` the paused frame cleans the proxy
  (flows at the proxy's size, cached separately by size), and with `auto` the paused frame and
  the export use the original (Phase 6.2 rule). No change; documented in the guide.

### 1.6 Commands (`crates/engine/src/tnr.rs`, registered from `commands.rs` next to `grade`)

All edits go through `Session::edit_sequence` (one undo step each, label shown), `journal: true`;
`clip` defaults to the first selected video clip (`grade::target`). Non-media clips are refused
("temporal NR applies to media clips and subclips").

| id | label | params | notes |
|---|---|---|---|
| `tnr.get` | Get Temporal NR | `{"clip":id?}` | query; `{enabled, params:{luma,chroma,frames,motion}, identity:bool}` with evaluated values at the playhead and raw `Param`s |
| `tnr.params` | List Temporal NR Parameters | `{}` | query; `TNR_PARAMS` as `grade.params` does |
| `tnr.set` | Set Temporal NR Parameter | `{"clip":id?,"param":str,"value":number?,"reset":str?,"time":ticks?}` | mirrors `grade.setParam` (keyframe at `time` when animated); creates `temporal_nr` on first use; `merge` key `tnr.<clip>.<param>` so a slider drag is one undo step |
| `tnr.toggle` | Enable / Disable Temporal NR | `{"clip":id?,"on":bool?}` | |
| `tnr.toggleAnimation` | Toggle Temporal NR Parameter Animation | `{"clip":id?,"param":str,"time":ticks?}` | as `grade.toggleAnimation` |
| `tnr.reset` | Reset Temporal NR | `{"clip":id?,"param":str?}` | one key or all (removes the field when everything is identity) |
| `tnr.clear` | Remove Temporal NR | `{"clip":id?}` | sets `None` |
| `tnr.copy` / `tnr.paste` | Copy / Paste Temporal NR | `{"clip":id?}` / `{"clips":[id]?}` | menu `Clip ▸ Temporal NR` (menu built from `CommandSpec.menu`; if `menus.rs` needs a hand-written entry, skip the menu entry: the commands still work from the command palette / MCP) |
| `clip.pasteAttributes` | (existing) | `+ "temporalNr": bool = true` | copies the field with the grade |

Hostile params: `param` must be one of the four keys; `value` finite (else refused, as
`grade.setParam`); `time` through `time_p`. Every refusal is an `EngineError::BadParams`, never a
panic. The MCP tools see the commands automatically (`command_list`).

### 1.7 UI placement (both workflows)

One shared widget, `crates/ui-egui/src/panels/tnr.rs` → `tnr_rows(ui, item, t, theme, &mut
cmds, &mut autos)`: an Enabled switch and the four rows (reusing `panels/grade.rs::float_rows`
made `pub(crate)` with a key-prefix argument, so the keyframe diamond / reset button / step-by-1
for `frames` behave exactly as in the Noise tab). Automation ids: `tnr.enabled`, `tnr.luma`,
`tnr.chroma`, `tnr.frames`, `tnr.motion`, each with `.reset` and `.anim`. Hidden on non-media
clips.

- **Premiere workflow — Effect Controls:** a fixed section **Temporal NR** after Time Remapping
  (`panels/effect_controls.rs:152`, where the fixed effects are listed), collapsible like Motion.
  The note under it: "Shown on the paused frame, at export and in the render cache; not while
  playing."
- **Resolve workflow — Inspector:** the same panel is relabelled Inspector by RS2b's terminology
  overlay, so the section appears there without more code (RS2b: on-screen words only).
- **Resolve workflow — Color page clip area:** the Noise tab (`panels/grade.rs` `"noise"`) keeps
  the node's spatial rows and replaces its "Temporal" block with a **"Temporal NR (clip)"**
  block drawn by `tnr_rows` for the selected clip, headed with the clip name so it is clearly not
  a node setting (Resolve's Motion Effects palette is per clip the same way). The "first active
  node" note goes. Both places edit through `tnr.*`, so they always agree.
- i18n: new strings in `es.tsv` and `zh-cn.tsv` (the 8 existing Temporal rows are reused where
  the English text is unchanged; "Temporal NR only runs on the first active node…" is deleted,
  "Temporal NR (clip)", "Enabled", the moved-to-clip toast are added). `i18n/mod.rs` is not
  touched (only the catalogue files).

### 1.8 Guide

- `docs/guide/color/noise.md`: the "Temporal noise reduction" section becomes **[Resolve-style]
  [Fork]** "a clip setting, not a node": steps use `{ui:tnr.luma}` … and `tnr.set`; "Limits"
  drops the first-node rule and the "Export Frame does not include it yet" line; "If you know
  Resolve" says: per clip like Resolve's, two strengths + frames + motion. Parameters for
  scripts: `tnr.set luma|chroma|frames|motion`.
- `docs/guide/README.md` table row for noise.md: "temporal per clip (6.7b)".
- `docs/guide/color/nodes.md` / `recipes.md`: any "put noise reduction on the first node" step
  that meant temporal NR now says "spatial NR on the first node; temporal NR is on the clip".
- `coach_ask.rs` `GUIDE_PAGES` unchanged (same files); the `coach_ui` test must know the new
  `{ui:tnr.*}` ids (they register in Effect Controls and the Noise tab).
- Coach lessons / hints that mention `grade.param.tnr.*` (grep `crates/engine/src/coach*.rs`,
  `assets/coach/`): re-point to `tnr.*`.

## 2. Files touched vs the RS lane's don't-touch list

The RS lane (RS2 / RS2b / RS3) owns `crates/ui-egui/src/{app.rs (lib.rs), menus.rs, theme.rs,
workspaces*, i18n/mod.rs}`, autosave / project code and the grade model / compiler. 6.7b must
touch:

| must touch | on the list? | why it can't be avoided |
|---|---|---|
| `crates/project/src/lib.rs` (`TrackItem` field), `crates/project/src/grade.rs` (`NODE_PARAMS`, schema types), new `project/src/tnr.rs` | **yes (project code, grade model)** | the setting lives on the clip |
| `crates/format/src/lib.rs` + fixtures | **yes (project / save code)** | schema v17 migration |
| `crates/render/src/grade.rs` (`compile_fx`, `changes_picture`, helpers moved out) | **yes (grade compiler)** | the op must stop coming from the node |
| `crates/engine/src/clip_ops.rs` (`pasteAttributes`), `commands.rs` (register `tnr::commands()`, migration toast) | partly (project code) | small, additive |
| `crates/ui-egui/src/panels/{grade.rs, effect_controls.rs, monitor.rs}`, new `panels/tnr.rs`, `i18n/{es,zh-cn}.tsv` | no | — |
| `crates/render/src/{lib.rs, plan.rs, preview.rs, temporal.rs, gpufx.rs (optional cleanup)}`, `export/src/pipeline.rs` (test), `engine/src/keyboard.rs` (Export Frame), bench `xm5.rs` | no | — |
| `menus.rs` | **yes** | **not touched**: menu entries come from `CommandSpec.menu`; if a hand-written entry is needed, defer it |

So 6.7b runs **after RS3 merges** (as §1a.40 planned) and nothing in it conflicts with RS3's
layouts. The RS lane and 6.7b are never in flight together.

## 3. Tests (exact list)

Project / format (`crates/project`, `crates/format`):
1. `temporal_nr_round_trips_and_skips_when_none` — serde: `None` writes no field; identity
   `Some` round-trips; unknown keys kept.
2. `temporal_nr_float_at_clamps_hostile_values` — NaN / ±inf / 7.0 / −1 / `Text("2")` → clamped
   or identity, `frames` rounds to 1 or 2.
3. `node_params_no_longer_list_tnr_keys` — `NODE_PARAMS` has no `tnr.` key; `node_param("tnr.luma")` is `None`.
4. `v16_to_v17_moves_first_enabled_nodes_tnr_to_the_clip` — fixture `v16-tnr.fcproj` (hand-made:
   three clips: (a) tnr on node 1 with a keyframe, (b) node 1 disabled + tnr on node 2 + ignored
   tnr on node 3, (c) bypassed grade with tnr): after load (a) has `temporal_nr` with the keyframe,
   (b) takes node 2's values and drops node 3's, (c) has `enabled: false`; **no node keeps a
   `tnr.*` key**; the project then saves as v17 and loads again identical (round trip).
5. `v16_to_v17_leaves_everything_else_alone` — a v16 fixture without TNR migrates to a document
   equal to the input plus the version stamp (byte-compare the `Value`).
6. `v16_to_v17_survives_hostile_documents` — `grade` not an object, `nodes` not a list, a node
   without `params`, `enabled: "yes"`, `items` with a non-sequence kind, a nested sequence: no
   panic, the step returns `Ok` or a `Migration` error, never corrupts other clips.
7. `fixtures.rs`: `v16_tnr_loads` + the existing "every schema" loop picks up the new fixture;
   `schema_version_matches_table` (17).
8. `migrated_projects_write_the_lineage_marker_and_v17`.

Render (`crates/render`, `temporal_tests.rs` rewritten to build clips with `temporal_nr`):
9. `clip_temporal_nr_renders_bit_exact_with_the_old_first_node_tnr` — **the key test**: the
   fixture project built the v1 way (as JSON with `tnr.*` on node 1, run through `v16_to_v17`)
   and the same project built with `temporal_nr` directly render identical images (CPU,
   `max_diff == 0.0`), with ±1 and ±2, with a later grade node that changes the picture, and
   inside a dissolve.
10. `the_op_is_first_in_the_layer_ops_before_every_grade_op` — `item_fx` output starts with
    `TemporalDenoise` and never contains one inside `Masked`.
11. `tnr_keys_left_on_a_node_are_ignored` — a v17 node carrying `tnr.luma` renders as without it.
12. `changes_picture_and_uses_at_follow_the_clip_field` — enabled / disabled / identity / non-media
    (nested, graphic, adjustment) cases; an adjustment layer with the field renders unchanged.
13. `ever_and_temporal_at_cover_keyframes` — static 0 with a keyframe to 0.5 later → `ever` true,
    `temporal_at` true only at times where strength > 0.
14. `segment_hash_changes_on_a_tnr_edit_and_not_on_grade_edits_elsewhere` — plus "migrated hash
    differs from the v16 hash" documented.
15. Existing: `only_temporal_renders_blend_and_they_clean_the_noise`, `neighbours_…`,
    `flows_are_cached_per_adjacent_pair`, `edges_and_holds…`, `neighbours_continue_through_a_dissolve`,
    `interpolated_frames_skip_temporal_nr`, `projects_without_temporal_nr_render_the_same…`,
    `a_temporal_only_grade_keeps_the_plain_path…` → re-pointed at the clip field, same assertions.
16. (optional cleanup) `apply_ops_with` replay tests still green after the simplification.

GPU (`crates/gpu/tests/tnr_render.rs`): 17. `gpu_temporal_render_matches_the_cpu_reference` and
18. `…keeps_the_plain_path…` with the clip field, on AMD and Intel (`MESA_VK_DEVICE_SELECT`).

Export / engine:
19. `export_renders_temporal_nr_like_the_paused_program_frame` (pipeline.rs) with the clip field.
20. `export_frame_includes_temporal_nr` (`frame_export_tests.rs`): a still of a TNR clip equals
    the export's frame and differs from the live frame.
21. `tnr_commands_edit_undo_and_refuse_hostile_params` (`engine/src/tnr_tests.rs`): set / toggle
    / reset / clear / copy / paste / toggleAnimation round trip with undo / redo; `param:"x"`,
    `value: NaN`, a non-media clip, no selection → `BadParams`; `grade.setParam tnr.luma` refused
    with the pointer message.
22. `paste_attributes_copies_temporal_nr_with_the_grade_flag_off_and_on`.
23. `grade_match_ignores_temporal_nr` — same grade, different TNR → "same grade".
24. `loading_a_v16_project_toasts_the_move` (file_tests.rs) and keeps the schema backup on save.

UI (`crates/ui-egui/tests/ui/`): 25. `tnr_rows_in_effect_controls_and_the_noise_tab_agree`
(Premiere + Resolve workflows: set via `tnr.set`, both widgets show it; `ui.elements` lists
`tnr.luma` in both places; a drag is one undo step); 26. `the_noise_tab_shows_no_first_node_note`;
27. `coach_ask_every_guide_ui_id_is_known` picks up `{ui:tnr.*}`; `coach_ask_tests` section tags.

Bench (quick pass, synthetic, logged in §7): `export-tnr`, `cache-tnr` with `add_tnr_node` →
`add_tnr_clip`; numbers within noise of Phase 6's.

## 4. Steps (≈ 11–14 h total)

| # | step | files | est. | notes |
|---|---|---|---|---|
| 1 | Model: `project/src/tnr.rs`, `TrackItem.temporal_nr`, `NODE_PARAMS` minus `tnr.*`, shared schema helpers; tests 1–3 | project | 1.5 h | after RS3 merge |
| 2 | Format v17 migration + fixtures + tests 4–8 + loader toast | format, engine `commands.rs` | 2 h | test author can write 4–6 from this doc in parallel |
| 3 | Render: `render/src/temporal_nr.rs`, `item_fx` prepend, `changes_picture`, helpers moved, `grade_match`; tests 9–15, 17–18 | render, gpu tests | 2.5 h | the bit-exact test (9) first, red → green |
| 4 | (optional) replay cleanup in `gpufx.rs` / `gpu/src/lib.rs` if parity stays green | render, gpu | 1 h | skip if any doubt |
| 5 | Commands `tnr.*`, `pasteAttributes`, Export Frame; tests 19–24 | engine | 2 h | leaf builder (engine only) once step 1 is committed |
| 6 | UI: `panels/tnr.rs`, Effect Controls section, Noise tab block, i18n rows; tests 25–26 | ui-egui | 2 h | leaf builder (ui-egui only) once step 5's command shapes are committed |
| 7 | Guide + coach pointers; test 27; bench rename; HANDOFF / §7 line | docs, engine, bench | 1 h | |
| 8 | In-between gates (fmt, clippy, tests of touched crates + dependents, `xtask layers`, `xtask wasm`), quick bench pass, screenshot of both placements | — | 1 h | group acceptance later with Y1 (§1a.41) |

Lanes: steps 5 and 6 can run as two leaf-builder worktrees after step 1 (and 5's command ids)
land; everything else is the integrator's. Reviewer on the migration diff (step 2) and the
render diff (step 3) before merge.

## 5. Open questions (decided provisionally, log in OWNER-QUESTIONS if the owner wants otherwise)

- **Ignored later-node TNR values are dropped** on migration (bit-exact rule). Alternative: copy
  the *strongest* node's values to the clip. Reversible: the schema backup keeps the original.
- **`HASH_VERSION` not bumped** (migrated projects re-render their TNR segments once, old parts
  are evicted as stale). Alternative: bump to v4 (every cache of every project invalidated).
- **Bypassed grade → clip TNR starts disabled.** Alternative: enabled (changes the picture of a
  bypassed clip on load).
- **Export Frame now includes TNR** (slower still, matches export). Alternative: a checkbox in
  the Export Frame dialog.
