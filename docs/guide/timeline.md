# Timeline: targeting, keys and markers

**[FilmCraft]** · track targeting and source patching, selecting and moving clips from the
keyboard, moving around the timeline, In / Out and split edits, and markers; modelled on
Premiere Pro's timeline; written for AI-PLAN AI3-PREP (feature pages)

## Track targeting and source patching

> id: timeline.targeting | does: chooses which tracks edits and keys act on, and where source clips land | cmd: timeline.setTargeting, timeline.toggleAllVideoTargets, timeline.toggleAllAudioTargets, timeline.toggleAllSourceVideo, timeline.toggleAllSourceAudio | resolve: track destination controls | premiere: track targeting and source patching | when: Insert / Overwrite land on the wrong track | see: editing.three-point

**[FilmCraft]**

Each track header has two switches on the left:

- **Source patch** (V1 / A1 next to the track): where the Source monitor's video and audio go
  when you Insert or Overwrite. Turn source audio off to bring in picture only.
- **Track target** (the track's name): which tracks the keyboard works on: going to the next
  edit point, pasting, adding an edit at the playhead, matching a frame.

`timeline.setTargeting` sets either switch for one track; the toggle commands switch all video
or all audio targets (or source patches) at once, and the move commands shift all targets one
track up or down. `timeline.toggleMuteTargetedAudio`, `toggleSoloTargetedAudio` and
`toggleOutputTargetedVideo` mute, solo or hide every targeted track.

## Selecting and moving clips from the keyboard

> id: timeline.keys | does: selects clips at or next to the playhead and nudges, slips or slides them a frame at a time | cmd: timeline.selectClipAtPlayhead, timeline.selectNextClip, timeline.selectPrevClip, timeline.selectGap, timeline.nudgeLeft, timeline.nudgeRight, timeline.nudgeUp, timeline.nudgeDown, timeline.slipLeft, timeline.slipRight, timeline.slideLeft, timeline.slideRight | resolve: keyboard trimming and nudging | premiere: Nudge, Slip and Slide Clip Selection | when: frame-exact moves without the mouse | see: editing.trimming

**[FilmCraft]**

1. Select: **Select Clip at Playhead** (`timeline.selectClipAtPlayhead`), then the next or
   previous clip (`timeline.selectNextClip` / `selectPrevClip`). `timeline.selectGap` selects an
   empty space so Delete or Ripple Delete can close it.
2. Move the selection:
   - **Nudge** one frame left or right (`timeline.nudgeLeft` / `nudgeRight`; `…5` for five
     frames), or to the track above / below (`timeline.nudgeUp` / `nudgeDown`);
   - **Slip**: the clip stays where it is and shows an earlier or later part of its source
     (`timeline.slipLeft` / `slipRight`, `…5`);
   - **Slide**: the clip moves and its neighbours trim to make room, so the sequence keeps its
     length (`timeline.slideLeft` / `slideRight`, `…5`).

Each press is one undo step. For the same moves with the mouse, see the trimming tools in
[editing.md](editing.md).

## Moving around the timeline

> id: timeline.navigate | does: steps the playhead by frames and jumps to edit points, clip starts and ends | cmd: playhead.step, playhead.stepForward, playhead.stepBack, playhead.stepForward5, playhead.stepBack5, playhead.nextEdit, playhead.prevEdit, playhead.nextEditAnyTrack, playhead.prevEditAnyTrack, playhead.selectedClipStart, playhead.selectedClipEnd, playhead.start, playhead.end, playhead.set | resolve: playhead navigation keys | premiere: Go to Next / Previous Edit Point, step keys | when: finding a cut or a frame precisely | see: editing.jkl

**[FilmCraft]**

- One frame forward or back (`playhead.stepForward` / `stepBack`), five at a time
  (`…5`), or any number (`playhead.step`).
- The next or previous edit point on the **targeted** tracks (`playhead.nextEdit` /
  `prevEdit`), or on any track (`…AnyTrack`).
- The start or end of the selected clip, of the sequence, or an exact time (`playhead.set`).

For playing and shuttling, J, K and L are in [editing.md](editing.md).

## In and Out points, and split edits

> id: timeline.in-out | does: marks In / Out, marks a clip or the selection, and sets separate audio and video In / Out for J- and L-cuts | cmd: markers.markIn, markers.markOut, markers.markClip, markers.markSelection, markers.clearIn, markers.clearOut, markers.clearInOut, markers.goToIn, markers.goToOut, markers.markSplitAudioIn, markers.markSplitAudioOut, markers.markSplitVideoIn, markers.markSplitVideoOut | resolve: In / Out, split edits | premiere: Mark In / Out, Mark Split | when: three-point edits, rendering or exporting a range, J- and L-cuts | see: editing.three-point

**[FilmCraft]**

**Mark In** / **Mark Out** (`markers.markIn` / `markOut`) set the range that Lift, Extract,
export and render work on; **Mark Clip** (`markers.markClip`) marks the clip under the playhead
on the targeted tracks, **Mark Selection** (`markers.markSelection`) the selected clips. Clear
them with `markers.clearIn`, `clearOut` or `clearInOut`; jump to them with `markers.goToIn` /
`goToOut`.

A **split edit** lets sound start before or end after the picture (a J-cut or an L-cut). In the
Source monitor, set separate In / Out points for audio and video (`markers.markSplitAudioIn`,
`markSplitVideoOut` and their pairs) before Insert or Overwrite.

| Option | Good for | Trade-offs |
|---|---|---|
| **Split In / Out in the Source monitor** | Placing a clip with its J- or L-cut already in it | Planning ahead in the Source monitor |
| **Unlink and trim on the timeline** | Making a J- or L-cut after the fact | Audio and video can drift out of sync if you move one later |

## Markers

> id: timeline.markers | does: adds, edits, finds and clears markers on the sequence or a clip | cmd: markers.add, markers.edit, markers.goNext, markers.goPrev, markers.clearCurrent, markers.clearAll, markers.filterColors, markers.showAllMarkerColors, markers.rippleSequenceMarkers | resolve: markers | premiere: markers | when: notes for later, beats to cut on, chapters, review comments | see: timeline.in-out

**[FilmCraft]**

1. Press **M** (`markers.add`) to drop a marker at the playhead on the sequence. In the Source
   monitor, M marks the source clip instead, so the marker travels with every use of that media.
2. Open a marker (`markers.edit`) to give it a name, a comment, a colour or a duration.
3. Jump between markers (`markers.goNext` / `goPrev`); show only some colours
   (`markers.filterColors`, `markers.showAllMarkerColors`).
4. Clear the selected marker (`markers.clearCurrent`) or all of them (`markers.clearAll`).

With **Ripple Sequence Markers** on (`markers.rippleSequenceMarkers`), sequence markers move with
the timeline when you ripple-delete or insert before them, so they stay on their moment.

Other kinds of markers:

- **Range markers** cover a stretch of time: **Add Range Marker** (`markers.addRange`, one second
  by default) or one over In to Out (`markers.addRangeInOut`).
- **Chapter markers** (`markers.addChapter`) mark chapters for players that show them.
- **Flash cue markers** (`markers.addFlashCue`) are cue points for interactive players.
- **Export Markers as CSV…** (`markers.exportCsv`) writes the list for a spreadsheet, a client's
  notes or YouTube chapter timestamps; **Copy Paste Includes Sequence Markers**
  (`markers.copyPasteIncludesSequenceMarkers`) carries markers along when you copy part of a
  sequence.
