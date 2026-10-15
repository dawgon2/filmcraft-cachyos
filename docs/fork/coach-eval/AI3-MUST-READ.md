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
