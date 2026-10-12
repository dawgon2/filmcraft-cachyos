# Lumetri Color and the node grader

**[FilmCraft] [Fork]** · the Premiere-style Lumetri Color panel, how it relates to the
Resolve-style node grader, and which one to reach for

## Why there are two

**[FilmCraft] [Fork]**

FilmCraft (upstream) is modelled on Premiere Pro, so its colour tool is **Lumetri Color**: one
panel of sliders, wheels and curves in fixed sections, stored as an effect on the clip. This fork
adds the **node grader** ([nodes.md](nodes.md)): a list of small correction steps, in the spirit
of DaVinci Resolve's Color page. Neither replaces the other. They exist so that a Premiere user
and a Resolve user can both start grading with the habits they already have.

Both are built from the same small colour operations (lift / gamma / gain, curves, HSL key,
saturation, LUT, colour space transform…; see [README.md](README.md)), so the same setting gives
the same picture in either; no grading maths is written twice.

## Where Lumetri sits in the picture

**[Fork]**

The order for every clip is: input transform → **your grade nodes** → the clip's effects
(**Lumetri is one of them**) → output transform. So:

1. A node grade and Lumetri on the same clip are both applied, nodes first.
2. Lumetri sees the picture the nodes made. A big change in the nodes changes what Lumetri's
   sliders do (and the reverse is not true).
3. Turning the node grade off (**Bypass** {ui:grade.bypass}) leaves Lumetri on; Lumetri has its
   own **fx** switch in its header.

## Using the Lumetri Color panel

**[FilmCraft]**

1. Select a clip and open Window ▸ **Lumetri Color**. (In the Color Grading workspace it is a tab
   beside **Primaries**.)
2. If the clip has no Lumetri yet, click **Add Lumetri Color to clip** (the same as
   `effects.apply` with the effect `lumetri`).
3. Work down the sections, each with an on / off switch:
   - **Basic Correction**: Input LUT (`lumetri.setInputLut`), Temperature, Tint, Saturation,
     Exposure, Contrast, Highlights, Shadows, Whites, Blacks (HDR White / HDR Specular in an HDR
     sequence).
   - **Creative**: a Look LUT (`lumetri.setLook`) with Intensity, Faded Film, Sharpen, Vibrance,
     Saturation.
   - **Curves**: RGB curves and the Hue vs Sat / Hue vs Hue / Hue vs Luma / Luma vs Sat / Sat vs
     Sat curves.
   - **Color Wheels & Match**: Shadows / Midtones / Highlights wheels and Color Match.
   - **HSL Secondary**: a hue / saturation / luma key with Soften, Denoise and Blur, Show Mask,
     and a correction for the keyed area.
   - **Vignette**: Amount, Midpoint, Roundness, Feather.
4. Every slider is an effect parameter: `effects.setParam` sets it, and it can be keyframed like
   any effect.
5. Built-in looks: Effects panel ▸ **Lumetri Presets**, or `lumetri.applyPreset`.

**Color Match:** turn on **Comparison View** {ui:lumetri.match.comparisonView}, park the
reference frame, then click **Apply Match** {ui:lumetri.match.apply} (`lumetri.applyMatch`). It
moves Lumetri's wheels. The node grader's **Shot Match** does the same job but adds an editable
node ([matching.md](matching.md)).

## Which one to use

**[Fork]**

| You want… | Reach for |
|---|---|
| a quick fix on one clip, Premiere-style | Lumetri |
| a Premiere look preset or a creative LUT with an intensity slider | Lumetri (Creative) |
| vibrance, faded film, sharpen or a vignette | Lumetri (the node grader has none of these yet) |
| the F-Log2 starter tree, one job per step, easy to switch steps off | nodes |
| an Offset wheel, windows with tracking, noise reduction, film grain | nodes |
| several qualifiers on one clip | nodes (one per node; Lumetri has a single HSL Secondary) |
| stills, wipe, "same grade?", shot comparison and the node explainer | nodes |

A sensible split when you use both: do the balance and the look in nodes, and use Lumetri only
for something nodes don't have yet (a vignette, vibrance).

## What doesn't carry over

**[Fork]**

- **No conversion.** Lumetri settings don't turn into nodes, or the reverse. To move a look
  from one to the other, rebuild it by eye with a still and the wipe ([matching.md](matching.md)).
- **Copying.** Clip ▸ Grade ▸ **Copy Grade** (`grade.copy`) / **Paste Grade** (`grade.paste`)
  copy only the nodes. Lumetri travels with the clip's effects: Edit ▸ **Copy** (`edit.copy`) and
  **Paste Attributes…** (`edit.pasteAttributes`).
- **Measuring tools.** "Same grade?" and the node explainer look only at the nodes. **Compare
  shots** measures the finished picture, Lumetri included. Shot Match solves on the picture
  *before* Lumetri, so on a clip with Lumetri check with **Compare shots** afterwards. Shot Match
  and Color Match each move only their own tool.
- **Workflow switch.** Settings ▸ General ▸ **Workflow** set to DaVinci Resolve
  (`settings.setWorkflow`, [getting-started.md](../getting-started.md)) opens the node grader as
  the colour panel; Lumetri stays one tab away and keeps working.

## If you know Premiere

**[FilmCraft]**

The Lumetri Color panel follows Premiere's sections and names, so Premiere habits carry over.
The one difference to keep in mind: the fork's node grader runs *before* Lumetri on each clip.
If you never open the node grader, nothing changes for you.
