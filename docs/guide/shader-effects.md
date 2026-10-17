# Shader effects

**[Fork]** · bundled examples and your own WGSL effects; written for Phase Y3.4

## Using a shader effect

> id: shader-effects.use | does: applies a bundled or user-written GPU shader effect to a clip | cmd: effects.apply, effects.listUser | resolve: DCTL effects (a different language; files don't carry over) | premiere: — (third-party plugins) | when: a stylised look no built-in effect gives | see: shader-effects.write, shader-effects.trouble

**[Fork]**

A **shader effect** is a small program that runs on the graphics card for every pixel of a clip.
Revolution ships four examples and loads your own from a folder; they appear in the **Effects** panel
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
of Revolution's data folder. Click **Open Effects Folder** at the bottom of **Video Effects ▸ User**
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
  program shows them; Revolution converts to and from its working colours around your code and keeps
  the result between 0 and 1.
- Not allowed (the app owns them): your own bindings, entry points, storage or workgroup memory,
  and names starting with `fx_`. A file can be at most 256 KB.

## Recipes: three effects to copy

> id: shader-effects.recipes | does: three short example effects to copy and change: a soft vignette, a two-colour duotone and a pulse | cmd: effects.reloadUser | resolve: a small DCTL | premiere: — | when: you want a starting point for your own shader effect | see: shader-effects.write, shader-effects.trouble

**[Fork]**

Why start from a recipe: a working file is easier to change than an empty one, and each of these
shows one idea (distance from the middle, a colour from brightness, change over time). To use one:

1. Click **Open Effects Folder** at the bottom of **Video Effects ▸ User**.
2. Make a new text file there, for example `Soft Vignette.wgsl`, and paste the recipe into it.
3. Click **Reload** (command `effects.reloadUser`). The effect appears under **Video Effects ▸ User
   ▸ My looks**; drag it onto a clip.
4. Change a number or a line, save, and click **Reload** again to see the result.

**Soft vignette**: darkens the edges. **Size** is where the darkening starts (0 is the middle,
1 the corners), **Softness** how gradually it fades in.

```wgsl
// @effect "Soft Vignette"
// @category "My looks"
// @param amount: float = 0.6 [0.0, 1.0] "Amount"
// @param radius: float = 0.5 [0.0, 1.0] "Size"
// @param softness: float = 0.4 [0.01, 1.0] "Softness"
fn effect(px: vec2<f32>) -> vec4<f32> {
    let c = sample(px);
    // 0 in the middle of the picture, 1 in its corners
    let d = distance(px, size * 0.5) / (length(size) * 0.5);
    let edge = smoothstep(P.radius, P.radius + P.softness, d);
    return vec4(c.rgb * (1.0 - P.amount * edge), c.a);
}
```

**Duotone**: replaces the colours with a blend of two, chosen by how bright each pixel is: dark
parts take **Shadows**, bright parts **Highlights**.

```wgsl
// @effect "Duotone"
// @category "My looks"
// @param shadows: color = #1b2a6b "Shadows"
// @param highlights: color = #ffd27a "Highlights"
// @param strength: float = 1.0 [0.0, 1.0] "Amount"
fn effect(px: vec2<f32>) -> vec4<f32> {
    let c = sample(px);
    // brightness with the usual Rec. 709 weights
    let y = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
    let tone = mix(P.shadows, P.highlights, y);
    return vec4(mix(c.rgb, tone, P.strength), c.a);
}
```

**Pulse**: brightens and dims the picture in a steady beat. It uses `time`, so it moves on its own
during playback without keyframes.

```wgsl
// @effect "Pulse"
// @category "My looks"
// @param speed: float = 1.0 [0.1, 10.0] "Pulses per second"
// @param depth: float = 0.3 [0.0, 1.0] "Depth"
fn effect(px: vec2<f32>) -> vec4<f32> {
    let c = sample(px);
    // 0 → 1 → 0 once per pulse
    let wave = 0.5 - 0.5 * cos(time * P.speed * 6.2831853);
    return vec4(c.rgb * (1.0 + P.depth * wave), c.a);
}
```

## When an effect doesn't work

> id: shader-effects.trouble | does: explains broken files, slow effects turned off, quarantine and missing effects | cmd: effects.listUser, effects.reloadUser, effects.unquarantine | resolve: — | premiere: — | when: an effect shows a warning sign, is missing, or stopped working; a custom effect switched itself off during playback (too slow) | see: shader-effects.write

**[Fork]**

- **A file with a mistake** is listed with a warning sign under **Video Effects ▸ User**; hover it
  to read the message, which names the line in your file. Fix it and click **Reload**. Command
  `effects.listUser` gives every file's outcome.
- **Too slow**: an effect that needs more than 50 ms per picture at 1080p (more for bigger pictures)
  three times in a row is turned off until Revolution restarts, with a message in the status line.
  The picture then shows without it.
- **The graphics card stopped** while an effect ran (a shader that never finishes can hang the
  card until the driver resets it): Revolution remembers that effect and loads it as broken next
  time ("the GPU stopped while this effect was running"). Change the file, which turns it into a new
  version, or run `effects.unquarantine` (with `id` for one effect, without for all) to try it
  again.
- **Missing effect**: a project that uses an effect whose file is gone keeps it with its values;
  **Effect Controls** shows a "Missing effect" row and the picture renders without it. Put the file
  back and click **Reload**.

Unlike Resolve's DCTL, which writes colour maths in a C-like language, these are WGSL compute
snippets; the two can't be converted automatically.
