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
| [pages.md](pages.md) | The Resolve workflow's pages (RS3): the page bar, each page's layout and toolbar, Deliver's render settings and queue, saving a page layout | written (RS3) |
| [projects.md](projects.md) | The Project Manager home screen (RS2): the projects folder, opening and making projects, sort / search / views, rename / duplicate / trash, export / import, At startup show | written (RS2) |
| [color/README.md](color/README.md) | How colour works in this fork, start here | written (G0–G1) |
| [color/color-management.md](color/color-management.md) | F-Log2 / F-Gamut input, DaVinci Wide Gamut / Intermediate working space, Rec.709 output, highlight roll-off | written (G0–G1) |
| [color/nodes.md](color/nodes.md) | What a node is, how the chain flows, one job per node, a starter tree for F-Log2 and other node orders with their trade-offs (CO1); later parallel / layer / splitter / outside / key mixer (C2) | written (G0–G1) |
| [color/primaries.md](color/primaries.md) | Lift / Gamma / Gain / Offset wheels, contrast and pivot, saturation, temperature / tint | written (G0–G1) |
| [color/curves.md](color/curves.md) | Custom and the five hue / luma / sat curves | partial (G0–G1) |
| [color/secondaries.md](color/secondaries.md) | Qualifier, windows, tracking (G2) | written (G2.1–G2.3) |
| [color/noise.md](color/noise.md) | Spatial noise reduction on a node (G2.4), temporal per clip (6.7 / 6.7b) | written (G2.4 spatial, 6.7b temporal) |
| [color/matching.md](color/matching.md) | Stills, wipe / split, copy / paste / append grades, "same grade?", shot comparison, Shot Match, node explainer, AI explanations | written (G0–G2b) |
| [color/finishing.md](color/finishing.md) | Grain, highlight soft clip, node LUTs, scopes (noise reduction: noise.md) | written (grain, soft clip, LUT; scopes GUIDE-8) |
| [color/lumetri.md](color/lumetri.md) | Lumetri Color (Premiere-style) and how it relates to the node grader | written (cloud) |
| [color/coach.md](color/coach.md) | Grading Coach: guided lessons (the lesson list, card buttons, outline, progress, the seven lessons, writing a lesson file), context hints (the list, the switch, writing a hint) and Ask the coach | written (G2.12) |
| [color/recipes.md](color/recipes.md) | Task recipes in plain steps ("make the sky bluer without touching skin", "fix grey skin", "match a wide and a close-up") | written (cloud: 8 recipes) |
| [render-cache.md](render-cache.md) | The render cache (Phase 6.3): when it renders, the render bar colours, invalidation, the 2 s H.265 pieces, Settings ▸ Media Cache ▸ Render Cache, `cache.*` commands | written (6.3) |
| [editing.md](editing.md) | Cutting, three-point editing, JKL, trimming, multicam (links to proxies / render cache) | written (GUIDE-7) |
| [cards.md](cards.md) | Copying camera cards (LINUX-PLAN L2.4): the verified copy, the checksum list, cards pulled mid-copy, the card bar and Eject, `card.*` commands, copy-first vs import-from-card | written (L2.4) |
| [audio.md](audio.md) | Audio clean-up (AUDIO-PLAN A2): Voice Isolation (DeepFilterNet3), installing its model (Settings ▸ Audio ▸ Voice isolation), Voice Isolation vs DeNoise vs Essential Sound; later one-click voice clean-up | partial (A2.5) |
| [proxies.md](proxies.md) | Automatic proxies (which clips get them, the background worker, badges, Auto / On / Off, cancelling, relinked media) | written (6.2) |
| [export.md](export.md) | Export presets, hardware HEVC and its fallbacks, bitrate vs constant quality (CRF), HDR → Main 10 | written (cloud; YouTube / Shorts specifics to add with Y phases) |
| [hardware.md](hardware.md) | What runs on the AMD VCN / Intel / CPU, fallbacks, what the badges mean | written (cloud; badges arrive with Phase 6) |
| [linux.md](linux.md) | Linux desktop integration (LINUX-PLAN L1 / L2): where files live (XDG folders), and the desktop services added as L1 / L2 land | written (L2.1; grows with each step) |
| [glossary.md](glossary.md) | Plain-language definitions (log, gamut, node, key, matte, LUT, …) with the Resolve and Premiere names side by side | written (cloud) |

## How pages are written

- **Real names only.** Every button, panel, menu item and command a page tells the reader to use
  is written with its on-screen label **and** its id: `` `grade.addNode` `` for commands,
  `` {ui:grade.addNode} `` for automation ids (`crates/ui-egui/src/automation.rs`). The coach
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
- The Grading Coach (G2.12) embeds every page (`crates/engine/src/coach_ask.rs`, `GUIDE_PAGES`;
  a test fails when a page is added without being listed) and searches them for "Ask the
  coach". Its tests are the guide check: every `##` section must carry a provenance tag
  (`coach_ask_tests.rs`), and every `{ui:…}` id must be a control the app shows, builds at run
  time or registers in a dialog (`crates/ui-egui/tests/ui/coach_ui.rs`,
  `coach_ask_every_guide_ui_id_is_known`). Command ids named in a page's steps are checked each
  time the coach answers; a step naming an unknown one is left out. A page's untagged
  introduction (a status line) is not searched.
- Behaviour claims that differ from Resolve or Premiere must say so explicitly ("unlike Resolve,
  …"), so nobody is misled by a habit.
