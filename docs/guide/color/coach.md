# Grading Coach: guided lessons

**[Fork]** · step-by-step lessons for people new to grading, short hints the first time something new comes up, and answers to questions from this guide (GRADE-PLAN G2.12)

## Why

> id: color.coach.why | does: guided lessons, context hints and Ask the coach help you learn the node grader | resolve: — | premiere: — (Learn panel tutorials, closest) | when: learning to grade in FilmCraft | see: color.coach.start

**[Fork]**

The node grader can be a lot to take in at first. A lesson walks you through one task, one step
at a time, on your own clip. Each step says what to do and why that step comes where it does,
outlines the control to use, and waits until you have actually done it before moving on. You
learn by doing the real thing, not by reading about it.

## Start a lesson

> id: color.coach.start | does: picks and starts a lesson on your own clip | ui: coach.lessons.open, coach.lessons.close | cmd: coach.start, coach.lessons, coach.showLessons | when: learning one task step by step | see: color.coach.card

**[Fork]**

Pick a lesson from the list. It works on your own clip, so open the project you want to practise
on first.

1. Open the Color Grading workspace. Click **Lessons** {ui:coach.lessons.open} at the bottom right
   (next to **Ask the coach**), or choose **Help ▸ Grading Lessons…** from any workspace.
2. The list shows every lesson with a one-line summary; a tick marks the ones you have finished.
   Click **Start** (**Again** for a finished one) next to the lesson you want. Close the list
   with **×** {ui:coach.lessons.close}.
3. The coach card appears at the bottom right of the window {ui:coach.card}. Read the step, then
   do it. The card moves to the next step by itself once you have done it.

In lessons that work on any clip (Your first grade, What a node is, Which node for what), the
"select a clip" step finishes by itself when a clip is already selected as it starts (clicking a
selected clip again changes nothing, so the coach doesn't wait for it). Lessons that need a
particular clip (a face, a noisy shot, a reference) always wait for you to click it. Press **Back** to return to such a step: it then waits for you.

For scripts and agents: the menu entry is the command `coach.showLessons`; each Start button is
`coach.lessons.item.<lesson id>`. `coach.lessons` lists the lessons (and `completed`),
`coach.start` with `{"lesson": "first-grade"}` starts one, and `coach.state` reports the lesson,
the step, its text and what it is waiting for.

## The card

> id: color.coach.card | does: the lesson card: step, why, Next, Back, Skip and Exit | ui: coach.card, coach.next, coach.back, coach.skip, coach.exit, coach.close | cmd: coach.next, coach.back, coach.skip, coach.exit | when: following a lesson | see: color.coach.outline

**[Fork]**

- The card shows the lesson's title, "Step n of m", where the feature comes from
  ([Resolve-style], [Fork], …), what to do, and **Why** that step matters.
- **Back** {ui:coach.back} (`coach.back`) goes to the previous step.
- **Skip** {ui:coach.skip} (`coach.skip`) moves on without doing the step.
- **Exit** {ui:coach.exit} (`coach.exit`) stops the lesson.
- **Next** {ui:coach.next} (`coach.next`) only appears on steps that are just reading or
  looking. Every other step finishes when you do it.
- When the last step is done, the card says **Lesson complete**; **Close** {ui:coach.close}
  removes it.

Undo does not take a lesson back a step. If you undo something a step asked for, press **Back**
and do it again.

## The outline

> id: color.coach.outline | does: a coloured outline marks the control a step is about | ui: coach.spotlight | when: you can't find the control a step names | see: color.coach.card

**[Fork]**

A coloured outline {ui:coach.spotlight} marks the control the step is about: a button, a slider,
a wheel or a whole panel. If the control is in a list that scrolls, the coach scrolls it into
view once. If the control isn't visible at all (for example the step needs another tab or
workspace first), the card says "The highlighted control isn't on screen yet."; follow the step
text to reveal it. The outline is hidden while a window such as Settings is open in front.

## Progress

> id: color.coach.progress | does: finished lessons are remembered between sessions | cmd: coach.state | when: checking what you have done | see: color.coach.lessons

**[Fork]**

Finished lessons are remembered between sessions, in your preferences (`color.coach.completed`;
`coach.lessons` shows `completed: true`). A lesson can be repeated at any time.

Hints you have seen are remembered the same way (`color.coach.hintsSeen`, see below).

## The lessons

> id: color.coach.lessons | does: the list of bundled lessons and what each teaches | when: choosing a lesson | see: color.coach.start

**[Resolve-style]** **[Fork]**

