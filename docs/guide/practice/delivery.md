# Practice notes: log footage, HDR and delivery

**[Standard]** · how log footage is commonly shot and handled, and how finished films are
delivered (SDR, HDR, YouTube, vertical, masters), one technique per note, with the Resolve and
Premiere way and the FilmCraft way side by side. Written in our own words from common practice
(AI-PLAN AI3 item 0a, 0c).

## Expose log a little bright

> id: practice.delivery.expose-log | does: exposes log footage brighter than "normal" so shadows stay clean, without clipping highlights | resolve: — (camera practice) | premiere: — (camera practice) | when: shooting F-Log2 or any log profile | see: practice.grading.normalise-log

**[Standard]**

- **Common practice:** expose so skin and the subject sit somewhat above where they would in a
  standard profile (about +1 stop is common for F-Log2), and use zebras / the histogram to keep
  highlights from clipping; bring it back down in the grade.
- **Why:** log's shadows are noisy; more light means less noise after normalising.
- **FilmCraft:** normalise with colour management, then set exposure on the balance node
  ([practice/grading.md](grading.md)); noise reduction if shadows still crawl
  ([color/noise.md](../color/noise.md)).
- **Pitfalls:** clipped highlights can't be brought back; bright is fine, clipped isn't.

## A viewing LUT on set, colour management in the edit

> id: practice.delivery.viewing-lut | does: uses a camera LUT only for monitoring and leaves the files log, then colour-manages in post | resolve: colour-managed project | premiere: log detection / input LUT | when: shooting log | see: color.color-management.input

**[Standard]**

- **FilmCraft:** keep the files in F-Log2; FilmCraft reads the colour space and converts in its
  working space, so no LUT is needed in the edit. A LUT on a node is for looks
  ([color/finishing.md](../color/finishing.md)).
- **Pitfalls:** baking the viewing LUT into the recording throws the extra range away.

## Grade in a wide working space

> id: practice.delivery.working-space | does: grades in a wide-gamut, log-like working space and converts to the delivery space at the end | resolve: DaVinci Wide Gamut / Intermediate, or ACES | premiere: wide-gamut colour management | when: log or HDR footage, mixed cameras | see: color.color-management.working

**[Standard]**

- **FilmCraft:** DaVinci Wide Gamut / Intermediate per sequence (`sequence.colorSettings`); output
  to Rec. 709 with highlight roll-off. ACES isn't offered (not yet).
- **Why:** wheels and curves behave the same on every camera, and highlights survive grading.
- **Pitfalls:** grading in the delivery space (Rec. 709) clips bright colours early.

## ACES

> id: practice.delivery.aces | does: says what to use instead of an ACES pipeline, which FilmCraft doesn't offer | resolve: ACEScct / ACEScc colour science in the project settings | premiere: no ACES working space | when: "can I use ACES?" | see: practice.delivery.working-space

**[Standard]**

- **FilmCraft:** no ACES yet: there is no ACES working space and no ACES input or output
  transforms. Use DaVinci Wide Gamut / Intermediate per sequence (`sequence.colorSettings`)
  instead: like ACEScct it is a wide-gamut, log-like space every camera is converted into.
- **Why:** ACES (the Academy Color Encoding System, a published standard) gives every camera and
  every facility the same working space; for one editor delivering Rec. 709 a wide working space
  does the same job.
- **Pitfalls:** an ACES LUT or look made for ACEScct expects ACEScct numbers; on a node in
  DaVinci Intermediate it looks close but shifts colours.

## SDR from an HDR or wide grade: tone map, don't clip

> id: practice.delivery.tone-map | does: folds highlights into SDR with a tone-mapping roll-off instead of clipping them | resolve: Timeline-to-Output tone mapping | premiere: auto tone mapping | when: delivering Rec. 709 from log or HDR | see: color.color-management.output

**[Standard]**

- **FilmCraft:** **Auto tone mapping** in Sequence Color Management folds HDR into Rec. 709; Highlight
  soft clip on a node for the rest.
- **Pitfalls:** tone mapping plus a strong highlight curve flattens whites twice.

## Deliver 10-bit where you can

> id: practice.delivery.ten-bit | does: exports 10-bit H.265 (Main 10) to avoid banding in skies and gradients | resolve: H.265 Main10 | premiere: HEVC Main 10 | when: YouTube uploads, masters, any gradient-heavy picture | see: export.hdr-main10

**[Standard]**

- **FilmCraft:** H.265 export with the Main 10 profile; HDR sequences use it automatically
  ([export.md](../export.md)).
- **Why:** 8-bit has 256 steps per channel; soft gradients show them after compression.
- **Pitfalls:** some old players can't play 10-bit; keep an 8-bit H.264 copy for them.

## Upload in a higher resolution for YouTube

> id: practice.delivery.youtube-res | does: uploads 4K (or above) even for a 1080p audience, at a generous bitrate | resolve: YouTube preset | premiere: YouTube presets | when: YouTube | see: export.presets

**[Standard]**

