# Effect and transition reference

**[Revolution]** · every effect and transition in the **Effects** panel, by folder. Revolution uses
Premiere's names and folders, so a tutorial that names an effect finds it here. Apply one by
dragging it onto a clip or an edit, or by searching its name in the Effects panel; agents use
`effects.apply` with its name. The feature pages say how to use the main ones; this page only
names them, so nobody thinks one is missing. Every effect here applies and renders (checked by
the test suite), and a test keeps this list identical to the program's own catalogue.

## Colour, light and image

> id: effect-reference.video-image | does: names the colour, exposure, lighting and glow effects in Video Effects | cmd: effects.apply, effects.setParam | resolve: Color page tools, OpenFX | premiere: Video Effects (same names) | when: looking for an effect or transition by name | see: color.lumetri.which, effects.stylize

**[Revolution]**

- **Adjust** (Video Effects ▸ Adjust): Levels, ProcAmp, Extract, Lighting Effects.
- **Color Correction** (Video Effects ▸ Color Correction): Brightness & Contrast, Tint, Lumetri Color, ASC CDL, Video Limiter, Vignette.
- **Image Control** (Video Effects ▸ Image Control): Black & White, Color Pass, Gamma Correction, Invert, Channel Mix, Color Replace, Rounded Crop.
- **Noise & Grain** (Video Effects ▸ Noise & Grain): Noise.
- **Generate** (Video Effects ▸ Generate): 4-Color Gradient, Gradient.
- **Lights & Glows** (Video Effects ▸ Lights & Glows): Lens Flare, Echo Glow, Edge Glow, Glint, Light Leaks, RGB Split, Volumetric Rays, Wonder Glow.

## Blur, distortion, stylising and time

> id: effect-reference.video-stylize | does: names the blur, distortion, perspective, stylize and time effects in Video Effects | cmd: effects.apply, effects.setParam | resolve: OpenFX and Fusion tools | premiere: Video Effects (same names) | when: looking for an effect or transition by name | see: effects.stylize

**[Revolution]**

- **Blur & Sharpen** (Video Effects ▸ Blur & Sharpen): Gaussian Blur, Directional Blur, Sharpen, Unsharp Mask, Bokeh Blur, Channel Blur, Compound Blur, Focus Blur, Reduce Interlace Flicker.
- **Distort** (Video Effects ▸ Distort): Mirror, Lens Distortion, Twirl, Wave Warp, Corner Pin, Magnify, Spherize, Turbulent Displace, Warp Stabilizer.
- **Perspective** (Video Effects ▸ Perspective): Basic 3D, Drop Shadow, Long Shadow.
- **Stylize** (Video Effects ▸ Stylize): Posterize, Mosaic, Find Edges, Strobe Light, Brush Strokes, Color Emboss, Roughen Edges.
- **Time** (Video Effects ▸ Time): Posterize Time.

## Moving, keying and utility effects

> id: effect-reference.video-transform | does: names the transform, keying, utility and burn-in effects in Video Effects | cmd: effects.apply, effects.setParam | resolve: Inspector Transform, keyers, Data Burn-In | premiere: Video Effects (same names) | when: looking for an effect or transition by name | see: practice.titles.shake, keying.what

**[Revolution]**

- **Transform** (Video Effects ▸ Transform): Horizontal Flip, Vertical Flip, Transform, Offset, 3D Rotate, Auto Reframe, Camera Shake, Grow, Shrink, Move, Spin, Spacer, Wiggle.
- **Keying** (Video Effects ▸ Keying): Ultra Key, Track Matte Key, Color Key, Luma Key, Alpha Adjust, Logo Cutout.
- **Utility** (Video Effects ▸ Utility): Simple Text, Auto Align, Cineon Converter, Clone, Stroke.
- **Video** (Video Effects ▸ Video): Metadata & Timecode Burn-in.

## Immersive (VR) video effects

> id: effect-reference.video-vr | does: names the effects for 360° immersive video | cmd: effects.apply | resolve: OpenFX VR tools | premiere: Video Effects ▸ Immersive Video | when: looking for an effect or transition by name | see: effects.apply

**[Revolution]**

- **Immersive Video** (Video Effects ▸ Immersive Video): VR Blur, VR Chromatic Aberrations, VR Color Gradients, VR De-Noise, VR Digital Glitch, VR Fractal Noise, VR Glow, VR Plane to Sphere, VR Projection, VR Rotate Sphere, VR Sharpen.

## Obsolete and Legacy effects

> id: effect-reference.video-legacy | does: names the older effects and transitions kept for old projects, still usable | cmd: effects.apply | resolve: — | premiere: Obsolete / Legacy bins | when: looking for an effect or transition by name | see: effects.stylize

**[Revolution]**

