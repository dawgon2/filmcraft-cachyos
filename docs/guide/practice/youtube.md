# Practice notes: YouTube and social video

**[Standard]** · how creators commonly cut, finish and publish online video, one technique per
note, with the Revolution way. Written in our own words from common practice (AI-PLAN AI3 item 0a,
0c); none of it is a rule: audiences and channels differ.

## A strong first seconds

> id: practice.youtube.hook | does: opens with the most interesting moment or the promise of the video instead of a long intro | resolve: — | premiere: — | when: almost every online video | see: practice.youtube.cold-open

**[Standard]**

- **Common practice:** show or say what the viewer gets in the first few seconds; logos and intros
  later or never.
- **Revolution:** move the best moment to the start (`edit.cut` / `edit.pasteInsert`), or a
  subsequence teaser (`sequence.makeSubsequence`).
- **Pitfalls:** promising something the video doesn't deliver.

## Cold open, then title

> id: practice.youtube.cold-open | does: starts in the middle of the action and brings the title card in after it | resolve: — | premiere: — | when: vlogs, stories, tutorials with a result to show | see: practice.youtube.hook, graphics.templates

**[Standard]**

- **Revolution:** the result shot first, then a short title from a template
  (`graphics.template.apply`).

## Pattern interrupts against drop-off

> id: practice.youtube.pattern-interrupt | does: changes the picture every few seconds in long talking sections: punch-ins, B-roll, graphics, angle changes | resolve: — | premiere: — | when: talking-head videos | see: practice.editing.jump-cuts, practice.editing.b-roll

**[Standard]**

- **Revolution:** alternate scale 100 % / 115 % on jump cuts (Motion ▸ Scale), B-roll on V2,
  a lower third or a callout that tracks its object ([tracking.md](../tracking.md)).
- **Pitfalls:** constant zooms are tiring; vary the device.

## Punch-ins from a high-resolution frame

> id: practice.youtube.punch-in | does: crops into 6.2K or 4K footage for a second framing without a second camera | resolve: Zoom in the Inspector | premiere: Motion Scale | when: single-camera interviews and vlogs | see: practice.delivery.vertical

**[Standard]**

- **Revolution:** in a 4K sequence, 6.2K open-gate footage can be scaled up to about 160 % with no
  loss (Motion ▸ Scale), and Position recentres the face.
- **Pitfalls:** punching in on footage that's already the sequence size softens it.

## Burned-in captions for vertical clips

> id: practice.youtube.vertical-captions | does: puts large, high-contrast words on screen for silent autoplay | resolve: Text+ captions | premiere: Captions with a style, burned in | when: Shorts, Reels, TikTok | see: text.captions, practice.delivery.captions

**[Standard]**

- **Revolution:** captions from the transcript (`transcript.createCaptions` with short `maxChars`
  and one line), styled big with a stroke (`captions.setStyle`), burned in at export; or
  **Upgrade Caption to Graphic** for word-by-word styling (`graphics.upgradeCaption`).
- **Pitfalls:** captions under the platform's buttons; keep them in the middle third.

## Safe areas for social

> id: practice.youtube.safe-areas | does: keeps text and faces away from where the platform puts its buttons and captions | resolve: safe area guides | premiere: safe margins | when: vertical and square deliveries | see: practice.youtube.vertical-captions

**[Standard]**

- **Common practice:** leave the bottom fifth and the right edge clear on vertical video; keep key
  text near the centre.
- **Revolution:** drag guides into the Program monitor to mark the clear zones; check with the
  exported file on a phone.

## Chapters from markers

> id: practice.youtube.chapters | does: turns markers into YouTube chapter timestamps | resolve: markers + export | premiere: chapter markers | when: tutorials, long videos | see: timeline.markers

**[Standard]**

- **Revolution:** a marker at each chapter start (`markers.add`, or `markers.addChapter`) named for
  the chapter, **Export Markers as CSV…** (`markers.exportCsv`), then paste the times and names
  into the description (the first must be 0:00).
- **Pitfalls:** chapters shorter than ten seconds are ignored by YouTube.

## Thumbnail-first planning

> id: practice.youtube.thumbnail-plan | does: plans a thumbnail moment during the shoot and pulls it from the edit | resolve: export still | premiere: Export Frame | when: every upload | see: practice.delivery.thumbnail

**[Standard]**

- **Revolution:** **Export Frame** (`file.exportFrame`) from the graded timeline at full resolution.
- **Pitfalls:** a frame with motion blur; pick a still moment or shoot a dedicated photo.

## Consistent loudness across uploads

> id: practice.youtube.loudness | does: delivers every video at about the same loudness so viewers never reach for the volume | resolve: Loudness meter | premiere: Loudness Normalization | when: a channel with many uploads | see: practice.audio.loudness-targets

**[Standard]**

- **Revolution:** Clean Up Voice at −14 LUFS on the voice clips, music ducked under it, a limiter on
  the Mix, check the meter.

## Batch exports: master, upload and vertical

> id: practice.youtube.batch-export | does: queues the upload file, a vertical version and a master in one go | resolve: Render Queue | premiere: Media Encoder queue | when: every finished video | see: export-queue.queue

**[Standard]**

- **Revolution:** add each to the export queue (`export.queue.add`) and start it
  (`export.queue.start`) when you leave the computer.

## Re-use a template project

> id: practice.youtube.template | does: starts each episode from a project with the bins, sequences, titles and sound already set up | resolve: project template, PowerBins | premiere: project template | when: series and regular uploads | see: file-menu.projects

**[Standard]**

- **Revolution:** **Save as Template…** (`file.saveAsTemplate`) on a finished, emptied project;
  **New Project from Template** for each episode; graphics templates and look packs for the
  rest ([packs.md](../packs.md)).

## B-roll library

> id: practice.youtube.b-roll-library | does: keeps reusable B-roll, music and sound effects in an organised library across projects | resolve: shared bins, external folders | premiere: libraries, Media Browser | when: channels that reuse shots and sounds | see: media.import

**[Standard]**

- **Revolution:** a folder structure on disk with clear names, imported per project; search bins by
  name or label inside the project (`file.newSearchBin`).

## Screen recordings: sharp and readable

> id: practice.youtube.screen-recordings | does: keeps screen recordings crisp: matching frame rate, no up-scaling, zooms on the important part | resolve: — | premiere: — | when: tutorials, software videos | see: media.clips

**[Standard]**

- **Revolution:** a sequence at the recording's size and frame rate (`file.newSequenceFromClip`),
  zooms with Motion ▸ Scale keyframes (ease in / out), constant-quality export so text stays sharp.
- **Pitfalls:** variable-frame-rate recordings drift out of sync; record at a constant rate.

## Music licensing check

> id: practice.youtube.music-rights | does: uses music with a licence that covers the platform, and keeps the licence | resolve: — | premiere: — | when: every video with music | see: essential-sound.remix

**[Standard]**

- **Common practice:** licensed libraries or your own music; keep the licence file with the
  project; don't rely on "fair use" for background music.
- **Revolution:** fit the licensed track's length with **Remix** (`clip.remix`) rather than cutting
  it badly.

## Review before publishing

> id: practice.youtube.review | does: watches the export once as a viewer, on a phone, before publishing | resolve: — | premiere: — | when: every upload | see: practice.delivery.check-export

**[Standard]**

- **Revolution:** send the export to your phone (KDE Connect, [linux.md](../linux.md)) and watch it
  through with sound on and off.
