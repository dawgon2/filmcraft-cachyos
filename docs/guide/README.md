# The FilmCraft fork guide

A user guide for **this fork** (Linux / AMD, X-M5 6.2K F-Log2 workflow, Resolve-style colour page)
written for two readers: **people using the app**, and the **Grading Coach** (GRADE-PLAN G2.12),
which answers questions only from this guide plus the live state of the app. Started 2026-10-11
(owner request); kept up to date by every phase (see "Keeping it true" below).

## Where each feature comes from (provenance)

Every page and every section carries one of these tags, so a reader (and the coach) always knows
what a feature is modelled on and which manual or habit will transfer:

| Tag | Meaning |
|---|---|
| **[Resolve-style]** | Behaviour or workflow modelled on DaVinci Resolve, from its public manual and observation only (clean-room: no Blackmagic code, assets, icons or manual text; our own words). If you know Resolve, it works the way you expect; differences are listed in the section. |
| **[FilmCraft]** | Comes from upstream FilmCraft (storytold/filmcraft), mostly modelled on Premiere Pro (e.g. Lumetri Color, Effect Controls, the timeline). Works the way Premiere users expect. |
| **[Fork]** | Designed in this fork, with no direct equivalent in Resolve or Premiere (e.g. zero-copy 6.2K playback, the node explainer, the Grading Coach, shot comparison wording). |
| **[Standard]** | Follows a published specification or a well-known method, named in the section (e.g. DaVinci Wide Gamut / Intermediate from Blackmagic's published white paper, BT.2390 tone mapping, Oklab). |

A section can carry two tags (e.g. **[Resolve-style] [Fork]** for Shot Match: Resolve's idea, our
own solver and explanations).

## Contents

| Page | Covers | Status |
|---|---|---|
| [getting-started.md](getting-started.md) | Install, the two workflows (Settings ▸ General ▸ Workflow: Premiere / Resolve), importing X-M5 clips, what the hardware path does | partial (workflow section) |
| [color/README.md](color/README.md) | How colour works in this fork, start here | written (G0–G1) |
| [color/color-management.md](color/color-management.md) | F-Log2 / F-Gamut input, DaVinci Wide Gamut / Intermediate working space, Rec.709 output, highlight roll-off | written (G0–G1) |
| [color/nodes.md](color/nodes.md) | What a node is, how the chain flows, one job per node, a starter tree for F-Log2; later parallel / layer / splitter / outside / key mixer (C2) | written (G0–G1) |
| [color/primaries.md](color/primaries.md) | Lift / Gamma / Gain / Offset wheels, contrast and pivot, saturation, temperature / tint | written (G0–G1) |
| [color/curves.md](color/curves.md) | Custom and the five hue / luma / sat curves | partial (G0–G1) |
| [color/secondaries.md](color/secondaries.md) | Qualifier, windows, tracking (G2) | to write |
| [color/matching.md](color/matching.md) | Stills, wipe / split, copy / paste / append grades, "same grade?", shot comparison, Shot Match | partial (G0–G1) |
| [color/finishing.md](color/finishing.md) | Noise reduction, grain, LUTs, scopes | partial (G0–G1) |
| [color/lumetri.md](color/lumetri.md) | Lumetri Color (Premiere-style) and how it relates to the node grader | to write |
| [color/recipes.md](color/recipes.md) | Task recipes in plain steps ("make the sky bluer without touching skin", "fix grey skin", "match a wide and a close-up") | to write |
| [editing.md](editing.md) | Cutting, JKL, proxies and the render cache (Phase 6), multicam | to write |
| [export.md](export.md) | Export presets, hardware HEVC, 6.2K masters, YouTube / Shorts | to write |
| [hardware.md](hardware.md) | What runs on the AMD VCN / Intel / CPU, fallbacks, what the badges mean | to write |
| [glossary.md](glossary.md) | Plain-language definitions (log, gamut, node, key, matte, LUT, …) with the Resolve and Premiere names side by side | to write |

## How pages are written

- **Real names only.** Every button, panel, menu item and command a page tells the reader to use
  is written with its on-screen label **and** its id: `` `grade.addNode` `` for commands,
  `` {ui:color.nodes.add} `` for automation ids (`crates/ui-egui/src/automation.rs`). The coach
  uses these ids to highlight controls; a check (below) rejects ids that don't exist.
- **Why before how.** Each section says what the tool is for and when to reach for it before the
  steps. Beginners first, with an "If you know Resolve" / "If you know Premiere" note where habits
  differ.
- **Steps are numbered and checkable** (one action each), so a lesson can be made from them.
- **Our own words.** Never copy text from Resolve's, Premiere's or anyone else's manual.
- Short pages, plain language, no marketing.

## Keeping it true

- Every working step that adds or changes something a user can see or do updates the matching
  page in the same commit (or the next one), with the right provenance tag. A phase isn't accepted
  while its pages are still "to write" for features it shipped.
- A guide check (to build with the Grading Coach, G2.12; a `cargo xtask guide` subcommand or a test)
  parses every page and fails if a named command id or automation id doesn't exist, or a page
  lacks a provenance tag.
- Behaviour claims that differ from Resolve or Premiere must say so explicitly ("unlike Resolve,
  …"), so nobody is misled by a habit.