- **Obsolete** (Video Effects ▸ Obsolete): Color Balance, Leave Color, Change to Color, Emboss, Median, Bevel Alpha, Circle, Grid, Timecode, Clip Name, Echo, Lightning, Cell Pattern, Checkerboard, Ellipse, Paint Bucket, Write-on.
- **Video Effects** (Legacy ▸ Video Effects): Camera Blur, Replicate, Crop, Edge Feather, Ramp, Alpha Glow, Block Dissolve, Directional Blur (Legacy), Gaussian Blur (Legacy), Gradient Wipe (Legacy), Linear Wipe (Legacy), Magnify (Legacy), Mosaic (Legacy), Noise (Legacy), Twirl (Legacy).
- **Video Transitions** (Legacy ▸ Video Transitions): Additive Dissolve (Legacy), Barn Doors, Center Split, Clock Wipe (Legacy), Cross Dissolve (Legacy), Cross Zoom (Legacy), Dip to Black (Legacy), Dip to White (Legacy), Film Dissolve (Legacy), Inset, Iris Box, Iris Cross, Iris Diamond, Iris Round, Non-Additive Dissolve, Push (Legacy), Radial Wipe (Legacy), Slide (Legacy), Split (Legacy), Whip (Legacy), Wipe.

## Video transitions

> id: effect-reference.transitions | does: names every video transition, by folder | cmd: sequence.applyVideoTransition, effects.setDefaultTransition | resolve: Video Transitions | premiere: Video Transitions (same names) | when: looking for an effect or transition by name | see: effects.transitions

**[Revolution]**

- **Animation** (Video Transitions ▸ Animation): Block Motion, Flip Motion, Fold Motion, Pop Motion, Pull Motion, Spin Motion, Spring Motion, Travel Motion.
- **Dissolve** (Video Transitions ▸ Dissolve): Additive Dissolve, Blur Dissolve, Burn Alpha, Cross Dissolve, Dip to Black, Dip to Color, Dip to White, Film Dissolve, Luma Fade, Morph Cut, Mosaic.
- **Grunge & Distort** (Video Transitions ▸ Grunge & Distort): Chaos, Earthquake, Flicker, Glass, Glitch, Grunge, Kaleidoscope, Liquid Distortion, TV Power, VHS Damage.
- **Immersive Video** (Video Transitions ▸ Immersive Video): VR Chroma Leaks, VR Gradient Wipe, VR Iris Wipe, VR Light Leaks, VR Light Rays, VR Mobius Zoom, VR Random Blocks, VR Spherical Blur.
- **Lights & Blurs** (Video Transitions ▸ Lights & Blurs): Burn Chroma, Chroma Leak, Cross Zoom, Directional Blur, Flare, Flash, Glow, Lens Blur, Light Leak, Light Sweep, Phosphor, Radial Blur, Ray, Solarize, Stripe, Zoom Blur.
- **Slide** (Video Transitions ▸ Slide): 3D Roll, Film Roll, Push, Roll, Slide, Split, Stretch, Whip.
- **Smart Tools** (Video Transitions ▸ Smart Tools): Motion Camera, Motion Tween, Shape Dissolve, Shape Flow.
- **Text** (Video Transitions ▸ Text): Text Animator, Typewriter.
- **Transformers** (Video Transitions ▸ Transformers): 3D Spin, 3D Spinback, Frame, Louver, Mirror, Page Peel, Slice, Wave.
- **Wipe** (Video Transitions ▸ Wipe): Clock Wipe, Linear Wipe, Neon Wipe, Panel Wipe, Plateau Wipe, Radial Wipe, Soft Wipe, Star Wipe, Stretch Wipe.

## Audio effects and transitions

> id: effect-reference.audio | does: names every audio effect and audio transition, by folder | cmd: effects.apply, sequence.applyAudioTransition | resolve: Fairlight effects | premiere: Audio Effects (same names) | when: looking for an effect or transition by name | see: essential-sound.controls, audio.cleanup-voice

**[Revolution]**

- **Audio Effects** (Audio Effects): Balance, Mute, Volume.
- **Amplitude and Compression** (Audio Effects ▸ Amplitude and Compression): Amplify, Channel Mixer, Channel Volume, DeEsser, Dynamics, Dynamics Processing, Hard Limiter, Multiband Compressor, Single-band Compressor, Tube-modeled Compressor.
- **Delay and Echo** (Audio Effects ▸ Delay and Echo): Analog Delay, Delay, Multitap Delay.
- **Filter and EQ** (Audio Effects ▸ Filter and EQ): Bandpass, Bass, FFT Filter, Graphic Equalizer (10 Bands), Graphic Equalizer (20 Bands), Graphic Equalizer (30 Bands), Highpass, Lowpass, Notch Filter, Parametric Equalizer, Scientific Filter, Simple Notch Filter, Simple Parametric EQ, Treble.
- **Modulation** (Audio Effects ▸ Modulation): Chorus/Flanger, Flanger, Phaser.
- **Noise Reduction/Restoration** (Audio Effects ▸ Noise Reduction/Restoration): Automatic Click Remover, DeHummer, DeNoise, Voice Isolation, DeReverb, Enhance Speech.
- **Reverb** (Audio Effects ▸ Reverb): Convolution Reverb, Studio Reverb, Surround Reverb.
- **Special** (Audio Effects ▸ Special): Binauralizer - Ambisonics, Distortion, Fill Left with Right, Fill Right with Left, GuitarSuite, Invert, Loudness Meter, Mastering, Panner - Ambisonics, Swap Channels, Vocal Enhancer.
- **Stereo Imagery** (Audio Effects ▸ Stereo Imagery): Stereo Expander, Stereo Width.
- **Time and Pitch** (Audio Effects ▸ Time and Pitch): Pitch Shifter.
- **Crossfade** (Audio Transitions ▸ Crossfade): Constant Power, Constant Gain, Exponential Fade.
