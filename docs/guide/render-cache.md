# The render cache

**[Resolve-style] [Fork]** · how Revolution renders heavy stretches of the timeline in the
background so they play smoothly, what the colours of the render bar mean, and the settings

## Why

> id: render-cache.why | does: renders heavy parts of the timeline ahead so they play smoothly | resolve: Smart render cache | premiere: Render Effects In to Out (preview files) | when: graded 6.2K, effect stacks, many cuts between long-GOP files | see: render-cache.when

**[Resolve-style]**

Some parts of a timeline are too heavy to play live: a graded 6.2K clip in a 4K sequence, a stack
of effects, a cut every few seconds between long-GOP camera files. The render cache renders those
parts into small, easy-to-play files while you are not using the computer for anything else, and
playback then shows the rendered pictures instead of working them out again. It is on by default
(Smart) and needs no setup.

If you know Resolve: this is the same idea as **Smart Cache**, and the timeline's render bar works
like Resolve's red / blue line. Unlike Resolve, there is no per-clip "Render Cache Color Output"
switch; the cache follows what is on the timeline.

If you know Premiere: the cache writes the same kind of files as **Sequence ▸ Render Effects In to
Out** (`sequence.renderEffectsInToOut`), automatically. Rendering by hand still works and fills the
same bar.

## When it renders, and when it waits

> id: render-cache.when | does: renders only while you are idle and stops the moment you play, scrub or export | resolve: Smart render cache idle rendering | premiere: — | when: understanding why the bar fills between edits | see: render-cache.bar

**[Fork]**

The cache never competes with you. It renders only when you have not played, scrubbed or exported
for about a second, and it stops (after the picture it is on) as soon as you press Play, move the
playhead or start an export. This keeps playback smooth: rendering decodes video on the graphics
card's video engine, and doing that while you play made the picture stutter on this machine.

It renders in this order:

1. the stretches within 30 seconds of the playhead, nearest first;
2. then the red stretches (they would drop frames);
3. then the yellow ones (they should play, but with little to spare);
4. then the rest (Smart includes graded clips even when the bar shows nothing).

Proxies (Phase 6.2) are made before cache files when both are waiting.

## What the render bar shows

> id: render-cache.bar | does: the coloured bar under the ruler shows what is cached, waiting or too heavy | resolve: render cache indicator line | premiere: render bar | when: reading the timeline's playback state | see: render-cache.correct

**[Revolution] [Fork]**

The thin bar under the timeline ruler:

| Colour | Meaning |
|---|---|
| none | the clip plays as it is (one ungraded clip without effects, the sequence's size and frame rate) |
| yellow | should play in real time without rendering |
| red | will probably drop frames until it is rendered |
| dim green | the cache is rendering these frames right now |
| green | rendered: playback shows the cached pictures |

Hover over the bar to read what a stretch is.

## How it stays correct

> id: render-cache.correct | does: cached pieces are named after everything in their picture, so a change is never shown stale | when: wondering whether an edit invalidates the cache | see: render-cache.files

**[Fork]**

Each cached file is named after everything that affects its pictures (the clips, their grade and
effects, the source file, the sequence settings). So:

- **Editing** a clip turns its stretch yellow or red again; the cache renders the new version.
- **Undo** brings the old version back, and its cached files are still there: it is green at once.
- **Relinking** a clip to another file is a different picture: it is rendered again.
- **Proxies** don't count: turning proxies on or off keeps the cache (it always renders from the
  original media, never from a proxy).
- **Moving** a stretch (a ripple edit) keeps its files: they are named relative to the stretch, not
  to the timeline position.

## How the files are made

> id: render-cache.files | does: 2-second H.265 10-bit pieces made by the graphics card's encoder | resolve: render cache format | premiere: preview file format | when: disk space, quality of cached pictures | see: render-cache.settings

**[Fork]**

Every stretch between two cuts is cached in pieces of 2 seconds. Each piece is a small H.265
(10-bit 4:2:0) file at the sequence size, made by the graphics card's video encoder, that starts
with a complete picture. At a cut, playback opens the next small file instead of decoding up to a
second of camera footage, and the next piece is opened just before it is needed. Where the video
encoder is not available, pieces are ProRes 422 HQ files instead (larger, same quality).

