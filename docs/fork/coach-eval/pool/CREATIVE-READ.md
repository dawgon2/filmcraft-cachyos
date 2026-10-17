# Creative training pairs: owner read (Q35 rule)

40 of the 207 creative training pairs (`pool/creative-*.jsonl`), chosen at random (Python `random.Random(20261016).sample`, so the choice can be repeated), for the owner to read before any tune uses creative pairs (AI-PLAN AI4 item 5c, OWNER-QUESTIONS Q41). Each shows the question, the recipe answer, the guide sections it rests on and its goal family. Families here are never in the held-out set.

What to look for: steps that would not work in Revolution, invented controls, gaps not admitted (or admitted wrongly), and answers that read as one right way where options would serve better. Note anything in OWNER-QUESTIONS Q41 or as a comment; the lane fixes the pair and re-checks its family.

## 1. How do I change the colour of a red car to blue in my shot?

1. Add a node for the change (`grade.addNode`) after your balance node. 2. Qualifier tab: Pick on the car's red (`grade.pickQualifier`), Add a lighter and a darker part of the paint, then set View to Matte: the car white, everything else black. Raise Qualifier softness or Matte blur if the edge chatters. 3. If other red things are caught, add a Circle or Rectangle window around the car on the same node (`masks.add`), tracked if it moves (`masks.track`): the node then acts only where both select. 4. View back to Normal; in Primaries push the hue towards blue (Gain and Gamma wheels towards blue, Saturation as needed). Options: a big hue jump (red to blue) can look painted on reflections; Lumetri's HSL Secondary does the same with its key and correction if you work in Premiere style.

*Sources:* `color.secondaries.qualifier`, `color.secondaries.windows`, `color.secondaries.tracking` · *family:* `recolour-object` · *from:* creative-12.jsonl line 1

## 2. Retro TV channel switch between clips, static and a jump.

Use the VHS Damage or Glitch transition (Video Transitions ▸ Grunge & Distort) on the edit, a few frames long. For noise between channels, a short static clip on black placed over the cut works too. Add a click or static burst on an SFX track at the same frame.

*Sources:* `effects.stylize`, `effects.transitions` · *family:* `crt-tv-look` · *from:* creative-12.jsonl line 14

## 3. The 'yep, that's me, you're probably wondering how I got here' freeze frame with a record scratch.

1. Insert Frame Hold Segment at the moment (`clip.insertFrameHoldSegment`, 2 s by default). 2. Punch in on the hold: Motion Scale up and Position on your face. 3. A text graphic with the line; optionally drain the colour: an Adjustment Layer (File ▸ New) above the hold with Lumetri, Saturation keyframed down to 0. 4. A record-scratch sound effect on the first frame of the hold and duck or cut the music there.

*Sources:* `file-menu.clip-video`, `practice.youtube.punch-in`, `graphics.text` · *family:* `meme-freeze-intro` · *from:* creative-12.jsonl line 16

## 4. Flashback in black and white with grain and a vignette.

On the look node: shift colours first (wheels or a qualifier), then Saturation to 0 on a later node; grain on the last node (`grade.param.grain.*`); a soft inverted ellipse window with Gain down for the vignette. Put the look node on every flashback shot (`grade.append`) so it stays identical.

*Sources:* `practice.looks.black-white`, `color.finishing.grain`, `practice.grading.vignette` · *family:* `flashback-dream` · *from:* creative-12.jsonl line 26

## 5. miniature effect but the sharp area should follow a car

Same setup (blurred copy on V2 with an inverted, feathered mask), then track the mask on the car: Mask Path's track buttons (`masks.track`). Lost tracks on plain road can be fixed by hand with mask keyframes.

*Sources:* `effects.opacity`, `tracking.what` · *family:* `tilt-shift-miniature` · *from:* creative-13.jsonl line 5

## 6. Everyone in the street freezes while I keep walking through them.

Shoot it locked off: the take where you walk through people who keep moving. 1. On V1, cut the take where the world should freeze (`sequence.addEdit`) and Frame Hold the part after it (`clip.frameHold`). 2. On V2, the same take (moving) with a mask around you on Opacity (`masks.add`), feathered and tracked as you walk (`masks.track`). 3. Where you pass in front of frozen people it works; where moving people cross you, the mask shows them moving, so stage the extras to stay still or out of your path.

*Sources:* `media.clips`, `effects.opacity`, `tracking.what` · *family:* `frozen-world-walk` · *from:* creative-13.jsonl line 13

## 7. Bullet-time style frozen moment with the camera moving around?

Not possible from one camera in Revolution: real bullet time needs many cameras or a 3-D scene, and the compositor has no 3D. What works: a frozen moment from a locked-off shot (Frame Hold + masked live layer), with a slow Scale / Position push on the hold to suggest camera movement.

