# Essential Sound

**[Revolution]** · the Essential Sound panel: audio types, presets, loudness matching, repair /
clarity / creative controls, ducking, and fitting music to a length with Remix; modelled on
Premiere Pro's Essential Sound; written for AI-PLAN AI3-PREP (feature pages)

## Audio types

> id: essential-sound.types | does: tags clips as Dialogue, Music, SFX or Ambience, each with the controls that suit it | cmd: essentialSound.setType, essentialSound.clearType, essentialSound.inspect, essentialSound.set | resolve: — (Fairlight FX by hand) | premiere: Essential Sound ▸ audio types | when: starting a mix: tell Revolution what each clip is; undoing all Essential Sound settings on a clip | see: essential-sound.presets, audio.manual-voice-cleanup

**[Revolution]**

Select audio clips and pick a type in Window ▸ **Essential Sound** (`essentialSound.setType`):

| Type | Sections |
|---|---|
| **Dialogue** | Loudness, Repair (Voice Isolation, Reduce Noise, Reduce Rumble, DeHum, DeEss, Reduce Reverb), Clarity (Dynamics, EQ, Enhance Speech), Creative (Reverb) |
| **Music** | Loudness, Duration, Ducking |
| **SFX** | Loudness, Creative (Reverb), Pan |
| **Ambience** | Loudness, Creative (Reverb, Stereo Width), Ducking |

The panel doesn't process sound itself: each control drives an ordinary effect on the clip
(Reduce Rumble is a High Pass, Reduce Noise a DeNoise, Dynamics a Dynamics Processing, and so on),
visible and keyframable in Effect Controls. **Clear Audio Type** (`essentialSound.clearType`)
removes the type and those effects. `essentialSound.set` changes one setting by name
(`repair.noise.amount`), `essentialSound.inspect` shows them.

## Presets

> id: essential-sound.presets | does: applies, saves and deletes named sets of Essential Sound settings per type | cmd: essentialSound.applyPreset, essentialSound.savePreset, essentialSound.deletePreset | resolve: — | premiere: Essential Sound presets | when: a starting point for a voice, a music bed, a room | see: essential-sound.types

**[Revolution]**

Each type has built-in presets, for example **Podcast Voice**, **Clean Up Noisy Dialogue**,
**Phone Call** and **Radio Announcer** for dialogue, **Duck Under Dialogue** for music, **In a
Room** and **Far Away** for effects, **Wide Outdoor** and **Interior Room Tone** for ambience
(the Browse tab lists them all). Apply one (`essentialSound.applyPreset`), adjust, and save your
own (`essentialSound.savePreset`); a preset never changes the clip's matched loudness or its clip
volume.

## Loudness and Auto-Match

> id: essential-sound.loudness | does: measures each clip's loudness and sets its gain to the type's target | ui: essentialSound.autoMatch, essentialSound.loudness.limit | cmd: essentialSound.autoMatch | resolve: Normalize Audio Levels | premiere: Essential Sound ▸ Loudness ▸ Auto-Match | when: clips recorded at different levels should sound equally loud | see: audio.cleanup-voice

**[Revolution]**

**Auto-Match** (`essentialSound.autoMatch`) measures each selected clip's integrated loudness
(EBU R128) and sets a match gain so it lands on its type's target: by default Dialogue −23,
Music −25, SFX −21 and Ambience −30 LUFS (preferences such as `audio.dialogueTargetLufs`
change them). These targets balance
clips against each other; the delivery level of the whole mix is a separate step (for YouTube
−14 LUFS: Clean Up Voice does both for a voice, see [audio.md](audio.md)). **Limit True Peak**
holds peaks under a ceiling.

## Repair, Clarity and Creative

> id: essential-sound.controls | does: what each Dialogue, SFX and Ambience control does and which effect it drives | ui: essentialSound.repair.*, essentialSound.clarity.* | resolve: Fairlight EQ, Dynamics, Noise Reduction, De-Esser, Reverb | premiere: Essential Sound ▸ Repair / Clarity / Creative | when: shaping a voice or a sound quickly; an echoey voice recorded in a big empty room; a fan hum or wind rumble behind a voice; a voice that should sound like a phone call or a bigger room | see: audio.manual-voice-cleanup

**[Revolution]**

| Control | Drives | What the amount does |
|---|---|---|
| Voice Isolation | Voice Isolation | wet mix (see [audio.md](audio.md)) |
| Reduce Noise | DeNoise | strength |
| Reduce Rumble | High Pass | cutoff from 30 to 150 Hz |
| DeHum | DeHummer at 50 or 60 Hz | depth |
| DeEss | DeEsser | threshold and reduction |
| Reduce Reverb | DeReverb | strength |
| Dynamics | Dynamics Processing | threshold and ratio together |
| EQ | Parametric EQ with a preset (Balanced Voice, Presence Lift, Warm Voice, Clear Muddiness, Soften Harshness, Bright Air, Radio Voice, Telephone) | how much of the preset |
| Enhance Speech | Enhance Speech (low or high tone) | mix |
| Reverb | Studio Reverb with a room preset (Small Room … Stone Church, Open Air) | wet level |
| Stereo Width (Ambience) | Stereo Width | width |

Each section's switch turns its effects on or off together. The order a person would apply them
in, and why, is in [audio.md](audio.md) ("Cleaning up a voice by hand").

## Ducking music under dialogue

> id: essential-sound.ducking | does: writes Volume keyframes that lower music or ambience while dialogue plays | cmd: essentialSound.generateDucking | resolve: Fairlight sidechain ducking | premiere: Essential Sound ▸ Ducking ▸ Generate Keyframes | when: a music bed under narration or an interview | see: mixing.automation

**[Revolution]**

On a Music or Ambience clip, turn **Ducking** on, choose what to duck against (Dialogue, Music,
SFX, Ambience, or untyped clips), the sensitivity, how far to reduce (−15 dB by default, −10 for
ambience) and the fade length, then **Generate Keyframes** (`essentialSound.generateDucking`). The
result is ordinary Volume keyframes on the clip, so you can move any of them by hand.

| Option | Good for | Trade-offs |
|---|---|---|
| **Generate ducking keyframes** | Long narration over music; quick and even | Ducks at every word gap unless the sensitivity is right; regenerate after re-cutting |
| **Ride the music fader yourself** ([mixing.md](mixing.md)) | Music that should breathe with the story | Takes a pass per scene |

## Fitting music to a length: Remix

> id: essential-sound.remix | does: retimes a music clip to a target length by cutting at musically similar beats | cmd: clip.remix, clip.remix.enable, clip.remix.properties, clip.remix.revert | resolve: — (manual music edits) | premiere: Remix (Essential Sound ▸ Duration) | when: a song must end exactly with the video | see: essential-sound.ducking

**[Revolution]**

1. Select the music clip and choose Clip ▸ Remix ▸ **Enable Remix** (`clip.remix.enable`): the
   clip is analysed for its beats.
2. Set the length in **Remix Properties…** (`clip.remix.properties`), where the segment and
   variation sliders choose between cut plans.
   The intro still plays from the start and the song still ends on its real ending; only the
   middle is rearranged.
3. **Revert Remix** (`clip.remix.revert`) brings the clip back as it was.

A remix never pushes or trims the next clip: if the longer version would run into it, the
command refuses. Speed changes don't apply to a remixed clip.
