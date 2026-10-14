# Practice notes: editing

**[Standard]** · how editors commonly work, one technique per note, with the Resolve and Premiere
way and the FilmCraft way side by side. Written in our own words from common practice (AI-PLAN
AI3 item 0a, 0c); each note says when it fits and what can go wrong.

## Selects before the edit

> id: practice.editing.selects | does: watches everything once and marks the usable moments before cutting | resolve: Media Pool bins, markers, favourites | premiere: subclips, markers, a selects sequence | when: any project with more footage than the final film | see: practice.editing.string-out, media.bins

**[Standard]**

- **Goal:** know the material, and cut from the best moments only.
- **Resolve / Premiere:** watch each clip, mark good takes and moments with markers or subclips,
  sort them into a selects bin.
- **FilmCraft:** markers in the Source monitor (M), **Make Subclip** (`clip.makeSubclip`), a bin
  per scene or a **Search Bin** for a label (`file.newSearchBin`).
- **Why:** the first watch is the only time you see the footage fresh.
- **Pitfalls:** marking everything as "good" defeats the point; be strict.

## The string-out

> id: practice.editing.string-out | does: lays all selects of a scene end to end on a sequence to see what you have | resolve: append to timeline | premiere: Automate to Sequence, append | when: before the first real cut of a scene | see: practice.editing.selects, practice.editing.assembly

**[Standard]**

- **Goal:** one sequence per scene with every usable piece, in order.
- **Resolve / Premiere:** append selects to a sequence (Premiere's Automate to Sequence does it in
  one step).
- **FilmCraft:** select the subclips and run **Automate to Sequence…** (`clip.automateToSequence`).
- **Why:** comparing takes side by side on a timeline is faster than in the bin.
- **Pitfalls:** string-outs get long; trim obvious waste as you go.

## Assembly, rough cut, fine cut, picture lock

> id: practice.editing.assembly | does: works in passes from everything in order to a locked edit | resolve: duplicate timelines per version | premiere: duplicate sequences per version | when: any film longer than a minute | see: practice.editing.versions

**[Standard]**

- **Goal:** structure first, detail last.
- **Common passes:** **assembly** (everything in script order) → **rough cut** (structure, the
  right takes) → **fine cut** (timing, trims, sound edits) → **picture lock** (no more timing
  changes, so colour and sound can finish).
- **FilmCraft:** one sequence per pass (duplicate a sequence in the Project panel,
  `edit.duplicate`), named by date or version.
- **Why:** polishing a scene that is later cut wastes the polish.
- **Pitfalls:** grading or mixing before lock means redoing it when timing changes.

## Keep versions, not one ever-changing sequence

> id: practice.editing.versions | does: duplicates the sequence before big changes so earlier versions stay | resolve: duplicate timeline | premiere: duplicate sequence | when: before restructuring, before client changes | see: practice.editing.assembly, media.saving

**[Standard]**

- **FilmCraft:** duplicate the sequence (`edit.duplicate` in the Project panel) and keep working on
  the copy; auto-saves and the journal cover crashes, versions cover "the old one was better".
- **Pitfalls:** too many versions without names; date them.

## Cut on action

> id: practice.editing.cut-on-action | does: cuts between angles in the middle of a movement so the cut hides in the motion | resolve: Trim / Roll edit | premiere: Rolling Edit | when: changing angle during an action: sitting, turning, a door | see: editing.trimming

**[Standard]**

- **Goal:** cuts the viewer doesn't notice.
- **Resolve / Premiere:** find the same moment of the movement in both angles and cut a few frames
  into it; fine-tune with a rolling edit.
- **FilmCraft:** Rolling Edit tool (`tool.rolling`) or `trim.edit` with the roll type, frame by
  frame (`trim.forward`, `trim.backward`).
- **Why:** the eye follows motion across the cut.
- **Pitfalls:** mismatched motion (a hand up in one angle, down in the next) breaks it.

## J-cuts and L-cuts

> id: practice.editing.j-l-cuts | does: lets sound start before (J) or carry on after (L) the picture cut | resolve: unlink and trim audio / video separately | premiere: Alt-drag a linked edit, split edits | when: dialogue scenes, interviews, scene transitions | see: timeline.in-out

**[Standard]**

- **Goal:** smoother, more natural conversations and transitions.
- **Resolve / Premiere:** trim only the audio or only the video side of an edit (unlink, or
  modifier-drag); or set split In / Out in the source.
- **FilmCraft:** split In / Out in the Source monitor (`markers.markSplitAudioIn` and pairs), or
  trim one side of the edit with linked selection off (`sequence.linkedSelection`).
- **Why:** in real life we hear someone before we turn to look at them.
- **Pitfalls:** keep sync: move audio and video of a clip together afterwards.

## B-roll over an interview

> id: practice.editing.b-roll | does: covers interview cuts with related pictures while the voice continues | resolve: overwrite on V2 | premiere: overwrite on V2 | when: interviews and documentaries | see: practice.editing.jump-cuts

**[Standard]**

- **Goal:** hide cuts in the interview and show what the speaker talks about.
- **Resolve / Premiere:** keep the interview on V1 / A1, place B-roll on V2 over the cuts, video
  only.
- **FilmCraft:** target V2 and turn off source audio (`timeline.setTargeting`), then Overwrite
  (`source.overwrite`).
- **Why:** the voice carries the story; pictures illustrate it.
- **Pitfalls:** B-roll that only decorates (unrelated pictures) feels like filler.

## Jump cuts on purpose

