# Colour management

**[Standard] [Fork]** · F-Log2 input, DaVinci Wide Gamut / Intermediate working space, Rec. 709 output

## Why

**[Standard]**

A log clip stores brightness with a curve that squeezes the sun and the shadows into a flat
picture, and its colours use a gamut (F-Gamut) wider than your screen. Grading numbers only mean
the same thing from clip to clip if all clips are first translated into one **working space**.
This fork uses **DaVinci Wide Gamut / DaVinci Intermediate** (Blackmagic's published
specification). Its signal has mid grey at about 0.336, which is why the Pivot slider defaults to
that value. Grade nodes work on this signal when the sequence's working space is DaVinci Wide
Gamut; in a Rec. 709 sequence they work on the Rec. 709 signal, in a Rec. 2100 sequence on the
PQ or HLG signal, so the same numbers look different in each.

## Tell FilmCraft what the clip is (input)

**[FilmCraft]**

Most files announce their colour space and need nothing. If an F-Log2 clip looks grey and flat
in the viewer, tell it explicitly:

1. In the Project panel select the clip (the footage item).
2. Choose Clip ▸ Modify ▸ **Interpret Footage…** (`clip.interpretFootage`). The window
   "Interpret Footage" opens.
3. Under "Override media colour space" pick **Fujifilm F-Log2 / F-Gamut** (id `flog2-fgamut`)
   {ui:colorDialog.space.flog2-fgamut}. "Use file metadata" {ui:colorDialog.space.auto} goes back
   to automatic. The line above shows what was detected from the file.
4. Press **OK** {ui:colorDialog.ok}. The clip should now look natural instead of grey.

Plain F-Log (not F-Log2) is `flog-fgamut`, "Fujifilm F-Log / F-Gamut".

## Choose the working space (sequence)

**[FilmCraft]** **[Fork]**

1. Choose Sequence ▸ **Color Management…** (`sequence.colorSettings`). The window "Sequence Color
   Management" opens.
2. Under "Working Color Space" pick **DaVinci Wide Gamut / Intermediate**
   {ui:colorDialog.working.dwg-di}. The other choices are Rec. 709 (the default), Rec. 2100 PQ
   and Rec. 2100 HLG.
3. The two checkboxes below matter in a Rec. 709 sequence: "Wide gamut color (composite in
   BT.2020)" {ui:colorDialog.wideGamut} and "Auto tone map media (HDR and log into SDR)"
   {ui:colorDialog.autoToneMap}, which folds HDR clips into the Rec. 709 picture instead of
   clipping them. In a DaVinci Wide Gamut sequence the log conversion happens regardless.
4. Press **OK** {ui:colorDialog.ok}.

Existing sequences keep their working space; only change it deliberately. Lumetri and the nodes
behave differently in different working spaces, as the same numbers do in Resolve with a
different timeline colour space.

## Output

**[Standard]** **[Fork]**

In a DaVinci Wide Gamut sequence the working space is converted to **Rec. 709** at the end,
with highlight roll-off (BT.2390 tone mapping) and colours outside Rec. 709 desaturated into
it, so very bright or very saturated values fold in gently instead of clipping. There is no
separate control yet: a per-grade highlight knob is planned for G2.

## If you know Resolve

**[Resolve-style]**

This matches Resolve's "DaVinci YRGB Color Managed" idea, but the setting lives in the sequence
and in Interpret Footage, not in a project-wide settings page. There is no per-clip input
transform node: the input transform is applied automatically before your first node.

## If you know Premiere

**[FilmCraft]**

Interpret Footage and Sequence Color Management are the same dialogs as upstream FilmCraft's
(**[FilmCraft]**); the DaVinci working space is the fork's addition (**[Fork]**).
