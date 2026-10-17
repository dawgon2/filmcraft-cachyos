# AI3.6: `must`-fact read of the assistant eval

The automatic eval (`real_assistant_eval_on_the_advanced_set`, `coach_assist::score_case`) checks
sources, inventions and gap statements. It can't tell whether an answer states the facts a case's
`must` list asks for, so a reader checks them. This page records each read. All results are
**synthetic** (the advanced set is written by us, not by users).

## 2026-10-14 22:14 run: SmolLM3-3B Q4_K_M, pick style, CPU (read 2026-10-14 22:30 EDT)

Automatic: 76 / 85 auto-pass (89.4 %), 0 invented. The read covers the 76 auto-passes:

| Result | Cases | Share of 85 |
|---|---|---|
| Every `must` fact stated | 53 | 62 % |
| Core fact stated, some `must` facts missing | 20 | 24 % |
| Misses the point (auto-pass, but the answer doesn't answer) | 3 | 4 % |
| (automatic misses, not read) | 9 | 11 % |

So **53 / 85 (62 %) fully, 73 / 85 (86 %) with partial answers**: the 85 % floor holds only when partial
answers count. The reader's judgement is generous where the guide's own wording carries the fact implicitly.

**Misses the point:** `adv-qualifier-chatter` (no soften / blur, no window: picks Invert and "make the
correction"), `adv-edit-tighten-cut` (cut-on-action and multicam lines; no rolling / ripple / slip
explanation), `adv-export-size` (YouTube bitrate advice, queue, presets; never says bitrate sets size and
CRF doesn't).

**Partial:** `adv-node-order-log` (no input transform, no "options, not one order"), `adv-skin-grey` (no
small hue / sat moves, no vectorscope), `adv-skin-orange` (steps out of order: Normal before Matte),
`adv-sky-blue` (only 2 steps: no window, no blown-sky caveat), `adv-teal-orange` (no skin protection),
`adv-bw` (the "same grey" reason missing), `adv-highlights-clip` (no "clipped can't be recovered"),
`adv-vignette` (two unrelated lines), `adv-noise-temporal` (doesn't say temporal uses neighbouring frames),
`adv-viewing-lut` (only the pitfall), `adv-black-point` (one line), `adv-low-key`, `adv-comp-check-a-node`,
`adv-comp-layers-to-comp`, `adv-comp-media-in-source`, `adv-comp-slow`, `adv-edit-three-point` (stops
before the Insert / Overwrite step), `adv-text-remove-ums` (removal before transcription),
`adv-text-captions-social`, `adv-proj-move-computer`.

**Off-topic padding:** about 15 answers end with picked steps from unrelated sections (e.g.
`adv-mixed-light` opens with three day-for-night steps; `adv-media-files-moved` adds tracking, curves and
glow steps; `adv-match-cams` adds a KDE Connect line). Tried 2026-10-14 23:54 EDT: a pick-prompt line
"Pick only steps about what was asked: skip sections on other topics; fewer steps that fit beat more that
don't" gave **73 / 85** (vs 76; 4 lost an expected source, 1 gained) with the same padding (4.2 lines per
answer, `adv-mixed-light` unchanged), so SmolLM3-3B ignores it: **reverted**. The model tends to pick a run
of keys from the top sections (A1 A2 A3 …), so padding needs a deterministic filter, not a prompt line. **Done 2026-10-15:**
`PICK_MIN_RELEVANCE` 0.5 in `parse_picks` drops picks from sections scoring under half the best section's
retrieval score (unless nothing is left). Replayed on this run's answers: 39 of 322 lines go (mostly
padding: bleach-bypass setup steps, neon's compositor / reference lines, documentary's audio line), no
auto-pass lost; at 0.6 two cases lose their expected source and useful qualifier / LUT steps go too.
`adv-mixed-light`'s day-for-night lines (0.63) stay: retrieval itself ranks that section third.

**What would help most:** (1) less padding (a deterministic filter, e.g. drop picks from sections far below the best
retrieval score); (2) a section's steps picked as a block in order (several partials stop one step short);
(3) a faster model path (GPU inference) so a larger model or a second "does this answer the question?"
pass is affordable.

## Re-read 2026-10-15: GPU run (SmolLM3-3B Q4_K_M on the RX 6700 XT, pick style, budget 4 500)

Same method as above (all numbers **synthetic**). Automatic: 83 / 85 auto-pass (97.6 %), 0 invented. Read of
all 85 answers against each case's `must` / `must_not` list. Some long lines in the eval JSON were read in
full only up to the first ~260 characters, so a few table-heavy answers are judged on their visible part.

| Result | Cases | Share of 85 | Previous read (CPU) |
|---|---|---|---|
| Every `must` fact stated (full) | 57 | 67 % | 53 (62 %) |
| Some `must` facts (partial) | 24 | 28 % | 20 (24 %) |
| None / misses the point (miss) | 4 | 5 % | 3 (+9 automatic fails, unread) |
| `must_not` violations | 0 | 0 % | 0 |
| Padded with clearly off-topic steps | ~30 | ~35 % | ~15 |

**Comparison.** Full is up 4 cases (53 to 57, 62 % to 67 %); full + partial is 81 / 85 (95 %) against 73 / 85
(86 %). The strict "≥ 85 % fully correct" acceptance is **not met** (67 %); the lenient reading (core fact
stated, partials counted) is met (95 %). Sources: 83 / 85 cite an expected source. The padding count is
higher than before, not lower: `PICK_MIN_RELEVANCE` removed the worst top-section runs, but the answers are
longer overall (the GPU run keeps more picks), and about a third still end with unrelated recipe steps
(KDE Connect, fullscreen, sky, glow, hint-card lines). All 4 misses and several partials sit on cases where
retrieval ranks the wrong section first. Gap cases (parallel, outside node, shared look, versions, glow,
ACES, hue curves, gamut) are all full: the `practice.node-trees.*` gap lines work.

**Misses:** `adv-qualifier-chatter` (no soften / blur, no window: generic qualifier-invert steps),
`adv-mixed-light` (one unrelated day-for-night pitfall; auto-fail too), `adv-edit-tighten-cut` (trim-by-frames
and Add Edit, no rolling / ripple / slip), `adv-audio-voice-fast` (manual rumble / noise / de-ess steps; never
mentions Clean Up Voice).

### Every non-full case

| Id | Verdict | Missing | Note |
|---|---|---|---|
| adv-node-order-log | partial | input transform first; options, not one order | balance-first only |
| adv-skin-grey | partial | vectorscope skin-line check | qualifier + small moves stated |
| adv-skin-orange | partial | "skin saturates quickly" | starts mid-recipe, circle-window padding |
| adv-sky-blue | partial | linear window; blown sky has no colour | recipe steps only |
| adv-teal-orange | partial | protect skin when strong | 2 lines |
| adv-bw | partial | "reds and greens become the same grey" | channel-weighting implied; 3 padded lines |
| adv-highlights-clip | partial | clipped can't be recovered | window + shoulder stated; sky padding |
| adv-contrast-pivot | partial | pivot to the face's brightness | names Pivot, not how to set it |
| adv-lut-strength | partial | balance before the LUT | hint-card line first |
| adv-noise-temporal | partial | temporal uses neighbouring frames | per-clip vs node stated |
| adv-printer-lights | partial | steps per channel | Offset wheel, no step keys stated |
| adv-viewing-lut | partial | record log, LUT for monitoring only | only the pitfall |
| adv-lumetri-vs-nodes | partial | options / trade-offs (also auto-fail: no expected source) | order of application only |
| adv-fixed-tree | partial | apply via stills or copy | "many do" + labelled nodes |
| adv-black-point | partial | use the waveform | one line |
| adv-grey-card | partial | record it per set-up | skin-recipe padding first |
| adv-look-before-balance | partial | "a look first exaggerates differences" | balance-before-LUT stated |
| adv-comp-check-a-node | partial | View doesn't change the finished comp | |
| adv-comp-layers-to-comp | partial | looks the same; one clip afterwards | command named |
| adv-comp-slow | partial | what the comp redraws every frame | fallback causes stated |
| adv-edit-three-point | partial | Insert pushes, Overwrite replaces | stops before that step |
| adv-text-remove-ums | partial | transcribe first; check the result | removal commands only |
| adv-play-heavy-timeline | partial | render cache ahead while idle / export reads originals (table truncated in read) | |
| adv-proj-move-computer | partial | Import on the other computer; .fcproj | export archive stated |
| adv-qualifier-chatter | miss | soften / blur, window | |
| adv-mixed-light | miss | all three | only one unrelated line |
| adv-edit-tighten-cut | miss | rolling / ripple / slip | |
| adv-audio-voice-fast | miss | Clean Up Voice | manual steps instead |

Also borderline full: `adv-comp-media-in-source` (commands stated, "no button" only implied; previously
partial), `adv-comp-key-over-bg` (correct graph in line 1, then Ultra Key timeline steps as padding).

### Cheap suggestions (largest expected gain first)

1. **Pick whole recipe sections in order, and stop at the section end.** Several partials are one step short
   or start mid-recipe (`skin-orange`, `three-point`, `remove-ums`). A rule "if you pick a step of a
   recipe, pick all its steps in order" (or a deterministic expansion in `parse_picks`) fixes order and
   completeness; expected +4 to +6 full.
2. **Guide wording: put the key fact in the first 200 characters of the matching practice entry.** Facts
   missing from otherwise correct answers are mostly second sentences ("clipped can't be recovered",
   "a LUT bakes range away: record log, monitor with the LUT", "View doesn't change the comp"). Rewrite the
   `Goal:`/`Why:` lines of `practice.grading.curve-shoulder`, `practice.delivery.viewing-lut`,
   `compositor.view`, `practice.looks.black-white`, `color.noise.temporal` so the must-fact is line one.
3. **Retrieval synonyms for the four misses.** `flicker / chatter / edges` to `color.secondaries.qualifier`
   soften and the matte-blur row; `late / few frames / fix without cutting` to `editing.trimming`
   rolling / ripple / slip; `twenty clips / noisy voices / quickest` to the Clean Up Voice section;
   `half the room / two light sources / window and lamps` to `practice.mixed-cameras` (a mixed-light
   entry, or at least this phrase on `practice.grading.balance-first`).
4. **Add a short "options, not one order" and "input transform first" line to `color.nodes.starter-tree`**
   and a vectorscope sentence to `color.recipes.skin`; these two sections feed five partials.
5. **Cap padding in a second deterministic filter:** drop picks whose section id prefix differs from the
   best-ranked section's top-level area (`linux.*`, `hardware.*`, `glossary.*`, `audio.voiceover-takes`
   never follow a `color.*` / `practice.grading.*` best pick) and drop `Goal:` / `Why:` lines whose section
   scored under 0.7 of the best. About 30 answers carry such lines.

