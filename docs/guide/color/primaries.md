# Primaries

**[Resolve-style]** · the broad corrections of a node: lift, gamma, gain, offset, contrast, saturation, white balance

## Why

> id: color.primaries.why | does: why primaries come first: they change the whole picture or whole brightness ranges | resolve: Primaries | premiere: Lumetri Basic Correction, Color Wheels | when: starting a grade | see: color.primaries.panel

**[Resolve-style]**

Primaries change the whole picture, or whole brightness ranges of it. Do these whole-picture
fixes first, before the skin or any one colour; fine work on
one colour (secondaries, G2) comes after.

## The panel

> id: color.primaries.panel | does: Lift, Gamma, Gain and Offset wheels, contrast, pivot, saturation, temperature and tint | ui: grade.wheel.*, grade.param.*, grade.tab.primaries | cmd: grade.setParam | resolve: Primaries Color Wheels | premiere: Lumetri Color Wheels | when: balancing and shaping a shot; making it black and white | see: color.primaries.white-balance

**[Resolve-style]**

Select a clip and a node. The **Primaries** panel has two tabs: **Primaries**
{ui:grade.tab.primaries} and **Curves** {ui:grade.tab.curves}. At the right of the tab row it
names the node you edit ("Node 02 · name"). With no grade it shows **Add Node**
{ui:grade.addFirst}.

### The four wheels

Each wheel has a puck (colour balance) and a ring (brightness of that range, the "master"):

| Wheel | Id | Moves | Use it to |
|---|---|---|---|
| **Lift** | {ui:grade.wheel.lift} | darkest tones | set the black level, tint shadows |
| **Gamma** | {ui:grade.wheel.gamma} | midtones | brighten or darken the middle, tint it |
| **Gain** | {ui:grade.wheel.gain} | brightest tones | set the white level, tint highlights |
| **Offset** | {ui:grade.wheel.offset} | everything evenly | overall brightness or colour cast |

The palette is laid out like Resolve's Primaries: a strip of numbers along the top, the four
wheels, and a second strip under them. Each wheel has a reset (↺) beside its title, id like
{ui:grade.wheel.lift.reset}. Under each wheel four boxes show the master (Y, white band), red,
green and blue offsets; drag a box sideways or click it and type. Under the boxes the ridged
**jog bar** is the master, the brightness of that range (id like {ui:grade.wheel.gamma.master}):
drag it left or right, double-click it to return to 0. Drag the puck toward a colour to add it;
double-click the wheel to centre the puck. Neutral is all zeros.

There is no single right order. Many colourists balance with Offset first, then shape with Lift /
Gamma / Gain; others set the black and white points with Lift and Gain first and balance after.
Try both on the same shot and keep the one that gets you there faster.

### The number strips

| Label | Id | Range (neutral) | Does |
|---|---|---|---|
| **Temp** | {ui:grade.param.temperature} | -100 to 100 (0) | cool to warm (blue → orange band) |
| **Tint** | {ui:grade.param.tint} | -100 to 100 (0) | green to magenta |
| **Contrast** | {ui:grade.param.contrast} | 0 to 4 (1) | stretches brightness around the pivot |
| **Pivot** | {ui:grade.param.pivot} | 0 to 1 (0.336, mid grey) | the brightness contrast leaves alone |
| **Saturation** | {ui:grade.param.saturation} | 0 to 4 (1) | 0 is black and white |
| **Grain**, **Grain size**, **Grain softness** | {ui:grade.param.grain.strength} | see [finishing.md](finishing.md#film-grain) | film grain |

Drag a box sideways or type a value. **Double-click a label** to reset that control (the label's
id is like {ui:grade.param.contrast.reset}). The node's **Reset** {ui:grade.resetNode} returns
every control of the node to neutral. The node's LUT menu sits under the bottom strip.

## White balance picker and Auto balance

> id: color.primaries.white-balance | does: removes a colour cast by picking something neutral, or automatically | ui: grade.wb.pick, grade.wb.auto | cmd: grade.wb.pick, grade.wb.auto | resolve: Highlight / white balance picker, Auto Balance | premiere: Lumetri WB Selector | when: orange, blue, green or magenta casts; nothing grey or white to pick, balance it by itself | see: color.primaries.panel

**[Resolve-style]**

**Why:** a colour cast (orange tungsten light, blue shade, a green fluorescent tube, a magenta
LED panel) is easiest to see on something that *should* have no colour: a grey card, a white wall,
a shirt, a road sign's white. Finding the right Temperature and Tint by eye takes a while and two
sliders interact; the picker solves both at once from that one neutral spot.

1. Select the clip and the node to balance (usually the first one), open **Primaries**.
2. Click the pen icon at the start of the top strip, **WB picker** {ui:grade.wb.pick}. The
   Program monitor shows a crosshair.
3. Click on something that should be grey or white, or drag a box over a larger neutral area
   (a box averages out noise and texture). Esc cancels.
4. **Temperature** and **Tint** move so that spot comes out neutral; the status line shows the
   values. The sliders stay adjustable: warm it back up a little by hand if the scene should
   feel warm.
5. No obvious neutral? Click the round **A** before it, **Auto balance** {ui:grade.wb.auto}: it averages the near-grey,
   mid-tone parts of the whole frame (skipping clipped highlights, deep shadows and strongly
   coloured areas) and balances on that.

Each pick or Auto balance is one undo step. The picker reads the picture **as it enters this
node**, so picking the same spot again gives the same values instead of over-correcting. It
refuses, and says why, a spot that is clipped (its colour is lost), too dark (noise decides) or
too colourful to be neutral. "Clipped" is judged on the clip's own footage, before any node or
Input LUT: a channel that reached the top of the camera's signal (code value 1.0 of SDR, HLG or
log footage such as F-Log2, or the content peak of PQ footage) counts as blown out in every
sequence, including DaVinci Wide Gamut and HDR ones, where values above 1.0 are otherwise real
highlights. When a cast is stronger than the sliders reach (±100), they stop at
the limit and the status line says the cast is only partly removed: balance the rest with the
**Gain** or **Offset** wheel. Keyframed Temperature / Tint get a keyframe at the playhead.