| Id | Lesson | What you learn |
|---|---|---|
| `nodes-and-flow` | What a node is and how the chain flows | Build two nodes and see the second one work on the first one's picture. |
| `first-grade` | Your first grade | Balance, exposure, contrast, saturation, then a look, and why in that order. |
| `which-node` | Which node for what | One job per node; a four-node starter tree for F-Log2. |
| `match-shots` | Match two shots with stills and wipe | Grab a still, wipe it against another shot, match brightness then colour, try Shot Match. |
| `fix-skin` | Fix skin with a qualifier | Pick skin by colour, check the selection, soften it, then adjust only the skin. |
| `tracked-window` | Brighten a face with a tracked window | A soft circle on the face, tracked through the shot, with a small lift inside. |
| `log-noise` | Clean up noise in log shadows | Noise reduction first, then lift the shadows, and compare. |
| `first-comp` | A moving title in a comp | Text, Transform, Merge and Media Out in the Compositor (see [the Compositor](../compositor.md)). |
| `key-comp` | Key a clip over a new background in a comp | Media In, Key, Solid, Merge and Color: a keyed clip over a new background, graded as one picture. |

## Context hints

> id: color.coach.hints | does: a one-time hint the first time you do something that needs explaining | ui: coach.hint.*, settings.color.coach.hints | cmd: coach.hint, coach.dismissHint, coach.resetHints | when: you want or don't want hints | see: color.coach.ask

**[Fork]**

The first time you do something on the Color page that usually needs a word of explanation, a
small card {ui:coach.hint.card} appears at the bottom right: why it matters first, then what it
is and when to use it, and a link to the lesson about it {ui:coach.hint.lesson}. Each hint shows
once. Hints never appear while a lesson runs; a hint whose moment comes during a lesson waits for
the next time.

- **Got it** {ui:coach.hint.dismiss} closes the card.
- **Turn hints off** {ui:coach.hint.off} switches every hint off. The same switch is **Settings ▸
  Color ▸ Grading Coach ▸ Show Grading Coach hints** {ui:settings.color.coach.hints}
  (`color.coach.hints`, on by default); turn it back on there.

| Hint | Shows the first time you… |
|---|---|
| Nodes | add a node |
| The qualifier | open the Qualifier tab |
| Windows | open the Windows tab |
| Tracking a window | track a window |
| Noise reduction | open the Noise tab |
| LUTs | set a node's LUT |
| Film grain | add grain to a node |
| Stills | grab a still |
| Wipe and split | compare a still with a wipe or a split |
| Shot Match | open the Match tab |
| Qualifier after a strong look | switch on a qualifier in a node that comes after a node with a strong look (a LUT, heavy contrast or saturation, a bent curve, a big colour shift) |
| AI explanations | turn on AI explanations |

For scripts and agents: `coach.hint` returns the hint on screen (`hint`: `id`, `title`, `why`,
`text`, `lesson`), whether hints are on (`enabled`) and the ids already shown (`seen`); `{"all":
true}` adds every hint. `coach.dismissHint` closes it (`{"off": true}` also turns hints off), and
`coach.resetHints` forgets the hints already shown so each can appear again.

The texts of the hints and the lessons are in English only for now; the buttons and labels around
them follow the interface language.

## Ask the coach

> id: color.coach.ask | does: answers a question with steps from this guide, every control checked | ui: coach.ask.* | cmd: coach.ask | when: you know what you want but not which tool | see: color.coach.hints

**[Fork]**

When you know what you want ("make the sky bluer", "my log footage looks flat") but not which
control does it, ask. The coach looks the question up in this guide, and only this guide, and
answers with the numbered steps of the section that fits best, naming the real buttons. It
doesn't invent anything: if most of your words aren't in the guide it says "The guide doesn't
cover this yet."; if you ask "is there …" or "where is …" about a word the guide never uses, it
names that word ("The guide doesn't mention "hologram"."); a "why …" or "what is X for" question
gets the section's explanation above its steps; if the guide covers it but has no
steps, it says so and offers the closest lesson instead. Every control and command an answer
names is checked against the app you are running; a step that names one this app doesn't have
is left out (the box says how many, and whether controls or commands).

1. Open the Color Grading workspace (or start a lesson). Click **Ask the coach**
   {ui:coach.ask.open} at the bottom right.
2. Type your question in the box {ui:coach.ask.input} in your own words, then press Enter or
   **Ask** {ui:coach.ask.go}.
3. Read the steps. Click a step that names a control (its id is `coach.ask.step.<n>`, from 1):
   the coach outlines that control, scrolling its panel if needed. Click it again to remove the
   outline.
