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

`grade.moveNode` (params `node`, `to`) moves a node to a position and `grade.labelNode` gives it a
name; the strip shows the name. Neither has a button or a drag yet; use the command (control
channel, MCP or command list).

## A starter tree for F-Log2

**[Fork]**

Order matters because each node sees what the one before produced. A sensible start:

1. **Balance**: Temperature / Tint and Offset, so whites are white and greys are grey.
2. **Exposure**: Gain (brights) and Gamma (mids), until face and sky sit where you want.
3. **Contrast**: Contrast and Pivot, or a gentle S in the Curves tab.
4. **Saturation**: one slider, last of the technical steps.
5. **Look**: anything creative, plus film grain.

The input conversion before node 1 and the Rec. 709 conversion after the last node are
automatic. Qualifier-based secondaries (skin, sky) will sit between 4 and 5 once G2 lands. The
app does not create this tree for you; build it with **+ Serial**.

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
