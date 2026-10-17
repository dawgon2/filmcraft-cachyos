# Transcripts, captions and narration

**[Revolution]** · editing by text, captions and text-to-speech narration, all in the **Text**
panel (Window ▸ Text: Transcript and Captions tabs) and the Sequence ▸ Transcript / Captions
menus; written for AI-PLAN AI3-PREP (feature pages)

## Transcripts: editing by text

> id: text.transcripts | does: turns a sequence's speech into text you can search and cut by selecting words | cmd: transcript.generate, transcript.inspect, transcript.search, transcript.select, transcript.extract, transcript.lift | resolve: — (Speech to Text in Resolve Studio, closest) | premiere: Text panel ▸ Transcript, text-based editing | when: interviews, talking heads, finding a line in hours of footage | see: text.speech-models, text.cleanup

**[Revolution]**

A transcript is the words spoken in a clip with the time of each word. Revolution keeps it on the
media item, so every clip cut from that file shares it, and shows the sequence's transcript in
the Text panel's **Transcript** tab: the words of the clips on the timeline, in timeline order,
grouped into paragraphs at pauses.

1. Open the sequence and choose Sequence ▸ Transcript ▸ **Transcribe…** (`transcript.generate`).
   Pick the speech model and the language (or let it detect the language); turn on speaker
   detection when several people talk.
2. When it finishes, the Transcript tab fills in. Click a word to move the playhead there.
3. Search for a phrase (`transcript.search`) to jump between the places it was said.
4. Select a run of words (`transcript.select` marks it as In / Out on the timeline) and choose
   **Extract** (`transcript.extract`: removes that part and closes the gap) or **Lift**
   (`transcript.lift`: removes it and leaves the gap).
5. Speakers are named "Speaker 1", "Speaker 2"…; rename one with `transcript.renameSpeaker`.

Every cut is an ordinary timeline edit: one undo step, visible on the timeline, adjustable with
the trim tools afterwards. Agents and scripts can bring their own transcript with
`transcript.set` and read the current one with `transcript.inspect`; `transcript.delete` removes
it.

| Option | Good for | Trade-offs |
|---|---|---|
| **Cut by text** | Interviews and long talks: finding and removing whole sentences fast | Word timing can be a few frames off; check the cut point on the timeline for breaths and room tone |
| **Cut on the timeline** | Music, action, anything where the picture leads | Slower for finding a line in long recordings |
| **Both** | Most dialogue edits: a rough cut by text, then fine trims on the timeline | — |

## Speech models

> id: text.speech-models | does: the Whisper speech models that transcribe, their sizes and how to get them | cmd: transcript.models, transcript.downloadModel | resolve: — | premiere: speech-to-text language packs | when: before the first transcription; transcripts with many errors | see: text.transcripts

**[Standard]** models: OpenAI's Whisper (MIT / Apache-2.0), run on your computer, offline.

Transcription runs locally with a Whisper model; nothing is uploaded. `transcript.models` lists
them with size, licence and source; `transcript.downloadModel` downloads one (a one-time
download, checked before use).

| Model | Good for | Trade-offs |
|---|---|---|
| **whisper-tiny** | Quick drafts of clear speech | Most mistakes on noisy audio and accents |
| **whisper-base** (default) | Most work | Middle of the road |
| **whisper-small** | Hard audio, names, other languages | About four times slower than base |

All three understand many languages. For noisy recordings, clean the voice first (see
[audio.md](audio.md), Voice Isolation): a model hears clean speech better.

## Removing pauses and filler words

> id: text.cleanup | does: removes long pauses and filler words like "um" and "uh" from the timeline in one step | cmd: transcript.removePauses, transcript.removeFillers | resolve: — | premiere: Text panel ▸ Pauses and Filler Words | when: tightening an interview or a talk | see: text.transcripts

**[Revolution]**

1. Transcribe first: both commands work from the sequence's transcript (Sequence ▸ Transcript ▸
   **Transcribe…**, `transcript.generate`). Then **Remove Pauses** (Sequence ▸ Transcript,
   `transcript.removePauses`) cuts silences longer than one second, keeping 0.15 s on each side so
   speech doesn't sound clipped (`minSeconds`, `keepSeconds` change both).
2. **Remove Filler Words** (`transcript.removeFillers`) cuts "um", "uh", "erm", "hmm" and similar;
   pass your own list as `fillers`.
3. Play the result through and put back a pause by hand where a thought ends.

Each is one undo step. The trade-off: a talk with every pause removed can feel breathless. Many
editors remove only the long pauses, then put a few back by hand where a thought ends.

## Captions

