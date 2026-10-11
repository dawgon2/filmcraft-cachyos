# Matching shots: stills, wipe, copy and paste

**[Resolve-style]** for stills and grade copying · **[Fork]** for the "same grade?" and Shot Match work still to come

## Why

**[Resolve-style]**

A cut feels wrong when two shots of one scene differ in colour. The quickest route: save a good
shot as a **still**, compare the next shot against it, then reuse or adjust the grade.

## Stills gallery

**[Resolve-style]**

The **Gallery** panel (left side of the Color Grading workspace) holds stills: a picture of the
Program frame plus the grade that made it, saved in the project.

| Control | Id | Command | Does |
|---|---|---|---|
| **Grab Still** | {ui:stills.grab} | `stills.grab` | saves the frame under the playhead and its clip's grade |
| click a thumbnail | `stills.item.<id>` | `compare.set` | chooses it as the reference and starts the comparison (Wipe, or Split if Split was on) |
| double-click a thumbnail | | `stills.apply` | gives its grade to the selected clips |
| **Delete** | {ui:stills.delete} | `stills.delete` | removes the selected still |
| **Off / Wipe / Split** | {ui:stills.mode.off} {ui:stills.mode.wipe} {ui:stills.mode.split} | `compare.set` | the comparison mode (Wipe and Split need a chosen still) |

`stills.label` renames a still; there is no rename button yet. With Workflow set to Resolve,
**Ctrl+Alt+G** runs `stills.grab`.

## Compare

**[Resolve-style]**

1. Park the playhead on the reference shot and press **Grab Still** {ui:stills.grab}.
2. Move to the shot to match and click the still's thumbnail: the wipe starts at once, the still
   covering the left part of the Program picture. Drag the divider {ui:compare.divider} to
   slide it (`compare.set` with `position` 0 to 1; `vertical` turns it into a top / bottom
   wipe, which has no button yet).
3. Press **Split** to see the whole still and the whole live frame side by side.
4. Press **Off** to leave comparison; **Wipe** brings it back.

A still keeps its look even if the source clip is regraded later.

## Copy, paste and append grades

**[Resolve-style]**

In the Clip ▸ Grade menu, for video clips:

1. Select the clip with the grade you like and choose **Copy Grade** (`grade.copy`).
2. Select the target clips.
3. Choose **Paste Grade** (`grade.paste`) to replace their grades, or **Append Grade**
   (`grade.append`) to add the copied nodes after the nodes they already have.
4. **Apply Grade From Clip** (`grade.applyFrom`, also in the menu) does copy and paste in one
   command from a source clip id (`from`).

`grade.clear` ("Remove Grade") takes a grade off a clip; it has no menu item yet, so run it from
the command list, the control channel or MCP. Each of these is one undo step however many
clips it touches.

### Paste Attributes

**[FilmCraft]**

Edit ▸ **Paste Attributes…** (`edit.pasteAttributes`, Ctrl+Alt+V, **[FilmCraft]**) pastes the
copied clip's motion, opacity, effects and so on; since G1 it also carries the grade. The
command's `grade` option (default on) leaves the grade out when set to false; the dialog has no
Grade checkbox yet, so from the dialog the grade always comes along.

## Coming in G2

**[Resolve-style]** **[Fork]**

- **"Same grade?"**: clips with identical grades share a badge (planned command
  `grade.compareData`).
- **Shot comparison and a scene check**: plain-language differences between two shots (planned
  command `grade.compareShots`).
- **Shot Match**: solve a node that matches one shot to another.

None of these exist yet; use the wipe and your eyes.

## If you know Resolve

**[Resolve-style]**

Stills, wipe and split work like the gallery and its wipe. There are no memories, versions,
groups or timeline grades yet: grades belong to clips.
