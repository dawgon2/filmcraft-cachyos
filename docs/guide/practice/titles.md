# Practice notes: titles and motion graphics

**[Standard]** · how editors and designers commonly make titles, lower thirds and simple
animation readable and consistent, one technique per note, with the Revolution way. Written in our
own words from common practice (AI-PLAN AI3 item 0a, 0c).

## Reading time: leave it up long enough

> id: practice.titles.reading-time | does: keeps text on screen long enough to read twice | resolve: — | premiere: — | when: every title, lower third and on-screen text | see: graphics.text

**[Standard]**

- **Common practice:** roughly the time it takes to read the text aloud twice, never under about
  two seconds for a short line.
- **Revolution:** trim the graphic clip; protected intro / outro keep the animation when you change
  its length (`graphics.setResponsiveTime`).
- **Pitfalls:** long sentences on screen: shorten the text instead.

## Contrast for legibility

> id: practice.titles.contrast | does: separates text from the picture with contrast, a shadow, a stroke or a box | resolve: Text+ shading | premiere: Essential Graphics shadow / stroke / background | when: text over busy or bright pictures; white text disappears over a bright sky | see: graphics.text

**[Standard]**

- **Revolution:** a soft shadow or a stroke on the text layer (`graphics.set`), or a shape layer
  behind it pinned to the text (`graphics.pin`).
- **Pitfalls:** thin fonts over moving footage flicker after compression.

## A small type system

> id: practice.titles.type-system | does: uses one or two typefaces and a few fixed sizes throughout a film | resolve: — | premiere: Essential Graphics styles | when: series, branded content | see: practice.titles.templates

**[Standard]**

- **Common practice:** one font for titles, one for body text, three sizes, the same colours.
- **Revolution:** build the set once as graphics templates (`graphics.template.export`) and share
  them in a look pack ([packs.md](../packs.md)).

## Templates for every repeated graphic

> id: practice.titles.templates | does: saves lower thirds, titles and callouts as templates with only the text editable | resolve: Fusion title templates | premiere: Motion Graphics templates | when: anything used more than twice | see: graphics.templates

**[Standard]**

- **Revolution:** **Export As Motion Graphics Template** with the name and role as editable
  properties; apply with `graphics.template.apply`, edit with `graphics.template.set`.
- **Why:** consistent design and much faster edits.

## Lower thirds that fit any name

> id: practice.titles.lower-third | does: a lower third whose box grows with the name and keeps its margins | resolve: Text+ with a background | premiere: Responsive Design ▸ Position | when: interviews | see: graphics.responsive

**[Standard]**

- **Revolution:** box shape pinned to the text layer (`graphics.pin`), left-aligned text
  (`graphics.alignTextLeft`), inside the safe margins.
- **Pitfalls:** names that run off screen; test with the longest name.

## Title-safe margins

> id: practice.titles.safe-margins | does: keeps text inside a margin from the frame edge | resolve: safe area overlay | premiere: safe margins | when: TV delivery, social crops, client monitors | see: practice.youtube.safe-areas

**[Standard]**

- **Common practice:** text sits about 5–10 % from each edge of the frame.
- **Revolution:** to place it there exactly, align to the frame (`graphics.alignFrame.left` …) and
  nudge inwards (`graphics.nudgeRight5`), or use Program monitor guides.

## Ease in and ease out on every move

> id: practice.titles.easing | does: uses eased keyframes so graphics accelerate and slow down naturally | resolve: Fusion spline ease | premiere: Ease In / Ease Out | when: any animated graphic; a title slide-in or move looks robotic or mechanical | see: effects.keyframes

**[Standard]**

- **Revolution:** right-click the keyframes ▸ **Ease In** / **Ease Out** or Bezier
  (`effects.setInterpolation`).
- **Pitfalls:** linear moves look mechanical; very long eases look sluggish.

## Animate in and out the same way

> id: practice.titles.in-out | does: gives a graphic a matching entrance and exit so it feels designed | resolve: — | premiere: — | when: lower thirds, callouts | see: practice.titles.easing

**[Standard]**

- **Revolution:** keyframe Position or Opacity at both ends; protect them with responsive time so
  trims keep the animation.
- **Pitfalls:** a graphic that pops off with no exit after easing in.

## Credits as a roll

> id: practice.titles.credits | does: scrolls credits at a readable, steady speed | resolve: Text+ scroll | premiere: Roll / Crawl | when: end credits | see: graphics.responsive

**[Standard]**

- **Revolution:** a paragraph text layer and **Roll** (`graphics.setRoll`) starting and ending off
  screen; the clip's length sets the speed.
