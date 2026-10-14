# Practice notes: titles and motion graphics

**[Standard]** · how editors and designers commonly make titles, lower thirds and simple
animation readable and consistent, one technique per note, with the FilmCraft way. Written in our
own words from common practice (AI-PLAN AI3 item 0a, 0c).

## Reading time: leave it up long enough

> id: practice.titles.reading-time | does: keeps text on screen long enough to read twice | resolve: — | premiere: — | when: every title, lower third and on-screen text | see: graphics.text

**[Standard]**

- **Common practice:** roughly the time it takes to read the text aloud twice, never under about
  two seconds for a short line.
- **FilmCraft:** trim the graphic clip; protected intro / outro keep the animation when you change
  its length (`graphics.setResponsiveTime`).
- **Pitfalls:** long sentences on screen: shorten the text instead.

## Contrast for legibility

> id: practice.titles.contrast | does: separates text from the picture with contrast, a shadow, a stroke or a box | resolve: Text+ shading | premiere: Essential Graphics shadow / stroke / background | when: text over busy or bright pictures | see: graphics.text

**[Standard]**

- **FilmCraft:** a soft shadow or a stroke on the text layer (`graphics.set`), or a shape layer
  behind it pinned to the text (`graphics.pin`).
- **Pitfalls:** thin fonts over moving footage flicker after compression.

## A small type system

> id: practice.titles.type-system | does: uses one or two typefaces and a few fixed sizes throughout a film | resolve: — | premiere: Essential Graphics styles | when: series, branded content | see: practice.titles.templates

**[Standard]**

- **Common practice:** one font for titles, one for body text, three sizes, the same colours.
- **FilmCraft:** build the set once as graphics templates (`graphics.template.export`) and share
  them in a look pack ([packs.md](../packs.md)).

## Templates for every repeated graphic

> id: practice.titles.templates | does: saves lower thirds, titles and callouts as templates with only the text editable | resolve: Fusion title templates | premiere: Motion Graphics templates | when: anything used more than twice | see: graphics.templates

**[Standard]**

- **FilmCraft:** **Export As Motion Graphics Template** with the name and role as editable
  properties; apply with `graphics.template.apply`, edit with `graphics.template.set`.
- **Why:** consistent design and much faster edits.

## Lower thirds that fit any name

> id: practice.titles.lower-third | does: a lower third whose box grows with the name and keeps its margins | resolve: Text+ with a background | premiere: Responsive Design ▸ Position | when: interviews | see: graphics.responsive

**[Standard]**

- **FilmCraft:** box shape pinned to the text layer (`graphics.pin`), left-aligned text
  (`graphics.alignTextLeft`), inside the safe margins.
- **Pitfalls:** names that run off screen; test with the longest name.

## Title-safe margins

> id: practice.titles.safe-margins | does: keeps text inside a margin from the frame edge | resolve: safe area overlay | premiere: safe margins | when: TV delivery, social crops, client monitors | see: practice.youtube.safe-areas

**[Standard]**

- **Common practice:** about 5–10 % from each edge for text.
- **FilmCraft:** align to the frame (`graphics.alignFrame.left` …) and nudge inwards
  (`graphics.nudgeRight5`), or Program monitor guides.

## Ease in and ease out on every move

> id: practice.titles.easing | does: uses eased keyframes so graphics accelerate and slow down naturally | resolve: Fusion spline ease | premiere: Ease In / Ease Out | when: any animated graphic | see: effects.keyframes

**[Standard]**

- **FilmCraft:** right-click the keyframes ▸ **Ease In** / **Ease Out** or Bezier
  (`effects.setInterpolation`).
- **Pitfalls:** linear moves look mechanical; very long eases look sluggish.

## Animate in and out the same way

> id: practice.titles.in-out | does: gives a graphic a matching entrance and exit so it feels designed | resolve: — | premiere: — | when: lower thirds, callouts | see: practice.titles.easing

**[Standard]**

- **FilmCraft:** keyframe Position or Opacity at both ends; protect them with responsive time so
  trims keep the animation.
- **Pitfalls:** a graphic that pops off with no exit after easing in.

## Credits as a roll

> id: practice.titles.credits | does: scrolls credits at a readable, steady speed | resolve: Text+ scroll | premiere: Roll / Crawl | when: end credits | see: graphics.responsive

**[Standard]**

- **FilmCraft:** a paragraph text layer and **Roll** (`graphics.setRoll`) starting and ending off
  screen; the clip's length sets the speed.
- **Pitfalls:** too fast to read; thin type that shimmers while scrolling.

## Callouts that follow the subject

> id: practice.titles.callouts | does: an arrow or label that stays on a moving object | resolve: Fusion tracker | premiere: — (After Effects) | when: product details, sports, tutorials | see: tracking.follow

**[Standard]**

- **FilmCraft:** **Attach to Track** on the graphic's Motion (`tracking.attach`), Position only for
  the steadiest result.

## Vertical text and Japanese titles

> id: practice.titles.vertical-text | does: sets vertical text and CJK titles with suitable fonts | resolve: Text+ vertical | premiere: vertical type | when: Japanese or Chinese titles, stylised vertical labels | see: graphics.fonts

**[Standard]**

- **FilmCraft:** `graphics.newVerticalText`; characters a Latin font lacks fall back to a CJK font
  automatically ([graphics.md](../graphics.md)).

## Kinetic captions, word by word

> id: practice.titles.kinetic-captions | does: highlights the current word in a caption line, a common short-form style | resolve: Text+ animation | premiere: caption styles + graphics | when: Shorts, Reels | see: practice.youtube.vertical-captions

**[Standard]**

- **FilmCraft:** upgrade the caption to a graphic (`graphics.upgradeCaption`) and colour the active
  word with a per-character style (`graphics.setCharStyle`) keyed to the word timings. Automatic
  word-by-word highlighting isn't offered yet.

## Logos: a source graphic

> id: practice.titles.logo-bug | does: a channel logo or bug that is identical everywhere and changes in one place | resolve: compound clip, Fusion clip | premiere: Upgrade to Source Graphic | when: branded series | see: graphics.source

**[Standard]**

- **FilmCraft:** **Upgrade to Source Graphic** (`graphics.upgradeToSourceGraphic`) so every use
  updates together.

## Graphics in the right colour space

> id: practice.titles.colour | does: picks title colours that look right after the grade and on Rec. 709 | resolve: graphics after the output transform | premiere: — | when: brand colours, logos | see: color.lumetri.order

**[Standard]**

- **Common practice:** graphics should not go through the creative grade; check brand colours on
  the exported file.
- **FilmCraft:** keep graphics on tracks above the graded clips; the node grade belongs to each clip
  ([color/lumetri.md](../color/lumetri.md) ▸ where it sits).
- **Pitfalls:** an adjustment layer with a look above the titles tints them too.

## Fewer words on screen

> id: practice.titles.fewer-words | does: shows key words or numbers instead of whole sentences | resolve: — | premiere: — | when: explainers, social video | see: practice.titles.reading-time

**[Standard]**

- **Common practice:** one idea per graphic; the voice says the rest.
- **Pitfalls:** viewers reading instead of watching.
