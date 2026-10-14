# Shader effects

**[Fork]** · bundled examples and your own WGSL effects; written for Phase Y3.4

## Using a shader effect

> id: shader-effects.use | does: applies a bundled or user-written GPU shader effect to a clip | cmd: effects.apply, effects.listUser | resolve: DCTL effects (a different language; files don't carry over) | premiere: — (third-party plugins) | when: a stylised look no built-in effect gives | see: shader-effects.write, shader-effects.trouble

**[Fork]**

A **shader effect** is a small program that runs on the graphics card for every pixel of a clip.
FilmCraft ships four examples and loads your own from a folder; they appear in the **Effects** panel
under **Video Effects ▸ User**, with the examples in its **Stylize** group:

- **Chromatic Aberration**: coloured fringes that grow towards the corners, like a cheap lens.
- **Glitch**: bands that jump sideways and split into red and blue copies (animated).
- **VHS**: wobble, colour bleeding, washed-out blacks, grain and scanlines (animated).
- **Light Leak**: a soft warm glow that drifts slowly over the picture (animated).

1. In the **Effects** panel type the effect's name in the search field, or open **Video Effects ▸
   User**.
2. Drag the effect onto a clip, or select the clip and double-click the effect (command
   `effects.apply`).
3. Set its controls in **Effect Controls** like any other effect; they can be keyframed.

Shader effects run **only on the GPU**. Where a frame renders on the CPU (no usable graphics card,
or the software renderer chosen in preferences), they leave the picture unchanged, and the User folder
says so. Exports in this fork use the GPU too.

## Writing your own effect

> id: shader-effects.write | does: a .wgsl file in the Effects folder becomes an effect with its own controls | cmd: effects.reloadUser | resolve: writing a DCTL | premiere: — | when: you can write a little shader code and want a look of your own | see: shader-effects.use, shader-effects.trouble

**[Fork]**

An effect is one text file ending in `.wgsl` (WebGPU's shading language) in the **Effects** folder
of FilmCraft's data folder. Click **Open Effects Folder** at the bottom of **Video Effects ▸ User**
to open it (it is created when missing), add or edit a file, then click **Reload** (command
`effects.reloadUser`). The file name becomes the effect's id: `My Look.wgsl` is `user.my-look`.

The file has a few comment lines that describe the effect and its controls, then a function
`effect` that returns the colour of one output pixel:

```wgsl
// @effect "Warm Spot"
// @category "My looks"
// @param amount: float = 0.5 [0.0, 1.0] "Amount"
// @param tint: color = #ffb060 "Color"
// @param centre: point = auto "Center"
fn effect(px: vec2<f32>) -> vec4<f32> {
    let c = sample(px);
    let r = distance(px, P.centre) / length(size);
    return vec4(mix(c.rgb, P.tint, P.amount * (1.0 - r)), c.a);
}
```

- **Controls** (`// @param`): `float` with a default and range, `bool` (`true` / `false`), `color`
  (`#rrggbb`) and `point` (`auto` is the middle of the clip, or `x, y` in pixels). At most 16.
  Read them as `P.<name>`.
- **What you can use**: `px` (the pixel, centres at .5), `size` (width and height in pixels),
  `time` (seconds since the clip started), `load(vec2<i32>)` (one pixel) and `sample(vec2<f32>)`
  (smoothly between pixels; outside the picture the nearest edge pixel).
- **Colours** are display colours (sRGB, 0 to 1) with the transparency separate, the way a paint
  program shows them; FilmCraft converts to and from its working colours around your code and keeps
  the result between 0 and 1.
- Not allowed (the app owns them): your own bindings, entry points, storage or workgroup memory,
  and names starting with `fx_`. A file can be at most 256 KB.

## When an effect doesn't work

> id: shader-effects.trouble | does: explains broken files, slow effects turned off, quarantine and missing effects | cmd: effects.listUser, effects.reloadUser, effects.unquarantine | resolve: — | premiere: — | when: an effect shows a warning sign, is missing, or stopped working | see: shader-effects.write

**[Fork]**

- **A file with a mistake** is listed with a warning sign under **Video Effects ▸ User**; hover it
  to read the message, which names the line in your file. Fix it and click **Reload**. Command
  `effects.listUser` gives every file's outcome.
- **Too slow**: an effect that needs more than 50 ms per picture at 1080p (more for bigger pictures)
  three times in a row is turned off until FilmCraft restarts, with a message in the status line.
  The picture then shows without it.
- **The graphics card stopped** while an effect ran (a shader that never finishes can hang the
  card until the driver resets it): FilmCraft remembers that effect and loads it as broken next
  time ("the GPU stopped while this effect was running"). Change the file, which turns it into a new
  version, or run `effects.unquarantine` (with `id` for one effect, without for all) to try it
  again.
- **Missing effect**: a project that uses an effect whose file is gone keeps it with its values;
  **Effect Controls** shows a "Missing effect" row and the picture renders without it. Put the file
  back and click **Reload**.

Unlike Resolve's DCTL, which writes colour maths in a C-like language, these are WGSL compute
snippets; the two can't be converted automatically.