- **Pitfalls:** too fast to read; thin type that shimmers while scrolling.

## Callouts that follow the subject

> id: practice.titles.callouts | does: an arrow or label that stays on a moving object | resolve: Fusion tracker | premiere: — (After Effects) | when: product details, sports, tutorials | see: tracking.follow

**[Standard]**

- **Revolution:** **Attach to Track** on the graphic's Motion (`tracking.attach`), Position only for
  the steadiest result.

## Vertical text and Japanese titles

> id: practice.titles.vertical-text | does: sets vertical text and CJK titles with suitable fonts | resolve: Text+ vertical | premiere: vertical type | when: Japanese or Chinese titles, stylised vertical labels | see: graphics.fonts

**[Standard]**

- **Revolution:** `graphics.newVerticalText`; characters a Latin font lacks fall back to a CJK font
  automatically ([graphics.md](../graphics.md)).

## Kinetic captions, word by word

> id: practice.titles.kinetic-captions | does: highlights the current word in a caption line, a common short-form style | resolve: Text+ animation | premiere: caption styles + graphics | when: Shorts, Reels | see: practice.youtube.vertical-captions

**[Standard]**

- **Revolution:** upgrade the caption to a graphic (`graphics.upgradeCaption`) and colour the active
  word with a per-character style (`graphics.setCharStyle`) keyed to the word timings. Automatic
  word-by-word highlighting isn't offered yet.

## Logos: a source graphic

> id: practice.titles.logo-bug | does: a channel logo or bug that is identical everywhere and changes in one place | resolve: compound clip, Fusion clip | premiere: Upgrade to Source Graphic | when: branded series | see: graphics.source

**[Standard]**

- **Revolution:** **Upgrade to Source Graphic** (`graphics.upgradeToSourceGraphic`) so every use
  updates together.

## Graphics in the right colour space

> id: practice.titles.colour | does: picks title colours that look right after the grade and on Rec. 709 | resolve: graphics after the output transform | premiere: — | when: brand colours, logos | see: color.lumetri.order

**[Standard]**

- **Common practice:** graphics should not go through the creative grade; check brand colours on
  the exported file.
- **Revolution:** keep graphics on tracks above the graded clips; the node grade belongs to each clip
  ([color/lumetri.md](../color/lumetri.md) ▸ where it sits).
- **Pitfalls:** an adjustment layer with a look above the titles tints them too.

## Fewer words on screen

> id: practice.titles.fewer-words | does: shows key words or numbers instead of whole sentences | resolve: — | premiere: — | when: explainers, social video; graphics full of paragraphs or whole sentences | see: practice.titles.reading-time

**[Standard]**

- **Common practice:** one idea per graphic; the voice says the rest.
- **Pitfalls:** viewers reading instead of watching.

## Camera shake and impacts

> id: practice.titles.shake | does: fakes a camera shake (an earthquake, an explosion, a hit) with Camera Shake or keyframed moves | resolve: Transform keyframes or a shake effect in Fusion | premiere: Transform effect keyframes (no built-in shake) | when: impacts, earthquakes, a punch or a beat drop; a locked-off shot that should feel handheld | see: effects.keyframes, effects.apply

**[Standard]**

- **Common practice:** small, fast, irregular moves that die out: a few strong jumps on the hit,
  then smaller ones over half a second to a second. Scale up a little so the frame's edges never
  show, and add motion blur so the jumps don't look like stutter.
- **Revolution:** options:
  1. **Camera Shake** (Effects ▸ Video Effects ▸ Transform, `effects.apply`): smooth random moves
     with **Position Amount**, **Rotation Amount**, **Zoom Amount** (keeps the edges covered),
     **Frequency**, **Complexity**, **Motion Blur** and a **Seed**. Keyframe Position Amount from
     high to 0 for a hit that dies out. Quickest, and every value can change over time.
  2. **Keyframe by hand:** add the **Transform** effect (`effects.apply`), animate **Position** and
     a little **Rotation** with a keyframe every 1–3 frames ([effects.md](../effects.md) ▸
     keyframes), set **Scale** to about 105–110 %, and turn on **Use Effect Shutter Angle** with a
     **Shutter Angle** around 180 for motion blur. Full control, slow to make.
  3. **Save it as a preset** (`effects.presets`) once it looks right and drop it on the next hit;
     quick, but every shake looks the same.
  4. **Write a shader effect** ([shader-effects.md](../shader-effects.md)) that offsets the picture
     by a noise function with an amount control; reusable and keyframable, needs a little WGSL.
  5. **At a cut:** the **Earthquake** transition (Video Transitions ▸ Grunge & Distort; **Shake** in
     pixels, motion blur) shakes the picture across the edit. Quick, but only around a cut.
