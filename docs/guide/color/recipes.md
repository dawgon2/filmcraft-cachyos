# Recipes

**[Fork]** · short, step-by-step solutions to everyday grading jobs, using only what the Color
page can do today

Each recipe says what it is for, then the steps. They assume the Color Grading workspace is open
(Window ▸ Workspaces ▸ **Color Grading**, `window.workspace.colorgrading`; **Shift+6** with
Workflow set to Resolve) and that you know where the
**Nodes** and **Primaries** panels are ([nodes](nodes.md), [primaries](primaries.md)). Every
step that adds a node is one undo step, so experiment freely: **Ctrl+Z** takes it back.

## 1. Turn a flat F-Log2 clip into a normal picture

> id: color.recipes.flog2-normal | does: a fixed set of nodes that turns flat F-Log2 into a normal picture | ui: grade.addFirst, grade.addNode | cmd: clip.interpretFootage, sequence.colorSettings, grade.labelNode | resolve: colour-managed log normalisation | premiere: Lumetri on log with an input LUT | when: the first grade of any X-M5 clip | see: color.nodes.starter-tree

**[Fork]** **[Resolve-style]**

**Why:** log footage looks grey on purpose. A small fixed set of nodes, always in the same order,
gives every clip the same starting point and makes later shots quicker. Start here when grading
any log clip.

1. Make sure the clip is read as F-Log2 (Clip ▸ Modify ▸ **Interpret Footage…**,
   `clip.interpretFootage`, see [colour management](color-management.md)) and that the sequence
   works in DaVinci Wide Gamut (Sequence ▸ **Color Management…**, `sequence.colorSettings`).
2. Select the clip. In Primaries press **Add Node** {ui:grade.addFirst}: node 01 appears.
3. Press **+ Serial** {ui:grade.addNode} three times: nodes 02, 03 and 04.
4. Name them (optional, by command): `grade.labelNode` with "Balance", "Exposure",
   "Contrast", "Saturation".
5. Click node 01. Use **Temperature** {ui:grade.param.temperature}, **Tint**
   {ui:grade.param.tint} and the **Offset** wheel {ui:grade.wheel.offset} until something
   that should be white or grey looks neutral.
6. Click node 02. Drag the **Gain** ring {ui:grade.wheel.gain} until the brightest important
   part (sky, a white shirt) sits where you want, then the **Gamma** ring
   {ui:grade.wheel.gamma} until faces look right.
7. Click node 03. Raise **Contrast** {ui:grade.param.contrast} a little at a time; leave
   **Pivot** at 0.336 unless the faces get too dark.
8. Click node 04. Raise **Saturation** {ui:grade.param.saturation} until colours look natural.
9. Press **Bypass** {ui:grade.bypass} twice to compare before and after.

Save this as a still (**Grab Still** {ui:stills.grab}) so you can give the same starting tree
to other clips (double-click the still, `stills.apply`).

## 2. Warm a shot up

> id: color.recipes.warm | does: warms a cold, blue shot | ui: grade.param.temperature, grade.wheel.gain | resolve: Temp / Gain wheel | premiere: Lumetri Temperature | when: a shot feels cold | see: color.primaries.panel

**[Resolve-style]**

**Why:** a cold, blue-ish shot feels unfriendly; warming it is the most common creative change.
Warming the highlights and mids while keeping the shadows neutral looks natural; warming
everything looks like a filter.

1. Select the clip and add a node after your technical nodes (**+ Serial** {ui:grade.addNode}).
2. Drag the puck of the **Gain** wheel {ui:grade.wheel.gain} a short way towards orange.
3. Drag the puck of the **Gamma** wheel {ui:grade.wheel.gamma} even less towards orange.
4. If the shadows now look muddy, drag the **Lift** puck {ui:grade.wheel.lift} slightly towards
   blue to keep them clean.
5. Toggle the node with **Disable / Enable** {ui:grade.toggleNode} to judge the effect. If it is
   too strong, reduce it on this node only; the rest of the grade is untouched.

Quicker but blunter: raise **Temperature** {ui:grade.param.temperature} on this node instead.

