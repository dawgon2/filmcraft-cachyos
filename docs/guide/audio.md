# Audio clean-up

Status: **partial** (AUDIO-PLAN A2, in progress on the Dell lane). Written so far: Voice Isolation
and installing its model. Coming with the next A2 steps: rendering the effect ahead of playback
and one-click voice clean-up.

## Voice Isolation

**[Fork]** effect, **[Standard]** model: DeepFilterNet3 (Schröter et al., open source, MIT /
Apache-2.0), run on the CPU in pure Rust.

### What it does

Voice Isolation keeps speech and removes everything else: fans, traffic, keyboard clicks, wind,
room hum. It is a trained neural network, so it handles noise that changes from moment to moment,
which a classic noise reducer can't follow.

It lives with the other restoration effects: Effects panel ▸ **Audio Effects ▸ Noise
Reduction/Restoration ▸ Voice Isolation**. Drop it on a clip like any audio effect.

- **Amount** (0–100 %, default 100 %): how much of the cleaned voice you hear. 100 % is fully
  cleaned; lower values mix the original back in, which keeps some room sound and hides small
  artefacts. Keyframe it like any effect parameter.
- **Delay:** the effect looks 20 ms ahead and works in 10 ms steps, 40 ms in all; playback and
  export compensate for it, so the clip stays in sync.

### Live playback and its cost

**[Fork]**

Voice Isolation runs live while you play, scrub to a point, shuttle or export: there is no
render step. A stereo clip costs about one channel's worth of work (the model listens to the mix
of both channels and cleans each with the same mask, so the stereo image stays). Measured on a
small desktop CPU (Intel i5-13500T, its slower cores): about a third of one core per track with
Voice Isolation. One such track plays with no dropouts through seeks, 2× shuttles and audio
device changes; with four, the first 2× shuttle can drop sound for about 0.2 s while it starts.

- **Beginner:** use it on the voice track(s) only, not on music or effects tracks.
- **Pro:** with many tracks using it on a slower computer, avoid fast shuttling during playback,
  or bounce the cleaned dialogue to a new clip once it sounds right.

### Installing the model

**[Fork]**

The model (about 8 MB, licence MIT OR Apache-2.0) is never shipped with the app. To install it:
**Settings ▸ Audio ▸ Voice isolation ▸ Download** ({ui:settings.audio.voiceIsolation.download}).
The download comes from the DeepFilterNet project's own repository, pinned to one release and
checked against its checksum before it is used; a progress bar and **Cancel download** show while
it runs. **Remove** ({ui:settings.audio.voiceIsolation.remove}) deletes it again. The same row
says whether the model is installed and loaded.

Without the model, Voice Isolation quietly works as **DeNoise** (Amount 100 % = 40 dB of
reduction), so a project that uses it still plays and exports everywhere, also on a computer
without the model. The same happens for sequences whose audio isn't at 48 kHz (the model's rate;
other rates are planned).

Offline computers: copy `DeepFilterNet3.zip` from the DeepFilterNet repository (or the
`model_120.ckpt.best` checkpoint inside it) and install it with the command
`audio.voiceIsolate.installFile` (`path`); a zip with the wrong checksum or a file that isn't the
DeepFilterNet3 checkpoint is refused.

Commands (also for the control channel and agents): `audio.voiceIsolate.status`, `.download`,
`.cancelDownload`, `.installFile`, `.remove`.

### Voice Isolation, DeNoise or Essential Sound? (options and trade-offs)

There is no single right tool; pick by the noise and how much time you have.

| Option | Good for | Trade-offs |
|---|---|---|
| **Voice Isolation** | Speech over noise that changes: traffic, keyboard, wind, crowd, music bleed | Speech only (it removes music and effects too); under a tenth of one CPU core per channel; needs the model |
| **DeNoise** | Steady noise: fan, air conditioning, hiss | Can't follow noise that changes; strong settings make "watery" artefacts |
| **Essential Sound ▸ Dialogue ▸ Repair** | A quick start for dialogue with several fixes in one panel (noise, rumble, hum, sibilance, reverb) | Its noise reduction is the classic kind (like DeNoise) |

- **Beginner:** put Voice Isolation on the voice clip at 100 % and listen. If the voice sounds a
  little processed, lower Amount to 70–85 %.
- **Pro:** keep restoration first in the effect stack (Voice Isolation, then EQ, de-esser,
  compression, loudness), so later effects work on clean speech. For a voice recorded in a quiet
  room, try DeNoise first: it changes the voice least. For music or sound effects, don't use Voice
  Isolation at all.

### Why it works this way

Classic noise reduction learns the noise floor and subtracts it, which works only while the noise
stays the same. DeepFilterNet was trained on thousands of hours of speech and noise, so it
recognises speech directly: it sets a gain for 32 frequency bands every 10 ms and, below 4.8 kHz,
rebuilds the voice's fine detail from the last few moments of sound ("deep filtering"). FilmCraft's
port gives the same output as the original to within 16-bit precision.