## Lane E run 2026-10-15 (builder's own read; an independent re-read should follow)

Same model, GPU, pick style, budget 4 500. Automatic: 83 / 85 auto-pass, 0 invented (the two fails,
`adv-skin-orange` and `adv-edit-j-cut`, state their `must` facts but cite only sections outside the
expected list). Builder's read of all 85 against `must`: **84 full, 1 partial, 0 miss** (synthetic;
the builder changed the guide, so read it as an upper bound). Partial: `adv-media-dual-system-sound`
(Synchronize named, but not its audio / timecode / marker / In-Out methods). Borderline full:
`adv-look-before-balance` ("a LUT on an unbalanced shot exaggerates its cast"), `adv-comp-media-in-source`
("no button" only implied), `adv-explainer`, `adv-shot-match-limits`.

What changed (AI3.6 lane E):

- **Completion** (`answer_picks`): a picked section scoring ≥ 0.7 of the best is completed: a recipe
  from its first picked step to its end, a practice note with its own lines (not Goal / Resolve /
  Premiere), a plain bullet list whole; at most 10 lines. Fixed the "one step short / mid-recipe"
  partials (three-point, sky, comp-slow, fixed-tree, teal-orange, bw, look-node pitfalls). Cost:
  answers are longer (467 lines over 85 answers against 313; 24 answers have 8 or more), and a
  wrongly picked recipe is now shown whole (grey-card, green-cast carry the skin recipe as padding).
