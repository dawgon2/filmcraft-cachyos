# Finishing: film grain, soft clip, LUTs, scopes

**[Fork]** (the grain kernel is this fork's own) · other finishing tools are coming in G2

## Why

> id: color.finishing.why | does: finishing touches after a grade: grain, highlight soft clip, LUTs and checking with scopes | resolve: Film Grain, Soft Clip, LUTs, scopes | premiere: Lumetri Creative, scopes | when: the grade is done but looks too clean or clips | see: color.finishing.grain

**[Fork]**

Digital footage can look too clean after a heavy grade. A little grain makes shots feel alike and
hides banding in smooth gradients.

## Film grain

> id: color.finishing.grain | does: adds film-like grain on a node | ui: grade.param.grain.* | cmd: grade.setParam | resolve: Film Grain | premiere: Lumetri / Noise effect (closest) | when: shots that look too clean or must match each other | see: color.finishing.soft-clip

**[Fork]**

Grain sits in the **Primaries** panel, in the group labelled "Film grain" below the other
sliders. It belongs to the selected node (the node's box then reads "Grain").

| Label | Id | Range (neutral) | Does |
|---|---|---|---|
| **Grain strength** | {ui:grade.param.grain.strength} | 0 to 1 (0 = off) | how visible |
| **Grain size** | {ui:grade.param.grain.size} | 1 to 4 (1) | cell size in the clip's source pixels |
| **Grain softness** | {ui:grade.param.grain.softness} | 0 to 1 (0.5) | crisp to smooth |

Steps:

1. Select a last node for the look (or add one with **+ Serial** {ui:grade.addNode}).
2. Raise **Grain strength** slowly (try 0.1 to 0.2).
3. Set **Grain size** while viewing at 100 percent.
4. Use the ↺ buttons to go back to neutral.

The grain is monochrome, weighted by brightness, and different on every frame, but the same
frame always gets the same grain, so an export matches the viewer.

## Highlight soft clip

> id: color.finishing.soft-clip | does: rolls bright highlights off smoothly instead of clipping flat | ui: grade.param.hl.* | cmd: grade.setParam | resolve: Soft Clip | premiere: Lumetri Highlights / Whites (closest) | when: skies, windows and lamps that clip | see: color.color-management.output

**[Resolve-style]**

### Why

Bright skies, windows and lamps can hit the top of the signal and clip to a flat white with a
hard edge. A soft clip bends the brightest part of the picture down smoothly, so highlights keep
some shape and roll off gently instead of slamming into white. Use it on a late node, after your
exposure and contrast.

### Steps

1. Select the node and open the **Curves** tab {ui:grade.tab.curves}.
2. Under **Soft clip**, raise **Highlight roll-off** {ui:grade.param.hl.rolloff} (0 is off, 1 is
   the full roll-off).
3. Move **Roll-off knee** {ui:grade.param.hl.knee} to choose where the bending starts, on a
   display-like scale where 1 is diffuse white (the same in every working space): 0.5 starts in
   the upper mid-tones, 0.95 only touches the brightest highlights.

Below the knee nothing changes; above it brightness approaches diffuse white smoothly and, at
full roll-off, never passes it. Parameters for scripts: `hl.rolloff`, `hl.knee` with
`grade.setParam`.

**[Fork]** Unlike Resolve's soft clip there is no separate low (shadow) soft clip and no
per-channel setting; the roll-off acts on each channel with the same curve, so a very saturated
highlight can shift slightly in hue as it rolls off. In an HDR working space it compresses
everything above diffuse white (the specular highlights HDR keeps), so use it for SDR looks. The
sequence's output stage still applies its own BT.2390 tone mapping afterwards
([color-management.md](color-management.md)).

## A LUT on a node

> id: color.finishing.lut | does: applies a LUT file on a node, with a mix amount | ui: grade.lut, grade.param.lut.mix | cmd: grade.setLut, lut.import | resolve: LUT on a node | premiere: Lumetri Input LUT / Creative Look | when: a camera LUT or a look from a file | see: packs.make

**[Resolve-style]**

### Why

A LUT (look-up table) is a ready-made colour transform in a file: a camera manufacturer's
conversion, a film-print emulation or a show look. On a node it sits after the node's own
corrections, and later nodes can still adjust it.

### Steps

1. Select the node; in the **Primaries** tab {ui:grade.tab.primaries} open the **LUT** menu
   {ui:grade.lut} at the bottom.
2. Pick a built-in LUT or one from the project's LUT library, or **Browse…** for a `.cube` /
   `.3dl` file (it is copied into the project, `lut.import`). The command is `grade.setLut`.
3. Lower **LUT mix** {ui:grade.param.lut.mix} to blend the LUT with the node's picture (a LUT
   that looks too strong usually wants 50–80 %). Balance the shot on an earlier node first: a
   LUT on an unbalanced shot exaggerates its cast.
4. Choose **None** to remove it.

**[Fork]** The LUT works on the node's grading signal (DaVinci Intermediate in a DaVinci Wide
Gamut sequence). A LUT built for another input (Rec.709, a camera log) gives the wrong result
there: put a colour space conversion before it, or use it in a Rec.709 sequence. An unknown or
missing LUT applies nothing.

