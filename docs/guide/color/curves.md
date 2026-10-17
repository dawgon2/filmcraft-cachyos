# Curves

**[Resolve-style]** · per-node tone curves (what exists today)

## Why

> id: color.curves.why | does: curves map brightness in to brightness out for contrast and per-channel colour | resolve: Custom curves | premiere: Lumetri RGB Curves | when: soft contrast, a filmic shoulder, a channel tint | see: color.curves.steps

**[Resolve-style]**

A curve draws "brightness in" against "brightness out". Use it for contrast with a soft shoulder,
for lifting only the shadows, or for pushing one range toward a colour through a single channel.

## Steps

> id: color.curves.steps | does: steps to shape a master or red / green / blue curve | ui: grade.curve.*, grade.tab.curves | cmd: grade.setParam | resolve: Custom curves | premiere: Lumetri RGB Curves | when: drawing a curve on a node | see: color.curves.limits

**[Resolve-style]**

1. Select a clip and a node; open **Primaries** and click the **Curves** tab
   {ui:grade.tab.curves}.
2. Pick the channel: **Y** (brightness) {ui:grade.curve.master}, **R** {ui:grade.curve.r},
   **G** {ui:grade.curve.g} or **B** {ui:grade.curve.b}.
3. Add points on the line and drag them to bend the curve. A straight diagonal is neutral.
4. Press **Reset curve** {ui:grade.curve.reset} to straighten the current channel.
5. For a first contrast, make a gentle S: shadows slightly down, highlights slightly up.

The node box in the Nodes strip then reads "Curves" (or "Primaries + Curves" when the node also
has wheel or slider changes).

## Limits

> id: color.curves.limits | does: what the curves can't do yet (hue curves, soft clip in the curve) | when: a curve you miss from Resolve | see: color.curves.resolve

**[Fork]**

- Four curves per node: master (Y), red, green, blue (keys `curve.master`, `curve.r`,
  `curve.g`, `curve.b`, set with `grade.setParam`).
- How many points can a curve have? It keeps at most 64 points when applied.
- Hue vs hue / sat / luma, luma vs sat and sat vs sat curves are in Lumetri Color's curves
  section, not yet in the node grader.

## If you know Resolve

> id: color.curves.resolve | does: how these curves compare with Resolve's Custom and hue curves | resolve: Custom curves, Hue vs curves | when: you come from Resolve | see: color.curves.steps

**[Resolve-style]**

This is the Custom curves tab with Y / R / G / B. Soft clip and the five hue-based curves are not
on the node panel yet.