> id: practice.editing.jump-cuts | does: cuts within one shot to remove pauses, keeping the jump visible as a style | resolve: blade + ripple delete | premiere: razor + ripple delete, text-based editing | when: vlogs, talking-head YouTube videos | see: practice.editing.b-roll, text.cleanup

**[Standard]**

- **Goal:** fast pacing with a single camera.
- **Resolve / Premiere:** blade out pauses and ripple delete; punch in (scale 110–120 %) on every
  other cut to soften the jump.
- **FilmCraft:** **Remove Pauses** / **Remove Filler Words** from the transcript
  (`transcript.removePauses`, `transcript.removeFillers`), or razor (`tool.razor`) and ripple
  delete (`edit.rippleDelete`); punch in with Motion ▸ Scale.
- **Why:** viewers accept jumps when the energy is high.
- **Pitfalls:** too many identical jumps feel nervous; vary with punch-ins and B-roll.

## Cut music on the beat

> id: practice.editing.music-beat | does: places cuts on beats or bar lines of the music | resolve: markers on beats, snapping | premiere: markers on beats (M while playing), snapping | when: montages, trailers, music videos | see: timeline.markers, essential-sound.remix

**[Standard]**

- **Goal:** picture and music that move together.
- **Resolve / Premiere:** play the music and tap markers on the beats, then cut to the markers
  with snapping on.
- **FilmCraft:** press **M** while playing to drop markers (`markers.add`), turn on **Snap**
  (`sequence.snap`); fit the song to the length with **Remix** (`clip.remix`).
- **Why:** rhythm makes a montage feel intentional.
- **Pitfalls:** cutting on every beat is monotonous; cut on phrases and accents.

## Room tone under dialogue edits

> id: practice.editing.room-tone | does: fills gaps in dialogue with the location's quiet background so cuts don't drop to silence | resolve: room tone clips on a track | premiere: room tone clips, crossfades | when: any dialogue edit with gaps | see: mixing.clip-or-track

**[Standard]**

- **Goal:** a continuous background under the voices.
- **Resolve / Premiere:** record or find a few seconds of room tone, loop it on its own track under
  the dialogue gaps, short crossfades at the edits.
- **FilmCraft:** room tone on its own audio track, typed **Ambience** in Essential Sound, audio
  crossfades at the edits (`sequence.applyAudioTransition`).
- **Why:** a sudden silence is louder than any noise.
- **Pitfalls:** room tone from another location changes colour of the noise; record it on set.

## Three-point edits with markers

> id: practice.editing.three-point-markers | does: places source clips exactly with In / Out and the timeline playhead, using markers as targets | resolve: three-point editing | premiere: three-point editing | when: building a cut from the keyboard | see: editing.three-point

**[Standard]**

- **FilmCraft:** mark In / Out in the Source monitor (`markers.markIn`, `markers.markOut`), park
  the timeline playhead (or a marker) and Insert or Overwrite (`source.insert`,
  `source.overwrite`).
- **Why:** keyboard editing is faster and more exact than dragging.
- **Pitfalls:** wrong source patching puts audio on the wrong track; check the patches first.

## Pacing: leave room, then tighten

> id: practice.editing.pacing | does: cuts long first and tightens in later passes, watching the scene in real time | resolve: Trim edit mode | premiere: Ripple Trim | when: fine cut | see: trimming.to-playhead

**[Standard]**

- **Goal:** a rhythm that feels right at normal speed.
- **Common practice:** leave a little air in the rough cut, then watch the scene through without
  stopping and tighten where attention drops.
- **FilmCraft:** **Ripple Trim Previous / Next Edit to Playhead** while playing
  (`trim.ripplePrevious`, `trim.rippleNext`).
- **Pitfalls:** trimming while paused makes everything too tight; judge in playback.

## Proxies for heavy footage

> id: practice.editing.proxies | does: edits with smaller proxy files and switches to the originals for colour and export | resolve: proxy or optimised media | premiere: proxies with the toggle | when: 6.2K, raw, or a slow laptop | see: proxies.what

**[Standard]**

- **FilmCraft:** automatic proxies build in the background ([proxies.md](../proxies.md)); in the
  default **Auto** mode playback uses the proxy and the paused frame is the original, so you can
  judge focus when stopped; `media.toggleProxies` switches proxies off to check.
- **Pitfalls:** judging focus or noise while a proxy is playing.

## Sync separate sound by waveform

> id: practice.editing.sync-sound | does: lines up audio from a separate recorder with the camera clip by their waveforms | resolve: Auto Sync Audio based on waveform | premiere: Synchronize / Merge Clips by audio | when: an external recorder or lavalier | see: media.clips

**[Standard]**

- **FilmCraft:** **Merge Clips** (`clip.mergeClips` with the audio method) for one item per take,
  or **Synchronize** (`clip.synchronize`) on the timeline.
- **Why:** a clap or any shared sound lines both recordings up to the frame.
- **Pitfalls:** very noisy camera audio may not match; clap at the start of every take.

## Rough audio levels early, real mix after lock

> id: practice.editing.temp-mix | does: keeps dialogue audible and music under it while cutting, leaving the real mix for after picture lock | resolve: Edit page clip levels, Fairlight after lock | premiere: clip volume while cutting, Essential Sound / mixer after lock | when: every edit | see: essential-sound.loudness, practice.editing.assembly

**[Standard]**

- **FilmCraft:** Essential Sound types and **Auto-Match** (`essentialSound.autoMatch`) for quick
  even levels while cutting; the mixer and automation ([mixing.md](../mixing.md)) after lock.
- **Pitfalls:** a detailed mix before lock is redone with every timing change.