The files live in the project's preview folder (next to the project, in
`FilmCraft Previews/<project name>/`; for an unsaved project, in the media cache). Deleting them is
always safe: Revolution renders them again.

## Settings and commands

> id: render-cache.settings | does: turns the cache on or off, sets its size, pauses it and clears unused files | ui: settings.mediaCache.renderCache, settings.mediaCache.renderCacheGb, settings.renderCache.* | cmd: cache.*, app.settings.mediaCache, sequence.renderEffectsInToOut | resolve: Render Cache: Smart / User / None | premiere: Render Effects In to Out, Delete Render Files | when: setting the cache up or freeing disk space | see: render-cache.why

**[Fork]**

1. Open **Settings ▸ Media Cache** (`app.settings.mediaCache`) and scroll to **Render Cache**.
2. Choose **Render cache** {ui:settings.mediaCache.renderCache} (`cache.setMode`):
   **Smart** (default: red, yellow and graded stretches), **All** (renders everything: every stretch, even simple clips
   that play as they are) or **Off** (nothing new is cached; files already rendered are still used).
3. Set **Render cache size limit** {ui:settings.mediaCache.renderCacheGb} (GB, 20 by default). Above
   it, Revolution deletes cache files that no sequence of the project uses first, then the ones
   playback used longest ago (down to 90 % of the limit), but never the pieces of the sequence
   you have open. If those alone don't fit, the cache stops and the status line says the size
   limit is reached (`cache.status` reports `limitReached`): raise the limit or press
   **Delete Unused**.
4. Press **OK** {ui:settings.ok} to keep the settings.

Below the settings, a line tells you how many pieces the cache holds and what it is doing. Next to
it:

- **Pause** {ui:settings.renderCache.pause} (`cache.pause`) stops the cache for this session (it
  stops the piece it is on and deletes that unfinished file); the same button then reads
  **Resume** (`cache.resume`).
- **Delete Unused** {ui:settings.renderCache.clearUnused} (`cache.clearUnused`) deletes the cache
  files no sequence of the project uses (for example versions you undid). Undo can no longer bring
  those back instantly; they are rendered again if needed.

For agents and scripts, `cache.status` reports the mode, the number of pieces, their size, the
size limit, how many are queued and how far the piece being rendered is.

## Good to know

> id: render-cache.notes | does: details: old preview files, where the cache lives, what isn't cached | cmd: cache.status | when: something about the cache looks odd | see: render-cache.settings

**[Fork]**

- The first time a project is opened with this version, preview files from older versions that no
  sequence uses are deleted once (they could never be used again).
- Rendering by hand (**Sequence ▸ Render Effects In to Out**, `sequence.renderEffectsInToOut`, and
  the other render commands) runs at once, before anything the cache has queued, and its files are
  used and counted the same way.
- Captions are never part of the cached pictures; they are drawn live on top.
- Two projects with the same name can share one preview folder (for example on one scratch
  disk). Each project only ever deletes the files it made itself; files of the other project, or
  from before this version, are left alone (`cache.status` counts them as `otherFiles`).
- [Fork] Files of no known project (made before the cache kept an index, or listed without a
  project) are never deleted, so they can pile up. If you know they are yours (no other project
  uses this preview folder), take them over:
  1. Open **Settings ▸ Media Cache ▸ Render Cache**. The **Claim Files of No Known Project** button
     (`settings.renderCache.claimFolder`, command `cache.claimFolder`) shows only while there are
     such files (`cache.status` › `unknownFiles`).
  2. Click it. The status line says how many files are now this project's.
  3. Click **Delete Unused** to remove the ones no sequence uses.
  Files another project made stay that project's.
- Replacing a source file in place (exporting it again under the same name) counts as a
  different picture: its stretches are rendered again once the project changes or is reopened.
- Unfinished files left by a crash (`….mov.part`) are deleted the next time the project opens.
