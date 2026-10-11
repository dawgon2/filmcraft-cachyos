# Colour in this fork: start here

**[Resolve-style] [Standard] [Fork]** · status: written for G0 + G1 (what exists today)

This fork adds a **node grader**: a list of small correction steps (nodes) on each clip, in the
spirit of DaVinci Resolve's Color page. It sits next to FilmCraft's Premiere-style **Lumetri
Color** panel; you can use either, or both.

## The idea in one minute

**[Standard]** **[Fork]**

Footage from the Fujifilm X-M5 in F-Log2 looks flat and grey on purpose: it keeps as much picture
as the sensor saw. Colour work has three jobs, always in this order:

1. **Translate** the footage into a common working space, so every clip starts from the same
   ground. Done for you by colour management ([color-management.md](color-management.md)).
2. **Grade** it: balance, exposure, contrast, saturation, look. This is what nodes are for
   ([nodes.md](nodes.md), [primaries.md](primaries.md), [curves.md](curves.md)).
3. **Deliver** it: the working space is converted to Rec. 709 for normal screens. Also automatic.

For every clip the picture flows like this (**[Standard]** spaces, **[Fork]** order):

```
clip → input transform (F-Log2/F-Gamut → working space)
     → your grade nodes, first to last
     → the clip's effects (Lumetri is one of them)
     → output transform (→ Rec. 709)
```

## Where things are

**[Fork]**

| Where | What | Page |
|---|---|---|
| Workspace **Color Grading** (Window ▸ Workspaces ▸ Color Grading, `window.workspace.colorgrading`; the **Color** page in the Resolve workflow) | Gallery, viewer, **Nodes**, **Primaries** (Lumetri Color as a tab beside it), **Lumetri Scopes** | this page |
| **Nodes** panel | add, select, bypass nodes | [nodes.md](nodes.md) |
| **Primaries** panel (tabs *Primaries* and *Curves*) | wheels, sliders, curves, film grain | [primaries.md](primaries.md), [curves.md](curves.md) |
| **Gallery** panel | stills, wipe, split | [matching.md](matching.md) |
| Clip ▸ Grade menu | copy, paste, append grades | [matching.md](matching.md) |
| Settings ▸ General ▸ Workflow | Premiere or Resolve layout and keys | [../getting-started.md](../getting-started.md) |

## First grade in six steps

**[Resolve-style]**

1. Open the workspace **Color Grading** and select a video clip in the timeline.
2. In the **Primaries** panel press **Add Node** {ui:grade.addFirst} (command `grade.addNode`).
3. Set the sequence's colour management and the clip's colour space first if the picture is flat
   ([color-management.md](color-management.md)).
4. Drag the **Gain** wheel's ring up until the brightest part looks right.
5. Raise **Contrast** a little, then **Saturation**.
6. Press **Bypass** {ui:grade.bypass} to compare before and after. Press it again to restore.

Every step is one undo step (Ctrl+Z); dragging a control counts as one step however long you drag.

## Not here yet

**[Fork]**

Qualifier (pick a colour), windows (shapes), tracking, noise reduction, LUT node, "same grade?",
Shot Match, node explainer and the Grading Coach arrive in phase G2. Parallel and layer nodes
arrive with C2. Each page says what is missing.
