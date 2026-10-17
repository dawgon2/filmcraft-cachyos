# Picture-model tune: review sample (AI4 item 3, OWNER-QUESTIONS Q44)

**Part 1: synthetic frames** (made with known lighting by `crates/engine/examples/vision_traindata.rs`,
seed 1, 2 133 training + 229 validation frames; 638 candidates dropped because their exposure or
white-balance words disagreed with Revolution's own scopes). The downloaded open footage (incl. log)
is part 2, added when it is fetched. Frames are local, never in git: they are in
`~/Apps/filmcraft-train/runs/vision-data-v1/` (paths below are relative to that folder).

Each label is what the picture model should say when asked that question about that frame: one or
two plain sentences, what is visible, no advice. "Measured" is Revolution's own fact sheet for the
frame (scene-linear, after the input transform for log frames). To review: open a frame, read its
label, tick or note what's wrong; then say "Q44 reviewed". **The tune doesn't wait for this**
(FORK-PLAN §1a.96 (c)); wrong labels you find are fixed and the tune is re-run.


## v00000-sunset

- Frame: `frames/v00000-sunset.png` (sunset, display)
- Question: how does the sky look?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. The sky is a sunset sky, purple above and orange at the horizon. Also, it is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 1.3 stops above middle grey; 47% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (38% of the frame) look slightly magenta tint, about 6000 K. / Saturation: high. / Skin-tone colours (faces, or any warm surface): 7% of the frame, about 17° towards red / magenta from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00080-sunset

- Frame: `frames/v00080-sunset.png` (sunset, display)
- Question: how does the sky look?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. The sky is a sunset sky, purple above and orange at the horizon. Also, it is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 1.6 stops above middle grey; 52% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (38% of the frame) look slightly warm (towards orange), magenta tint, about 5100 K. / Saturation: high. / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 14° towards red / magenta from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00151-interior-lamp

- Frame: `frames/v00151-interior-lamp.png` (interior-lamp, display)
- Question: is the white balance off?
- Label: An interior: a warm orange table lamp on the right lights a plain wall. The neutral areas look neutral, no clear colour cast.
- Measured: Exposure: mids about 0.3 stops below middle grey; 3% clipped highlights, 0% crushed blacks; contrast 3.3 stops (5–95 %). / White balance: near-neutral areas (84% of the frame) look slightly warm (towards orange), slightly magenta tint, about 5400 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 45% of the frame, about 10° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00223-sunset

- Frame: `frames/v00223-sunset.png` (sunset, Fujifilm F-Log2)
- Question: what light is in this shot?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. Also, the colours are very saturated. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.5 stops below middle grey; 14% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (38% of the frame) look slightly magenta tint, about 5200 K. / Saturation: high. / Skin-tone colours (faces, or any warm surface): 44% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00293-interview

- Frame: `frames/v00293-interview.png` (interview, ARRI LogC3)
- Question: why does my footage look so flat and grey?
- Label: An interview against a plain grey backdrop, the key light from the right. It looks flat and washed out, like log footage as the camera recorded it, before any input transform. Also, it is overexposed: bright overall.
- Measured: Exposure: mids about 1.8 stops above middle grey; 4% clipped highlights, 0% crushed blacks; contrast 1.6 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look slightly cool (towards blue), slightly magenta tint, about 6600 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v00369-overcast

- Frame: `frames/v00369-overcast.png` (overcast, display)
- Question: is this shot exposed right?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. The exposure looks about right.
- Measured: Exposure: mids about 0.5 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look neutral, about 6800 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00444-overcast

- Frame: `frames/v00444-overcast.png` (overcast, display)
- Question: is the white balance off?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. The neutral areas show a magenta tint.
- Measured: Exposure: mids about 0.9 stops above middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look slightly magenta tint, about 6400 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 10° towards red / magenta from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00524-night-street

- Frame: `frames/v00524-night-street.png` (night-street, display)
- Question: what light is in this shot?
- Label: A night street: orange street lamps and a pink neon sign, the rest close to black.
- Measured: Exposure: mids about 3.0 stops below middle grey; 3% clipped highlights, 0% crushed blacks; contrast 5.0 stops (5–95 %). / White balance: near-neutral areas (33% of the frame) look warm (towards orange), magenta tint, about 4100 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 37% of the frame, about 14° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00594-interview

- Frame: `frames/v00594-interview.png` (interview, display)
- Question: is this shot exposed right?
- Label: An interview against a plain grey backdrop, the key light from the left. It is overexposed: bright, with clipped white highlights. Also, there is a warm orange cast over everything.
- Measured: Exposure: mids about 1.9 stops above middle grey; 4% clipped highlights, 0% crushed blacks; contrast 1.1 stops (5–95 %). / White balance: near-neutral areas (95% of the frame) look very warm (towards orange), magenta tint, about 5100 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 86% of the frame, about 20° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00667-sunset

- Frame: `frames/v00667-sunset.png` (sunset, display)
- Question: is this shot exposed right?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. The exposure looks about right.
- Measured: Exposure: mids about 0.5 stops below middle grey; 1% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (63% of the frame) look magenta tint, about 5000 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 43% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00748-overcast

- Frame: `frames/v00748-overcast.png` (overcast, display)
- Question: is the white balance off?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. The neutral areas look neutral, no clear colour cast. Also, it is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 3.7 stops above middle grey; 50% clipped highlights, 0% crushed blacks; contrast 2.9 stops (5–95 %). / White balance: near-neutral areas (50% of the frame) look warm (towards orange), green tint, about 5800 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v00815-overcast

- Frame: `frames/v00815-overcast.png` (overcast, S-Log3)
- Question: why does my footage look so flat and grey?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. It looks flat and washed out, like log footage as the camera recorded it, before any input transform. Also, there is a cool blue cast over everything.
- Measured: Exposure: mids about 0.6 stops above middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look slightly cool (towards blue), about 7100 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v00884-interview

- Frame: `frames/v00884-interview.png` (interview, display)
- Question: is this shot exposed right?
- Label: An interview against a plain grey backdrop, the key light from the right. It is underexposed: the picture is dark overall. Also, there is a warm orange cast over everything.
- Measured: Exposure: mids about 1.7 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.2 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look very warm (towards orange), slightly magenta tint, about 4100 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 77% of the frame, about 28° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v00966-daylight

- Frame: `frames/v00966-daylight.png` (daylight, Fujifilm F-Log2)
- Question: is the white balance off?
- Label: An exterior in daylight: a clear blue sky over green ground, the sun high. The neutral areas show a green tint. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 1.5 stops above middle grey; 51% clipped highlights, 0% crushed blacks; contrast 2.9 stops (5–95 %). / White balance: near-neutral areas (10% of the frame) look slightly cool (towards blue), green tint, about 8000 K. / Saturation: high. / Skin-tone colours (faces, or any warm surface): 3% of the frame, about 29° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01040-sunset

- Frame: `frames/v01040-sunset.png` (sunset, display)
- Question: is this shot exposed right?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. The exposure looks about right.
- Measured: Exposure: mids about 0.2 stops below middle grey; 8% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (39% of the frame) look slightly magenta tint, about 5900 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 43% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01109-overcast

- Frame: `frames/v01109-overcast.png` (overcast, display)
- Question: do the skin tones look right?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. There is one face, lit evenly, from the sky; the skin takes on a cool blue cast and a magenta tint. Also, it is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 2.8 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (43% of the frame) look slightly cool (towards blue), about 8500 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 2% of the frame, about 31° towards red / magenta from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01193-interview

- Frame: `frames/v01193-interview.png` (interview, DJI D-Log)
- Question: what light is in this shot?
- Label: An interview against a plain grey backdrop, the key light from the right. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.2 stops below middle grey; 1% clipped highlights, 0% crushed blacks; contrast 0.9 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look neutral, about 6300 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01266-bright-beach

- Frame: `frames/v01266-bright-beach.png` (bright-beach, display)
- Question: is the white balance off?
- Label: A bright beach: pale sand and a pale blue sky in strong sunlight. The neutral areas show a magenta tint. Also, it is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 4.0 stops above middle grey; 100% clipped highlights, 0% crushed blacks; contrast 0.5 stops (5–95 %). / White balance: too few near-neutral areas to judge. / Saturation: moderate. / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v01342-interview

- Frame: `frames/v01342-interview.png` (interview, display)
- Question: what light is in this shot?
- Label: An interview against a plain grey backdrop, the key light from the left. It is overexposed: bright overall.
- Measured: Exposure: mids about 2.1 stops above middle grey; 1% clipped highlights, 0% crushed blacks; contrast 1.4 stops (5–95 %). / White balance: near-neutral areas (98% of the frame) look neutral, about 6400 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 3% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01415-night-street

- Frame: `frames/v01415-night-street.png` (night-street, display)
- Question: is this shot exposed right?
- Label: A night street: orange street lamps and a pink neon sign, the rest close to black. It is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 0.8 stops below middle grey; 5% clipped highlights, 0% crushed blacks; contrast 5.1 stops (5–95 %). / White balance: near-neutral areas (81% of the frame) look slightly warm (towards orange), slightly magenta tint, about 5200 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 27% of the frame, about 13° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01488-interview

- Frame: `frames/v01488-interview.png` (interview, display)
- Question: what light is in this shot?
- Label: An interview against a plain grey backdrop, the key light from the left. It is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 3.0 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.1 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look neutral, about 6100 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v01561-interview

- Frame: `frames/v01561-interview.png` (interview, Fujifilm F-Log2)
- Question: do the skin tones look right?
- Label: An interview against a plain grey backdrop, the key light from the left. There is one face, lit from the left; the skin takes on a warm orange cast. Also, it is underexposed: the picture is dark overall. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 1.5 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.3 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look slightly warm (towards orange), slightly magenta tint, about 5500 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01637-overcast

- Frame: `frames/v01637-overcast.png` (overcast, display)
- Question: how does the sky look?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. The sky is a flat grey overcast sky. Also, there is a warm orange cast over everything, and it is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 1.1 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look slightly warm (towards orange), about 6100 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v01717-daylight

- Frame: `frames/v01717-daylight.png` (daylight, display)
- Question: is this shot exposed right?
- Label: An exterior in daylight: a clear blue sky over green ground, the sun high. The exposure looks about right.
- Measured: Exposure: mids about 1.0 stops above middle grey; 0% clipped highlights, 0% crushed blacks; contrast 2.9 stops (5–95 %). / White balance: near-neutral areas (11% of the frame) look slightly cool (towards blue), about 7100 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 5° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01792-overcast

- Frame: `frames/v01792-overcast.png` (overcast, display)
- Question: what light is in this shot?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. Also, there is a warm orange cast over everything.
- Measured: Exposure: mids about 0.6 stops above middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look warm (towards orange), slightly magenta tint, about 5000 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 52% of the frame, about 26° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v01871-bright-beach

- Frame: `frames/v01871-bright-beach.png` (bright-beach, display)
- Question: what light is in this shot?
- Label: A bright beach: pale sand and a pale blue sky in strong sunlight.
- Measured: Exposure: mids about 1.9 stops above middle grey; 0% clipped highlights, 0% crushed blacks; contrast 0.5 stops (5–95 %). / White balance: near-neutral areas (35% of the frame) look slightly cool (towards blue), slightly green tint, about 7100 K. / Saturation: moderate. / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v01945-interior-lamp

- Frame: `frames/v01945-interior-lamp.png` (interior-lamp, ARRI LogC4)
- Question: do the skin tones look right?
- Label: An interior: a warm orange table lamp on the left lights a plain wall. There is one face, lit from the left; the skin takes on a magenta tint. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.6 stops above middle grey; 4% clipped highlights, 0% crushed blacks; contrast 3.5 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look slightly magenta tint, about 6100 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 5% of the frame, about 7° towards red / magenta from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02013-night-street

- Frame: `frames/v02013-night-street.png` (night-street, display)
- Question: is this shot exposed right?
- Label: A night street: orange street lamps in a dark scene, most of it close to black. It is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 2.1 stops below middle grey; 4% clipped highlights, 0% crushed blacks; contrast 4.4 stops (5–95 %). / White balance: near-neutral areas (69% of the frame) look slightly magenta tint, about 5500 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 42% of the frame, about 13° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02087-interview

- Frame: `frames/v02087-interview.png` (interview, display)
- Question: is this shot exposed right?
- Label: An interview against a plain grey backdrop, the key light from the right. It is underexposed: the picture is dark overall. Also, there is a warm orange cast over everything.
- Measured: Exposure: mids about 1.3 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.5 stops (5–95 %). / White balance: near-neutral areas (97% of the frame) look warm (towards orange), slightly magenta tint, about 4900 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02163-interior-lamp

- Frame: `frames/v02163-interior-lamp.png` (interior-lamp, display)
- Question: do the skin tones look right?
- Label: An interior: a warm orange table lamp on the left lights a plain wall. There is one face, lit from the left; the skin takes on a magenta tint.
- Measured: Exposure: mids about 0.8 stops below middle grey; 2% clipped highlights, 0% crushed blacks; contrast 3.3 stops (5–95 %). / White balance: near-neutral areas (76% of the frame) look slightly warm (towards orange), magenta tint, about 5100 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 56% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02237-interior-lamp

- Frame: `frames/v02237-interior-lamp.png` (interior-lamp, display)
- Question: is this shot exposed right?
- Label: An interior: a warm orange table lamp on the right lights a plain wall. The exposure looks about right.
- Measured: Exposure: mids about 0.4 stops below middle grey; 1% clipped highlights, 0% crushed blacks; contrast 3.4 stops (5–95 %). / White balance: near-neutral areas (99% of the frame) look slightly warm (towards orange), about 6000 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 2% of the frame, about 20° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02315-interior-lamp

- Frame: `frames/v02315-interior-lamp.png` (interior-lamp, Canon Log 3)
- Question: do the skin tones look right?
- Label: An interior: a warm orange table lamp on the left lights a plain wall. There is one face, lit from the left; the skin takes on a magenta tint. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.0 stops below middle grey; 2% clipped highlights, 0% crushed blacks; contrast 3.5 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look slightly warm (towards orange), slightly magenta tint, about 5600 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 21% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02395-interview

- Frame: `frames/v02395-interview.png` (interview, display)
- Question: do the skin tones look right?
- Label: An interview against a plain grey backdrop, the key light from the right. There is one face, lit from the right, the skin close to natural. Also, it is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 2.3 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.5 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look neutral, about 6800 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 5° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02466-interior-lamp

- Frame: `frames/v02466-interior-lamp.png` (interior-lamp, display)
- Question: what light is in this shot?
- Label: An interior: a warm orange table lamp on the left lights a plain wall. It is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 2.2 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.4 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look slightly warm (towards orange), about 5700 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 18° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02544-interview

- Frame: `frames/v02544-interview.png` (interview, display)
- Question: what light is in this shot?
- Label: An interview against a plain grey backdrop, the key light from the right. It is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 1.4 stops above middle grey; 4% clipped highlights, 0% crushed blacks; contrast 1.1 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look slightly cool (towards blue), about 6900 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v02618-overcast

- Frame: `frames/v02618-overcast.png` (overcast, Fujifilm F-Log2)
- Question: is this shot exposed right?
- Label: An overcast exterior: a flat grey sky and soft, even light with no shadows. The exposure looks about right. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.1 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.6 stops (5–95 %). / White balance: near-neutral areas (100% of the frame) look neutral, about 6700 K. / Saturation: low (muted). / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

## v02689-interview

- Frame: `frames/v02689-interview.png` (interview, Canon Log 3)
- Question: is this shot exposed right?
- Label: An interview against a plain grey backdrop, the key light from the left. The exposure looks about right. It looks flat and washed out, like log footage as the camera recorded it, before any input transform.
- Measured: Exposure: mids about 0.9 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 1.1 stops (5–95 %). / White balance: near-neutral areas (97% of the frame) look slightly green tint, about 6800 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, about 12° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02766-interview

- Frame: `frames/v02766-interview.png` (interview, display)
- Question: is the white balance off?
- Label: An interview against a plain grey backdrop, the key light from the left. The neutral areas look neutral, no clear colour cast.
- Measured: Exposure: mids about 1.1 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 0.8 stops (5–95 %). / White balance: near-neutral areas (96% of the frame) look neutral, about 6500 K. / Saturation: low (muted). / Skin-tone colours (faces, or any warm surface): 4% of the frame, on the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02843-interior-lamp

- Frame: `frames/v02843-interior-lamp.png` (interior-lamp, display)
- Question: do the skin tones look right?
- Label: An interior: a warm orange table lamp on the right lights a plain wall. There is one face, lit from the right, the skin close to natural. Also, it is underexposed: the picture is dark overall.
- Measured: Exposure: mids about 1.9 stops below middle grey; 0% clipped highlights, 0% crushed blacks; contrast 3.4 stops (5–95 %). / White balance: near-neutral areas (74% of the frame) look warm (towards orange), about 5000 K. / Saturation: moderate. / Skin-tone colours (faces, or any warm surface): 35% of the frame, about 28° towards yellow from the skin line.
- Label right? [ ] yes  [ ] no, because:

## v02921-sunset

- Frame: `frames/v02921-sunset.png` (sunset, display)
- Question: is this shot exposed right?
- Label: A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette. It is overexposed: bright, with clipped white highlights.
- Measured: Exposure: mids about 2.3 stops above middle grey; 57% clipped highlights, 0% crushed blacks; contrast 5.6 stops (5–95 %). / White balance: near-neutral areas (38% of the frame) look slightly magenta tint, about 5800 K. / Saturation: high. / Skin-tone colours: little or none in the frame.
- Label right? [ ] yes  [ ] no, because:

