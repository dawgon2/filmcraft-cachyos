# Secondaries: qualifier, windows, tracking

**[Resolve-style]** · limiting a node to part of the picture (Phase G2; this page grows with it)

A *primary* correction changes the whole picture. A *secondary* changes only part of it: the
skin, the sky, a face, one corner. In the node grader a secondary is simply a node whose
correction is limited by a **qualifier** (a colour selection) and, later in G2, by **windows**
(shapes) that can be **tracked**.

## The qualifier: select by colour

> id: color.secondaries.qualifier | does: limits a node's correction to a range of hue, saturation and brightness | ui: grade.qual.*, grade.param.qual.*, grade.tab.qualifier | cmd: grade.pickQualifier, grade.setParam | resolve: Qualifier (HSL) | premiere: Lumetri HSL Secondary | when: skin, sky, one colour to change or protect | see: color.secondaries.windows

**[Resolve-style]**

### Why

Use a qualifier when the part you want to change has its own colour: skin, a blue sky, green
grass, a red jacket. The qualifier builds a **matte** (white where the node acts, black where it
doesn't) from the node's *input* picture, and the node's corrections (wheels, sliders, curves,
grain) then apply only inside that matte.

### Steps

1. Select the clip and add a node for the secondary (`grade.addNode`, **Add Node**). One job per
   node: keep the secondary on its own node, after your balance and exposure nodes.
2. In **Primaries**, open the **Qualifier** tab {ui:grade.tab.qualifier}.
3. Click **Pick** {ui:grade.qual.pick}, then click the colour you want in the Program picture.
   The command `grade.pickQualifier` samples the node's input there and switches the qualifier
   on with a hue, saturation and luminance range around it.
4. Click **Add** {ui:grade.qual.add} and click another part of the same object to widen the
   selection (`grade.pickQualifier` with `"mode": "add"`). Esc cancels an armed pick.
5. To see what is selected, set **View** to **Selection** {ui:grade.qual.show.1} (the selected
   area in colour, the rest grey) or **Matte** {ui:grade.qual.show.2} (white = selected).
   Switch back to **Normal** {ui:grade.qual.show.0} before you judge the grade.
