# Getting started

Status: **partial** (the Workflow section is written; the rest is to write).

## Install

**[Fork]**

*To write.*

## Choose your workflow: Premiere or Resolve

**[Resolve-style] [Fork]** on top of **[FilmCraft]**'s Premiere-style layout

### Why

FilmCraft is laid out like Premiere Pro. If your hands know DaVinci Resolve, one setting switches
the keyboard and the window furniture to Resolve's habits, and switching back puts everything as
it was.

### Steps

1. Choose Edit ▸ Preferences ▸ **General…** (`app.settings.general`, Ctrl+,). The window is
   titled "Preferences"; the first row of the **General** category is **Workflow**.
2. Pick **DaVinci Resolve** in the Workflow list (command `settings.setWorkflow`, parameter
   `workflow`: `premiere` or `resolve`). It applies at once; there is no OK to press.
3. Check the result:
   - A **page bar** appears along the bottom of the window: **Media**, **Edit**, **Color**,
     **Fairlight**, **Deliver** (ids {ui:page.media}, {ui:page.edit}, {ui:page.color},
     {ui:page.fairlight}, {ui:page.deliver}), with Project Manager and Settings on its right.
     Each page is a built-in workspace laid out like Resolve's page (Media Page, Edit Page,
     Color Grading, Fairlight Page, Deliver Page); Deliver holds the export settings and queue.
     The Premiere header gives way to the menus and a toolbar of panel buttons for the page.
     The whole story is in [pages.md](pages.md).
   - The keyboard preset becomes **DaVinci Resolve Compatible**.
   - The Color page is the **Color Grading** workspace: the node grader's Gallery, Nodes and
     Primaries panels around the viewer, with Lumetri Scopes below. Lumetri Color is not gone:
     it stays as a tab next to Primaries.
4. To go back, pick **Premiere Pro** in the same list. Your previous keyboard preset returns.
   The page bar disappears; the Color Grading workspace itself is available in both workflows
   (Window ▸ Workspaces ▸ **Color Grading**, `window.workspace.colorgrading`). With the Resolve
   workflow on, Window ▸ Workspaces ▸ **Color** (`window.workspace.color`, Shift+6) opens Color
   Grading too instead of the Premiere-style Color workspace.

Read the current state with `settings.getWorkflow`.

### The default, and the Resolve theme

**[Fork]** In this fork the default workflow is **Resolve** (RESOLVE-UI-PLAN RS1). A fresh install
starts in Resolve. Preferences written before RS1 are switched to Resolve **once**, the first time
the new build starts, through the same switch as above, so your previous keyboard preset and dark
theme are remembered and come back when you choose **Premiere Pro**. After that, your choice is
final: a Premiere workflow you picked yourself is never switched again. An unreadable preferences
file starts over in Resolve. Upstream FilmCraft builds (without the fork feature) keep Premiere as
the default and never migrate. A session without a data directory (the headless `filmcraft-cli`,
its MCP server, or the app when it has no data folder) also starts in Resolve, with the DaVinci
Resolve Compatible keys and the Resolve theme, so `settings.getWorkflow` and the keyboard agree.

The Resolve workflow shows the **Resolve** dark theme (near-black bars, cool grey panels, an
orange-red accent). Choosing **Premiere Pro** puts back the dark theme you had. Settings ▸
Appearance ▸ Dark Theme (and View ▸ Appearance ▸ **Resolve**, `view.theme.resolve`) lets either
workflow use any theme, and each workflow remembers its own: pick the Resolve theme while in
Premiere, or Darkest while in Resolve, and that choice comes back every time you return to that
workflow (switching only swaps between the two remembered themes). A highlight colour you set yourself still replaces the Resolve accent.

### Terminology follows the workflow

**[Resolve-style]** wording, **[Fork]** mechanism (RESOLVE-UI-PLAN RS2b). In the Resolve workflow
the menus, panel titles and labels use Resolve's word for each thing; the Premiere workflow keeps
the wording below in its right-hand column. Only the words change: command ids (`sequence.*`
stays `sequence.*`), automation ids, project files and the control protocol are the same in both
workflows, so a project opens identically in either. Switching the workflow
(`settings.setWorkflow`) relabels everything at once, with no restart. In other interface
languages the Resolve word is used only where the overlay has one; every other string shows its
normal translation.

| Resolve workflow | Premiere workflow |
|---|---|
| Timeline (File ▸ New ▸ Timeline…, Timeline Settings…, the Timeline menu) | Sequence |
| Compound Clip (New Compound Clip…) | Nest |
| Media Pool | Project panel |
| Source Viewer / Timeline Viewer | Source Monitor / Program Monitor |
| Inspector | Effect Controls |
| Mixer | Audio Track Mixer |
| Render Settings, Render Queue (Add to Render Queue) | Export Options, export queue (Send to Queue) |
| Blade Tool | Razor Tool |
| Adjustment Clip… | Adjustment Layer… |
| Render Cache (Render Cache In to Out) | Render In to Out, render previews |
| Working Folders | Scratch Disks |

If you know Premiere, read "Timeline" as "Sequence" everywhere in this guide; where a page names a
renamed thing it gives both terms, for example "Timeline (Premiere: Sequence)". The Export mode
is the Deliver page in this workflow ([pages.md](pages.md)), and the collect / consolidate
"Project Manager…" item is relabelled by the Project Manager home screen (RS2), not by this
overlay.

### What happens to your shortcuts

- Edited shortcuts are never lost. Leaving Premiere with unsaved key edits first saves them as a
  preset named "<name> (before Resolve)". Going back with edits in the Resolve set saves them as
  "DaVinci Resolve Compatible (edited)". Saved presets are never overwritten.
- If you chose a different preset by hand while in Resolve, that one stays when you go back.
- If saving fails, the switch is refused and nothing changes.

### The Resolve keymap

Only keys that Blackmagic documents publicly are assigned; everything else keeps its FilmCraft
default. Ctrl takes the place of Cmd on Linux and Windows. For the colour work:

| Keys | Command |
|---|---|
| Alt+S | `grade.addNode` (add serial node) |
| Shift+S | `grade.insertNode` (insert node before) |
| Ctrl+D | `grade.toggleNode` (enable / disable node) |
| Shift+D | `grade.bypass` |
| Ctrl+Alt+G | `stills.grab` |
| Shift+2 / 4 / 6 / 7 / 8 | the pages: Media, Edit, Color, Fairlight, Deliver |

Ctrl+W is assigned to `compare.set`, but pressed from the keyboard it changes nothing yet (the
command needs a still and a mode; use the Gallery buttons). The editing keys (undo, select all,
delete, ripple delete and so on) also follow Resolve's; open the shortcuts dialog to see the
full list.

### If you know Resolve

The pages are FilmCraft workspaces, not separate programs, so edits, colour and export share one
project and one undo history; each page's layout can be rearranged and saved ([pages.md](pages.md)). The keymap is a convenience, not a copy of Resolve.

## Import X-M5 clips

**[FilmCraft]** **[Fork]**

*To write.*

## What the hardware path does

**[Fork]**

Decoding, display, the grade and H.265 export run on the AMD graphics card and its video
engine, with automatic fallbacks; 4:2:2 clips can't use the video engine. The whole story, and
the switches, are in [hardware.md](hardware.md).
