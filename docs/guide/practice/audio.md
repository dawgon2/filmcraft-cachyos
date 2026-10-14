# Practice notes: sound

**[Standard]** · how editors and mixers commonly treat dialogue, music and effects, one technique
per note, with the Resolve (Fairlight) and Premiere way and the FilmCraft way side by side.
Written in our own words from common practice (AI-PLAN AI3 item 0a, 0c).

## Dialogue first, then everything else

> id: practice.audio.dialogue-first | does: sets the dialogue level first and builds music and effects around it | resolve: Fairlight: dialogue bus first | premiere: Essential Sound: dialogue first | when: every mix | see: practice.audio.music-under-dialogue

**[Standard]**

- **Goal:** words that are always understood.
- **Common practice:** even out every dialogue clip, then bring music and effects up underneath.
- **FilmCraft:** type dialogue clips and **Auto-Match** them (`essentialSound.autoMatch`), or **Clean
  Up Voice** (`audio.cleanupVoice`); then the music and effects tracks.
- **Pitfalls:** mixing music first and squeezing the voice in afterwards.

## Music under dialogue

> id: practice.audio.music-under-dialogue | does: keeps a music bed well below the voice, dipping further while someone speaks | resolve: sidechain ducking or automation | premiere: Essential Sound Ducking, keyframes | when: narration or interviews over music | see: essential-sound.ducking, mixing.automation

**[Standard]**

- **Goal:** music you feel but don't fight.
- **Common practice:** music roughly 15–20 dB under the voice while words play, up between
  sentences and at the end.
- **FilmCraft:** **Ducking** on the music clip (`essentialSound.generateDucking`), or ride the
  music fader in **Touch** mode ([mixing.md](../mixing.md)).
- **Pitfalls:** pumping (music jumping up in every word gap); slow the fades or lower sensitivity.

## High-pass every voice

> id: practice.audio.high-pass | does: removes rumble below the voice with a high-pass filter around 70–100 Hz | resolve: EQ high-pass band | premiere: Essential Sound Reduce Rumble, Parametric EQ | when: every dialogue clip | see: audio.manual-voice-cleanup

**[Standard]**

- **FilmCraft:** **Reduce Rumble** in Essential Sound (3–4 = 66–78 Hz) or a High Pass effect.
- **Why:** traffic, handling and air conditioning live down there; voices don't.
- **Pitfalls:** too high (150 Hz+) thins deep voices.

## Noise reduction: as little as works

> id: practice.audio.noise-amount | does: removes background noise only until the voice starts to change | resolve: Voice Isolation, Noise Reduction | premiere: Enhance Speech, DeNoise | when: noisy locations | see: audio.voice-isolation

**[Standard]**

- **FilmCraft:** **Voice Isolation** for changing noise, **Reduce Noise** for steady hiss; raise
  the amount while listening to the voice, stop at the first artefact ([audio.md](../audio.md)).
- **Pitfalls:** "watery" or robotic speech from too much; noise reduction on the music or effects
  tracks.

## De-ess after EQ and compression

> id: practice.audio.de-ess | does: tames sharp "s" and "t" sounds with a de-esser | resolve: De-Esser | premiere: DeEsser, Essential Sound DeEss | when: bright voices, close microphones, after a presence boost | see: essential-sound.controls

**[Standard]**

- **FilmCraft:** **DeEss** in Essential Sound (3–6), or the DeEsser effect.
- **Why:** EQ and compression bring sibilance up; de-essing after catches it.
- **Pitfalls:** too much and the speaker lisps.

## Gentle compression on voice

> id: practice.audio.voice-compression | does: evens out the voice's level with a compressor: moderate ratio, medium attack | resolve: Dynamics compressor | premiere: Dynamics Processing, Essential Sound Dynamics | when: speakers who get loud and quiet | see: essential-sound.controls

**[Standard]**

- **Common practice:** around 2:1–4:1, a few dB of gain reduction on loud words, attack slow enough
  to keep consonants.
- **FilmCraft:** **Dynamics** in Essential Sound (2–3 for steady speakers, 4–6 for wild ones) or
  the Dynamics Processing effect.
- **Pitfalls:** heavy compression brings the room noise up between words.

## A limiter on the master

> id: practice.audio.master-limiter | does: holds the mix's peaks under the delivery ceiling with a limiter on the Mix | resolve: Fairlight bus limiter | premiere: Hard Limiter on the Mix track | when: before export | see: mixing.inserts, practice.audio.loudness-targets

**[Standard]**

- **FilmCraft:** a Hard Limiter insert on the **Mix** strip (`mixer.addInsert`) at −1 dB, or each
  voice clip's **Limit True Peak** (Essential Sound ▸ Loudness).
- **Pitfalls:** using the limiter to make things loud squashes them; set levels first.

## Loudness targets by platform

