# Practice notes: workflow and organisation

**[Standard]** · how editors commonly organise media, projects and backups so work stays fast and
safe, one technique per note, with the Revolution way. Written in our own words from common
practice (AI-PLAN AI3 item 0a, 0c).

## One folder per project

> id: practice.workflow.project-folder | does: keeps footage, audio, graphics, exports and the project file under one folder | resolve: project media folders | premiere: project folder with media | when: every project; how to lay out the folders for a new project on disk | see: projects.where

**[Standard]**

- **Common practice:** `Project/Footage/<date or card>`, `Audio`, `Music`, `Graphics`, `Exports`,
  `Project`.
- **Revolution:** projects are files you can keep in that folder (Settings ▸ General ▸ projects
  folder for the Project Manager); verified card copies go into `Footage`
  ([cards.md](../cards.md)).
- **Pitfalls:** media scattered over desktop and downloads breaks links when moving.

## Copy cards with verification, keep them until backed up

> id: practice.workflow.card-copy | does: copies every card with a checksum check before formatting it | resolve: Clone tool | premiere: Ingest with verify | when: every shoot | see: cards.copy-details

**[Standard]**

- **Revolution:** the card bar's verified copy (`card.offload`); format the card in the camera only
  after the copy is verified **and** backed up.

## Three copies, two kinds of storage, one elsewhere

> id: practice.workflow.backups | does: keeps three copies of footage on two kinds of storage with one off site | resolve: — | premiere: — | when: any footage you can't reshoot | see: practice.workflow.card-copy

**[Standard]**

- **Common practice:** the working drive, a backup drive, and a copy somewhere else (another
  building or cloud).
- **Revolution:** the checksum list from the card copy lets you verify any copy later.
- **Pitfalls:** two copies on the same drive count as one.

## Name things consistently

> id: practice.workflow.naming | does: names sequences, exports and versions with dates and versions | resolve: — | premiere: — | when: everything you'll look for later | see: practice.editing.versions

**[Standard]**

- **Common practice:** `Client_Project_v03_2026-10-14`, never `final_final2`.
- **Revolution:** rename sequences and items (`clip.rename`); the export file name defaults to the
  sequence name.

## Bins by type and by scene

> id: practice.workflow.bins | does: organises the project with bins by type (footage, audio, graphics, sequences) and by scene or day | resolve: Media Pool bins, Smart Bins | premiere: bins, search bins | when: projects with more than a few clips | see: file-menu.bins

**[Standard]**

- **Revolution:** bins (`file.newBin`, `project.moveToBin`) and search bins for labels or names
  (`file.newSearchBin`).

## Labels mean the same thing every time

> id: practice.workflow.labels | does: uses label colours with a fixed meaning (interview, B-roll, music, graphics) | resolve: clip colours | premiere: labels | when: busy timelines | see: edit-menu.select

**[Standard]**

- **Revolution:** **Label** (`edit.label.*`) and **Select Label Group** to act on all of one kind.

## Save often, and let auto-save cover the rest

> id: practice.workflow.saving | does: saves at milestones and relies on auto-save and the journal for crashes | resolve: Live Save | premiere: Auto Save | when: always | see: media.saving

**[Standard]**

- **Revolution:** Save (Cmd+S) at every milestone; the journal recovers unsaved changes after a
  crash (`file.recover`); Save a Copy before experiments (`file.saveCopy`).

## Proxies or the render cache for smooth playback

> id: practice.workflow.smooth-playback | does: picks proxies for heavy source files and the render cache for heavy effects | resolve: proxies vs render cache | premiere: proxies vs preview rendering | when: playback stutters | see: proxies.what, render-cache.why

**[Standard]** **[Fork]**

In Revolution, proxies are queued automatically for heavy camera files and make them play
smoothly, and the render cache is on by default (Smart): it renders heavy graded or effect-laden
stretches ahead while you are idle. An export still reads the original files, so neither lowers
the quality of what you deliver.

| Option | Good for | Trade-offs |
|---|---|---|
| **Proxies** | Heavy camera files (6.2K, 4:2:2) | Disk space; built once per clip |
| **Render cache** | Heavy grades and effects | Rebuilt when the edit changes |
| **Lower playback resolution** | A quick fix while cutting | A softer picture while playing |

## Keep the Linux desktop out of the way

> id: practice.workflow.desktop | does: uses full screen, a quiet desktop and do-not-disturb during sessions | resolve: — | premiere: — | when: long edit sessions | see: linux.fullscreen

**[Standard]**

- **Revolution:** full screen by default, the screen stays on while playing, notifications when a
  long export finishes ([linux.md](../linux.md)).

## Organise the timeline by track roles

> id: practice.workflow.track-roles | does: keeps fixed roles per track: V1 main, V2 B-roll, V3 graphics; A1–A2 dialogue, A3 music, A4 effects | resolve: track names and colours | premiere: track names | when: every edit; which tracks to use for what, a standard track layout | see: mixing.submixes

**[Standard]**

- **Revolution:** name tracks and route them to submixes for dialogue, music and effects
  (`mixer.setStrip`, `mixer.addSubmix`).
- **Why:** levels and exports stay predictable; another editor finds their way at once.

## Keyboard first

> id: practice.workflow.keyboard | does: learns the twenty most-used shortcuts and adds keys for frequent commands | resolve: keyboard customisation | premiere: keyboard shortcuts | when: from the first week | see: shortcuts.change

**[Standard]**

- **Revolution:** a preset that matches your habits (`shortcuts.loadPreset`), then your own keys
  (`shortcuts.set`).

## Review copies with burned-in timecode

> id: practice.workflow.review-copies | does: sends small review files with timecode on screen so notes point to exact frames | resolve: data burn-in | premiere: Timecode overlay on export | when: client and team reviews | see: export-queue.quick

**[Standard]**

- **Revolution:** a review export with the timecode overlay on, at a modest bitrate; collect notes as
  markers ([timeline.md](../timeline.md)).

## Archive when finished

> id: practice.workflow.archive | does: archives project, original media and master together, and removes proxies and caches | resolve: Project Archive | premiere: Project Manager | when: the project is delivered; freeing up disk space on a finished project without losing what I need | see: practice.delivery.archive

**[Standard]**

- **Revolution:** when the client has approved the film, wrap up the project for good: export the
  project, keep originals and the master, clear unused render files
  (Settings ▸ Media Cache ▸ Clear unused).

## Hand-off to a colourist or mixer

> id: practice.workflow.handoff | does: sends a locked edit to another app for colour or sound with an interchange file | resolve: XML / AAF import | premiere: XML / AAF export | when: working with specialists | see: media.interchange

**[Standard]**

- **Revolution:** FCPXML or OTIO for colour, AAF / OMF for sound (`file.exportFcpxml`,
  `file.exportAaf`); include a reference export to check against.

## Keep notes in the project

> id: practice.workflow.notes | does: writes decisions, client feedback and to-dos where the next person (or you, later) will find them | resolve: timeline notes, markers | premiere: markers and comments | when: any project that lasts more than a day | see: timeline.markers

**[Standard]**

- **Revolution:** markers with comments on the sequence; project notes (`project.setNotes`).