## 2b. Cool a shot down, or make the light bluer

> id: color.recipes.cool | does: cools a warm shot or makes its light bluer while it still looks natural | ui: grade.wheel.gain, grade.wheel.gamma, grade.wheel.lift, grade.param.temperature | resolve: Temp / Gain wheel | premiere: Lumetri Temperature | when: a shot feels too warm, or you want the light bluer or colder (daylight, moonlight) while keeping the image natural | see: color.recipes.warm, practice.grading.day-for-night

**[Resolve-style]**

**Why:** light has a colour, and moving it towards blue reads as cooler: overcast daylight, shade,
early morning, a screen's glow. It looks natural when the light itself turns blue but skin and
whites stay believable; turning the whole picture blue, shadows included, looks like a tint laid
over it. So the aim is light that turns blue while the image keeps a natural look.

1. Select the clip and add a node after your technical nodes (**+ Serial** {ui:grade.addNode}).
2. Drag the puck of the **Gain** wheel {ui:grade.wheel.gain} a short way towards blue: the bright
   parts carry the colour of the light.
3. Drag the puck of the **Gamma** wheel {ui:grade.wheel.gamma} even less towards blue.
4. Keep the **Lift** puck {ui:grade.wheel.lift} near the centre, or nudge it slightly towards
   orange if the shadows turn heavy blue: neutral shadows keep it looking real.
5. Look at a face: if skin turns grey or purple, the move is too strong. Reduce it, or protect the
   skin with a qualifier on its own node (**Fix grey or lifeless skin**, below).
6. Toggle the node with **Disable / Enable** {ui:grade.toggleNode} to compare.

Options: **Temperature** {ui:grade.param.temperature} lowered on this node is quicker but cools
everything evenly (more like a filter). For only the light from one source, such as a window,
isolate it with a window or qualifier (see **Make the sky bluer without touching skin**). For a
strong night look, see Day for night.

## 2c. Still too orange? Push a warm shot all the way to blue

> id: color.recipes.warm-to-blue | does: turns strongly warm light (a tungsten lamp, a sunset) blue when small cooling moves only make it less orange | ui: grade.wb.pick, grade.wb.auto, grade.wheel.offset, grade.wheel.gain, grade.param.temperature | resolve: WB picker, Offset / Gain wheels | premiere: Lumetri WB Selector, Temperature | when: the shot is still orange, nowhere near blue, after cooling it; very warm light you want blue | see: color.recipes.cool, color.recipes.warm

**[Resolve-style]**

**Why:** a tungsten lamp is far more orange than daylight. Gentle moves towards blue only pull it
back towards neutral, so it ends up "less orange" but never blue. Neutralise the warm light
first, then add blue on top: the blue then has room to show.

1. On the cooling node, or a new node before it, click the white-balance picker **WB**
   {ui:grade.wb.pick} and click or drag a box on something that should be white or grey (a white
   wall, a lampshade, paper). The orange cast is removed in one step. **Auto** {ui:grade.wb.auto}
   tries the same on its own.
2. Now cool it: drag the **Offset** wheel {ui:grade.wheel.offset} towards blue (it moves the
   whole picture), then the **Gain** wheel {ui:grade.wheel.gain} further towards blue for the
   light itself. Go further than feels safe, then back off.
3. Or lower **Temperature** {ui:grade.param.temperature} well below 0 on this node.
4. Check a vectorscope: the cloud of colours should sit towards blue, not orange. If skin goes grey
   or purple, protect it with a qualifier on its own node (**Fix grey or lifeless skin**).

Options: a light that is the subject (the lamp itself) can stay warm while the room goes blue: put
the cooling on a node with a window or qualifier that leaves the bulb out. A blue that still looks
believable usually keeps a little warmth in the brightest highlight.

## 3. Make the sky bluer without touching skin

> id: color.recipes.sky | does: deepens the sky with a qualifier and a gradient window, leaving skin alone | ui: grade.qual.pick, grade.window.add.linear | cmd: grade.pickQualifier | resolve: qualifier + linear window | premiere: Lumetri HSL Secondary + mask | when: a pale sky | see: color.secondaries.qualifier