4. When there are no steps, or you'd rather be walked through it, click the lesson link
   {ui:coach.ask.lesson} to start that lesson.
5. Close the box with **×** {ui:coach.ask.close}.

Under the answer, "From the guide" names the section the steps came from and its provenance
([Resolve-style], [FilmCraft], [Fork], [Standard]), so you know whether a Resolve or Premiere
habit applies. Any difference from Resolve or Premiere is only what that section itself says.

For scripts and agents: `coach.ask` with `{"question": "…"}` returns `steps` (each with `text`
and, when it names them, `ui` and `command`), `sources` (`page`, `heading`, `provenance`),
`stepsSource` (which source the steps came from), `lesson` (`id`, `title`, only when it fits the
question), `confidence` and `coverage` (0 to 1), `covered` (false: the guide doesn't cover the
question, so there are no steps or sources), `removed` (steps left out) with `removedControls`,
`removedCommands` and `removedIds`, and `fallback` (no steps: see `lesson`, `summary` and
`message`). Command ids are always checked; any other id-shaped word (a preference key, a
parameter name) can't be, so it is left out of the text. Without the app's window (the CLI, the
headless MCP server) automation ids can't be checked: `uiChecked` is false and steps carry no
`ui` (their text still names the control).

Today the coach finds sections by their words (a small search index with a list of beginner
words, such as "darker" or "grainy", matched to the guide's terms). A control that only appears
in a window you haven't opened yet this session (for example **OK** in the Interpret Footage
dialog) counts as unknown, so its step is left out until you have opened that window once.

## Write a hint

> id: color.coach.write-hint | does: how contributors add a context hint | when: adding a hint | see: color.coach.write-lesson

**[Fork]**

For contributors. The hints are one data file, `assets/coach/hints.toml` (its header and
`crates/engine/src/coach_hints.rs` are the full reference). Each `[[hint]]` has an `id`, a
`title`, `why` (shown first), `text`, an optional `lesson` and `when`, a list of triggers: a
command with an optional `params` pattern (the same patterns as a lesson step), a control shown on
screen (`element`), or a rule the engine checks on the grade (`rule = "qualifierAfterHeavyLook"`).
Write why before what, in plain words of your own.

## Write a lesson

> id: color.coach.write-lesson | does: how contributors write a lesson file | when: adding a lesson | see: color.coach.write-hint

**[Fork]**

For contributors. A lesson is a TOML file in `assets/lessons/`, added to `BUNDLED_FILES` in
`crates/engine/src/coach.rs` (the module documentation there is the full reference). No other
code is needed.

```toml
id = "my-lesson"            # lower case, digits, `.`, `_`, `-`; the file is my-lesson.toml
title = "Short title"
summary = "One sentence for the lesson list."
provenance = ["resolve-style"]   # resolve-style | filmcraft | fork | standard
order = 8                   # position in the list

[[step]]
text = "What to do, in plain words."
why = "Why it matters, before how."
target = ["grade.addFirst", "grade.addNode"]   # automation id(s) to outline; first on screen wins
expect = { command = ["grade.addNode", "grade.insertNode"] }
```

1. `expect` is exactly one of:
   - `{ command = "id" }` (or a list of up to 8 ids), optionally with `params`: the step is done
     when that command succeeds with matching parameters. A table in `params` must match the
     same keys (other keys are ignored); `{ gt = 1.0 }`, `ge`, `lt`, `le`, `ne` compare numbers;
     `{ any = true }` means "present"; `{ nonzero = true }` means "a number, or a number in a
     list, other than 0"; `{ in = ["a", "b"] }` means "one of these"; a list matches item by
     item. Example: `params = { clips = [{ any = true }] }` (at least one clip selected).
   - `{ element = "automation.id" }`: done when that control is on screen (for "open this tab"
     steps).
   - `{ manual = true }`: the reader presses Next.
2. A step may add `already = "clipSelected"`: it finishes by itself when it starts (or on
   `coach.state`) while a clip on a video track is selected. Use it only where any clip will do,
   never where the lesson needs a particular clip (a face, a noisy shot, a reference).
3. Use only real command ids and automation ids. The engine test checks every command id; the UI
   test in `crates/ui-egui/tests/ui/coach_ui.rs` walks each lesson and checks that every outlined
   control is on screen.
4. Write in your own words, plain language, why before how. Never copy a manual.
5. Limits: 64 KiB per file, 40 steps, 1 200 characters per text, patterns at most 6 deep and 64
   values. Unknown fields are refused.
