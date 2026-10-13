# Nodes

**[Resolve-style]** · a clip's grade is a chain of nodes, applied first to last

## Why

**[Resolve-style]**

One big correction is hard to reason about. Splitting a grade into nodes, **one job per node**
(balance, exposure, contrast, a look), lets you switch any step off, see what each did, and copy
only the steps you want to other clips. Today the chain is **serial**: node 1 feeds node 2 feeds
node 3. The grade belongs to the clip, not to the sequence.

## The Nodes panel

**[Resolve-style]** **[Fork]**

Select a video clip. The **Nodes** panel (in the Color Grading workspace) shows "Nodes · clip
name", then buttons, then the strip: a dot (input), your nodes as numbered boxes 01, 02 …, and a
dot (output). The selected node is outlined; the Primaries panel edits it.

| Button | Id | Command | Does |
|---|---|---|---|
| **+ Serial** | {ui:grade.addNode} | `grade.addNode` | new node after the selected one |
| **+ Before** | {ui:grade.insertNode} | `grade.insertNode` | new node before the selected one |
| **Delete** | {ui:grade.removeNode} | `grade.removeNode` | removes the selected node |
| **Disable / Enable** | {ui:grade.toggleNode} | `grade.toggleNode` | switches it off without losing settings (the box is struck through) |
| **Reset** | {ui:grade.resetNode} | `grade.resetNode` | puts all of its controls back to neutral |
| **Bypass** | {ui:grade.bypass} | `grade.bypass` | whole grade off; press again to restore |

**Delete**, **Disable / Enable** and **Reset** need a selected node; **Bypass** needs a grade.
With no nodes the strip reads "No nodes: + Serial adds one"; while bypassed the header shows
"bypassed".

Each node box has its own id, `grade.node.<id>` (the node's stable number). Click a box to select
it (`grade.selectNode`); double-click it to disable or enable it. A node without a name is
labelled by what it holds ("Primaries", "Curves", "Grain", joined with "+" when it holds several,
or "Empty"). A grade holds at most 64 nodes; a damaged file with more shows "+N more (not
rendered)".

## Steps: build a three-node grade

**[Resolve-style]**

1. Select a clip. In **Primaries** press **Add Node** {ui:grade.addFirst} (or **+ Serial** in
   Nodes). The strip shows one box, 01.
2. Press **+ Serial** {ui:grade.addNode}: box 02 appears after 01 and is selected.
3. Press **+ Serial** again: box 03.
4. Click box 01 and change **Gain** in Primaries. Click box 03 and raise **Grain strength**.
5. Select box 02 and press **Disable** {ui:grade.toggleNode}: the picture changes by exactly
   that node's work. Press **Enable** to restore it.
6. Press **Bypass** {ui:grade.bypass}: the clip looks ungraded. Press it again.

## Delete a node

**[Resolve-style]**

A node you no longer need only makes the grade harder to read. Deleting (removing) it takes its
work out of the picture; the nodes after it move up and keep their settings. To keep its
settings for later, use **Disable** {ui:grade.toggleNode} instead.

1. Select the clip, then click the node's box in the Nodes panel (`grade.selectNode`).
2. Press **Delete** {ui:grade.removeNode} (`grade.removeNode`). The box disappears and the
   picture changes by that node's work.
3. Changed your mind? **Ctrl+Z** brings the node back with its settings.

## Reorder and name nodes

**[Fork]**

Naming nodes ("Balance", "Skin", "Look") makes a grade readable at a glance, as in Resolve.

1. Select the node, then click **Rename** {ui:grade.renameNode} in the Nodes panel, or right-click
   the node and choose **Rename**.
2. Type the name in the field on the node and press **Enter** (or click elsewhere). **Esc**
   leaves the old name. An empty name clears it, and the node shows what it changes instead.
3. **Ctrl+Z** undoes a rename (`grade.labelNode`, params `node`, `label`).

Right-clicking a node also offers **Disable** / **Enable**, **Reset** and **Delete**.
`grade.moveNode` (params `node`, `to`) moves a node to a position; it has no drag yet, so use the
command (control channel, MCP or command list).

## A starter tree for F-Log2

**[Resolve-style] [Fork]**

There is no single correct node order. What every order has in common: **each node sees the
picture the node before it produced**, so the order decides what each later correction works
on. Below is one simple, common starting point; the next sections show other orders and *why*
a colourist would choose them.