**[Resolve-style]**

**Why:** a primary change moves every colour at once. A qualifier selects only the sky's colour,
so faces stay as they are.

1. Select the clip and add a node at the end (**+ Serial** {ui:grade.addNode}).
2. Open the **Qualifier** tab {ui:grade.tab.qualifier} and click **Pick**
   {ui:grade.qual.pick}, then click the sky in the Program monitor (`grade.pickQualifier`).
3. Click **Add** {ui:grade.qual.add} and click a lighter and a darker part of the sky. A sky
   blown out to white has no colour left to select: darken what detail is left with a gradient
   (Linear) window instead.
4. Set **View** to **Matte** {ui:grade.qual.show.2}. The sky should be white and everything else,
   especially faces, black. If a face shows grey, narrow **Hue width**
   {ui:grade.param.qual.hueWidth}; if the sky is speckled, raise **Qualifier softness**
   {ui:grade.param.qual.soft} or **Matte blur** {ui:grade.param.qual.blur}.
5. Set **View** back to **Normal** {ui:grade.qual.show.0}.
6. Open the **Primaries** tab {ui:grade.tab.primaries}, raise **Saturation**
   {ui:grade.param.saturation} a little and drag the **Gain** puck {ui:grade.wheel.gain} slightly
   towards blue.
7. Optional: if blue clothing or water also changed, add a **Linear** window
   {ui:grade.window.add.linear} in the **Windows** tab {ui:grade.tab.windows} and drag it so it
   covers only the top of the frame. The node then acts only where both the qualifier and the
   window agree.

## 3b. Change the colour of one light (a lamp), leave the room natural

> id: color.recipes.one-light | does: turns one light and its glow another colour (a warm lamp blue) while the room keeps its natural light | ui: grade.window.add.ellipse, grade.window.0.softness, grade.qual.pick, grade.wheel.gain, grade.wheel.offset | resolve: qualifier + circle window | premiere: HSL Secondary + mask | when: make the lamp light blue but keep the room natural; recolour one light, bulb or practical | see: color.recipes.sky, color.recipes.warm-to-blue

**[Resolve-style]**

**Why:** a primary change recolours the whole picture. To change only one light, select the lamp
and the glow right around it on its own node, change that, and leave the room on the other
nodes alone. The glow must fade out softly, otherwise a blue patch with a hard edge looks pasted on.

1. Select the clip and add a node at the end (**+ Serial** {ui:grade.addNode}).
2. In the **Windows** tab {ui:grade.tab.windows} add a **Circle** window
   {ui:grade.window.add.ellipse} and drag it over the lamp and the bright glow around it. Raise
   **Softness** {ui:grade.window.0.softness} a lot, so the change fades out with distance like
   real light does.
3. Optional, for a cleaner edge: in the **Qualifier** tab {ui:grade.tab.qualifier} click **Pick**
   {ui:grade.qual.pick} on the lampshade's glow, **Add** {ui:grade.qual.add} a brighter and a
   dimmer part of it, and check **View ▸ Matte** {ui:grade.qual.show.2}: the shade and its glow
   white, the rest of the room black. Raise **Qualifier softness** {ui:grade.param.qual.soft}
   until the edge is gentle, then **View ▸ Normal** {ui:grade.qual.show.0}. The node now acts
   only where the window and the qualifier agree.
4. In the **Primaries** tab drag the **Gain** wheel {ui:grade.wheel.gain} well towards blue. A
   very orange light needs a big move: add the **Offset** wheel {ui:grade.wheel.offset} towards
   blue as well, until the glow reads blue rather than white.
5. Leave the brightest core a little lighter than the colour (a bulb looks white at its centre).
   Toggle the node with **Disable / Enable** {ui:grade.toggleNode} to compare: the walls further
   away should look as they did.
6. If the camera moves, track the window (**Track forward** {ui:grade.window.0.track.forward}).

Options: the lamp lights the room, so its spill on the nearest wall is part of its light; a wider,
softer window that tints that spill a little too looks more believable than a blue lamp next to a
warm wall. Keeping the room exactly as it was is the stylised choice.

