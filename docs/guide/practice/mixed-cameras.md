# Practice notes: mixed cameras and difficult light

**[Standard]** · how colourists commonly handle footage from several cameras and awkward lighting,
one technique per note, with the Resolve / Premiere way and the Revolution way. Written in our own
words from common practice (AI-PLAN AI3 item 0a, 0c).

## Transform every camera into the same working space

> id: practice.mixed-cameras.per-camera-input | does: sets each camera's own input colour space so all of them land in one working space | resolve: per-clip Input Color Space | premiere: per-clip colour management | when: two or more camera models, log and non-log mixed (a log camera plus a phone or action camera on one timeline); setting up colour management per camera; one camera's clips look washed out or wrong next to the others before any grading | see: color.color-management.input

**[Standard]**

- **Revolution:** Interpret Footage per camera (`clip.interpretFootage`): F-Log2 for the X-M5,
  Rec. 709 for a phone, the right log for any other camera; one working space for the sequence.
- **Pitfalls:** one camera left on "auto" detecting the wrong space.

## Pick a hero camera and match to it

> id: practice.mixed-cameras.hero | does: grades the main camera first and matches the others to it | resolve: reference still from the A camera | premiere: Comparison View with the A camera | when: multi-camera shoots | see: practice.grading.match-order

**[Standard]**

- **Revolution:** grade the A camera, grab a still (`stills.grab`), Shot Match each other camera to
  it (`grade.shotMatch`), then check the scene (`grade.checkScene`).

## Match phones to a real camera

> id: practice.mixed-cameras.phone | does: brings phone footage closer to a cinema camera: less sharpening, softer contrast, matched skin | resolve: Shot Match, Blur, curves | premiere: Lumetri, Gaussian Blur | when: phone B-roll in a camera film; a phone's HDR-processed bright sky or highlights look crunchy or weird when pulled down | see: practice.mixed-cameras.hero

**[Standard]**

- **Common practice:** lower contrast and saturation, a touch of blur to fight phone sharpening,
  match skin on the vectorscope, add the same grain as the main camera.
- **Revolution:** check the phone clip's input colour space first (`clip.interpretFootage`; phones
  record Rec. 709 or HLG), then Shot Match, Contrast down, a light Gaussian Blur effect, grain on
  the last node.
- **Pitfalls:** phones' HDR-processed highlights don't roll off like a camera's; don't push them.

## A grey card or white balance reference

> id: practice.mixed-cameras.grey-card | does: records a grey card at the start of each set-up to make balancing exact later | resolve: white balance picker on the card | premiere: Lumetri WB Selector | when: shoots where the light changes; matching to a grey card or a colour checker chart (automatic chart match) | see: color.primaries.white-balance

**[Standard]**

- **Common practice:** worth it: record a few seconds of the card in each set-up (each new light
  or camera setting), in the subject's light.
- **Revolution:** the white-balance picker (`grade.wb.pick`) on the card's frame; copy the node to
  the set-up's other clips (`grade.copy`, `grade.paste`).
- **Colour charts:** automatic chart matching isn't offered yet.

## Mixed light sources in one shot

> id: practice.mixed-cameras.mixed-light | does: balances for the main light and treats the other light source with a window or qualifier | resolve: windows / qualifiers on the second source | premiere: Lumetri masks | when: window light plus tungsten lamps, LED panels of different colour | see: practice.grading.balance-first

**[Standard]**

- **Common practice:** balance for the subject's key light; reduce the cast of the other source
  where it falls with a soft window or a hue qualifier; leave some of it for realism.
- **Revolution:** balance node with the white-balance picker (`grade.wb.pick`) on something
  neutral in the key light; then a new node with a soft window over the part the other light
  hits, and Temperature part of the way back.
- **Pitfalls:** fully neutralising both makes the scene look fake.

## Fluorescent and LED green / magenta casts

> id: practice.mixed-cameras.green-cast | does: removes the green or magenta tint from fluorescent or cheap LED light with Tint | resolve: Tint, Offset | premiere: Lumetri Tint | when: offices, shops, cheap LEDs | see: color.primaries.white-balance

**[Standard]**

- **Revolution:** Tint towards magenta (or green) on the balance node, check skin on the
  vectorscope.
- **Pitfalls:** flickering lights change the cast frame to frame; Temporal NR doesn't fix flicker.

## Backlit subjects

> id: practice.mixed-cameras.backlit | does: brings a backlit face up without blowing out the background | resolve: window on the face + Gamma / Gain | premiere: Lumetri mask + Exposure | when: interviews against windows, sunsets behind people | see: color.recipes.face

**[Standard]**