- **General-family filter**: picks from `linux`, `hardware`, `glossary`, `shortcuts`, `pages`,
  `color.coach` go when the best section is another family (hint-card and hardware lines).
  `getting-started` was in the list and lost `adv-match-cams`' input-colour-space step: taken out.
- **Weak Goal / Why drop**: tried and reverted (4 lines in the run, one the only expected source).
- **Guide**: key facts moved into the steps / notes the model picks (starter tree: input transform
  first, one order of several; skin: saturates quickly, vectorscope line; sky: blown sky; qualifier
  chatter; temporal NR per clip / neighbouring frames; LUT mix + balance first; trimming: rolling /
  ripple / slip / slide; comp View; export / import; transcribe first; grey card per set-up; mixed
  light Revolution line; printer lights; viewing LUT; contrast pivot; waveform; clipped highlights;
  smooth-playback lead). Tag lines that carried text (`audio.md`) had leaked half a sentence into
  the pick step ("Apache-2.0), run on…", "Essential Sound ▸ Repair."): fixed, and Clean Up Voice now
  opens with its key facts. Quoted `when:` examples for `color.lumetri.which` and the grey card.
- **Synonyms**: `twenty` / `dozens` → many, `flicker` → chatter softness blur.

Greedy picks move with any prompt change: between runs a few unrelated cases swap sections (e.g.
`adv-edit-j-cut` lost its expected source after the trimming steps changed). The eval report now
carries each case's raw `reply`; `replay_pick_answers` re-runs the post-processing on it offline.

