# Export presets, the queue and Quick Export

**[FilmCraft]** · managing export presets, exporting several sequences or ranges one after
another, and one-click Quick Export; modelled on Premiere Pro's Export mode and Media Encoder
queue; written for AI-PLAN AI3-PREP (feature pages). Encoding itself is in
[export.md](export.md).

## Managing export presets

> id: export-queue.presets | does: saves, favourites, deletes, exports and imports export presets | cmd: export.presets.list, export.presets.get, export.presets.save, export.presets.favorite, export.presets.delete, export.presets.export, export.presets.import, export.formats | resolve: Deliver render presets, Save as New Preset | premiere: Preset Manager, Import / Export presets | when: your own delivery settings used again and again; sharing them with a team | see: export.presets

**[FilmCraft]**

Built-in presets come with FilmCraft; your own are saved in your data folder (Preset Manager,
`export.presets.save`). A preset of yours with the same name as a built-in one takes its place.
Mark the ones you use most as favourites (`export.presets.favorite`). Preset files can be
exported and imported (`export.presets.export`, `export.presets.import`) to move them between
computers. `export.formats` lists every format with its options; `export.resolve` shows the
settings a preset plus your changes add up to, without exporting.

## The export queue

> id: export-queue.queue | does: lines up several exports and encodes them one after another in the background | cmd: export.queue.add, export.queue.start, export.queue.stop, export.queue.list, export.queue.cancel, export.queue.retry, export.queue.remove, export.queue.move, export.queue.clear | resolve: Render Queue | premiere: Adobe Media Encoder queue (Send to Media Encoder) | when: a YouTube master, a vertical cut and a review copy of the same edit; several sequences overnight | see: pages.deliver

**[FilmCraft]**

1. On the Export page set the file up as usual and click **Send to Queue**
   (`export.queue.add`) instead of Export. The queue keeps a snapshot of the project as it was, so
   you can keep editing.
2. Add as many as you like: other presets, other sequences (`sequences`), other ranges (whole
   sequence, In to Out, a custom range).
3. **Start** the queue (`export.queue.start`): items encode one after another as background jobs
   (visible in the jobs list); **Stop** (`export.queue.stop`) stops the queue.
4. Reorder waiting items (`export.queue.move`), cancel an item (`export.queue.cancel`),
   retry a failed one (`export.queue.retry`), remove one (`export.queue.remove`) or clear the
   finished ones (`export.queue.clear`).

The queue lives in the session: it is not saved with the project.

| Option | Good for | Trade-offs |
|---|---|---|
| **Export now** | One file you need now | You wait (or work around it) for each one |
| **Queue, then start** | Several deliveries of one edit; overnight renders | The snapshot doesn't include edits made after adding |
| **Quick Export** | A fast review copy | Uses the last preset; fewer choices |

## Quick Export

> id: export-queue.quick | does: exports the active sequence with the last preset used to a file next to the project, in one click | cmd: export.quick | resolve: Quick Export | premiere: Quick Export | when: sending a draft for review | see: export-queue.queue

**[FilmCraft]**

**Quick Export** (`export.quick`) skips the Export page: the active sequence, the last preset you
used (the first time, *Match Source – Adaptive High Bitrate*), a file next to the project.
