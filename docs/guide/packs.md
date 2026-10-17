# Look packs

**[Fork]** · sharing your effect presets, LUTs and graphics templates as one file; written for
Phase Y3.5

## What a look pack is

> id: packs.what | does: one file that holds effect presets, LUTs and graphics templates | cmd: packs.* | resolve: — (PowerGrades and LUTs export separately) | premiere: — (presets and Motion Graphics templates export separately) | when: moving your looks to another computer or sharing them | see: packs.make, packs.open

**[Fork]**

A **look pack** (`.fcpack`) bundles three things you build up over time so you can move them to
another computer or give them to someone else in one file:

- your **effect presets** (Effects ▸ Presets: saved effect settings, with their keyframes and
  masks),
- **LUTs** from the current project's LUT library (`.cube` / `.3dl`),
- your installed **graphics templates** (`.fcgt`: lower thirds, titles).

Neither Premiere nor Resolve has one file for all three: Premiere exports presets (`.prfpset`) and
Motion Graphics templates separately; Resolve exports PowerGrades and LUTs separately. A Revolution
pack is a plain zip archive with a small `pack.json` list inside, so you can also look into it with
any zip tool.

## Making a pack

> id: packs.make | does: exports chosen presets, LUTs and templates as a .fcpack; imports a folder of LUTs | cmd: packs.export, lut.importFolder | when: sharing looks; bringing in downloaded LUTs | see: packs.open

**[Fork]**

Run **Export Look Pack…** (command `packs.export`) with a file name. By default it takes all your
presets, every LUT in the project and all your installed templates; to choose, pass their names
(`presets`, `luts`, `templates`). An empty selection is refused rather than writing an empty
pack.

To bring in a whole folder of LUTs you downloaded (without a pack), use **Import LUT Folder…**
(command `lut.importFolder`): every `.cube` and `.3dl` directly in that folder goes into the
project's LUT library in one undo step; files that don't read are skipped and listed.

## Opening a pack

> id: packs.open | does: imports a pack without overwriting anything, after checking every file in it | cmd: packs.import | when: a pack someone sent you | see: packs.make

**[Fork]**

Run **Import Look Pack…** (command `packs.import`). Presets and templates go into your own
library (they stay after you close the project); LUTs go into the **current project's** LUT
library, so open the project you want them in first.

Importing never overwrites anything: a preset or template whose name you already have arrives as
"Name (2)", and a LUT that is exactly one you already have is reused instead of added twice.

Packs come from other people, so Revolution checks them before taking anything: only the three
kinds of files above are accepted, with plain names (no folders or paths that could reach outside
the library), at most 2048 files and 512 MB unpacked, and every LUT and template must read
correctly. Adobe `.mogrt` files inside a pack are refused. If anything is wrong, nothing from that
part is added and the message says which file broke which rule.
