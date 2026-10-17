# Practice notes: editing

**[Standard]** · how editors commonly work, one technique per note, with the Resolve and Premiere
way and the Revolution way side by side. Written in our own words from common practice (AI-PLAN
AI3 item 0a, 0c); each note says when it fits and what can go wrong.

## Selects before the edit

> id: practice.editing.selects | does: watches everything once and marks the usable moments before cutting | resolve: Media Pool bins, markers, favourites | premiere: subclips, markers, a selects sequence | when: any project with more footage than the final film (hours of footage for a short film): where do I start? | see: practice.editing.string-out, media.bins

**[Standard]**

- **Goal:** know the material, and cut from the best moments only. With hours of footage for a
  short film, this is where to start.
- **Resolve / Premiere:** watch each clip, mark good takes and moments with markers or subclips,
  sort them into a selects bin.
- **Revolution:** markers in the Source monitor (M), **Make Subclip** (`clip.makeSubclip`), a bin
  per scene or a **Search Bin** for a label (`file.newSearchBin`).
- **Why:** the first watch is the only time you see the footage fresh.
- **Pitfalls:** marking everything as "good" defeats the point; be strict.

## The string-out

> id: practice.editing.string-out | does: lays all selects of a scene end to end on a sequence to see what you have | resolve: append to timeline | premiere: Automate to Sequence, append | when: before the first real cut of a scene | see: practice.editing.selects, practice.editing.assembly

**[Standard]**

- **Goal:** one sequence per scene with every usable piece, in order.
- **Resolve / Premiere:** append selects to a sequence (Premiere's Automate to Sequence does it in
  one step).
- **Revolution:** select the subclips and run **Automate to Sequence…** (`clip.automateToSequence`).
- **Why:** comparing takes side by side on a timeline is faster than in the bin.
- **Pitfalls:** string-outs get long; trim obvious waste as you go.

## Assembly, rough cut, fine cut, picture lock

> id: practice.editing.assembly | does: works in passes from everything in order to a locked edit | resolve: duplicate timelines per version | premiere: duplicate sequences per version | when: any film longer than a minute | see: practice.editing.versions

**[Standard]**

- **Goal:** structure first, detail last.
- **Common passes:** **assembly** (everything in script order) → **rough cut** (structure, the
  right takes) → **fine cut** (timing, trims, sound edits) → **picture lock** (no more timing
  changes, so colour and sound can finish).
- **Revolution:** one sequence per pass (duplicate a sequence in the Project panel,
  `edit.duplicate`), named by date or version.
- **Why:** polishing a scene that is later cut wastes the polish.
- **Pitfalls:** grading or mixing before lock means redoing it when timing changes.

## Keep versions, not one ever-changing sequence

> id: practice.editing.versions | does: duplicates the sequence before big changes so earlier versions stay | resolve: duplicate timeline | premiere: duplicate sequence | when: before restructuring, before client changes | see: practice.editing.assembly, media.saving

**[Standard]**

- **Revolution:** duplicate the sequence (`edit.duplicate` in the Project panel) and keep working on
  the copy; auto-saves and the journal cover crashes, versions cover "the old one was better".
- **Pitfalls:** too many versions without names; date them.

## Cut on action

> id: practice.editing.cut-on-action | does: cuts between angles in the middle of a movement so the cut hides in the motion | resolve: Trim / Roll edit | premiere: Rolling Edit | when: changing angle during an action: sitting, turning, a door | see: editing.trimming

**[Standard]**

- **Goal:** cuts the viewer doesn't notice.
- **Resolve / Premiere:** find the same moment of the movement in both angles and cut a few frames
  into it; fine-tune with a rolling edit.
- **Revolution:** Rolling Edit tool (`tool.rolling`) or `trim.edit` with the roll type, frame by
  frame (`trim.forward`, `trim.backward`).
- **Why:** the eye follows motion across the cut.
- **Pitfalls:** mismatched motion (a hand up in one angle, down in the next) breaks it.

## J-cuts and L-cuts

> id: practice.editing.j-l-cuts | does: lets sound start before (J) or carry on after (L) the picture cut | resolve: unlink and trim audio / video separately | premiere: Alt-drag a linked edit, split edits | when: dialogue scenes, interviews, scene transitions; hearing the next speaker before we see them, or sound carrying on after the cut | see: timeline.in-out

**[Standard]**

- **Goal:** smoother, more natural conversations and transitions.
- **Resolve / Premiere:** trim only the audio or only the video side of an edit (unlink, or
  modifier-drag); or set split In / Out in the source.
- **Revolution:** split In / Out in the Source monitor (`markers.markSplitAudioIn` and pairs), or
  trim one side of the edit with linked selection off (`sequence.linkedSelection`).
- **Why:** in real life we hear someone before we turn to look at them.
- **Pitfalls:** keep sync: move audio and video of a clip together afterwards.