- **Revolution:** a soft ellipse window on the face (tracked), Gamma up a little, and a highlight
  shoulder for the background (clipped parts can't be recovered).
- **Pitfalls:** a visible halo around the head from a hard window edge.

## Underexposed log footage

> id: practice.mixed-cameras.underexposed | does: rescues dark log shots: exposure with Offset, noise reduction first, careful contrast | resolve: Offset, Spatial / Temporal NR | premiere: Exposure, noise reduction | when: shots a stop or more too dark | see: color.noise.steps

**[Standard]**

- **Revolution:** a noise reduction node first (`grade.insertNode`, spatial NR) or temporal NR on the
  clip, then Offset / Gain for exposure, gentle contrast; grain to hide what remains.
- **Pitfalls:** lifting shadows of 8-bit footage reveals blocks; log 10-bit holds up much better.

## Overexposed skies and highlights

> id: practice.mixed-cameras.overexposed | does: recovers what's left of bright highlights and hides what's gone | resolve: Highlight wheel, Soft Clip | premiere: Lumetri Highlights / Whites | when: skies and windows near clipping | see: color.finishing.soft-clip

**[Standard]**

- **Revolution:** Highlight soft clip and a master-curve shoulder; a gradient window can darken a
  sky that still has detail.
- **Pitfalls:** clipped areas hold no detail to recover and turn grey when pulled down; leave
  them white.

## Colour continuity across a day

> id: practice.mixed-cameras.day-continuity | does: keeps a scene shot over hours consistent as the sun moves | resolve: shot-by-shot match, group grades | premiere: shot matching | when: interviews and scenes shot over a long time, from noon to sunset; the light keeps changing between answers or takes | see: color.matching.check-scene

**[Standard]**

- **Revolution:** grade the scene's best-lit shot, match the rest to it, and use **Check scene**
  (`grade.checkScene`) to find the drifting ones.
- **Pitfalls:** making late-day shots fully neutral; a little warmth progression reads naturally.

## Drone and action-camera footage

> id: practice.mixed-cameras.drone | does: matches small-sensor footage: tame sharpening and saturation, fix lens distortion, match contrast | resolve: Shot Match, lens correction | premiere: Lens Distortion, Lumetri | when: drone and action cameras in a film | see: practice.mixed-cameras.phone

**[Standard]**

- **Revolution:** Shot Match to the main camera, saturation down a little, grain to match texture;
  correct lens distortion with the **Lens Distortion** effect if the footage needs it.
- **Pitfalls:** "flat" drone profiles with 8-bit compression band when graded hard.

## Interviews: two cameras, one look

> id: practice.mixed-cameras.interview | does: matches a wide and a close interview camera so cuts between them are invisible | resolve: Shot Match + skin match | premiere: Color Match | when: two-camera interviews | see: practice.mixed-cameras.hero, editing.multicam

**[Standard]**

- **Revolution:** match the close-up to the wide (or the other way), skin first; cut with multicam
  ([editing.md](../editing.md)) after both are graded, or grade the multicam angles' sources.
- **Pitfalls:** matching backgrounds instead of the face.

## Night streets and practical lights

> id: practice.mixed-cameras.night-street | does: keeps city lights rich and skin natural in high-ISO night footage | resolve: NR, qualifiers on lights | premiere: noise reduction, HSL | when: night exteriors | see: practice.looks.neon

**[Standard]**

- **Revolution:** temporal NR on the clip, blacks kept deep, sodium-orange lights partly
  neutralised with a qualifier if skin is the priority.
- **Pitfalls:** lifted blacks full of noise.

## Skin tones of very different depths in one frame

> id: practice.mixed-cameras.skin-range | does: exposes and balances for several people with very different skin depths | resolve: windows per face | premiere: masks per face | when: group shots, two-person interviews | see: practice.grading.skin-line

**[Standard]**

- **Common practice:** expose so the darkest skin keeps detail and the lightest doesn't clip; use
  a soft window per face for small exposure differences rather than a global move.
- **Revolution:** ellipse windows per face (tracked), small Gamma moves; all faces near the skin
  line, each at its own depth.
- **Pitfalls:** one global move that's right for one person and wrong for the other.

## Archival and old footage

> id: practice.mixed-cameras.archival | does: integrates old, low-quality footage without over-correcting it | resolve: light balance, NR, keep its character | premiere: Lumetri, noise reduction | when: documentaries using archive material; old VHS, 1990s or archive footage next to new 4K shots: clean it up or keep it as it is? | see: media.clips

**[Standard]**

- **Common practice:** balance gently, remove distracting noise or flicker, keep its age; match
  only brightness and contrast to the new footage.
- **Revolution:** interlaced sources: **Field Options…** (`clip.fieldOptions`) to deinterlace;
  Scale to Frame Size or Fit to frame for old aspect ratios (`clip.fitToFrame`).
- **Pitfalls:** making 1990s video look new looks fake.

## Screen content in shot

> id: practice.mixed-cameras.screens | does: handles phones and monitors in the picture: reflections, colour and replacement | resolve: Planar tracker, qualifiers | premiere: Corner Pin, masks | when: screens visible in a scene | see: tracking.screen

**[Standard]**

- **Revolution:** replace what's on the screen with Corner Pin's Track Surface
  ([tracking.md](../tracking.md)); otherwise a qualifier tames an over-blue screen glow.
- **Pitfalls:** screens flicker on camera at some shutter speeds; reshoot or replace them.
