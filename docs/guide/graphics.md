# Titles and graphics

**[Revolution]** · graphic clips with text and shape layers, the Essential Graphics panel,
templates, responsive design and source graphics; modelled on Premiere Pro's Essential Graphics;
written for AI-PLAN AI3-PREP (feature pages)

## Graphic clips and their layers

> id: graphics.layers | does: a graphic clip holds text and shape layers drawn over the picture | cmd: graphics.newText, graphics.newVerticalText, graphics.newRectangle, graphics.newEllipse, graphics.newPolygon, graphics.newShape, graphics.newFromFile, graphics.list, graphics.deleteLayer | resolve: Text+ / Fusion titles | premiere: Essential Graphics ▸ Edit, graphic clips | when: titles, lower thirds, labels, simple shapes such as a rectangle behind text | see: graphics.text, graphics.templates

**[Revolution]**

A **graphic clip** sits on a video track like any clip and holds **layers**: text and shapes,
drawn back to front. The Essential Graphics panel (Window ▸ Essential Graphics) lists the layers
of the selected graphic clip; Effect Controls shows each layer's properties, where they can be
keyframed.

1. Park the playhead where the title should start and choose Graphics and Titles ▸ New Layer ▸
   **Text** (`graphics.newText`, Cmd+T; `graphics.newVerticalText` for vertical text). A new
   graphic clip, five seconds long, lands on the first video track above the picture that is free
   there (a track is added when none is).
2. Shapes work the same way: `graphics.newRectangle`, `graphics.newEllipse`,
   `graphics.newPolygon`. **From file…** (`graphics.newFromFile`) imports a picture and places it
   the same way.
3. Select layers in the panel (`graphics.selectLayer`, `graphics.selectNextLayer` /
   `selectPreviousLayer`); `graphics.deleteLayer` removes one.

Every change is one undo step; properties that are animated get a keyframe at the playhead.

## Text

> id: graphics.text | does: types and styles text: font, size, leading, alignment, per-character styles | cmd: graphics.setText, graphics.setTextType, graphics.setCharStyle, graphics.set, graphics.fontSizeUp, graphics.fontSizeDown, graphics.leadingUp, graphics.leadingDown, graphics.alignTextLeft, graphics.alignTextCenter, graphics.alignTextRight | resolve: Text+ inspector | premiere: Essential Graphics ▸ Text | when: writing and styling a title | see: graphics.fonts

**[Revolution]**

Type into a text layer in the Program monitor, or set it with `graphics.setText`. Font, size,
colour, stroke, shadow and the rest are layer properties (`graphics.set`); the keyboard
shortcuts that make text bigger or smaller (`graphics.fontSizeUp` / `fontSizeDown`, five steps at a time with Shift) and line spacing
(`graphics.leadingUp` / `leadingDown`) work while a text layer is selected. Style part of a line
(one word bold or in another colour) with `graphics.setCharStyle` on a range of characters.
`graphics.setTextType` switches a layer between point text (no box: dragging its handles scales
the text) and paragraph text (the text wraps inside a box whose handles resize it).

## Arranging and aligning

> id: graphics.arrange | does: aligns layers to the frame or each other, spaces them evenly and changes their order | cmd: graphics.align, graphics.distribute, graphics.arrangeLayer, graphics.bringToFront, graphics.sendToBack, graphics.bringForward, graphics.sendBackward | resolve: Fusion transform / align | premiere: Essential Graphics ▸ Align and Transform | when: laying out a lower third or a title card | see: graphics.responsive

**[Revolution]**

- **Align** to the frame (`graphics.alignFrame.left` … `.vcenter`), to the selection
  (`graphics.alignSelection.*`) or within a group (`graphics.alignGroup.*`).
- **Distribute** several layers evenly by their centres (`graphics.distributeHorizontally`,
  `distributeVertically`) or by the space between them (`distributeSpaceHorizontally`,
  `distributeSpaceVertically`).
- **Order**: bring to front / forward, send to back / backward.
- **Nudge** a layer by a pixel with the arrow keys (`graphics.nudgeLeft` …), five with Shift.

