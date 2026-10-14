# Keying

**[FilmCraft] [Fork]** · making part of a picture transparent so another picture shows through
(green screen, a bright sky, a logo on black); written for Phase Y3.1 (keys on the graphics card)

## What a key is, and why

> id: keying.what | does: makes parts of a clip see-through by colour or brightness | resolve: 3D Keyer, Qualifier as a key | premiere: Ultra Key | when: green screen, sky replacement, overlays | see: keying.ultra-key

**[FilmCraft]**

A **key** decides, pixel by pixel, how see-through a clip is, from what the pixel looks like. The
classic use is a **green screen**: you film someone in front of an evenly lit green backdrop, key
out the green, and put the clip on a track **above** the background you want behind them. Whatever
is keyed out shows the track below.

The see-through amount is called the **matte** (or alpha): white = solid, black = gone, grey =
partly see-through (hair, motion blur, glass). A good key is mostly about a clean matte.

FilmCraft has three keyers, all in **Effects ▸ Video Effects ▸ Keying**:

| Keyer | Keys out | Good for | Trade-off |
|---|---|---|---|
| **Ultra Key** | one colour (by its hue and saturation) | green / blue screens | the most controls; needs an evenly lit screen |
| **Color Key** | one colour (by distance, brightness counts a little) | flat graphic colours, quick tests | a hard edge; no spill or cleanup controls |
| **Luma Key** | dark (or bright) areas | a logo or text on black, a sky | keys by brightness only, so dark clothes go too |

If you know Premiere: these are the same effects with the same names and controls. If you know
Resolve: Resolve keys in the Color page (the qualifiers and the 3D / HSL / Luma keyers in a node)
or in Fusion (Delta Keyer); FilmCraft's keyers are effects on the clip instead, like Premiere's.

## Keying a green screen with Ultra Key

> id: keying.ultra-key | does: removes a green or blue backdrop with Ultra Key | cmd: effects.apply | resolve: 3D Keyer / Delta Keyer (Fusion) | premiere: Ultra Key | when: a subject filmed in front of a green screen | see: keying.check, keying.cleanup

**[FilmCraft]**

1. Put the background on **Video 1** and the green-screen clip on **Video 2**, above it.
2. Apply **Ultra Key** to the green-screen clip: drag it from Effects ▸ Video Effects ▸ Keying onto
   the clip (command `effects.apply` with `effect` `ultra_key`).
3. In **Effect Controls**, set **Key Color** to the screen: use the eyedropper on a mid-green
   part of the backdrop, not a shadow or a bright spot.
4. Pick a **Setting**: **Default** first. If edges look chewed, try **Relaxed**; if green is left
   in the background, try **Aggressive**. **Custom** uses the sliders below as you set them.
5. **Check the matte** (next section) before judging the composite.

Why the order matters: every later control (cleanup, spill) works on what the key colour and the
setting produced, so fix the key colour first.

## Checking the key: Output ▸ Alpha Channel

> id: keying.check | does: shows the key's matte in black and white to find holes and edges | resolve: Highlight / show matte | premiere: Ultra Key Output ▸ Alpha Channel | when: before cleaning up a key | see: keying.cleanup

**[FilmCraft]**

Set Ultra Key's **Output** to **Alpha Channel**: the Program monitor shows the key's alpha in
black and white instead of the picture (Premiere and Resolve users may know this as "show
alpha"). Look for:

- **grey speckles in the background** → raise **Matte Generation ▸ Tolerance** or
  **Transparency** (Setting: Custom), or light the screen more evenly next time;
- **holes in the person** (grey inside white) → lower **Tolerance**, or raise **Pedestal** a
  little to push faint background noise to black;
- **hard, jagged edges** → **Matte Cleanup ▸ Soften** a little.

Set **Output** back to **Composite** when the alpha looks clean (Output is part of the effect, so
it also changes what you render). **Color Channel** shows the
foreground colours with the spill removed but nothing keyed, to judge spill on its own.

## Cleaning up edges and green spill

> id: keying.cleanup | does: choke, soften and spill controls that fix outlines and green tinge | resolve: Matte Finesse, Despill | premiere: Ultra Key Matte Cleanup and Spill Suppression | when: green edges, hair, reflections on the subject | see: keying.check

**[FilmCraft]**

- **Choke** shrinks the matte inwards, trimming a thin green outline. Small values (5–20) are
  usually enough; large values eat into hair.
- **Soften** feathers the edge so it blends into the background.
- **Contrast** and **Mid Point** make the matte's greys harder (more solid or more gone) around
  the mid point.
- **Spill Suppression ▸ Spill** removes the green cast that the screen throws onto skin and hair
  edges. **Desaturate**, **Range** and **Luma** control how strongly, which colours, and whether
  the lost brightness is put back.
- **Color Correction** (Saturation, Hue, Luminance) adjusts the kept foreground to sit better in
  the new background.

There is no single right order: a common one is key colour → setting → matte check → choke /
soften → spill → colour correction, but going back to the key colour after looking at spill is
normal.

## Luma Key and Color Key

> id: keying.luma-color | does: keys by brightness (Luma Key) or by one colour (Color Key) | cmd: effects.apply | resolve: Luma Keyer, HSL qualifier as a key | premiere: Luma Key, Color Key | when: titles on black, a bright sky, a simple flat backdrop | see: keying.what

**[FilmCraft]**

**Luma Key** removes pixels by brightness. **Cutoff** is the brightness at and below which a pixel
is fully gone; **Threshold** the brightness at and above which it is fully kept; between them it
fades. Typical use: a white logo on black, Cutoff ≈ 10, Threshold ≈ 30.

**Color Key** removes one colour: **Key Color** with the eyedropper, **Color Tolerance** for how
far from that colour still goes, **Edge Feather** to soften the edge. Unlike Premiere, **Edge
Thin** does nothing yet (FilmCraft's Color Key has never used it); use Ultra Key's Choke when you
need to trim an edge.

## On the graphics card

> id: keying.gpu | does: all three keyers run on the graphics card with the same result as the processor path | when: keying 4K and above in real time | see: keying.ultra-key

**[Fork]**

All three keyers run on the graphics card (they show the **GPU accelerated** badge), so a key on
4K footage plays without falling back to the slower processor path. They give the same result as
the processor path (checked against it on AMD and Intel graphics). If a key's numbers are invalid
(for example from a damaged project file) that clip is drawn on the processor instead, so it still
shows.