## Scopes: reading exposure, balance and skin

> id: color.finishing.scopes | does: waveform, parade and vectorscope readings for exposure, balance and skin | ui: scopes.*, pageToolbar.LumetriScopes | cmd: scopes.read | resolve: Scopes | premiere: Lumetri Scopes | when: checking a grade objectively | see: color.primaries.panel

### Why

**[Revolution]** (the Lumetri Scopes panel comes from upstream) · reading advice **[Fork]**

Your eyes adapt and your monitor may be off; the scopes don't. They measure the frame under the
playhead, so you can set exposure, check that white and black are neutral and see where skin sits
without guessing. Look at them before and after each correction.

### Which scope for which job

**[Fork]**

| Scope | What it shows | Reach for it to |
|---|---|---|
| **Waveform** | brightness of every column of the picture, left to right; bottom = black, top = white | set exposure: blacks just above the bottom, highlights below the top, faces in the middle |
| **Parade** | the same, split into side-by-side traces (RGB, YUV or RGB-White) | check balance: on something white or grey the red, green and blue traces sit at the same height |
| **Vectorscope YUV** | hue as the angle, saturation as the distance from the centre | check skin (it should lie along the skin-tone line) and spot colour casts (a neutral shot clusters in the centre) |
| **Vectorscope HLS** | the same idea with hue labels around the circle | name a cast by its hue |
| **Histogram** | how many pixels sit at each brightness | see clipping (a pile-up at either end) and overall contrast |

### Steps: set exposure and balance with the scopes

**[Fork]**

1. Open the **Lumetri Scopes** panel: on the Color page click its button in the page toolbar
   {ui:pageToolbar.LumetriScopes}; in the Premiere workflow it is a tab in the Color workspace.
2. Click the wrench {ui:scopes.wrench} (or right-click the scopes) and tick **Waveform** and
   **Parade**; untick the others to give them more room.
3. In the waveform, adjust **Lift** until the darkest part of the trace sits just above the bottom
   line, then **Gain** until the brightest part sits just below the top
   ([primaries.md](primaries.md)).
4. Adjust **Gamma** until faces sit roughly in the middle of the scale.
5. Find something that should be white or grey. In the parade its red, green and blue traces
   should line up; if one is higher, correct it with **Temperature / Tint** or the wheels.
6. Tick **Vectorscope YUV** and check that skin points along the skin-tone line; a shot with no
   strong colours should stay near the centre.
7. Turn on **Clamp Signal** {ui:scopes.clamp} only if you want to hide values outside the legal
   range; leave it off while grading so you can see clipping.

The bottom bar picks the colour space the scopes measure in {ui:scopes.space} (Automatic follows
the sequence: Rec. 2100 for an HDR working space, otherwise Rec. 709) and the signal scale
{ui:scopes.scale} (8 Bit, Float or HDR). Agents read the same numbers with `scopes.read`.

### If you know Resolve or Premiere

**[Resolve-style]**

The scopes are one panel with several scopes side by side, as in Premiere's Lumetri Scopes; Resolve
shows them in its own scope viewer. The readings mean the same: Resolve's "Waveform", "Parade",
"Vectorscope" and "Histogram" map one to one.

## If you know Resolve

> id: color.finishing.resolve | does: how grain, soft clip and LUTs compare with Resolve's | resolve: Film Grain OFX, Soft Clip, LUTs | when: you come from Resolve | see: color.finishing.grain

**[Resolve-style]**

Here grain is a setting of a node.