## B-roll over an interview

> id: practice.editing.b-roll | does: covers interview cuts with related pictures while the voice continues | resolve: overwrite on V2 | premiere: overwrite on V2 | when: interviews and documentaries | see: practice.editing.jump-cuts

**[Standard]**

- **Goal:** hide cuts in the interview and show what the speaker talks about.
- **Resolve / Premiere:** keep the interview on V1 / A1, place B-roll on V2 over the cuts, video
  only.
- **Revolution:** target V2 and turn off source audio (`timeline.setTargeting`), then Overwrite
  (`source.overwrite`).
- **Why:** the voice carries the story; pictures illustrate it.
- **Pitfalls:** B-roll that only decorates (unrelated pictures) feels like filler.

## Jump cuts on purpose

> id: practice.editing.jump-cuts | does: cuts within one shot to remove pauses, keeping the jump visible as a style | resolve: blade + ripple delete | premiere: razor + ripple delete, text-based editing | when: vlogs, talking-head YouTube videos | see: practice.editing.b-roll, text.cleanup

**[Standard]**

- **Goal:** fast pacing with a single camera.
- **Resolve / Premiere:** blade out pauses and ripple delete; punch in (scale 110–120 %) on every
  other cut to soften the jump.
- **Revolution:** **Remove Pauses** / **Remove Filler Words** from the transcript
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
- **Revolution:** press **M** while playing to drop markers (`markers.add`), turn on **Snap**
  (`sequence.snap`); fit the song to the length with **Remix** (`clip.remix`).
- **Why:** rhythm makes a montage feel intentional.
- **Pitfalls:** cutting on every beat is monotonous; cut on phrases and accents.

## Room tone under dialogue edits

> id: practice.editing.room-tone | does: fills gaps in dialogue with the location's quiet background so cuts don't drop to silence | resolve: room tone clips on a track | premiere: room tone clips, crossfades | when: any dialogue edit with gaps | see: mixing.clip-or-track

**[Standard]**

- **Goal:** a continuous background under the voices.
- **Resolve / Premiere:** record or find a few seconds of room tone, loop it on its own track under
  the dialogue gaps, short crossfades at the edits.
- **Revolution:** room tone on its own audio track, typed **Ambience** in Essential Sound, audio
  crossfades at the edits (`sequence.applyAudioTransition`).
- **Why:** a sudden silence is louder than any noise.
- **Pitfalls:** room tone from another location changes colour of the noise; record it on set.

## Three-point edits with markers

> id: practice.editing.three-point-markers | does: places source clips exactly with In / Out and the timeline playhead, using markers as targets | resolve: three-point editing | premiere: three-point editing | when: building a cut from the keyboard | see: editing.three-point

**[Standard]**

- **Revolution:** mark In / Out in the Source monitor (`markers.markIn`, `markers.markOut`), park
  the timeline playhead (or a marker) and Insert or Overwrite (`source.insert`,
  `source.overwrite`).
- **Why:** keyboard editing is faster and more exact than dragging.
- **Pitfalls:** wrong source patching puts audio on the wrong track; check the patches first.

## Pacing: leave room, then tighten

> id: practice.editing.pacing | does: cuts long first and tightens in later passes, watching the scene in real time | resolve: Trim edit mode | premiere: Ripple Trim | when: fine cut; scenes feel rushed or choppy after trimming frame by frame | see: trimming.to-playhead

**[Standard]**

- **Goal:** a rhythm that feels right at normal speed.
- **Common practice:** leave a little air in the rough cut, then watch the scene through without
  stopping and tighten where attention drops.
- **Revolution:** **Ripple Trim Previous / Next Edit to Playhead** while playing
  (`trim.ripplePrevious`, `trim.rippleNext`).
- **Pitfalls:** trimming while paused makes everything too tight; judge in playback.

## Proxies for heavy footage

> id: practice.editing.proxies | does: edits with smaller proxy files and switches to the originals for colour and export | resolve: proxy or optimised media | premiere: proxies with the toggle | when: 6.2K, raw, or a slow laptop; is the clip soft or out of focus, or is that just the proxy? | see: proxies.what

**[Standard]**

- **Revolution:** automatic proxies build in the background ([proxies.md](../proxies.md)); in the
  default **Auto** mode playback uses the proxy and the paused frame is the original, so you can
  judge focus when stopped; `media.toggleProxies` switches proxies off to check.
- **Pitfalls:** judging focus or noise while a proxy is playing.

## Sync separate sound by waveform

> id: practice.editing.sync-sound | does: lines up audio from a separate recorder with the camera clip by their waveforms | resolve: Auto Sync Audio based on waveform | premiere: Synchronize / Merge Clips by audio | when: an external recorder or lavalier | see: media.clips

**[Standard]**