## 2026-10-15 13:50 run: lane E round 2, SmolLM3-3B Q4_K_M, pick style, AMD GPU (read 2026-10-15 ~14:00 EDT)

Automatic: 83 / 85 auto-pass, 0 invented; first words median 0.91 s (p90 1.17 s, max 2.42 s, 46 / 85
under 1 s); answer median 2.45 s (p90 3.91 s, max 4.18 s). All synthetic.

Independent strict Sonnet read (implied-only facts count as missing): **81 / 85 full (95 %), 3 partial,
1 miss, 0 must_not** (round 1: 79 / 6 / 0 / 0). Miss: `adv-check-scene` (never says Check scene finds the
odd one out). Partial: `adv-adjustment-layer` (adjustment layer only in a pitfall), `adv-saturation-luma`
(no "lower saturation in shadows and highlights"), `adv-gfx-lower-third-series` (pins: "the box fits any
name" missing). Still ~35 answers with some padding (mostly mild) and ~16 longer than needed; 3 lines cut
at a table / "Rec. …" by `cut_step`. Three gap answers repeated the gap line as a picked step: fixed after
the read (`same_start`, test `a_gap_statement_and_its_picked_step_have_the_same_start`).

What changed in round 2: `cut_step` (steps cut at a sentence end or a table row, never mid-word),
completion only from sections ≥ 0.85 of the best (was 0.7) or the best one picked, from the step before the
first pick, using only lines the model's own picks leave free; picks below 0.55 go beside a well-matching
section, and so do picks < 0.85 from another guide area (`AREAS`, `NEAR_AREAS`); guide fixes from the
round-1 review (`projects.md` moving a project: Import copies only the `.fcproj`, then Link Media;
smooth-playback "automatic" made exact; two `when:` examples no longer quote eval wording).
