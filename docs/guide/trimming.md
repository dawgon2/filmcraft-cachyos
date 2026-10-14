# Trimming, the Source monitor and multicam in detail

**[FilmCraft]** · edit-point selection and trimming to the playhead, dynamic trimming, the
Source monitor, and the multicam controls; modelled on Premiere Pro; written for AI-PLAN
AI3-PREP (feature pages). The basics are in [editing.md](editing.md).

## Selecting edit points

> id: trimming.edit-points | does: selects one or more edit points to trim, nearest to the playhead or by clicking | cmd: trim.selectEditPoint, trim.selectNearest, trim.clear, trim.toggleType | resolve: Trim edit mode selection | premiere: Select Nearest Edit Point, Toggle Trim Type | when: trimming from the keyboard | see: trimming.to-playhead, editing.trimming

**[FilmCraft]**

**Select Nearest Edit Point** (`trim.selectNearest`) picks the cut closest to the playhead on
the targeted tracks; `trim.selectEditPoint` selects a given one, and several can be selected to
trim tracks together. **Toggle Trim Type** (`trim.toggleType`) switches the selected
edit between a regular trim, a ripple trim and a rolling edit; `trim.clear` deselects.

## Trimming to the playhead

> id: trimming.to-playhead | does: moves a clip's start or end to the playhead in one key, with or without ripple | cmd: trim.previous, trim.next, trim.ripplePrevious, trim.rippleNext, trim.extendToPlayhead, trim.extendPreviousEdit, trim.extendNextEdit, trim.nudge | resolve: Trim Start / End to Playhead | premiere: Trim / Ripple Trim Previous / Next Edit to Playhead, Extend Edit | when: fast rough-to-fine cutting while playing | see: trimming.edit-points

**[FilmCraft]**

| Command | Does |
|---|---|
| **Ripple Trim Previous / Next Edit to Playhead** (`trim.ripplePrevious`, `trim.rippleNext`) | cuts from the playhead back to the previous edit (or on to the next) and closes the gap |
| **Trim Previous / Next Edit to Playhead** (`trim.previous`, `trim.next`) | the same, leaving a gap |
| **Extend Selected Edit to Playhead** (`trim.extendToPlayhead`) | moves the selected edit point to the playhead |
| **Extend Previous / Next Edit to Playhead** (`trim.extendPreviousEdit`, `trim.extendNextEdit`) | moves the nearest edit without selecting it first |
| **Trim by Frames** (`trim.nudge`, `trim.forward` / `trim.backward`) | moves the selected edit point by frames |

Ripple versions keep everything after the edit in sync; the plain ones are safer when other
tracks must not move.

## Dynamic trimming and playing around the edit

> id: trimming.dynamic | does: trims while playing with J, K and L, and plays around a selected edit | cmd: trim.shuttle, trim.shuttleStop, trim.cancelDynamic, trim.playAround, trim.monitor | resolve: dynamic trim mode (W) | premiere: Trim Monitor, dynamic trimming with JKL, Play Around | when: finding the edit point by ear and eye in motion | see: editing.jkl

**[FilmCraft]**

With an edit point selected, J, K and L shuttle the **edit** instead of the playhead
(`trim.shuttle`): the cut follows playback and stops where you stop (`trim.shuttleStop`);
`trim.cancelDynamic` throws the change away. **Play Around Edit** (`trim.playAround`) plays a
few seconds before and after the cut so you can judge it. `trim.monitor` reports the Trim
monitor's two sides (outgoing and incoming frame) for agents.

## The Source monitor

> id: trimming.source-monitor | does: views a clip before it goes on the timeline, marks In / Out and places it | cmd: source.open, source.close, source.closeAll, source.setPlayhead, source.inspect, source.insert, source.overwrite | resolve: Source viewer | premiere: Source Monitor | when: choosing the part of a clip to use | see: editing.three-point

**[FilmCraft]**

Double-click a clip in the Project panel (`source.open`) to see it in the Source monitor. Mark
In and Out there, then **Insert** (`source.insert`) or **Overwrite** (`source.overwrite`) it at
the timeline's playhead on the source-patched tracks ([timeline.md](timeline.md)). Close one clip
(`source.close`) or all (`source.closeAll`); `source.setPlayhead` and `source.inspect` drive it
from scripts.

## Multicam controls

> id: trimming.multicam | does: the multicam monitor's layout and pages, recording angle cuts, audio follows video, editing the camera list | cmd: multicam.recordStart, multicam.recordStop, multicam.switchAngle, multicam.cut, multicam.audioFollowsVideo, multicam.editCameras, multicam.gridLayout, multicam.page, multicam.nextPage, multicam.prevPage, multicam.toggleView | resolve: Multicam viewer | premiere: Multi-Camera view and its settings | when: cutting a multi-camera recording | see: editing.multicam

**[FilmCraft]**

1. Make the multicam clip and put it on a sequence ([editing.md](editing.md)), then show the
   multi-camera view in the Program monitor (`multicam.toggleView`).
2. Play and click angles: with recording on
   (`multicam.recordStart` / `recordStop`) each click becomes a cut; without it, `multicam.cut` /
   `multicam.switchAngle` change the angle at the playhead.
3. **Audio Follows Video** (`multicam.audioFollowsVideo`): the sound switches with the picture;
   off, one angle's sound plays throughout (the usual choice for an interview with one good
   microphone).
4. With many cameras, choose the grid (`multicam.gridLayout`) and step through pages of angles
   (`multicam.page`, `nextPage`, `prevPage`). **Edit Cameras…** (`multicam.editCameras`) renames
   angles or turns them off. `multicam.autoAdjustQuality` lowers the preview quality so many
   angles play smoothly.