> id: practice.audio.loudness-targets | does: mixes to the integrated loudness each platform expects | resolve: Loudness meter, delivery targets | premiere: Loudness Meter, Loudness Normalization on export | when: before export | see: audio.cleanup-voice, essential-sound.loudness

**[Standard]**

| Delivery | Integrated loudness | Peak |
|---|---|---|
| YouTube and most streaming | about −14 LUFS | −1 dBTP |
| Podcasts | about −16 LUFS (stereo) | −1 dBTP |
| Broadcast (EBU R128 / ATSC A/85) | −23 LUFS / −24 LKFS | −1 / −2 dBTP |

- **FilmCraft:** **Clean Up Voice** matches voice clips to −14 LUFS under −1 dBTP; check the whole
  mix on the loudness meter before export.
- **Pitfalls:** platforms turn loud mixes down, so louder than the target only costs dynamics.

## Crossfades at every audio edit

> id: practice.audio.crossfades | does: puts a short crossfade on audio cuts so they don't click | resolve: audio crossfade | premiere: Constant Power crossfade | when: every dialogue and music edit | see: effects.transitions

**[Standard]**

- **FilmCraft:** **Apply Default Transitions to Selection** (`sequence.applyDefaultTransitionsToSelection`)
  or an audio transition per edit (`sequence.applyAudioTransition`), a few frames long.
- **Pitfalls:** long crossfades on dialogue make words overlap.

## Ambience beds and layered effects

> id: practice.audio.ambience-beds | does: lays a continuous ambience under scenes and builds effects from several layers | resolve: Fairlight tracks per layer | premiere: Essential Sound Ambience / SFX | when: documentary, narrative, travel videos | see: practice.editing.room-tone, essential-sound.types

**[Standard]**

- **FilmCraft:** ambience on its own track typed **Ambience** (Stereo Width, Reverb), effects
  typed **SFX** with Pan; duck ambience under dialogue if needed.
- **Pitfalls:** an ambience change at every picture cut; let beds run across cuts within a scene.

## Keep dialogue in the centre

> id: practice.audio.pan-dialogue | does: keeps voices centred and pans only effects and ambience | resolve: pan | premiere: Panner | when: stereo mixes | see: mixing.track-mixer

**[Standard]**

- **FilmCraft:** leave dialogue pan at 0; pan effects with Essential Sound ▸ Pan or the Panner.
- **Why:** phones and many TVs fold to mono or sit off-axis; centred voices survive.
- **Pitfalls:** a mono microphone recorded on one channel only plays on one side: fix it with
  Audio Channels (`clip.audioChannels`).

## Check on small speakers

> id: practice.audio.small-speakers | does: listens to the mix on a phone or laptop speaker before delivery | resolve: monitoring | premiere: monitoring | when: before export | see: practice.audio.loudness-targets

**[Standard]**

- **Common practice:** a last pass on headphones, studio speakers and a phone.
- **Why:** most viewers hear small speakers that lose bass and mask quiet voices.
- **Pitfalls:** mixing only on headphones makes reverb and stereo too wide.

## Fix plosives and handling noise

> id: practice.audio.plosives | does: reduces "p" pops and bumps with a high-pass or a short fade on the event | resolve: EQ, clip gain on the pop | premiere: High Pass, keyframed volume dip | when: close-mic voice-overs, lavaliers rubbing | see: practice.audio.high-pass

**[Standard]**

- **FilmCraft:** Reduce Rumble or a High Pass removes most pops; for a single bump, a quick Volume
  keyframe dip on the clip (Effect Controls) or cut it out under room tone.
- **Pitfalls:** heavy high-passing to fix one pop thins the whole voice.

## Wind noise

> id: practice.audio.wind | does: removes wind rumble and gusts from outdoor recordings | resolve: Voice Isolation, high-pass | premiere: Enhance Speech, DeNoise, high-pass | when: outdoor interviews, vlogs | see: audio.voice-isolation

**[Standard]**

- **FilmCraft:** Reduce Rumble at 4 plus **Voice Isolation** at 10: on FilmCraft's own test set
  Voice Isolation beat DeNoise on wind by about 4–8 dB of SI-SDR at 0–5 dB signal-to-noise.
- **Pitfalls:** gusts that distort the microphone can't be repaired; a furry windscreen at the
  shoot beats any plugin.

## Headroom while mixing

> id: practice.audio.headroom | does: keeps channel and mix peaks well below 0 dBFS until the final limiter | resolve: meters | premiere: Audio Track Mixer meters | when: throughout the mix | see: practice.audio.master-limiter

**[Standard]**

- **Common practice:** peaks around −6 dBFS while mixing, the limiter and loudness at the end.
- **FilmCraft:** watch the strip meters in the Audio Track Mixer; lower track faders rather than
  limiting.
- **Pitfalls:** a red Mix meter means the export clips.