Difference from Resolve: Revolution's picker and Auto balance only move this node's Temperature
and Tint (two controls in linear light, see the sliders above); Resolve's tools may spread the
correction differently, so the numbers won't match between the two applications.

## Steps: fix a flat F-Log2 clip

> id: color.primaries.flog2-steps | does: steps to turn a flat F-Log2 clip into a balanced picture | ui: grade.addFirst, grade.param.contrast, grade.param.pivot | cmd: grade.setParam | resolve: first primary correction on log | premiere: Lumetri Basic Correction on log | when: the first node of an X-M5 clip | see: color.color-management.input

**[Fork]**

1. Confirm the clip is interpreted as F-Log2 ([color-management.md](color-management.md)).
2. Select the clip, add a node, open **Primaries**.
3. Drag the **Gain** jog bar right until highlights sit where you want, the **Lift** jog bar until blacks
   are black.
4. Raise **Contrast** in small steps; keep **Pivot** at 0.336 unless faces get too dark.
5. Raise **Saturation** until colours look right.
6. Fix a colour cast with **WB picker** on something neutral (or **Auto balance**), then adjust
   **Temperature** / **Tint** by taste.
7. Check with the **Lumetri Scopes** panel, then press **Bypass** {ui:grade.bypass} to compare.

Dragging a control is one undo step no matter how long you drag.

## Commands

> id: color.primaries.commands | does: the grade commands for scripts and agents | cmd: grade.setParam, grade.params, grade.get, grade.toggleAnimation | when: scripting a grade | see: color.primaries.panel

**[Fork]**

From a script or an agent, `grade.setParam` sets one parameter of a node (`param`, `value`; a
node's contrast is `contrast`); `grade.params` lists every parameter with
its range; `grade.get` returns the whole grade. `grade.toggleAnimation` turns keyframes on for a
parameter at a time; the panel has no keyframe button yet and shows the value at the playhead.
Values read from a file are clamped to the ranges above. `grade.wb.pick` runs the white balance picker for an agent or script on a
point or a box of the frame (`x`, `y`, optionally `x2`, `y2`: 0…1 of the frame; a point samples a 5 × 5 pixel spot, two corners a box) and `grade.wb.auto` solve the
node's Temperature and Tint and return them, with `clamped` and a `message` when the sliders hit
their limit.

## If you know Resolve

> id: color.primaries.resolve | does: how these wheels compare with Resolve's primaries | resolve: Lift / Gamma / Gain / Offset | when: you come from Resolve | see: color.primaries.panel

**[Resolve-style]**

Lift / Gamma / Gain / Offset do the same jobs, but they act on the sequence's grading signal
(DaVinci Intermediate in a DaVinci Wide Gamut sequence) and the numbers shown are the wheel
offsets, not Resolve's internal values. There are no separate shadows / highlights /
colour-boost / mid-detail sliders yet.

## If you know Premiere

> id: color.primaries.premiere | does: how the node grader relates to Lumetri's wheels and sliders | premiere: Lumetri Color | when: you come from Premiere | see: color.lumetri.what

**[Revolution]**

Lumetri's wheels and sliders are a separate tool (**[Revolution]**) that lives in the clip's
effect stack. Nodes run before the effect stack, so a node grade and Lumetri can be combined.
See [lumetri.md](lumetri.md) for when to use which.
