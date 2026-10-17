# The File and Clip menus

**[Revolution]** · projects and sequences, project templates and settings, search bins and
offline files, more export formats and frames, auto-saves; and the Clip menu's video, audio and
metadata options; modelled on Premiere Pro's menus; written for AI-PLAN AI3-PREP (feature pages)

## Projects, sequences and templates

> id: file-menu.projects | does: makes, opens, closes and saves projects and sequences, and makes projects from templates | cmd: file.newProject, file.open, file.close, file.closeProject, file.closeAllProjects, file.closeAllOtherProjects, file.saveAll, file.newSequence, file.newSequenceFromClip, file.saveAsTemplate, file.newProjectFromTemplate, file.openDemoProject | resolve: Project Manager, New Timeline | premiere: File ▸ New / Open / Close, Sequence From Clip, project templates | when: starting work; reusing the same bins for every client job | see: projects.what, media.saving

**[Revolution]**

- **New ▸ Sequence…** (`file.newSequence`) makes an empty sequence; **Sequence From Clip**
  (`file.newSequenceFromClip`) makes one that matches the selected clip's size and frame rate:
  the quickest way to get settings right for camera footage.
- **Save as Template…** (`file.saveAsTemplate`) keeps the current project's bins, sequences and
  settings as a starting point; **New Project from Template** (`file.newProjectFromTemplate`)
  starts a project from one.
- **Save All** (`file.saveAll`) saves every open project; **Close Project** and its variants close
  them. **Demo Project** (`file.openDemoProject`) opens a small sample to try things on.

## Project settings and scratch disks

> id: file-menu.project-settings | does: the project's general settings and where captures, previews and auto-saves are written | cmd: file.projectSettings.general, file.projectSettings.scratchDisks | resolve: Project Settings ▸ Master Settings, Working Folders | premiere: Project Settings ▸ General, Scratch Disks | when: a fast drive for previews; recordings saved with the project | see: render-cache.files

**[Revolution]**

File ▸ Project Settings ▸ **General…** (`file.projectSettings.general`) and **Scratch Disks…**
(`file.projectSettings.scratchDisks`): where captured audio and video (voice-over recordings,
narrations), preview files and auto-saves go.

## Search bins, bins from selection and offline files

> id: file-menu.bins | does: bins that fill themselves from a search, bins from the selection, and placeholder offline files | cmd: file.newSearchBin, project.editSearchBin, project.deleteSearchBin, file.newBinFromSelection, file.newOfflineFile | resolve: Smart Bins | premiere: Search Bin, Bin From Selection, Offline File | when: "every clip with this name / label / metadata", a placeholder for media that hasn't arrived | see: media.bins

**[Revolution]**

- A **Search Bin** (`file.newSearchBin`) shows every item matching its rules (a column, a
  condition and text; several rows, all or any), and updates as the project changes; edit or
  delete it with `project.editSearchBin` / `project.deleteSearchBin`.
- **Bin From Selection** (`file.newBinFromSelection`) puts the selected items in a new bin.
- **Offline File…** (`file.newOfflineFile`) is a placeholder with a name, size, frame rate and
  timecode, to cut with before the real file exists; link it later ([media.md](media.md)).

## More import and export formats

> id: file-menu.formats | does: imports image sequences and from the Media Browser; exports frames, AAF / OMF / ALE / FCP 7 XML and part of a project | cmd: file.importImageSequence, file.importFromMediaBrowser, file.exportFrame, file.exportAaf, file.exportOmf, file.exportAle, file.exportFcp7Xml, file.exportInterchange, file.exportSelectionProject | resolve: Export Timeline, Export Frame as Still | premiere: Export Frame, Export AAF / OMF / XML, Selection as Premiere Project | when: a still for a thumbnail; handing a sound mix to Pro Tools; moving part of a project | see: media.interchange

**[Revolution]**

- **Export Frame** (`file.exportFrame`): the Program (or Source) monitor's frame as PNG, JPEG,
  TIFF or BMP, 8- or 16-bit, optionally imported back into the project: the usual way to make a
  YouTube thumbnail from the edit.