> id: text.captions | does: caption tracks: make them from the transcript, import a file, edit, style and export | cmd: transcript.createCaptions, captions.import, captions.export, captions.split, captions.merge, captions.trim, captions.move, captions.setText, captions.setStyle | resolve: subtitle tracks | premiere: Captions (Text panel ▸ Captions, caption tracks) | when: accessibility, social video watched without sound, translations | see: text.transcripts

**[Revolution]**

Captions live on **caption tracks** above the video tracks (C1, C2…), each caption a block of
text with a start and an end.

1. **From the transcript:** Sequence ▸ Transcript ▸ **Create Captions from Transcript…**
   (`transcript.createCaptions`). By default captions hold at most 42 characters per line on two
   lines, last between 1 and 7 seconds and break at pauses of a second; each of these is a
   parameter (`maxChars`, `lines`, `minSeconds`, `maxSeconds`, `gapFrames`).
2. **From a file:** File ▸ Import a `.srt`, `.vtt` or `.scc` file, or `captions.import`: it
   arrives as a new caption track.
3. **Edit** in the Text panel's Captions tab: change the text (`captions.setText`), split
   (`captions.split`) or merge (`captions.merge`) captions, trim their ends (`captions.trim`) or
   move them (`captions.move`); `captions.next` / `captions.previous` step through them.
4. **Style** a track's look with `captions.setStyle`; hide or show tracks with
   `captions.hideAll` / `captions.showAll`.
5. **Export** a sidecar file (File ▸ Export ▸ Captions…, `captions.export`): SubRip (SRT),
   WebVTT, SCC and other formats; or burn the captions into the picture when you export the
   video.

By hand: **Add New Caption Track…** (`captions.newTrack`, Cmd+Alt+A; formats Subtitle, CEA-608,
CEA-708 or Teletext) and **Add Caption at Playhead** (`captions.add`, Cmd+Alt+C, three seconds by
default); set a caption's exact In / Out (`captions.setTimes`); change a track's name, format,
language, lock or visibility (`captions.setTrack`) or show only the active one
(`captions.showActiveOnly`); select, go to and delete captions (`captions.select`,
`captions.goTo`, `captions.delete`) or a whole track (`captions.deleteTrack`).

| Option | Good for | Trade-offs |
|---|---|---|
| **Sidecar file** (SRT / VTT) | YouTube and most platforms: viewers can turn them off, platforms index them | One more file to upload |
| **Burned in** | Social clips that autoplay without sound | Can't be turned off or translated; text size is fixed |
| **From the transcript, then corrected** | Fast captions for long speech | Proofread names and numbers |
| **Typed by hand** | Short pieces, songs, careful timing | Slow for long talks |

## Narration: text to speech

> id: text.narration | does: turns a written script into a spoken narration clip with a chosen voice | cmd: tts.voices, tts.create, tts.edit, tts.inspect, tts.preview, tts.downloadVoices | resolve: — | premiere: — (third-party text-to-speech) | when: placeholder voice-over, explainers, a narrator you can change later | see: text.narration-vs-recording

**[Revolution]**

1. Write the script in the Text to Speech panel (Window ▸ Text to Speech) and pick a voice,
   pitch and pace (`tts.voices` lists them). **Preview** (`tts.preview`) speaks it without
   changing the project.
2. **Create** (`tts.create`) writes `Narration <n>.wav`, imports it and places it at the
   playhead on a free audio track; it never overwrites a clip.
3. To change the words or the voice later, select the clip and edit it (`tts.edit`): the clip
   keeps its place and length (longer speech can be revealed by trimming the clip's end out), and
   the old version stays in the project so Undo is exact.

The built-in voice works without a download. **Natural voices** (Kokoro-82M, nine US English
voices) are a one-time 336 MB download (`tts.downloadVoices`), checked before use.

## Narration or recording a voice-over?

> id: text.narration-vs-recording | does: when to use text-to-speech and when to record a real voice | cmd: tts.create, audio.voiceover.start | resolve: Fairlight voice-over recording | premiere: Voice-over Record | when: deciding how a video gets its voice | see: text.narration, audio.voiceover-takes

**[Revolution]**

| Option | Good for | Trade-offs |
|---|---|---|
| **Text to speech** | Drafts and timing before the real voice exists; explainers; changing a line late without re-recording | Sounds synthetic; little control over emphasis |
| **Record a voice-over** (`audio.voiceover.start`) | The final voice; personality and emphasis | Needs a quiet room and a microphone; re-recording a line takes time (Takes help: [audio.md](audio.md)) |
| **Both** | Cut to a text-to-speech draft, then record the real voice over the same timing | Two passes |

- **Beginner:** start with text to speech to get the timing right, then record your own voice and
  clean it up with Clean Up Voice ([audio.md](audio.md)).
- **Pro:** keep the narration clips as a guide track (muted) while recording, so the recorded
  takes land on the same beats.
