# Noise reduction

**[Resolve-style]** · spatial noise reduction on a node (Phase G2.4)

## Why

**[Resolve-style]**

Log footage such as F-Log2 is noisy in the shadows, and lifting the shadows in a grade makes the
noise more visible. Noise reduction smooths small random speckle while keeping edges sharp, so do
it *before* the correction that lifts the shadows.

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

## How it works

**[Standard]** **[Fork]**

An edge-preserving *bilateral* filter: each pixel is averaged with its neighbours, weighted by
distance and by how similar they are, so a real edge (a big difference) is left alone while
small random differences are averaged out. Brightness (luma) and colour (two colour differences)
are filtered separately with their own strengths. The filter size follows the picture size, so a
reduced-resolution preview looks like the full-size export.

## Limits

**[Fork]**

- Spatial only (one frame at a time). Temporal noise reduction (using neighbouring frames, as
  Resolve Studio offers) is not built yet: it is planned for after the render cache (Phase 6),
  where it will apply in the cache and at export.
- At full strength, faint colour edges soften (a property of this kind of filter); keep NR chroma
  moderate.
- The node's qualifier doesn't limit the noise reduction (it keys the cleaned picture); windows
  do.
- It costs GPU time: on the RX 6700 XT about 3–20 ms per 1080p frame depending on the radius,
  much more on the Intel GPU. Use lower playback resolution while grading.

## If you know Resolve

**[Resolve-style]**

Resolve's Spatial NR palette has more modes (faster / better, mode per channel, blend); here it
is one filter with two strengths and a radius. Parameters for scripts: `nr.luma`, `nr.chroma`
(0…1), `nr.size` (σ in source pixels), set with `grade.setParam`.
