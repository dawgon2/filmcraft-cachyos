# Mixing audio

**[FilmCraft]** · the Audio Track Mixer and the Audio Clip Mixer: levels, pan, track effects,
submixes and sends, and automation recorded while you play; modelled on Premiere Pro's mixers;
written for AI-PLAN AI3-PREP (feature pages)

## The Audio Track Mixer

> id: mixing.track-mixer | does: one strip per audio track, submix and the master Mix: fader, pan, mute, solo, record arm | cmd: mixer.inspect, mixer.setStrip, mixer.setValue | resolve: Fairlight mixer | premiere: Audio Track Mixer | when: balancing whole tracks rather than single clips | see: mixing.clip-or-track, mixing.automation

**[FilmCraft]**

Open Window ▸ **Audio Track Mixer** (on the Fairlight page in the Resolve workflow). Each audio
track (A1, A2…) has a strip, each submix (S1, S2…) has one, and the master **Mix** has the last
one. A strip has a fader (volume in dB), a pan knob, **Mute**, **Solo**, **Record arm** (for
voice-over recording) and its automation mode. `mixer.setStrip` sets these from a script
(`volumeDb`, `pan`, `muted`, `solo`, `mode`, its `output` and how its input channels are used);
`mixer.inspect` lists every strip.

## Clip volume or track volume?

> id: mixing.clip-or-track | does: when to set a level on the clip and when on the track | cmd: clipMixer.set, mixer.setValue | resolve: clip gain vs track fader | premiere: Audio Clip Mixer vs Audio Track Mixer | when: deciding where a level change belongs | see: mixing.track-mixer, mixing.clip-mixer

**[FilmCraft]**

| Option | Good for | Trade-offs |
|---|---|---|
| **Clip level** (Effect Controls ▸ Volume, the Audio Clip Mixer, Essential Sound's Clip Volume) | Evening out one clip against its neighbours; moves with the clip when you edit | Many clips to touch when a whole track is too loud |
| **Track level** (the strip's fader and its automation) | A whole layer: all the music, all the dialogue | Track automation stays in place on the timeline when clips move |
| **Both** | Most mixes: clips even with each other, tracks balanced against each other | Two places to look when something is too loud |

- **Beginner:** set clip levels so each track sounds even, then use the track faders for the
  balance between dialogue, music and effects.
- **Pro:** keep clip levels for corrections and track automation for the mix moves (music dips,
  swells), so re-cutting the picture doesn't undo the mix.

## Track effects (inserts)

> id: mixing.inserts | does: effects on a whole track, submix or the Mix, before or after its fader | cmd: mixer.addInsert, mixer.setInsert, mixer.removeInsert | resolve: Fairlight track effects | premiere: Audio Track Mixer effect slots | when: one EQ or compressor for a whole track; a limiter on the Mix | see: mixing.submixes

**[FilmCraft]**

Each strip has effect slots. `mixer.addInsert` puts an audio effect in a slot (`effect`, `slot`,
`postFader`), `mixer.setInsert` turns it on or off, moves it after the fader or sets its
parameters, `mixer.removeInsert` takes it out. Pre-fader effects see the track before its volume;
post-fader effects see it after.

| Option | Good for | Trade-offs |
|---|---|---|
| **Clip effects** (Effect Controls, Essential Sound) | Fixing one recording: noise, hum, a harsh voice | One instance per clip |
| **Track inserts** | The same treatment for everything on a track; a limiter on the Mix for delivery | Applies to every clip on the track, even ones that don't need it |

## Submixes and sends

> id: mixing.submixes | does: groups tracks into a submix and sends part of a track to a submix | cmd: mixer.addSubmix, mixer.deleteSubmix, mixer.addSend, mixer.setSend, mixer.removeSend, mixer.setStrip | resolve: Fairlight buses | premiere: submix tracks and sends | when: one fader for all dialogue; one reverb shared by several tracks | see: mixing.inserts

**[FilmCraft]**

A **submix** is a strip without clips that takes the sound of the tracks routed to it.

1. **Add Audio Submix Track** (Sequence menu, `mixer.addSubmix`): S1 appears in the mixer.
2. Route tracks to it: set a strip's **output** to S1 (`mixer.setStrip` with `output`). Its
   sound now goes through S1, then to the Mix. A submix can feed a later submix, never an earlier
   one (no feedback loops).
3. Or keep the track on the Mix and add a **send** (`mixer.addSend`): a copy of the track at the
   send's level goes to the submix as well (`mixer.setSend` sets the level, `mixer.removeSend`
   removes it). Put a reverb insert on the submix and send several tracks to it for one shared
   room.

## Automation: recording mixer moves while playing

> id: mixing.automation | does: records fader, pan, mute, send and effect moves during playback as track keyframes | cmd: mixer.recordStart, mixer.recordStop, mixer.touch, mixer.release, mixer.setStrip | resolve: Fairlight automation (Latch, Touch, Write) | premiere: track automation modes | when: riding music under dialogue, fades and swells by ear | see: mixing.keyframes

**[FilmCraft]**

Set a strip's automation mode, play, and move its controls: the moves are heard at once and
written as keyframes when playback stops (one undo step).

| Mode | Records | When you let go |
|---|---|---|
| **Off** | nothing, and ignores existing automation | — |
| **Read** (default) | nothing; plays the automation | the control follows the automation |
| **Latch** | from the first touch | keeps writing the last value until playback stops |
| **Touch** | only while you hold the control | glides back to the existing automation |
| **Write** | every control, from the start of playback | keeps writing the last value until playback stops |

After a Write pass the strip switches to Touch (Settings ▸ Audio ▸ "Switch to Touch after
Write"), so the next pass doesn't overwrite everything again. Recorded moves are thinned into a
few keyframes (Settings ▸ Audio sets how much).

- **Beginner:** use **Touch**: you only change what you touch.
- **Pro:** **Latch** for moves that should hold (a music level that stays down after the
  interview starts); **Write** for a first full pass over a scene.

## Editing track keyframes

> id: mixing.keyframes | does: adds, moves and deletes track automation keyframes, or writes a whole curve | cmd: mixer.setKeyframe, mixer.moveKeyframe, mixer.deleteKeyframe, mixer.clearLane, mixer.writeAutomation | resolve: Fairlight automation editing | premiere: track keyframes in the timeline | when: fixing a recorded move; drawing a fade exactly | see: mixing.automation

**[FilmCraft]**

Every automated control is a **lane** of its strip: `volume`, `pan`, `mute`,
`send.<n>.level` and `fx.<slot>.<parameter>`. `mixer.setKeyframe` adds or changes a keyframe at a
time, `mixer.moveKeyframe` moves one, `mixer.deleteKeyframe` removes one, `mixer.clearLane`
removes a lane's keyframes, and `mixer.writeAutomation` writes a list of (time, value) points in
one step (for agents and scripts).

## The Audio Clip Mixer

> id: mixing.clip-mixer | does: a fader and pan per track that write the clip's own Volume and Panner keyframes | cmd: clipMixer.set, clipMixer.setMode, clipMixer.touch, clipMixer.release | resolve: clip gain and clip keyframes | premiere: Audio Clip Mixer | when: riding one clip's level by ear; the level should move with the clip | see: mixing.clip-or-track

**[FilmCraft]**

The Audio Clip Mixer looks like the track mixer, but its controls belong to the **clip under the
playhead** on each track. With the same Off / Read / Latch / Touch / Write modes
(`clipMixer.setMode`), a pass writes keyframes on the clips' **Volume** and **Panner** effects, in
the clips' own time, so they travel with the clips when you edit. `clipMixer.set` sets one clip's
level or pan directly.