*Sources:* `compositor.basics`, `media.clips`, `effects.keyframes` · *family:* `frozen-world-walk` · *from:* creative-13.jsonl line 15

## 8. Slow-mo only for the moment the ball hits, normal speed around it.

A stepped ramp: Add Edit at the start and end of the hit (`sequence.addEdit`), the middle piece at the slow speed (e.g. 20 % for 120 fps in 24), the outer pieces at 100 %, Ripple on. Revolution has no smooth ramp curve yet (Time Remapping keyframes aren't played); a very short dissolve between the pieces softens the steps.

*Sources:* `practice.editing.speed-ramp`, `media.clips` · *family:* `high-fps-slowmo` · *from:* creative-15.jsonl line 3

## 9. How do I get an infrared or false-colour map look?

Options: the Duotone shader recipe (two colours by brightness), or write your own .wgsl effect with a colour ramp (`effects.reloadUser` loads it); shader effects run on the GPU only.

*Sources:* `shader-effects.recipes`, `shader-effects.use` · *family:* `thermal-look` · *from:* creative-15.jsonl line 12

## 10. Fast-forward through a boring part with a '>> 8x' label.

Cut the part out (`sequence.addEdit` at both ends), set it to 800 % with Speed / Duration (`clip.speedDuration`, Ripple on), and put a small text graphic '>> 8×' on V2 over it. Mute or replace its sound (sped-up audio chipmunks): a whoosh or the music.

*Sources:* `media.clips`, `graphics.text`, `editing.cutting` · *family:* `fast-forward-skip` · *from:* creative-15.jsonl line 19

## 11. eyes flash red for one second

Windows on the eyes on their own node, tracked; push Gain towards red. To make it last one second, cut the clip around the moment (`sequence.addEdit`) and put that node only on the middle piece, or keyframe an Opacity-masked red glow copy on and off.

*Sources:* `color.secondaries.windows`, `color.secondaries.tracking`, `editing.cutting` · *family:* `glowing-eyes` · *from:* creative-17.jsonl line 5

## 12. Glowing eyes but the person blinks and the glow stays.

The window keeps glowing over closed lids: keyframe the effect off for the blink frames (cut the clip and drop the node there, or keyframe the glow copy's Opacity to 0 during the blink).

*Sources:* `color.secondaries.tracking`, `effects.keyframes` · *family:* `glowing-eyes` · *from:* creative-17.jsonl line 6

## 13. spinning headline but I also want it tilted in 3D

3D Rotate (Video Effects ▸ Transform) with X / Y Rotation and Perspective gives the tilt; keyframe Z Rotation for the spin and Z Position to fly it in.

*Sources:* `effects.shape-move`, `effects.keyframes` · *family:* `newspaper-spin` · *from:* creative-17.jsonl line 14

## 14. Parallax photo edges show holes when I move the layers.

Moves bigger than the overlap reveal gaps; keep the moves small, scale the background up a little, or paint the gap out in an image editor before importing.

*Sources:* `effects.keyframes`, `media.import` · *family:* `photo-parallax` · *from:* creative-17.jsonl line 18

## 15. Clean up a recorded video call with four people for YouTube.

1. Clean Up Voice on the call audio (`audio.cleanupVoice`) or Voice Isolation for keyboard and room noise. 2. Remove long pauses and filler words from the transcript (`transcript.removePauses`, `transcript.removeFillers`). 3. Punch in on the speaker's tile with Motion Scale / Position when one person talks for a while. 4. Captions from the transcript.

*Sources:* `audio.cleanup-voice`, `text.cleanup`, `practice.youtube.punch-in` · *family:* `video-call-recording` · *from:* creative-17.jsonl line 19

## 16. Debris and smoke after the blast.

Stock smoke and debris overlays on black (Screen) or on transparency, layered over the shot and the explosion; no particles or simulation in Revolution.

*Sources:* `effects.opacity`, `compositor.basics` · *family:* `explosion-composite` · *from:* creative-17.jsonl line 27

## 17. Grey skin after normalising F-Log2, on 40 interview clips.

Fix one clip with the skin node, then copy that node to the others (`grade.append`) and check each: skin keys can need small hue-width changes per clip. Check scene (`grade.checkScene`) finds clips that drift.

*Sources:* `color.recipes.skin`, `color.matching.copy-paste`, `color.matching.check-scene` · *family:* `grey-skin-fix` · *from:* creative-19.jsonl line 3

## 18. The scene starts happy and turns sad; I want the colour to cool down with it.

Keyframe a grade across the turn: Lumetri on the clip (`effects.apply`, `lumetri`) with Temperature, Saturation and Exposure keyframed from warm and bright to cool and muted over the moment of change, eased (`effects.setInterpolation`). For a change across several shots, put Lumetri on an Adjustment Layer above them so one set of keyframes covers all.

*Sources:* `color.lumetri.panel`, `effects.keyframes`, `media.generated` · *family:* `mood-shift-grade` · *from:* creative-19.jsonl line 4

## 19. Web and broadcast versions from one edit.

Queue two exports: the web one (full range, −14 LUFS) and the broadcast one (limiter, −23 LUFS on a separate mix). A duplicate of the sequence for the broadcast mix keeps both.

*Sources:* `export-queue.queue`, `practice.audio.loudness-targets`, `practice.editing.versions` · *family:* `broadcast-delivery` · *from:* creative-19.jsonl line 15

## 20. Translation labels for many signs, same style.

Make one label, export it as a template with the text editable (`graphics.template.export`), and apply it per sign.

*Sources:* `graphics.templates`, `practice.titles.templates` · *family:* `sign-translation-label` · *from:* creative-19.jsonl line 18

## 21. My silent B-roll needs footsteps and small sounds so it feels real.

Add footsteps and handling sounds from a sound library on an SFX track, lined up with what you see (frame-step to the contacts); type them SFX in Essential Sound; a quiet ambience bed under all. Keep them a little lower than real so they don't distract.

*Sources:* `essential-sound.types`, `practice.audio.ambience-beds`, `timeline.navigate` · *family:* `add-foley` · *from:* creative-19.jsonl line 19

## 22. Muffled sound plus heartbeat during the shock moment.

Cut the highs of the scene's audio with an EQ (keyframed in for the moment), lower it, and bring up the heartbeat; release it all when the shock ends.

*Sources:* `effect-reference.audio`, `effects.keyframes`, `essential-sound.types` · *family:* `heartbeat-tension` · *from:* creative-19.jsonl line 24

## 23. Shadows point different ways between shots.

Grading can't move shadows: soften their contrast a little and rely on cutting choices; plan the order of shooting next time.

*Sources:* `practice.mixed-cameras.day-continuity`, `practice.editing.pacing` · *family:* `sun-moved-continuity` · *from:* creative-19.jsonl line 27

## 24. A bright red bin in the background keeps pulling my eye.

On its own node, select it (qualifier on the red plus a window around it, tracked if the camera moves) and lower its saturation and brightness until it sinks back. A soft window keeps the edge invisible.

*Sources:* `color.secondaries.qualifier`, `color.secondaries.windows`, `color.secondaries.tracking` · *family:* `distracting-object-tame` · *from:* creative-19.jsonl line 28

## 25. Glossy car commercial look for my friend's car.

Balance first, then a look node: deep blacks and strong contrast (S-curve), cool shadows and slightly warm highlights, Saturation restrained; a qualifier on the paint colour to make it rich. Speed and energy from the edit: short shots, a few slow-motion pieces (Speed / Duration, Optical Flow), a Lens Flare on a highlight.

*Sources:* `practice.grading.look-node`, `practice.grading.split-tone`, `effects.lens-light` · *family:* `car-ad-look` · *from:* creative-20.jsonl line 4

## 26. reflections on the car paint look dull

Lift the highlights on the paint with a qualifier or a window on its own node (Gain up, a little contrast), keeping the rest; Glint or Wonder Glow on the brightest reflections for sparkle. Don't clip them: check the waveform.

*Sources:* `color.secondaries.qualifier`, `effects.lens-light`, `color.finishing.scopes` · *family:* `car-ad-look` · *from:* creative-20.jsonl line 5

## 27. aftermovie: mix of phone and camera footage, different colours

Set each source's input colour space, match the phones to the camera (less sharpening, softer contrast, skin matched), then copy one look node to all (`grade.append`).

*Sources:* `practice.mixed-cameras.per-camera-input`, `practice.mixed-cameras.phone`, `practice.grading.look-node` · *family:* `festival-aftermovie` · *from:* creative-20.jsonl line 8

## 28. time-lapse world behind a person standing still

Shoot long on a tripod. Background: the clip sped up (Speed / Duration, e.g. 1000 %); foreground: the person from a normal-speed moment (or a Frame Hold) masked on top. They must hold still enough for the mask to work.

*Sources:* `media.clips`, `effects.opacity` · *family:* `frozen-person-moving-world` · *from:* creative-20.jsonl line 14

## 29. Punchy zoom burst on a hit, like in action edits.

On the hit frame: a fast Scale push of 3–5 frames (Transform effect with Use Effect Shutter Angle for motion blur), Camera Shake for a few frames after, and a Zoom Blur or Cross Zoom transition if it lands on a cut. A hit sound on the same frame.

*Sources:* `practice.titles.shake`, `effects.stylize`, `effect-reference.transitions` · *family:* `impact-zoom-burst` · *from:* creative-20.jsonl line 16

## 30. Speed-ramp into the punch for extra impact.

A stepped ramp: the build-up faster, the hit at normal speed or slow (Add Edit + Speed / Duration per piece), then the zoom burst and shake on the hit frame.

*Sources:* `practice.editing.speed-ramp`, `practice.titles.shake` · *family:* `impact-zoom-burst` · *from:* creative-20.jsonl line 18

## 31. Stock footage in Rec.709 and my F-Log2 in one look.

Set the stock's input colour space to Rec. 709 and your clips to F-Log2 so both land in the working space; grade the hero (your footage) first and match the stock to it.

*Sources:* `practice.mixed-cameras.per-camera-input`, `color.color-management.input`, `practice.mixed-cameras.hero` · *family:* `stock-footage-match` · *from:* creative-20.jsonl line 21

## 32. enhance effect with scanning lines and a timecode

Add the VHS shader or a scanline overlay on the zoomed pieces and a Metadata & Timecode Burn-in for the security-cam timecode; text graphics for 'ENHANCING…'.

*Sources:* `practice.looks.vhs`, `effect-reference.video-transform`, `graphics.text` · *family:* `csi-enhance` · *from:* creative-20.jsonl line 23

## 33. The zoomed-in frame looks too blurry even for the joke.

Zooming past the footage's resolution softens it: start from higher-resolution footage, or embrace it with stronger pixelation steps; Sharpen only a little (halos).

*Sources:* `practice.youtube.punch-in`, `practice.node-trees.sharpen` · *family:* `csi-enhance` · *from:* creative-20.jsonl line 24

## 34. Text like 'Tokyo — 3:00 AM' appearing at the start of a scene.

A text graphic in a clean font, lower left inside the safe margins, fading in with eased Opacity keyframes and out after a few seconds; save it as a template with the text editable for every location.

*Sources:* `graphics.text`, `practice.titles.in-out`, `graphics.templates` · *family:* `location-time-card` · *from:* creative-20.jsonl line 28

## 35. location card that types itself letter by letter

Reveal it with a rectangle mask on the text graphic whose edge is keyframed left to right in steps (Hold keyframes), plus a typing sound; per-character styles keyed by hand also work.

*Sources:* `effects.opacity`, `effects.keyframes`, `graphics.text` · *family:* `location-time-card` · *from:* creative-20.jsonl line 29

## 36. awkward silence joke, how long do I hold

Hold longer than feels comfortable on the awkward shot, then cut on a small action (a blink, a sip) or a sound; trim frame by frame with a rolling edit until it plays right in real time.

*Sources:* `practice.editing.cut-on-action`, `editing.trimming` · *family:* `comedy-timing` · *from:* creative-21.jsonl line 5

## 37. Moody fashion film with slow motion and a strobe feel.

Slow motion from high-frame-rate footage (Speed / Duration so every frame shows), a contrasty, slightly desaturated look node, and the strobe: Strobe Light (Video Effects ▸ Stylize) or Posterize Time on some shots, cut to the music's beats. Keep flashing short and gentle.

*Sources:* `practice.editing.speed-ramp`, `effects.stylize`, `practice.editing.music-beat` · *family:* `fashion-film` · *from:* creative-21.jsonl line 7

## 38. make the clothes' colours pop but skin stay natural

Qualify the garment colours on their own node and raise saturation there; protect skin with a skin qualifier node (or limit with windows) so faces stay on the skin line.

*Sources:* `color.secondaries.qualifier`, `practice.grading.qualify-skin`, `practice.grading.skin-line` · *family:* `fashion-film` · *from:* creative-21.jsonl line 8

## 39. A short explainer video for my app with icons and text.

1. Write the script; a text-to-speech draft (`tts.create`) gives timing before recording the real voice. 2. Screen recordings in a sequence at their size, zooms with eased Scale keyframes. 3. Icons and labels as shape and text layers, saved as templates with only the text editable. 4. Music typed Music with Ducking under the voice. 5. Captions from the transcript.

*Sources:* `text.narration-vs-recording`, `practice.youtube.screen-recordings`, `graphics.templates` · *family:* `explainer-video` · *from:* creative-21.jsonl line 10

## 40. warm romantic grade on the save the date clips

Gain slightly warm, soft contrast, lifted blacks a touch, Wonder Glow with a low intensity on the highlights; copy the look node to all clips (`grade.append`).

*Sources:* `practice.looks.golden-hour`, `practice.looks.pastel`, `practice.grading.look-node` · *family:* `save-the-date` · *from:* creative-21.jsonl line 29
