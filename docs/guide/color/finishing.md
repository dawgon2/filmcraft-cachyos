# Finishing: film grain

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

## Coming in G2

**[Fork]**

Noise reduction (spatial, then temporal), a LUT node and a highlight roll-off control. These are
not in the app yet.

## If you know Resolve

**[Resolve-style]**

Here grain is a setting of a node.
