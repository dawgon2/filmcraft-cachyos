# Coach questions the owner hit (FORK-PLAN §1a.28)

Real questions the owner asked "Ask the coach" that it failed. When §1a.28 is planned, every
entry becomes (a) a regression question in `assets/coach/questions.toml` (retrieval must find
the section) and (b) training / evaluation data for the model-backed process assistant (answers
grounded in the guide **and the current clip / grade facts**, checked so they can't invent
controls). Add new ones at the bottom; keep the owner's wording exactly (typos included).

| # | Date | Question (verbatim) | Where | Coach said | Should answer from | Needs clip facts? |
|---|---|---|---|---|---|---|
| 1 | 2026-10-12 | "what is the best order for nodes? What should I tackle first, and what should I tackle last?" | Color page | "The guide doesn't cover this yet." (coverage 0.667, confidence 0.163) | `color/nodes.md` "A starter tree", `color/recipes.md` | no |
| 2 | 2026-10-12 | "what would the qualifier be useful for on this cli" (cut off in the field: "clip") | Color page, Qualifier tab, node 01 empty, clip `MVI_3486.MP4` | "The guide doesn't cover this yet." | `color/secondaries.md` "The qualifier: select by colour" | **yes** |

## Notes per entry

**2.** The clip: a floor lamp lit by a warm tungsten bulb against a wall, dark and almost all
orange / amber (parade: red high, blue low), nothing graded yet. A good answer, in the guide's
words: what a qualifier does (selects by colour and builds a matte), then what it could pick
*here*: the bright amber glow on the wall (cool or desaturate the hot spot without touching the
rest), the dark orange shadows (neutralise the cast in the shadows only), or the lamp's dark
metal (keep it neutral while warming the wall). It should also say when a qualifier is the wrong
tool on this clip: with one dominant hue a plain white-balance / primaries correction on node 01
comes first, and a window may isolate the lamp more cleanly than colour. Without clip facts, the
fallback is the general section with "use a qualifier when the part you want has its own colour",
never "doesn't cover this". Also: the question box should not cut off text while typing (it
scrolled; check the field shows the whole question or wraps).