6. Refine with the sliders: **Hue centre** / **Hue width** {ui:grade.param.qual.hueWidth},
   **Saturation low / high**, **Luminance low / high**, **Qualifier softness**
   {ui:grade.param.qual.soft} (how gradual the matte's edges are) and **Matte blur**
   {ui:grade.param.qual.blur} (softens a noisy, speckled matte).
7. Tick **Invert** {ui:grade.qual.invert} to act on everything *except* the selection.
8. Go back to the **Primaries** tab and make the correction; it lands only inside the matte.

### Tips

- Pick on the node's *input*: the picker sees the picture before this node and before any
  Lumetri / effects, the same picture the qualifier keys.
- On F-Log2 footage, a qualifier on a node *before* your normalising node works on the flat log
  picture (low saturation): it still works, but the ranges are narrower. Most people qualify
  after the image is normalised.
- A selection that flickers or crawls usually needs more **Qualifier softness** or a little
  **Matte blur**, not a wider range.
- Skin gone grey or blotchy after a secondary (on the sky, say) means the node's selection
  reaches into the face. Set **View** to **Matte** {ui:grade.qual.show.2}: the face should be
  black. Narrow **Hue width** {ui:grade.param.qual.hueWidth} or raise **Qualifier softness**
  until it is.

### Parameters (for scripts)

**[Fork]**

All are node parameters set with `grade.setParam` (0…1 unless noted): `qual.on` (switch, ≥ ½ is
on), `qual.hue`, `qual.hueWidth` (1 = every hue), `qual.satLow`, `qual.satHigh`, `qual.lumLow`,
`qual.lumHigh` (ranges on the node's input as HSL of the grading signal), `qual.soft`
(0.001…0.5, edge width), `qual.blur` (σ up to 20 pixels of the sequence frame), `qual.invert` (switch),
`qual.show` (0 normal, 1 selection, 2 matte). A range that reaches 0 or 1 has no soft edge at that
end, so the full default range selects everything, black and white included.

### If you know Resolve

**[Resolve-style]**

- Like Resolve's HSL qualifier: hue / saturation / luminance ranges, softness, matte blur,
  invert, a highlight view, and picker / add tools on the viewer.
- Differences: there is no "subtract" picker and no 3D / RGB / luma-only qualifier mode yet;
  the softness is one value for every edge; **View ▸ Selection / Matte is saved with the node**
  and shows on the Program monitor *and in an export* (unlike Resolve's viewer-only highlight),
  and nodes after the one being viewed are skipped while it is on. Turn it off before exporting.
- The picker doesn't work on a grade on an adjustment layer yet (set the ranges by hand there).
- The matte is not shown as a separate node output yet (key output / key mixer come with
  COMPOSITOR-PLAN C2).

### If you know Premiere

**[FilmCraft]**

Lumetri Color's **HSL Secondary** is the same idea inside one effect. In the node grader every
node can carry its own qualifier, and a node's whole correction (wheels, curves, grain) is
limited by it, not only the secondary's own hue / saturation sliders.

## Windows: select by shape

> id: color.secondaries.windows | does: limits a node's correction to a shape: ellipse, rectangle or linear gradient | ui: grade.window.*, grade.tab.windows | cmd: masks.* | resolve: Power Windows | premiere: Lumetri masks | when: vignettes, a face, a sky gradient; combined with a qualifier | see: color.secondaries.qualifier, color.secondaries.tracking

**[Resolve-style]**

### Why

Use a window when the part you want has a *place* rather than a colour: brighten a face,
darken the sky with a graduated edge, add a vignette around the subject. A window is a shape on
the picture; the node acts inside it (or outside, inverted). With a qualifier on the same node,
the node acts only where **both** select (skin *and* inside the circle round the face).

### Steps

1. Select the node and open the **Windows** tab {ui:grade.tab.windows}.
2. Click **Circle** {ui:grade.window.add.ellipse}, **Rectangle** {ui:grade.window.add.rectangle}
   or **Linear** {ui:grade.window.add.linear} (a straight soft edge across the frame, like a
   graduated filter). This runs `masks.add` with `"node"`; the new window is selected.
3. Drag the window's shape and handles on the Program monitor (the same handles as effect masks).
   Click a window's name {ui:grade.window.0} to select it again.
4. Set **Softness** {ui:grade.window.0.softness} (the feather, in source pixels) and tick
   **Invert** {ui:grade.window.0.invert} to act outside the shape.
5. Correct the picture in **Primaries**; the change stays inside the window.
6. **✕** {ui:grade.window.0.delete} deletes a window. A node holds up to 16 windows; several
   windows combine by their mask mode (add, subtract, intersect, …) like effect masks.

### For scripts

**[Fork]**

Windows are masks: every `masks.*` command takes `"node": <node id>` instead of `"effect"`
(`masks.add` with `"shape": "ellipse" | "rectangle" | "polygon" | "bezier" | "linear"`, `masks.set`,
`masks.select`, `masks.remove`, `masks.moveVertex`, `masks.list` lists windows with their
`node`). Paths are in the clip's source pixels.

### If you know Resolve

**[Resolve-style]**

- Like Resolve's Window palette: circle, linear, polygon / curve shapes, softness, invert, and
  the window limiting the node together with its qualifier.
- Differences: shapes are FilmCraft's mask paths (a circle is a 4-point Bézier ellipse; there is
  no separate inside / outside softness), windows are listed per node rather than in a fixed
  palette, and there is no "outside node" yet (COMPOSITOR-PLAN C2).

## Tracking a window

> id: color.secondaries.tracking | does: makes a window follow its subject from frame to frame | ui: grade.window.0.track.* | cmd: masks.track | resolve: Tracker (Window) | premiere: mask tracking | when: a window on something that moves | see: color.secondaries.windows, tracking.what

**[Resolve-style]** **[FilmCraft]**

### Why

When the subject moves (a person walking, a car passing), a window has to follow it. The tracker follows the detail inside the
window from frame to frame and keyframes the window's shape.

### Steps

1. Put the playhead on a frame where the window sits well on the subject.
2. In the **Windows** tab, click **▶** (Track forward) {ui:grade.window.0.track.forward} or
   **◀** (Track backward) {ui:grade.window.0.track.backward}. This runs `masks.track` with
   `"node"` in the background; the window gets a keyframe per frame.
3. Scrub through the shot to check it. If the track drifts, move the window back on the frame
   where it went wrong and track again from there (the new keyframes replace the old ones).

### Notes

- Tracking uses FilmCraft's point tracker (features inside the window, robust fit; method
  Position / Position + Rotation / Position + Scale + Rotation from `masks.set`'s
  `trackMethod`). It needs some detail inside the window: a window on a flat wall will "lose
  track".
- Unlike Resolve there is no planar / 3D tracker and no separate tracker graph yet.
- Copying a grade to another clip (`grade.paste`, `grade.append`) copies its windows as they are:
  same position in source pixels and the same tracking keyframes. On a clip of another size or
  with other motion, move or re-track the windows after pasting.