- **AAF** / **OMF** (`file.exportAaf`, `file.exportOmf`): the edit for a sound mix in a
  dedicated audio app; **Avid Log Exchange** (`file.exportAle`) and **Final Cut Pro XML**
  (`file.exportFcp7Xml`) for other editors; `file.exportInterchange` picks the format by name.
- **Selection as Revolution Project…** (`file.exportSelectionProject`) writes the selected items as
  a new project file.
- **Import Image Sequence…** (`file.importImageSequence`) brings numbered stills in as one clip.

## Auto-saves and recovery files

> id: file-menu.autosaves | does: shows the auto-save status, browses auto-saved copies and lists recoverable sessions | cmd: file.autoSaveStatus, file.listAutoSaves, file.recoveryList | resolve: Project Backups | premiere: Auto-Save folder | when: going back to an older version of the project after a mistake | see: media.saving

**[Revolution]**

`file.autoSaveStatus` tells when the last auto-save ran; **Browse Auto-Saves**
(`file.listAutoSaves`) lists the saved copies to open one; `file.recoveryList` lists sessions
whose unsaved changes can still be recovered.

## Clip menu: video options

> id: file-menu.clip-video | does: fit or fill the frame, frame hold options and segments, field options, poster frames | cmd: clip.fitToFrame, clip.fillFrame, clip.frameHoldOptions, clip.insertFrameHoldSegment, clip.fieldOptions, clip.setPosterFrame, clip.clearPosterFrame | resolve: Retime / Freeze Frame, Clip Attributes, Scaling | premiere: Clip ▸ Video Options (Set to Frame Size, Frame Hold, Field Options) | when: clips of another size; a freeze in the middle of a clip; interlaced footage | see: media.clips

**[Revolution]**

- **Fit to frame** / **Fill frame** (`clip.fitToFrame`, `clip.fillFrame`): scale a clip so all of
  it shows (black bars where the shape differs) or so it fills the frame (the edges cut off).
- **Insert Frame Hold Segment** (`clip.insertFrameHoldSegment`) cuts in a frozen frame (2 s by
  default) at the playhead; **Frame Hold Options…** (`clip.frameHoldOptions`) choose which frame
  a hold shows.
- **Field Options…** (`clip.fieldOptions`): for interlaced footage, reverse the field order or
  always deinterlace / remove flicker.
- **Set Poster Frame** (`clip.setPosterFrame`, Cmd+P) picks the frame a clip's thumbnail shows;
  **Clear Poster Frame** (`clip.clearPosterFrame`).

## Clip menu: audio, timecode and metadata

> id: file-menu.clip-audio | does: clip volume keys, timecode, metadata, replacing from the Source monitor, automating to a sequence | cmd: clip.volumeUp, clip.nudgeVolumeUp1, clip.audioPeak, clip.modifyTimecode, clip.updateMetadata, clip.replaceFromSource, clip.automateToSequence | resolve: clip gain keys, Clip Attributes | premiere: Clip ▸ Audio Options, Modify ▸ Timecode, Automate to Sequence | when: quick level changes; a rough assembly from selects | see: media.audio-channels

**[Revolution]**

- Clip volume keys: up or down a step (`clip.volumeUp`, `clip.volumeDown`, `…Many` for bigger
  steps) or by 1 or 3 dB (`clip.nudgeVolumeUp1`, `clip.nudgeVolumeUp3` and the Down pair);
  `clip.audioPeak` reports a clip's peak level.
- **Modify ▸ Timecode…** (`clip.modifyTimecode`) gives an item a start timecode or tape name, or
  resets it to the file's own.
- **Update Metadata…** (`clip.updateMetadata`) edits an item's metadata.
- **Replace With Clip ▸ From Source Monitor** (`clip.replaceFromSource`; `…MatchFrame` lines the
  new media up on the matching frame) keeps the timeline clip's place and effects.
- **Automate to Sequence…** (`clip.automateToSequence`) places the selected items on the
  sequence one after another (or at unnumbered markers), as inserts or overwrites, with optional
  transitions between them: a fast first assembly from selects.