- **Revolution:** **Merge Clips** (`clip.mergeClips` with the audio method) for one item per take,
  or **Synchronize** (`clip.synchronize`) on the timeline, by audio, timecode, a marker or In / Out.
- **Why:** a clap or any shared sound lines both recordings up to the frame.
- **Pitfalls:** very noisy camera audio may not match; clap at the start of every take.

## Rough audio levels early, real mix after lock

> id: practice.editing.temp-mix | does: keeps dialogue audible and music under it while cutting, leaving the real mix for after picture lock | resolve: Edit page clip levels, Fairlight after lock | premiere: clip volume while cutting, Essential Sound / mixer after lock | when: every edit | see: essential-sound.loudness, practice.editing.assembly

**[Standard]**

- **Revolution:** Essential Sound types and **Auto-Match** (`essentialSound.autoMatch`) for quick
  even levels while cutting; the mixer and automation ([mixing.md](../mixing.md)) after lock.
- **Pitfalls:** a detailed mix before lock is redone with every timing change.

## Removing a person or object from a shot

> id: practice.editing.clean-plate | does: removes a person or object from a shot with an empty take of the same shot | resolve: Fusion paint, Object Removal (Studio) | premiere: Content-Aware Fill (After Effects) | when: erasing a passer-by, a sign or a stand from a locked-off shot | see: effects.opacity, tracking.what

**[Standard]**

- **Common practice:** on set, record a few seconds of the shot empty (the **clean plate**) before
  or after the take, with the camera locked off and the exposure unchanged. In the edit, put the
  empty take on the track above and show it only where the object is, through a mask.
- **Revolution:** Revolution can't remove an object and fill in behind it automatically. With an empty take:
  1. Put the take on V1 and the empty take on V2 (a Frame Hold of an empty moment works too,
     `clip.frameHold`, if nothing in the background moves).
  2. On V2's **Opacity**, draw a mask around the object (`masks.add`), feather it, and track it if
     the object moves (`masks.track`; [effects.md](../effects.md) ▸ Opacity, blend modes and masks).
  3. For a ghost or a clone, the cut-out person goes on a third track above, with its own mask,
     lower Opacity and a **Screen** blend mode (ghost) or Normal (clone).
  A comp ([compositor.md](../compositor.md)) does the same with Media In, Mask and Merge nodes.
- **Pitfalls:** a moving camera (the empty take no longer lines up; only works with tracking and luck);
  light that changed between the two takes (clouds, a door); shadows and reflections of
  the removed object left behind.

## Speed ramps and slow-motion moments

> id: practice.editing.speed-ramp | does: speeds a shot up then drops into slow motion on the key moment, and what Revolution can do today | resolve: Retime Controls, speed points, Retime Curve | premiere: Time Remapping keyframes, Speed / Duration | when: action beats, sports, a reveal, music hits | see: media.clips

**[Standard]**

- **Common practice:** fast up to the moment, slow through it (often 20–50 %), back to normal
  after; smooth ramps feel natural, hard steps feel punchy. Footage shot at a high frame rate
  (50 / 60 / 120 fps) slows down cleanly; 24 / 25 fps footage needs made-up frames.
- **Revolution today, stepped ramp:** cut the clip at the start and end of the moment
  (**Add Edit**, `sequence.addEdit`), select the middle piece and set its speed with **Speed / Duration…**
  (`clip.speedDuration`, e.g. 30 %, **Ripple** on), the outer pieces faster (e.g. 150 %). For
  slow motion from normal-frame-rate footage choose **Optical Flow** (`clip.setTimeInterpolation`)
  so in-between frames are made from the motion; **Frame Blending** is softer, **Frame Sampling**
  steppy. A short dissolve between the pieces softens the steps.
- **Revolution: no smooth speed ramps yet:** a clip's **Time Remapping** keyframes are kept (they
  copy, paste and come in from other editors' timelines) but don't change playback or export yet;
  use the stepped ramp above.
- **Pitfalls:** optical flow warps fast or crossing movement (arms, wheels); keep slow-motion
  pieces short; check the sound (ripple moves it, slow pieces of dialogue sound wrong: use the
  music or a sound effect over the ramp).

## Steadying shaky footage

> id: practice.editing.stabilize | does: steadies a handheld or wobbly shot with the Warp Stabilizer effect | resolve: Stabilization in the Color page Tracker | premiere: Warp Stabilizer | when: handheld walks, long-lens shake, a locked-off look from a handheld shot | see: effects.apply

**[Revolution]**

- **Revolution:** apply **Warp Stabilizer** (Effects ▸ Video Effects ▸ Distort, `effects.apply`).
  The first time it plays it analyses the clip's camera motion (features tracked frame to frame),
  then: **Result** **Smooth Motion** keeps the camera move but removes the shake (**Smoothness**,
  50 % by default) or **No Motion** holds the frame still like a tripod; **Framing** chooses how the
  moving edges are hidden (**Stabilize, Crop, Auto-scale** by default, up to **Maximum Scale**;
  **Synthesize Edges** fills them instead).