## Responsive design: pins, protected time, rolls

> id: graphics.responsive | does: keeps a design working when text or duration changes: pins, protected intro / outro, roll and crawl | cmd: graphics.pin, graphics.setResponsiveTime, graphics.setRoll | resolve: — (Fusion expressions) | premiere: Responsive Design: Position and Time, Roll / Crawl | when: a lower third whose box must fit any name; a title that must animate the same at any length | see: graphics.templates

**[Revolution]**

- **Pin** a layer's edges to another layer or the frame (`graphics.pin`): a box pinned to a text
  layer grows and shrinks with the name typed into it.
- **Protected time** (`graphics.setResponsiveTime`): the first and last seconds of the clip keep
  their animation when you trim or stretch the clip; only the middle changes length.
- **Roll / crawl** (`graphics.setRoll`): credits that scroll up, or a ticker that runs sideways,
  with options for starting and ending off screen.

## Templates

> id: graphics.templates | does: saves a graphic as a reusable template with chosen editable properties, and applies templates | cmd: graphics.template.list, graphics.template.apply, graphics.template.set, graphics.template.controls, graphics.template.export, graphics.template.install, graphics.template.remove | resolve: Fusion title templates, Effects Library titles | premiere: Motion Graphics templates (.mogrt) | when: "the same lower third in every episode"; sharing a design | see: graphics.layers, packs.what

**[Revolution]**

1. **Apply** a template from Essential Graphics ▸ Browse (`graphics.template.apply`): it places a
   new graphic clip at the playhead. Built-in templates are Revolution's own designs.
2. Change what the template's author made editable (the name in a lower third, a colour) in
   Essential Graphics ▸ Edit (`graphics.template.controls` lists them, `graphics.template.set`
   changes them).
3. **Make your own:** select a graphic clip and choose Export As Motion Graphics Template
   (`graphics.template.export`), picking which properties stay editable. Templates are saved as
   `.fcgt` files in your data folder; `graphics.template.install` adds one someone sent you,
   `graphics.template.remove` removes one. Adobe `.mogrt` files (Premiere's templates) can't be imported: they are refused without
   being opened.
   To share templates together with presets and LUTs, use a look pack ([packs.md](packs.md)).

| Option | Good for | Trade-offs |
|---|---|---|
| **Template** | A design used many times; handing a title to someone who shouldn't change the layout | Only the chosen properties are editable |
| **Copy and paste a graphic clip** | A one-off variation in this project | Every copy is separate: changing the design means changing each |
| **Source graphic** (below) | The same title on many clips that must stay identical | One edit changes all of them, even when you didn't mean it to |

## Source graphics and upgrading captions

> id: graphics.source | does: shares one set of layers between every clip of a graphic, and turns captions into graphics | cmd: graphics.upgradeToSourceGraphic, graphics.upgradeCaption | resolve: compound clip (closest) | premiere: Upgrade to Source Graphic, Upgrade Caption to Graphic | when: a logo bug or title that must be identical everywhere; styling a caption beyond caption styles | see: graphics.templates, text.captions

**[Revolution]**

**Upgrade to Source Graphic** (`graphics.upgradeToSourceGraphic`) makes a graphic's layers
belong to its project item, so every clip of that item shows the same layers and editing one
updates the others. **Upgrade Caption to Graphic** (`graphics.upgradeCaption`) turns a caption
into a graphic clip with a text layer, for styling and animation that captions don't have.

## Fonts

> id: graphics.fonts | does: lists the fonts a project uses and replaces fonts across projects | cmd: graphics.fonts.used | resolve: — | premiere: Replace Fonts in Projects | when: a font is missing on this computer; changing a typeface everywhere | see: graphics.text

**[Revolution]**

`graphics.fonts.used` lists every font the project's graphics use. **Replace Fonts in Projects**
swaps one font for another in every graphic at once. Characters a font doesn't have (Japanese in
a Latin font, for example) are drawn with a fallback font, character by character; builds made
with the open-licensed craft-fonts set include fonts for Japanese text.