- **Common practice:** upload 4K at a high bitrate (or constant quality); YouTube gives higher
  resolutions a better codec and more bits, so even 1080p viewers see a cleaner picture.
- **FilmCraft:** a 4K H.265 preset with constant quality (CRF) or a high bitrate
  ([export.md](../export.md)).
- **Pitfalls:** uploading a heavily compressed file makes YouTube compress compression.

## Vertical cuts from open-gate footage

> id: practice.delivery.vertical | does: reframes 3:2 open-gate footage into a 9:16 vertical version without losing resolution | resolve: timeline resolution per timeline, reframing | premiere: Auto Reframe, a vertical sequence | when: Shorts, Reels, TikTok from the same shoot | see: getting-started.import-xm5, tracking.follow

**[Standard]**

- **FilmCraft:** duplicate the sequence (`edit.duplicate`), set it to 1080 × 1920 or 2160 × 3840
  (`sequence.settings`), and reposition each clip with Motion ▸ Position; the 6240 × 4160 frame
  leaves room. Automatic reframing isn't offered (not yet); tracking can make a crop follow a
  subject ([tracking.md](../tracking.md)).
- **Pitfalls:** titles and lower thirds placed for 16:9 fall outside the vertical frame.

## Keep a master, then make deliveries from it

> id: practice.delivery.master | does: exports one high-quality master and makes compressed deliveries from it or from the timeline | resolve: Deliver page master + presets | premiere: Media Encoder master + presets | when: every finished film | see: export-queue.queue

**[Standard]**

- **FilmCraft:** a high-quality master (H.265 4:2:2 10-bit or ProRes) plus the web versions in
  the export queue (`export.queue.add`) from the same timeline.
- **Why:** re-exporting from the project later needs every file in place; a master doesn't.
- **Pitfalls:** making deliveries by re-compressing a web file.

## Legal levels for broadcast, full range for the web

> id: practice.delivery.levels | does: keeps video levels inside broadcast limits when a broadcaster asks, and uses the normal range elsewhere | resolve: Data Levels, broadcast safe | premiere: Video Limiter | when: broadcast deliveries | see: hardware.jobs

**[Standard]**

- **FilmCraft:** the export's video limiter for broadcast specs; web deliveries don't need it.
- **Pitfalls:** limiting twice (limiter plus a curve) dulls whites.

## Gamma 2.4 or 2.2: grade for where it is watched

> id: practice.delivery.display-gamma | does: chooses the monitoring gamma for the viewing environment | resolve: Rec. 709 Gamma 2.4 / 2.2 output | premiere: display colour management | when: setting up a grading monitor | see: color.color-management.output

**[Standard]**

- **Common practice:** Rec. 709 with gamma 2.4 for a dim grading room and TV delivery; many web
  viewers watch on gamma-2.2-like screens in brighter rooms, so web grades are sometimes checked on
  a typical laptop too.
- **FilmCraft:** output is Rec. 709; check the export on the screens your audience uses.
- **Pitfalls:** "it looked darker on YouTube" is often a monitor or viewing-room difference, not
  the export.

## A thumbnail from the edit

> id: practice.delivery.thumbnail | does: exports a sharp, graded still from the timeline for a thumbnail or poster | resolve: Grab Still / export still | premiere: Export Frame | when: YouTube thumbnails, social posts | see: file-menu.formats

**[Standard]**

- **FilmCraft:** park on the frame and **Export Frame** (`file.exportFrame`) as PNG, 16-bit if it
  will be edited further; the 6.2K original gives a large still.
- **Pitfalls:** exporting a frame while proxies are shown; pause first (Auto mode shows the
  original when paused).

## Captions as a separate file

> id: practice.delivery.captions | does: delivers captions as SRT / VTT next to the video rather than burned in, unless the platform needs burn-in | resolve: subtitle export | premiere: sidecar caption export | when: YouTube, accessibility | see: text.captions

**[Standard]**

- **FilmCraft:** **Export ▸ Captions…** (`captions.export`) as SRT or WebVTT; burn in only for
  autoplaying social clips.
- **Pitfalls:** burned-in captions can't be translated or turned off.

## Check the exported file, not the timeline

> id: practice.delivery.check-export | does: watches the delivered file start to end on a normal player before sending it | resolve: — | premiere: — | when: every delivery | see: practice.audio.small-speakers

**[Standard]**

- **Why:** compression, levels, captions and audio sync can all change on export.
- **FilmCraft:** import the exported file back (File ▸ Import) or open it in your usual player;
  check loudness on the meter.
- **Pitfalls:** checking only the first minute.

## Name and archive the project with its media

> id: practice.delivery.archive | does: archives the project, its media and the master together so the film can be reopened years later | resolve: Export Project Archive (.dra) | premiere: Project Manager collect files | when: when a project is finished | see: projects.export-import, media.relink

**[Standard]**

- **FilmCraft:** export the project (`projects.export`), keep the card copies with their checksum
  lists ([cards.md](../cards.md)) and the master in one folder; Make Offline / Link Media
  reconnect it later ([media.md](../media.md)).
- **Pitfalls:** archiving the project without the original camera files, or only the proxies.