- **Pitfalls:** shakes that are too big or too long tire the eye; shaking the titles with the
  picture (put titles on a track above the shaken clip); forgetting the scale, so black edges flash.

## Text behind a person

> id: practice.titles.behind | does: puts a title between the background and a person, so they seem to stand in front of the words | resolve: a key or Magic Mask on a copy of the shot above the text | premiere: a masked, tracked copy of the shot above the title | when: big titles behind a presenter, words behind a building or a mountain | see: effects.opacity, tracking.what, keying.ultra-key

**[Standard]**

- **Revolution:** stack three layers: the shot on V1, the title on V2, and a **copy of the shot on
  V3** (Alt-drag or copy / paste). On the V3 copy draw a mask on **Opacity** around the person
  (or the building) so only they show, feather it a little, and if they move, track the mask
  (`masks.track`; [tracking.md](../tracking.md)). The title now shows everywhere except where the
  person is.
- **Clear sky or flat background:** key the background out of the V3 copy instead (**Ultra Key**
  on the sky's colour, [keying.md](../keying.md)); no mask drawing, and hair or leaves edge better.
- **Revolution: no automatic person cut-out (Magic Mask) yet;** for a moving person the tracked,
  hand-adjusted mask is the way, keyframing its shape where tracking drifts.
- **Pitfalls:** fast movement and hair are hard to mask cleanly: choose shots where the person
  moves little; keep the title's top behind the head, not cutting through the face.

## Overlays shot on black: snow, rain, sparks, fire

> id: practice.titles.overlays | does: lays stock footage shot on black (snow, rain, sparks, fire, dust, a muzzle flash) over a shot | resolve: Composite Mode Screen / Add, Fusion particles | premiere: Blend Mode Screen / Linear Dodge (Add) | when: weather, sparks or fire that wasn't there, a gunshot flash in a short film | see: effects.opacity, practice.looks.double-exposure

**[Standard]**

- **Revolution:** put the overlay clip on the track above the shot and set its **Opacity ▸ Blend
  Mode** to **Screen** (black disappears, bright parts add up) or **Linear Dodge (Add)** for a
  hotter look ([effects.md](../effects.md) ▸ Opacity). Scale and place it with **Motion**, lower
  Opacity for a lighter fall, and mask it if it must stay behind something.
- **Selling it:** light from fire or a flash should touch the scene: on the shot's grade, a soft
  window over the lit side with warm Gain, its strength keyframed to flicker with the flames
  ([color/secondaries.md](../color/secondaries.md)); for snow or rain, a touch of cool, low-contrast
  grade on the shot.
- **Revolution: no particle generator yet** (Resolve's Fusion particles, After Effects' particle
  plugins); use overlay footage, or a `.wgsl` shader effect for simple generated patterns
  ([shader-effects.md](../shader-effects.md)).
- **Pitfalls:** an overlay brighter than the scene's light looks pasted on; matching the
  overlay's motion blur and grain to the shot; loops that visibly repeat.

## Handwriting that writes itself on

> id: practice.titles.write-on | does: reveals handwriting or a signature stroke by stroke, as if a pen were writing it | resolve: Fusion paint strokes or a write-on plug-in | premiere: the Write-on effect with keyframed brush positions | when: a signature, handwritten lettering, a map route drawing itself | see: effects.keyframes, effects.opacity

**[Revolution]**

1. Put the finished writing on its own track above the shot (text in a handwriting font, or a
   picture of the real signature on a transparent background).
2. On that clip apply **Video Effects ▸ Obsolete ▸ Write-on** and set **Paint Style** to
   **Reveal Original Image**: the brush now uncovers the writing instead of painting over it.
3. Set **Brush Size** just wider than the pen line, **Stroke Length (secs)** to 0 (the whole trail
   stays) and keep **Brush Spacing** small (0.01 s) so the trail has no gaps.
4. Move the playhead a few frames at a time and keyframe **Brush Position** along each letter in
   the order a hand would write it (`effects.setParam`, `effects.toggleAnimation`). More
   keyframes on curves, fewer on straight strokes.
5. Play it back; where the reveal misses a bit of a letter, add a keyframe there or widen the
   brush slightly.

- **Revolution:** the brush follows the keyframes you set; nothing traces the letters for you, so a
  long sentence is slow work. Keep it short (a signature, a name), or reveal the rest with a soft
  wipe (a feathered **Opacity** mask whose path you keyframe).
- **Pitfalls:** a brush wider than the gap between letters uncovers the next letter early; a
  dotted trail means the spacing is too large for how fast the brush moves.