## 4. Brighten a face

> id: color.recipes.face | does: lifts a face a little with a soft window, without touching the background | ui: grade.window.add.ellipse, grade.window.0.softness | resolve: circle window + gamma | premiere: Lumetri mask + exposure | when: a face in shadow | see: color.secondaries.windows

**[Resolve-style]**

**Why:** viewers look at faces first. Lifting a face a little, without touching the background,
draws the eye and is the classic job for a soft circle.

1. Add a node at the end (**+ Serial** {ui:grade.addNode}).
2. In the **Windows** tab {ui:grade.tab.windows} click **Circle**
   {ui:grade.window.add.ellipse}.
3. Drag the circle over the face on the Program monitor and make it a bit larger than the face.
4. Raise **Softness** {ui:grade.window.0.softness} until you can't see where the circle ends.
5. In **Primaries**, raise the **Gamma** ring {ui:grade.wheel.gamma} slightly.
6. If the person moves, track the window (see
   [secondaries](secondaries.md#tracking-a-window)) so it follows the face.

## 5. Match a wide shot and a close-up

> id: color.recipes.match | does: matches two angles of one scene with a still, Shot Match and a check | ui: stills.grab, grade.match.shotMatch | cmd: grade.shotMatch, grade.compareShots | resolve: gallery wipe + Shot Match | premiere: Lumetri Comparison View + Apply Match | when: two angles that don't cut together | see: color.matching.shot-match

**[Resolve-style] [Fork]**

**Why:** two angles of the same scene rarely come out of the camera looking the same. Cutting
between them only works when they match.

1. Grade the shot you like best (usually the wide) first.
2. Park the playhead on it and press **Grab Still** {ui:stills.grab}.
3. Move to the close-up and click the still's thumbnail: a wipe appears. Drag the divider
   {ui:compare.divider} across a part both shots share: a face if there is one (match skin
   first, then the rest), otherwise a wall or the sky.
4. Select the close-up, open the **Match** tab {ui:grade.tab.match}, choose the wide shot as
   **Reference** {ui:grade.match.reference} and click **Shot Match**
   {ui:grade.match.shotMatch} (`grade.shotMatch`). A node labelled "Shot Match" is added.
5. Look at the wipe again and fine-tune the new node by hand: levels first (Lift, Gamma, Gain),
   then colour (usually Temperature).
6. Click **Compare shots** {ui:grade.match.compare} (`grade.compareShots`) to read in plain words
   what still differs.
7. Press **Off** {ui:stills.mode.off} to leave the comparison.

## 6. Give one grade to a whole scene

> id: color.recipes.scene | does: grades one clip and copies it to the scene, then checks for the odd one out | ui: grade.match.checkScene | cmd: grade.copy, grade.paste, grade.append, grade.checkScene | resolve: copy grade, shared nodes | premiere: Paste Attributes | when: many clips in the same place and light | see: color.matching.copy-paste

**[Resolve-style]**

**Why:** clips shot in the same location and light (15 clips of one scene, say) can share one
grade; grading one and copying it to the others is the quickest way, saves time and keeps them consistent.

1. Select the clip whose grade you want to reuse and choose Clip ▸ Grade ▸ **Copy Grade**
   (`grade.copy`).
2. Select the other clips of the scene in the timeline.
3. Choose **Paste Grade** (`grade.paste`) to replace their grades, or **Append Grade**
   (`grade.append`) to keep their own nodes and add the copied ones after them.
4. Click **Check scene** {ui:grade.match.checkScene} (`grade.checkScene`) in the **Match** tab:
   the clip that stands out most is named first. Fix that one by hand.

## 7. Finish with film grain

> id: color.recipes.grain | does: adds a little grain on a last node to hide banding and tie clips together | ui: grade.param.grain.* | resolve: Film Grain | premiere: Noise / grain effect | when: skies with banding, clips from different sources | see: color.finishing.grain

**[Fork]**

**Why:** a little grain hides banding in a sky or any smooth gradient after a grade, and makes clips from different
sources feel like one film. Add it last, so nothing after it sharpens or blurs it.

1. Select the last node of the grade, or add one with **+ Serial** {ui:grade.addNode}.
2. Raise **Grain strength** {ui:grade.param.grain.strength} until you can just see it at full
   size on the Program monitor, then back off a little.
3. Use **Grain size** {ui:grade.param.grain.size} for coarser grain and **Grain softness**
   {ui:grade.param.grain.softness} for smoother grain.
4. Use the same three numbers on the other clips so the grain matches everywhere. (Copying the
   grade with recipe 6 would bring this clip's other nodes along too.)

## 8. Fix grey or lifeless skin

> id: color.recipes.skin | does: restores warmth and saturation in grey skin with a qualifier | ui: grade.qual.pick, grade.param.saturation | cmd: grade.pickQualifier | resolve: skin qualifier, hue vs sat | premiere: Lumetri HSL Secondary on skin | when: faces look grey under green-ish or overcast light | see: color.secondaries.qualifier

**[Resolve-style]**

**Why:** after a log normalisation, or under green-ish or overcast light, faces can look grey,
pale or a little ill while the rest of the shot is fine. Raising saturation or warmth for the
whole picture would push the background too far, so select the skin with a qualifier and give
only the skin a small lift. Small is the point: skin that is too orange looks worse than grey
skin.

1. Get the overall balance right first (recipe 1); this fix goes on a node of its own after
   it. Add one with **+ Serial** {ui:grade.addNode} and, if you like, name it "Skin"
   (`grade.labelNode`).
2. Open the **Qualifier** tab {ui:grade.tab.qualifier}, click **Pick** {ui:grade.qual.pick}
   and click the cheek or forehead in the Program monitor (`grade.pickQualifier`). Avoid shiny
   highlights and deep shadow.
3. Click **Add** {ui:grade.qual.add} and click a lighter and a darker part of the same face (and
   a hand or arm, if they show) so the whole skin tone range is in.
4. Set **View** to **Matte** {ui:grade.qual.show.2}. Skin should be white. Wooden furniture,
   brick, sand or orange clothing often come in too, because they share skin's hue: narrow
   **Hue width** {ui:grade.param.qual.hueWidth} until most of them turn black, and accept a
   little spill rather than holes in the face. Raise **Qualifier softness**
   {ui:grade.param.qual.soft} so the edges fade instead of cutting, and **Matte blur**
   {ui:grade.param.qual.blur} if the face looks speckled.
5. Set **View** back to **Normal** {ui:grade.qual.show.0}. Always judge the colour itself in
   Normal: the matte only tells you *where* the node acts.
6. Open the **Primaries** tab {ui:grade.tab.primaries}. Raise **Saturation**
   {ui:grade.param.saturation} a little (a few steps, not a lot: skin saturates quickly and turns
   orange; if it already has, pull Saturation back here instead), then raise **Temperature**
   {ui:grade.param.temperature} slightly for warmth. If the face is also dull, lift the **Gamma**
   ring {ui:grade.wheel.gamma} a touch.
7. Press **Disable / Enable** {ui:grade.toggleNode} a few times. The face should look healthier,
   not obviously different; if you notice the change at once, it is too strong: halve it. On the
   vectorscope ([finishing.md](finishing.md)), the face's trace should lie along the skin-tone
   line.
8. Set **View** to **Matte** {ui:grade.qual.show.2} one last time while you play the clip: if
   the matte flickers on a moving face, raise **Qualifier softness**
   {ui:grade.param.qual.soft} again, then go back to **Normal** {ui:grade.qual.show.0}.

## If you know Resolve

> id: color.recipes.resolve | does: how the recipes follow common Resolve habits | resolve: technical nodes first, secondaries and look later | when: you come from Resolve | see: color.nodes.orders

**[Resolve-style]**

The recipes follow the usual Resolve habits (fixed technical nodes first, secondaries and look
after, grain last). There is no node graph, no parallel nodes and no saved node-tree preset yet,
so recipe 1 uses a still to carry the starter tree. See [nodes](nodes.md#if-you-know-resolve).