- **Differs from Premiere:** Revolution's stabilizer is 2-D (position, scale, rotation); its
  **Perspective** and **Subspace Warp** methods use the same 2-D model, so rolling-shutter jelly
  and strong parallax aren't corrected.
- **Options:** Smooth Motion for walking shots (keeps the move), No Motion for a fake locked-off
  shot (crops more); lower Smoothness if the frame floats.
- **Pitfalls:** big auto-scale softens the picture (check Maximum Scale); motion blur baked into
  shaky frames stays; stabilise before you add a shake or titles on top.

## Flicker in timelapses

> id: practice.editing.flicker | does: what to do about timelapse or light flicker, which Revolution has no dedicated tool for | resolve: Deflicker (Studio) | premiere: third-party plugins | when: timelapses that pulse in brightness, cheap LED or fluorescent light flicker | see: color.noise.temporal

**[Standard]**

- **Common practice:** a deflicker filter averages each frame's brightness with its neighbours;
  best fixed at the shoot (manual exposure, a shutter matched to the mains frequency).
- **Revolution: no deflicker effect yet.** Workarounds: **Temporal noise reduction** on a node
  smooths fine frame-to-frame changes but not big brightness pulses
  ([color/noise.md](../color/noise.md)); for a few bad frames, keyframe Exposure (Lumetri) or
  Offset to even them out; **Frame Blending** at a slightly changed speed mixes neighbouring frames.
- **Pitfalls:** temporal NR leaves trails on moving subjects; keyframing by hand is slow on long
  timelapses.

## Split screens and picture-in-picture

> id: practice.editing.split-screen | does: shows two or more clips side by side, with a hard, soft or diagonal edge, or one small inside the other | resolve: Transform and Cropping in the Inspector, Fusion for more | premiere: Motion + Crop, Opacity masks | when: two people on a phone call, before / after, a reaction next to the action, a small face-cam over a screen recording | see: effects.opacity, compositor.basics

**[Standard]**

- **Revolution, two halves:** put one clip on V1 and the other on V2. On the V2 clip apply
  **Crop** (Effects ▸ Legacy ▸ Video Effects; search "Crop") with **Left** about 50 %, and move it
  with **Motion ▸ Position** so its picture sits in the right half; on V1, slide **Position** so
  the subject sits in the left half. **Edge Feather** on Crop softens the seam.
- **Diagonal or soft edge:** instead of Crop, draw a pen mask on V2's **Opacity** along the
  diagonal (`masks.add`), and raise **Mask Feather** for a soft blend ([effects.md](../effects.md)
  ▸ Opacity).
- **Picture-in-picture:** on V2, **Motion ▸ Scale** about 25–35 % and **Position** in a corner,
  with a thin border or shadow from a graphic behind it if it needs to stand out.
- **Three or more panels:** a comp in the Compositor (Media In per clip, Transform, Merge) keeps
  the layout in one place ([compositor.md](../compositor.md)).
- **Pitfalls:** frame each shot for its half when you shoot (eyelines facing each other); match
  colour and brightness of the halves; keep the seam out of faces.

## Timelapses and hyperlapses

> id: practice.editing.timelapse | does: makes a timelapse from interval photos or a long clip, and a hyperlapse from a walk | resolve: image sequence import, Retime | premiere: image sequence import, Speed / Duration | when: a sunset, clouds, a city at night, a long walk shown in seconds | see: media.import, media.clips

**[Standard]**

- **From interval photos:** import the numbered stills as one clip: the image-sequence preference
  in **File ▸ Import**, or **File ▸ Import from Media Browser** with the stills picked as an image
  sequence (`mediaBrowser.import` with `imageSequence`; [media.md](../media.md)). Each photo is one
  frame; **Speed / Duration…** (`clip.speedDuration`) makes it run faster or slower.
- **From a long clip:** **Speed / Duration…** at 1000–4000 % (a 20-minute clip at 6000 % lasts
  20 seconds), or set the **Duration** you need to fit the music. **Frame Sampling** keeps it crisp
  and steppy; **Frame Blending** smooths motion into soft streaks (`clip.setTimeInterpolation`).
- **Hyperlapse (moving camera):** speed the walk up, then add **Warp Stabilizer** with a high
  Smoothness ([practice.editing.stabilize](editing.md)); a walk filmed with steady steps and a
  fixed point to aim at stabilises far better.
- **Pitfalls:** brightness flicker between photos (auto exposure; see the flicker note, no
  deflicker yet); fast-moving people turn into a strobing blur; plan the shot's length:
  frames needed = seconds wanted × frame rate.