A short serial tree that works for most F-Log2 shots (one job per node, so you can switch one
off to see what it did):

1. **Balance** (first): Temperature / Tint and Offset, so whites are white and greys are grey.
   It goes first because every later node then works on a neutral picture: a brightness change
   on a colour cast pushes the cast further.
2. **Exposure**: Gain (brights) and Gamma (mids), until face and sky sit where you want.
3. **Contrast**: Contrast and Pivot, or a gentle S in the Curves tab.
4. **Saturation**: one slider, after step 3, because a steeper tone curve also makes colours
   look more saturated.
5. **Secondaries** (optional): a qualifier or window node for skin, sky or one object
   ([secondaries.md](secondaries.md)). It comes after the primaries so it selects colours that
   already look right; selecting from the flat log picture is harder.
6. **Look** (last or near last): anything creative, plus film grain on the very last node.

The input conversion before node 1 and the Rec. 709 conversion after the last node are
automatic ([color-management.md](color-management.md)). The app does not build this tree for
you; add nodes with **+ Serial**.

## Other node orders, and why you'd choose them

**[Resolve-style] [Standard]**

These are all in use; pick by how you work and what the footage needs.

- **Beginner: three nodes.** Balance → Exposure / contrast → Look. Fewer nodes, less to keep
  track of; you lose the ability to switch off exposure and contrast separately. Good for a
  first edit or when shots need little work.
- **Typical: the six-step tree above.** One job per node, so a client note ("less contrast")
  touches one node, and copying a grade to the next shot copies a readable structure.
- **Noise reduction first.** Spatial NR on node 1 (Noise tab) cleans the picture before any
  contrast or saturation boost makes the noise more visible, and qualifiers key more cleanly on
  a clean picture. Some colourists put it last instead so it only runs once the look is set
  (NR is slow; on a slow machine they disable it while grading). Temporal NR isn't a node here:
  it is a clip setting that always runs before the grade ([noise.md](noise.md)).
- **Exposure before balance.** Some people set exposure first because colour casts are easier
  to judge at the right brightness. It works when the cast is small; with a strong cast,
  balance first is safer (see the starter tree).
- **LUT / colour-space bookends.** In Resolve many colourists start the tree with a colour
  space conversion node and end it with one, so every node in between works in one space. Here
  the input transform (F-Log2 → working space) and the output transform are automatic, so the
  bookends are already there. A creative LUT on a node ([finishing.md](finishing.md)) works on
  the working signal (DaVinci Intermediate in a DaVinci Wide Gamut sequence): use one made for
  that signal, or a Rec. 709 LUT only in a Rec. 709 sequence. Corrections *before* the LUT node
  change what goes into it (the LUT reacts strongly); trims *after* it only adjust its result.
- **Highlight soft clip near the end.** It shapes the brightest values after everything else
  has pushed them, so later nodes can't clip them again.
- **Grain last.** Grain added before other nodes gets graded too (its contrast and colour
  change); last, it looks the same on every shot.
- **Pro trees with parallel and layer nodes** (Resolve, and here after phase C2): balance and
  exposure in series, then several secondaries side by side in a parallel node so each one
  keys from the same picture and none affects the others' selection, then a look node. More
  flexible and easier to match across many shots, but more to manage. Here only serial nodes
  exist today; a qualifier node placed after the primaries does the same job one at a time.

**Footage makes a difference too.** Log footage (F-Log2) needs balance and exposure before any
look; Rec. 709 footage often needs only a small balance and a look. Noisy low-light shots
benefit from NR early. With mixed cameras, balance each camera's shots to match first (a
balance node per shot), then share the look node across all of them.

## Undo

**[FilmCraft]**

Every node command is one undo step. Delete a node, undo, and it comes back with its settings.

## If you know Resolve

**[Resolve-style]**

Same idea as the node graph, minus the graph: no parallel, layer, splitter or outside nodes yet.
With Workflow set to Resolve the keys follow Resolve's: add serial node **Alt+S**, insert before
**Shift+S**, enable / disable **Ctrl+D**, bypass **Shift+D** (Ctrl where Resolve uses Cmd on
macOS). In the default Premiere workflow these commands have no shortcut.

## Later: parallel and layer nodes (C2)

**[Resolve-style]**

Planned: parallel, layer, splitter / combiner, outside and key-mixer nodes. Grades saved today
will open unchanged then.
