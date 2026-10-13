# The process assistant: plan outline (FORK-PLAN §1a.28 (b)), draft for the owner to place

*2026-10-12, draft. Written with C1 item 6 (b) in mind: "a written plan for the process assistant
(model-backed answers grounded in the guide and the project, checked so it can't invent
controls), for the owner to place." The owner decides where it goes (a phase of its own after C1,
or folded into Phase 7 / the Coach lane); nothing here is scheduled. Reading order: DIAGNOSIS.md
(why the keyword coach misses, what C1 fixes), then this.*

## 0. The owner's vision, restated as requirements

"The AI is an assistant that helps someone through their entire grading (and later editing)
process, not just a node-sentence rewriter." Concretely:

1. It answers **workflow questions** ("what order", "what first", "what next on *this* clip")
   from the guide **and the live project**: the clip on screen, its grade, its scopes, the
   sequence's colour management, the workflow setting.
2. It **never invents a control, command, value or fact**: every button, command id and number
   it names is checked (the same `UiIdCheck` / command registry as today's coach), every claim
   about the clip comes from a measurement the engine made (the node explainer's facts, the
   shot-comparison facts, the WB solver, the proxy / cache status), and the guide is the only
   source for "how FilmCraft works".
3. It **runs offline on the owner's machine** with G2.11's local model infrastructure (`ai.*`
   commands, pinned downloads, CPU inference), is **optional** (the keyword coach of C1 stays the
   answerer on installs without a model), and is **agent-drivable** (one command, JSON in and out).
4. Its failures are **logged into the training / evaluation set** (COACH-QUESTIONS.md →
   `coach-eval/questions.toml`), the same loop as the FilmCraft-tuned explainer
   (docs/fork/TRAINING.md).

Out of scope for the first version: doing the grade for the user (it suggests, the user acts, or
the user asks the existing lesson engine to walk them), editing-page questions until the editing
guide exists, anything that leaves the machine.

## 1. Shape: retrieve → ground → write → check → show

```
question + context (page, tab, clip id, node id)
   │
   ├─ 1. retrieve   the C1 keyword coach: top-5 sections (+ route hits) from the guide;
   │                 never fewer than the sections the keyword answer would show
   ├─ 2. ground     facts from the engine, as a small JSON "fact sheet":
   │                 clip (name, colour space / interpreted as, size, fps, proxy / 4:2:2 badge),
   │                 grade (nodes in order: label, what each holds, bypassed / disabled, the
   │                 explainer sentence per node = measured effect), the frame (parade / waveform
   │                 summary: dominant hue, mean / black / white levels, clipped share, saturation),
   │                 sequence (working space, output), workflow (Premiere / Resolve), and
   │                 what the UI shows (the automation ids on screen)
   ├─ 3. write      the local model, with: the guide sections (verbatim, with their `{ui:…}` and
   │                 command ids), the fact sheet, the question, and a fixed instruction:
   │                 answer in ≤ 6 numbered steps or ≤ 2 paragraphs, only name controls that
   │                 appear in the sections given, only state clip facts that appear in the
   │                 fact sheet, say "the guide doesn't say" when it doesn't
   ├─ 4. check      (a) every `{ui:…}` and command id in the output exists (today's check);
   │                 (b) every number / level word / direction about the clip matches the fact
   │                 sheet (the explainer's grammar check, widened);
   │                 (c) every control or feature named exists in the given sections' text
   │                 (a noun-phrase whitelist built from the sections: "Hue width", "Linear
   │                 window", …); a sentence that fails is dropped, and if fewer than half the
   │                 sentences survive the answer is replaced by the keyword coach's answer
   │                 with a "plain answer" hint (same UX as the explainer's "plain template")
   └─ 5. show       in the Ask-the-coach box: the answer, "From the guide" sources, "From this
                     clip" facts used, the lesson link; steps that name a control stay clickable
```

Steps 1, 4(a) and 5 exist. Step 2 is mostly assembling what the engine already measures
(`grade.explainGrade`, `grade.compareShots` metrics, `grade.wb.auto`'s analysis, the Lumetri
Scopes data, `media.proxyStatus`, `cache.status`, `settings.getWorkflow`, `coach.hint`'s element
list). Step 3 is a new prompt + the existing `ai.*` runner. Step 4(b)/(c) is the explainer's
check generalised from "one node sentence" to "a short answer about the guide and the clip".

## 2. Why the check can hold ("can't invent controls")

The explainer already proves the pattern: the model only rewrites facts, and a strict grammar
refuses anything else (35/36 pass with the tuned model, most stock models fail). For the
assistant the grammar is wider, so the check is a **whitelist** rather than a grammar:

- **Controls and commands**: must be ids or on-screen labels that occur in the retrieved
  sections (and, for ids, exist in this build). This alone removes "invented controls".
- **Clip facts**: numbers, "warmer / cooler / darker / brighter / clipped / noisy" and tonal
  ranges must match the fact sheet (the explainer's checker, reused).
- **Feature claims**: a sentence saying FilmCraft *has / can / does* something must contain a
  noun phrase from the sections. The guide's own "not yet" lines ("no parallel nodes yet", "no
  albums") are in the sections, so "not yet" answers are allowed and grounded.
- **Order claims** ("do X before Y"): X and Y must both be in a numbered list or an order
  sentence of the sections (nodes.md's starter tree, noise.md's "before", finishing's "late
  node", lumetri.md's order). This is why guide gap 1 in DIAGNOSIS.md (one complete order list)
  should be written first: it is the ground truth for every "what order" answer.

Whatever fails is dropped sentence by sentence; the user sees fewer sentences, never a wrong one;
the fallback is always the deterministic keyword answer.

## 3. Data and training

- **Evaluation set**: `coach-eval/questions.toml` grows a `clip` field (which fixture clip, which
  grade) for the `needs_clip` questions, and `facts` become the expected grounded statements.
  The owner's logged misses stay verbatim. Target: every `needs_clip` question gets an answer
  that uses at least one clip fact and passes the check; every `uncovered` question gets "the
  guide doesn't say" (the model can see that no section answers, unlike word matching).
- **Fixtures**: `target/fixtures/coach/` with a handful of generated clips that have a known
  story (a warm tungsten lamp like MVI_3486, a flat F-Log2 grey card, a blown sky, a noisy
  shadow), each with a saved grade, so the fact sheet is deterministic in tests.
- **Training** (optional, later): same recipe as the tuned explainer (TRAINING.md): generate
  (question, fact sheet, sections) → answer pairs with a large model offline, filter by the
  check, fine-tune the small model, measure pass rate and latency. Stock small models will pass
  the check less often; the keyword fallback keeps the UX whole meanwhile.
- **Owner loop**: a thumbs-down in the Ask box appends the question, context and the fact sheet
  (not the picture) to a local file the owner can paste into COACH-QUESTIONS.md.

## 4. Steps (for the owner to size and place)

| Step | What | Acceptance | Size |
|---|---|---|---|
| PA0 | Fact sheet: `coach.context` command returning the JSON of §1 step 2 for the selected clip / node / frame, from existing measurements only; tests on fixtures | deterministic on the fixture clips; ≤ 100 ms with the frame analysed at preview size | ~1 day |
| PA1 | Checker: generalise the explainer's checker to answers (controls / commands / numbers / direction words / feature noun phrases), with a sentence-level drop and the "fewer than half" fallback; unit tests on hand-written good and bad answers | 0 invented controls on a 50-answer adversarial set; ≥ 90 % of a 50-answer good set kept | ~1–2 days |
| PA2 | Writer: prompt + `ai.*` runner call with the retrieved sections and the fact sheet; `coach.ask` gains `"assistant": true` (default: on when a model is installed and Settings ▸ Color ▸ AI explanations is on) and returns `answer`, `used_facts`, `checked`, `dropped`; the keyword answer stays in the same JSON | the 12 `needs_clip` questions answered with a clip fact; owner miss 2 answers as COACH-QUESTIONS.md describes; latency ≤ the explainer's (≤ 15 s, template at once) | ~2 days |
| PA3 | UI: the Ask box shows the assistant's answer above the steps, "From this clip" facts, a "plain answer" hint when the check replaced it, thumbs-down logging; automation ids for everything | UI test drives a question through the control channel and screenshots it | ~1 day |
| PA4 | Eval + training loop: fixtures, the grown `questions.toml`, a pass-rate report in `cargo xtask` (or an engine test gated on a model being present); optional fine-tune after measuring stock models | report checked in; pass rate and latency per model listed in the guide's AI section | ~1–2 days + training time |
| PA5 | Guide: `color/coach.md › Ask the coach` describes the assistant, what it reads from the project, what "checked" means here, privacy (nothing leaves the machine) | guide check passes | ½ day |

Dependencies: C1 item 6 (a) first (the retrieval it builds on, and the eval set), guide gap 1
(the order list) before PA2's order answers can be checked. Independent of the compositor work.

## 5. Open points for the owner (OWNER-QUESTIONS candidates, not blocking)

1. Should the assistant **act** (add the suggested node with the user's confirmation, start a
   lesson at the right step) or only **advise**? The lesson engine can already walk steps; a
   "do it" button per step is a small addition once the answer is checked. Draft answer: advise
   in PA2, "walk me through it" via the lesson engine in PA3, "do it" later.
2. Editing-process questions ("do I grade before or after I cut") need the editing guide pages;
   write them before the assistant is asked about editing, or let it say "not yet".
3. Model choice: start with the installed explainer model (same runner, same size budget), or
   allow a second, slightly larger instruct model for the assistant with its own download.
