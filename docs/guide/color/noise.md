# Noise reduction

**[Resolve-style]** · spatial noise reduction on a node (Phase G2.4) and temporal noise
reduction on the first node (fork Phase 6.7)

## Why

**[Resolve-style]**

Log footage such as F-Log2 is noisy in the shadows, and lifting the shadows in a grade makes the
noise more visible. Noise reduction smooths small random speckle while keeping edges sharp, so do
it *before* the correction that lifts the shadows.

There are two kinds. **Spatial** noise reduction looks at one frame: it averages each pixel with
similar pixels around it, which also softens fine detail. **Temporal** noise reduction looks at
the frames before and after: noise changes from frame to frame while the picture doesn't, so
averaging the same point over several frames removes noise and keeps detail. It is the better
tool for static or slowly moving shots; spatial is the fallback where things move fast.

## Steps

**[Resolve-style]**

1. Add a node at the **start** of the chain (`grade.insertNode`, **Insert Node Before** on the
   first node): noise reduction works best on the picture before contrast and saturation.
2. Open the **Noise** tab {ui:grade.tab.noise}.
3. Raise **NR luma** {ui:grade.param.nr.luma} for grainy brightness noise and **NR chroma**
   {ui:grade.param.nr.chroma} for coloured blotches. Start around 0.3 and compare with the node
   switched off (`grade.toggleNode`).
4. **NR radius** {ui:grade.param.nr.size} sets how far each pixel looks for similar neighbours
   (0.25 … 3.5 source pixels): larger is smoother and slower.
5. To clean only part of the picture (say the dark background), add a window to the same node
   ([secondaries.md](secondaries.md)): the noise reduction stays inside it. (A qualifier on the
   node does *not* limit the noise reduction, see Limits.)

## Temporal noise reduction

**[Fork]**

Temporal noise reduction is too heavy to run while playing, so it renders only where there is
time for it: at **export**, in the **render cache** ([render-cache.md](../render-cache.md)) and
on the **paused** Program monitor frame. While playing or scrubbing the monitor shows the clip
without it (from the render cache once that part is rendered).

1. Select the **first** node of the clip's grade (only the first active node's temporal settings
   are used, see Limits).
2. Open the **Noise** tab {ui:grade.tab.noise}; the temporal rows are under **Temporal**.
3. Raise **Temporal NR luma** {ui:grade.param.tnr.luma} (brightness noise) and **Temporal NR
   chroma** {ui:grade.param.tnr.chroma} (colour noise). 0.5 is a good start; 1.0 is strong.
4. **Temporal NR frames** {ui:grade.param.tnr.frames} is how many frames on each side are
   averaged: 1 (three frames in all) or 2 (five frames, cleaner and slower).
5. **Temporal NR motion** {ui:grade.param.tnr.motion} (0 … 1, default 0.5) is how much
   disagreement in the motion estimate is accepted. Lower it if moving edges leave a faint ghost;
   raise it for more cleaning on difficult but slow motion.
6. Pause the Program monitor on the shot and wait a moment: the paused frame renders with
   temporal noise reduction (the frame without it shows meanwhile). Each control has a reset
   button ({ui:grade.param.tnr.luma.reset} and the like).
7. The render bar marks the clip for rendering; let the render cache build it, or export.

Spatial and temporal noise reduction combine: temporal runs first, then the node's spatial NR,
then its correction. With temporal NR on, keep spatial NR low (0.1–0.2) or off.

## How it works

**[Standard]** **[Fork]**

Spatial: an edge-preserving *bilateral* filter: each pixel is averaged with its neighbours,
weighted by distance and by how similar they are, so a real edge (a big difference) is left alone
while small random differences are averaged out. Brightness (luma) and colour (two colour
differences) are filtered separately with their own strengths. The filter size follows the
picture size, so a reduced-resolution preview looks like the full-size export.

Temporal (own code from published methods): the motion between the frame and each neighbour is
estimated with dense pyramidal Lucas–Kanade optical flow (Lucas & Kanade 1981; Bouguet 2001) on a
small brightness copy (at most 640 pixels wide), both ways. Each neighbour is warped onto the
frame along that motion and blended in with a weight that falls as it differs from the frame — a
motion-compensated temporal filter (Dubois & Sabri 1984) with per-pixel range weights, i.e. a
temporal bilateral filter (Bennett & McMillan 2005). Where the forward and backward motion
disagree (occlusions, fast motion) the neighbour counts less (forward–backward consistency,
Kalal et al. 2010). The neighbours are the clip's own frames: never across a cut, following its
speed, reverse and holds (a hold has no other frame to average, so nothing changes there).
Motion between neighbouring frames is computed once and reused.

## Limits

**[Fork]**

- Temporal noise reduction renders only at export, in the render cache and on the paused Program
  frame, never while playing or scrubbing. Export Frame does not include it yet.
- The paused frame matches the export exactly only at playback resolution **Full**
  {ui:program.resolution}: at 1/2 or 1/4 the motion and the blend work on the smaller picture,
  so the cleaning is close but not identical.
- On a retimed clip with **Frame Blending** or **Optical Flow**, frames made between two source
  frames get no temporal NR, and such frames are not used as neighbours (the motion is measured
  between real source frames). Frame Sampling (the default) is not affected.
- During a transition the clips keep their temporal NR: the frames shown from their handles have
  neighbours within the transition.
- Temporal noise reduction runs on the **first active node** of a grade only (a provisional rule
  while the owner decides, question Q9); on other nodes the Noise tab says the values are ignored.
  It applies to the whole picture: the node's windows and qualifier don't limit it.
- It works on media clips (and subclips); on nested sequences, graphics and adjustment layers it
  does nothing.
- It is slow: at 6.2K expect an export several times slower with it on (motion estimation on the
  CPU, the blend on the GPU). Use 1 frame each side unless 2 is visibly cleaner.
- Spatial: at full strength, faint colour edges soften (a property of this kind of filter); keep
  NR chroma moderate.
- Spatial: the node's qualifier doesn't limit the noise reduction (it keys the cleaned picture);
  windows do.
- Spatial costs GPU time: on the RX 6700 XT about 3–20 ms per 1080p frame depending on the
  radius, much more on the Intel GPU. Use lower playback resolution while grading.

## If you know Resolve

**[Resolve-style]**

Resolve's Spatial NR palette has more modes (faster / better, mode per channel, blend); here it
is one filter with two strengths and a radius. Resolve's Temporal NR is per node too and has more
controls (motion estimation type, motion range, frame count up to 5, thresholds per channel,
blend); here it has two strengths, 1 or 2 frames each side and one motion tolerance, and it runs
only at export, in the render cache and on the paused frame (Resolve renders it live when the
system keeps up). Parameters for scripts: `nr.luma`, `nr.chroma` (0…1), `nr.size` (σ in source
pixels), `tnr.luma`, `tnr.chroma` (0…1), `tnr.frames` (1 or 2), `tnr.motion` (0…1), set with
`grade.setParam`.
