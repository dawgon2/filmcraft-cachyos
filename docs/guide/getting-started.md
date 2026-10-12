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
     {ui:page.fairlight}, {ui:page.deliver}). Each opens a FilmCraft workspace: Media is
     "Assembly", Edit is "Editing", Color is "Color Grading", Fairlight is "Audio"; Deliver
     opens Export.
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
the default and never migrate.

The Resolve workflow shows the **Resolve** dark theme (near-black bars, cool grey panels, an
orange-red accent). Choosing **Premiere Pro** puts back the dark theme you had. Settings ▸
Appearance ▸ Dark Theme (and View ▸ Appearance ▸ **Resolve**, `view.theme.resolve`) lets either
workflow use any theme. A highlight colour you set yourself still replaces the Resolve accent.

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
project and one undo history. The keymap is a convenience, not a copy of Resolve.

## Import X-M5 clips

**[FilmCraft]** **[Fork]**

*To write.*

## What the hardware path does

**[Fork]**

Decoding, display, the grade and H.265 export run on the AMD graphics card and its video
engine, with automatic fallbacks; 4:2:2 clips can't use the video engine. The whole story, and
the switches, are in [hardware.md](hardware.md).
