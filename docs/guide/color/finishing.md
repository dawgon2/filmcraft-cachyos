# Finishing: film grain, soft clip, LUTs

**[Fork]** (the grain kernel is this fork's own) · other finishing tools are coming in G2

## Why

**[Fork]**

Digital footage can look too clean after a heavy grade. A little grain makes shots feel alike and
hides banding in smooth gradients.

## Film grain

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
3. Lower **LUT mix** {ui:grade.param.lut.mix} to blend the LUT with the node's picture.
4. Choose **None** to remove it.

**[Fork]** The LUT works on the node's grading signal (DaVinci Intermediate in a DaVinci Wide
Gamut sequence). A LUT built for another input (Rec.709, a camera log) gives the wrong result
there: put a colour space conversion before it, or use it in a Rec.709 sequence. An unknown or
missing LUT applies nothing.

## If you know Resolve

**[Resolve-style]**

Here grain is a setting of a node.
