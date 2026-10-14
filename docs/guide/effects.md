# Effects, keyframes, transitions and presets

**[FilmCraft]** · applying and arranging clip effects, animating them with keyframes,
transitions between clips, and effect presets; modelled on Premiere Pro's Effects panel and
Effect Controls; written for AI-PLAN AI3-PREP (feature pages)

## Applying effects

> id: effects.apply | does: adds a video or audio effect to the selected clips and arranges, switches off or removes it | cmd: effects.list, effects.apply, effects.move, effects.toggleEnabled, effects.remove, effects.reset | resolve: Effects Library ▸ OpenFX / Audio FX, Inspector | premiere: Effects panel, Effect Controls | when: blur, sharpen, distort, keying, audio repair on a clip | see: effects.keyframes, effects.presets

**[FilmCraft]**

1. Find the effect in the Effects panel (Window ▸ Effects; `effects.list` lists them by kind and
   folder) and drag it onto a clip, or select clips and run `effects.apply` with the effect's
   name.
2. Its controls appear in **Effect Controls** (the **Inspector** in the Resolve workflow) under
   the clip's built-in Motion, Opacity and Time Remapping (video) or Volume and Panner (audio).
3. Effects run top to bottom: drag one up or down to change the order (`effects.move`). The
   switch next to its name bypasses it (`effects.toggleEnabled`); `effects.reset` puts its
   parameters back to their defaults; `effects.remove` deletes it.

Text and shape layers aren't effects you apply: make them with Graphics ▸ New Layer
([graphics.md](graphics.md)). Grading has its own tools: the node grader and Lumetri
([color/README.md](color/README.md)).

## Changing values and keyframes

> id: effects.keyframes | does: sets effect parameters and animates them over time with keyframes and their interpolation | cmd: effects.setParam, effects.resetParam, effects.toggleAnimation, effects.addKeyframe, effects.setKeyframe, effects.moveKeyframe, effects.deleteKeyframe, effects.setInterpolation | resolve: Inspector keyframes, Keyframe editor | premiere: Effect Controls keyframes | when: a move, a fade, a blur that grows | see: effects.apply

**[FilmCraft]**

1. Drag a value or type it (`effects.setParam`); the reset arrow puts it back
   (`effects.resetParam`).
2. Click the stopwatch to animate the parameter (`effects.toggleAnimation`): a keyframe is added
   at the playhead, and from now on every change at another time adds or changes a keyframe there
   (`effects.addKeyframe`, `effects.setKeyframe`).
3. Move keyframes in time (`effects.moveKeyframe`) or delete them (`effects.deleteKeyframe`);
   clicking the stopwatch again removes all of them and keeps the value at the playhead.
4. Right-click a keyframe for its interpolation (`effects.setInterpolation`): **Linear**,
   **Bezier**, **Auto Bezier**, **Continuous Bezier**, **Hold**, **Ease In**, **Ease Out**.

| Interpolation | Feels like | Use it for |
|---|---|---|
| **Linear** | Mechanical, constant speed | Fades, wipes, anything that should feel exact |
| **Ease In / Ease Out**, **Bezier** | Natural: speeds up and slows down | Moves, zooms, titles flying in |
| **Hold** | A jump, no motion between keyframes | Steps, cuts inside an effect, flashing |

## Transitions

> id: effects.transitions | does: puts a dissolve, wipe or audio crossfade on an edit point and sets the default one | cmd: sequence.applyVideoTransition, sequence.applyAudioTransition, sequence.applyDefaultTransitionsToSelection, sequence.setTransition, sequence.removeTransition, effects.setDefaultTransition | resolve: Video and Audio Transitions | premiere: Video Transitions, Audio Transitions, Apply Default Transitions | when: softening a cut, a passage of time, smoothing an audio edit | see: effects.apply

**[FilmCraft]**

1. Park the playhead on an edit (or select clips) and apply the default video transition
   (`sequence.applyVideoTransition`) or audio crossfade (`sequence.applyAudioTransition`);
   **Apply Default Transitions to Selection** (`sequence.applyDefaultTransitionsToSelection`)
   does every edit of the selected clips at once.
2. Or drag a transition from Effects ▸ Video / Audio Transitions onto the edit.
3. Select it to change its length and alignment (`sequence.setTransition`) or delete it
   (`sequence.removeTransition`). Which transition is the default is set with
   `effects.setDefaultTransition`; the default lengths are preferences
   (`timeline.videoTransitionDuration`, `timeline.audioTransitionDuration`).

A transition needs media beyond the edit (handles) on both clips; with too little, it gets
shorter. Audio crossfades: **Constant Power** keeps the loudness steady across the edit and is
the usual choice; **Constant Gain** is a straight linear fade, which can sound like a small dip in
the middle; **Exponential Fade** suits fades to and from silence.

## Effect presets

> id: effects.presets | does: saves configured effects with their keyframes and masks under a name and applies them to other clips | cmd: presets.list, presets.save, presets.apply, presets.rename, presets.delete, presets.export, presets.import | resolve: Effects favourites, PowerGrades (for grades) | premiere: Effects ▸ Presets bin | when: the same blur, zoom or audio chain on many clips | see: packs.what, effects.keyframes

**[FilmCraft]**

1. Select a clip, choose its effects in Effect Controls and **Save Preset…** (`presets.save`)
   with a name and how to treat keyframes:
   - **Scale**: keyframes stretch over the length of the clip they are applied to;
   - **Anchor to In Point** / **Anchor to Out Point**: they keep their distance from the
     clip's start or end (an intro or outro animation);
   - **none**: only the values at the playhead are saved.
2. Apply it from the Presets bin (Effects ▸ Presets) or with `presets.apply`: positions and mask
   shapes are scaled to the clip's frame size, so a preset made on 4K lands in the same place on
   HD. Motion, Opacity and Time Remapping in a preset replace the clip's own.
3. Rename, delete, export or import presets (`presets.rename`, `presets.delete`,
   `presets.export`, `presets.import`); to share presets with LUTs and graphics templates, use a
   look pack ([packs.md](packs.md)).
