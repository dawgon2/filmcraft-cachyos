# Creative gaps (roadmap list)

What the creative set (`creative-*.toml` / `.jsonl`, AI-PLAN AI4 item 5c) found Revolution can't do
yet, one line per gap, with the families that ran into it and the workaround the answers give.
Gaps are data: an admitted-gap family is a valid question, and this list is the roadmap input.

| Gap | Families | Workaround the answers give |
|---|---|---|
| Object removal / fill behind a moving subject | `ghost-figure` | clean plate (a frame or take without the subject), tripod shots only |
| Sky replacement (new clouds) | `stormy-sky` | qualifier + gradient window: darken, desaturate, add contrast |
| Word-by-word caption pop animation | `word-by-word-captions` | split captions into short ones, or keyframe caption graphics by hand |
| Wall-clock / countdown text (Metadata & Timecode Burn-in counts timecode up only) | `security-camera-look`, `workout-timer` | Timecode Burn-in for a ticking counter; keyframed text changes for a clock or countdown |
| Automatic object removal / content-aware fill | `remove-passer-by` | clean plate through a tracked mask, or blur / Mosaic the object |
| Deflicker (timelapse brightness jumps) | `photo-timelapse` | keyframe Exposure / Offset for bad frames; temporal NR for fine changes |
| Repairing distorted wind gusts | `windy-interview-rescue` | cut around them, cover with B-roll |
| Automatic subject cut-out (text behind a person) | `text-behind-subject` | a copy of the shot above the title through a hand-drawn, tracked mask (or a key on a plain sky) |
| Smooth speed ramps (Time Remapping keyframes are kept but not played) | `speed-ramp-action` | stepped ramp: Add Edit + Speed / Duration per piece, Optical Flow, short dissolves |
| 3-D stabilising (rolling-shutter jelly, parallax) | `steady-handheld` | Warp Stabilizer (2-D); lower Smoothness |
| Recovering clipped highlights / sky replacement | `blown-sky-rescue` | leave clipped areas white; soft clip, curve shoulder, gradient window on what has detail |
| Dedicated beauty / face-refinement tool | `skin-soften-beauty` | skin qualifier + low spatial NR inside the key |
| Layer mixer node (bleach bypass) | `gritty-war-look` | a desaturated duplicate on V2 with the Overlay blend mode, or lower saturation + more contrast on the look node |
| Grade versions (Local / Remote, A / B looks on one clip) | `client-look-options` | a Gallery still per look (`stills.grab`), compare with Wipe / Split, apply the chosen one (`stills.apply`) |
| Automatic colour-chart matching | `grey-card-balance` | white-balance picker on the chart's grey patch or a grey card (`grade.wb.pick`), node copied to the set-up's clips |
| Parallel nodes (several keys from the same untouched picture) | `independent-secondaries` | order the serial chain so each qualifier keys before the changes that would disturb it (`grade.moveNode`); limit each with a window |
| Printer-light step keys and shared nodes (one trim kept live across many shots) | `film-lab-printer-timing` | typed equal Offset steps per channel (`grade.wheel.offset`); a one-node reference grade copied with Append Grade (`grade.append`), re-appended after a change |
| A Compositor button for Explain comp (`comp.explain` is control channel / MCP only) | `learn-from-a-comp-template` | send `comp.explain` over MCP or the control channel; view each node's output with a double-click (`comp.view`) |
| Automatic sky replacement | `new-sky-key`, `stormy-sky` | key the sky (Color / Luma Key) over a new sky clip, tracked if the camera moves |
| Particles (snow, rain, confetti) | `snow-rain-overlay`, `confetti-burst` | stock overlay footage on black with a Screen / Add blend, or on green with Ultra Key; a `.wgsl` shader effect for simple generated patterns |
| Extruded 3-D text and a 3-D camera (3D Rotate moves flat layers in perspective) | `flying-3d-text` | 3D Rotate with Z Position / rotation keyframes, or render the 3-D part elsewhere and import it |
| Audio-reactive effects / visualiser | `music-reactive-pulse` | markers on the beats + keyframes or a preset; the Pulse shader at the song's tempo |
| Automatic person cut-out / rotoscoping | `glowing-outline`, `text-behind-subject` | Bézier masks keyframed and tracked by hand, or shoot on green |
| Real dolly zoom | `dolly-zoom` | Scale keyframes countering a camera move |
| AI style transfer (painted / cartoon looks) | `painterly-look` | Brush Strokes, Posterize, Find Edges, Roughen Edges filters |
| Depth map / automatic background blur | `fake-shallow-focus` | a blurred copy through a hand-drawn, tracked, inverted mask; Focus Blur |
| Text animation presets (kinetic type) | `kinetic-typography` | keyframes by hand, saved as a preset or template |
| 360° editing workflow / reframing 360 to flat (immersive effects exist, no guide steps) | `three-sixty-video` | VR Rotate Sphere / VR Projection keyframes, or reframe in the camera maker's app |
| Light wrap for composites | `ground-the-composite` | a blurred background copy masked to the subject's edge on Screen |
| Datamosh (codec smearing) | `datamosh-look` | Glitch shader / transition, Turbulent Displace, Echo |
| Paint / clone (healing) tool for spots and small blemishes | `sensor-dust-spot` | a small blur inside a feathered, tracked mask, or a clean patch from a copy of the shot, offset and masked; a preset carries the fix to other clips |
| Declipping (audio recorded too loud) | `distorted-loud-clip` | lower the gain so it gets no worse, use a second recording (camera mic) for the worst stretches, cover or cut them |

Resolved since first listed (the effect catalogue had them; see `docs/guide/effect-reference.md`): a built-in shake (**Camera Shake**, **Wiggle**), automatic reframing (**Auto Reframe**), frame-drop stutter (**Posterize Time**), a glow (**Wonder Glow**).
