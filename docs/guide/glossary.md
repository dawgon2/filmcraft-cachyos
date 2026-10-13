# Glossary

**[Standard] [Resolve-style] [FilmCraft] [Fork]** · plain-language meanings of the words this
guide uses, with what Resolve and Premiere call the same thing

Each entry says what the word means, why it matters to you, and where to find it in this fork.
The "Resolve" and "Premiere" columns give the nearest name in each program, so a habit from
either transfers; "—" means that program has no direct equivalent. Every entry carries a tag
saying where the idea comes from (see [README](README.md#where-each-feature-comes-from-provenance)).

## At a glance

| This guide | Resolve calls it | Premiere calls it |
|---|---|---|
| log footage | log / camera log | log footage |
| gamut | gamut / colour space | colour space |
| working space | timeline colour space | working colour space (sequence) |
| node | node (corrector node) | — (closest: one Lumetri Color effect) |
| serial node | serial node | — (closest: stacking effects) |
| wheel | colour wheel | colour wheel (Lumetri) |
| curve | curve | curve (Lumetri RGB / Hue Saturation curves) |
| still | still (gallery) | — (closest: comparison view reference frame) |
| wipe | wipe / split screen | comparison view (split) |
| key | key / qualifier | HSL Secondary key |
| matte | matte / key output | mask (track matte for layers) |
| LUT | LUT | LUT (Lumetri Input / Creative LUT) |
| grain | film grain | grain (noise effect) |
| proxy | proxy media / optimized media | proxy |
| render cache | render cache | render files / preview files |
| timeline (Resolve workflow wording; see [getting-started](getting-started.md#terminology-follows-the-workflow)) | timeline | sequence |

## Log

**[Standard]**

A way a camera stores brightness so that very dark and very bright parts both fit in the file.
It squeezes highlights and shadows together, so straight out of the camera a log clip looks
grey and flat. That is expected: it is raw material, not a finished picture. FilmCraft turns it
back into a normal-looking picture once it knows which log curve the camera used (for the
X-M5, **F-Log2**). See [colour management](color/color-management.md).

- Resolve: log, or the camera's name for it. Premiere: log footage.

## Gamut

**[Standard]**

The range of colours a signal can describe. A camera gamut such as **F-Gamut** holds more
saturated colours than a normal screen (**Rec. 709**) can show. When a wide gamut is shown on a
smaller one, the colours outside have to be brought inside somehow; this fork folds them in
gently instead of cutting them off.

- Resolve: gamut / colour space. Premiere: colour space.

## Working space

**[Standard] [Fork]**

The one colour space every clip is translated into before you grade, so the same slider move
does the same thing on every clip. This fork's recommended working space is **DaVinci Wide
Gamut / DaVinci Intermediate** (from Blackmagic's published specification). You choose it per
sequence in Sequence ▸ **Color Management…** (`sequence.colorSettings`). At the end the working
space is converted to the output (normally Rec. 709).

- Resolve: timeline colour space (with colour management on). Premiere: the sequence's working
  colour space.

## Node

**[Resolve-style]**

One step of a clip's grade. A node holds a set of corrections (wheels, curves, a key, a window,
noise reduction, a LUT, grain) and can be switched off on its own. The idea is **one job per
node**, so you can see and undo each step separately. Nodes appear as numbered boxes in the
**Nodes** panel; **+ Serial** {ui:grade.addNode} (`grade.addNode`) adds one. See
[nodes](color/nodes.md).

- Resolve: node (corrector node). Premiere: no direct equivalent; the nearest is one Lumetri
  Color effect on a clip.

## Serial node

**[Resolve-style]**

A node that takes the picture from the node before it and hands its result to the next one:
node 1 → node 2 → node 3. Order matters, because each node sees what the earlier ones already
did. Today every node in this fork is serial; parallel and layer nodes are planned.

- Resolve: serial node. Premiere: no direct equivalent; stacking several effects on a clip
  behaves similarly.

## Wheel

**[Resolve-style] [FilmCraft]**

A round control that pushes the picture's colour towards a hue (drag the dot) and its
brightness up or down (the slider under it). There are four: **Lift** (shadows), **Gamma**
(mid-tones), **Gain** (highlights) and **Offset** (everything at once). See
[primaries](color/primaries.md).

- Resolve: colour wheels (Lift / Gamma / Gain / Offset). Premiere: Lumetri's colour wheels
  (Shadows / Midtones / Highlights).

## Curve

**[Resolve-style] [FilmCraft]**

A graph that maps input to output: drag a point up and that brightness (or hue, or saturation)
comes out higher. The custom curve shapes contrast; the hue and saturation curves change one
colour range at a time ("make only the blues less saturated"). See [curves](color/curves.md).

- Resolve: curves (custom, hue vs hue, hue vs sat, …). Premiere: Lumetri RGB curves and Hue
  Saturation curves.

## Still

**[Resolve-style]**

A saved frame plus the grade that was on its clip when you saved it. You keep stills as
references: to compare a new shot against, or to give the same grade to other clips. **Grab
Still** {ui:stills.grab} (`stills.grab`) saves a still of the shot under the playhead into the
gallery. See
[matching](color/matching.md).

- Resolve: still (in the gallery). Premiere: no direct equivalent; the comparison view's
  reference frame is the nearest.

## Wipe

**[Resolve-style]**

A way to see a still and the current picture at once, divided by a line you can drag, so you
can judge whether two shots match. **Split** shows them side by side instead. Both are set with
`compare.set`.

- Resolve: wipe / split screen. Premiere: comparison view (split mode).

## Key

**[Standard] [Resolve-style]**

A selection of part of the picture made **by its colour or brightness**, for example "the skin"
or "the sky", so a node changes only that part. This fork makes keys with the **qualifier**
(the Qualifier tab {ui:grade.tab.qualifier}). See [secondaries](color/secondaries.md).

- Resolve: key, made with a qualifier. Premiere: HSL Secondary key in Lumetri.

## Matte

**[Standard]**

The black-and-white picture a key or window produces: white where the node acts, black where
it doesn't, grey for partly. Looking at the matte (Qualifier ▸ View ▸ **Matte**
{ui:grade.qual.show.2}) is the quickest way to see whether a selection is clean.

- Resolve: matte / key output (highlight mode). Premiere: the HSL Secondary "mask" view; for
  layers, a track matte.

## Window

**[Resolve-style]**

A selection of part of the picture made **by shape** (circle, rectangle, gradient) instead of by
colour. Windows can follow a moving object with tracking. The Windows tab is
{ui:grade.tab.windows}. See [secondaries](color/secondaries.md#windows-select-by-shape).

- Resolve: window (power window). Premiere: mask on an effect.

## LUT

**[Standard]**

A look-up table: a file (`.cube` or `.3dl`) that maps every input colour to an output colour.
Used to convert between colour spaces or to apply a ready-made look. A LUT can't adapt to the
shot, so it is best used as one step in a node, not as the whole grade. **Import LUT…**
(`lut.import`) adds one to the project; the **LUT** menu {ui:grade.lut} puts it on a node. See
[finishing](color/finishing.md#a-lut-on-a-node).

- Resolve: LUT. Premiere: Lumetri's Input LUT and Creative Look.

## Grain

**[Standard] [Fork]**

Fine random texture added on purpose, like the grain of photographic film. It hides banding,
makes a heavily graded shot feel less "digital" and ties shots from different cameras together.
Set on a node with **Grain strength** {ui:grade.param.grain.strength}. See
[finishing](color/finishing.md#film-grain).

- Resolve: film grain. Premiere: no built-in film grain in Lumetri; a noise effect is the
  nearest.

## Proxy

**[FilmCraft]**

A smaller, easier-to-decode copy of a clip used while editing, so playback stays smooth; the
export still uses the original. Clip ▸ Proxy ▸ **Create Proxies…** (`media.createProxies`)
makes them and View ▸ **Toggle Proxies** (`media.toggleProxies`) switches between proxies and
originals. Automatic proxies for 6.2K and 4:2:2 clips are planned for Phase 6.

- Resolve: proxy media / optimized media. Premiere: proxy.

## Render cache

**[FilmCraft]**

Finished frames of part of a sequence saved to disk, so a heavy section (many effects, a big
grade) plays back without being worked out again every time. Sequence ▸ **Render In to Out**
(`sequence.renderInToOut`) fills it for the marked range and **Delete Render Files**
(`sequence.deleteRenderFiles`) throws it away. A smarter automatic cache is planned for Phase 6.

- Resolve: render cache. Premiere: rendering previews (render files).
