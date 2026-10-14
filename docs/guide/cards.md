# Copying camera cards

**[Standard] [Fork]**

Why: a memory card is the only copy of a shoot until it is backed up. Editing straight from the
card is risky (it gets formatted for the next shoot, or pulled out mid-playback), and a plain
file-manager copy doesn't tell you whether every byte arrived. FilmCraft copies the whole card
to a folder on your media drive, checks every file, and only then imports the clips.

## When you put a card in

> id: cards.insert | does: offers a verified copy of a camera card when the desktop mounts it | ui: project.card.* | cmd: card.offload | resolve: Clone tool (Media page) | premiere: Media Browser ingest with verify | when: every shoot, before the card is formatted | see: cards.copy-details

1. Insert the card (or connect the camera as a drive). When the desktop mounts it, a bar at the
   top of the Project panel says **Camera card <name>**.
2. Click **Copy** {ui:project.card.copy}. The first time, FilmCraft asks for the folder cards are
   copied to and remembers it (**Settings ▸ Media ▸ Copy camera cards to**, `media.cardDestination`;
   change it there). Each card gets its own folder inside it, named after the card and the date
   (`FUJI 2026-10-13`).
3. The bar shows the progress; the copy is also in the job list, where you can cancel it.
4. When the bar says **<name> is copied and verified**, the clips are in a bin of the same name.
   Click **Eject** {ui:project.card.eject} and take the card out when the status line says it can
   be removed. **Dismiss** {ui:project.card.dismiss} hides the bar without copying.

FilmCraft notices cards the desktop mounts under `/run/media` or `/media` (Plasma, GNOME and most
others, X11 or Wayland), with a `DCIM`, `PRIVATE`, `XDROOT`, `CONTENTS` or `CLIP` folder. Other
drives (a USB stick, a backup disk) get no bar. Eject goes through UDisks2, the service file
managers use; without it, eject the card from the file manager.

**Settings ▸ Media ▸ Camera cards** (`media.cameraCards`):

- **Offer to copy** (the default): the bar above. Good if most cards you insert are for editing.
- **Ignore**: no bar; copy with `card.offload` or by hand. Pick this if you insert cards for other
  reasons (photos for another app) and find the bar in the way.

## What the copy does

> id: cards.copy-details | does: copies the whole card, checks every file and writes a checksum list | cmd: card.* | resolve: Clone tool checksums | premiere: Ingest with copy verification | when: knowing whether a card is safe to format | see: cards.insert

- **Everything on the card is copied**, in the card's own folder structure (`DCIM/100_FUJI/…`,
  `PRIVATE/M4ROOT/…`), so camera sidecar files (`.XML`, `.THM`) and any tool that expects the
  card layout still work. Hidden files and other computers' leftovers (`.Trashes`,
  `System Volume Information`) are skipped; links are never followed.
- **Every file is verified:** each one is written under a temporary name, flushed to the drive,
  read back and compared with the card by CRC-32 before it gets its real name. A failed or
  cancelled copy leaves no half-written file behind.
- **A checksum list** `offload-crc32.txt` (CRC-32, size, path per file) is written in the copy's
  folder, so you can check the copy again later or after moving it.
- **Nothing is overwritten.** A file that is already there with the same bytes is kept (so an
  interrupted copy can simply be run again and carries on); a *different* file with the same name
  is left alone and counted in the result.
- **A card pulled out mid-copy** (or a failing card) stops the copy with a message saying how many
  files are already copied and verified. Put the card back and run the copy again.
- **Then the media is imported** into a top-level bin named after the copy's folder (only files
  FilmCraft can import; sidecars stay on disk).

Which cards are recognised: any folder with `DCIM` (Fujifilm, Canon, Nikon, Panasonic stills and
most phones), `PRIVATE` (Sony XAVC / AVCHD), `XDROOT` (XDCAM), `CONTENTS` (P2) or `CLIP` at the
top, in any letter case.

## From scripts or an agent

> id: cards.commands | does: the card commands for scripts and agents | cmd: card.* | when: automating ingest | see: cards.insert

| Command | What it does |
|---|---|
| `card.inspect` (`mount`) | The card's layout (`dcim`, `private`, `xdcam`, `p2`, `clip`, or none), how many files and media files, total bytes |
| `card.offload` (`mount`, `destination`, `name`, `import`, `wait`) | Copies the card at `mount` into `destination/name` (default name: the card's folder name and today's date) as a background job in the job list; `import: false` copies without importing |
| `card.copy` (`mount`, `destination`) | What the bar's Copy does: `card.offload` into Settings ▸ Media ▸ Copy camera cards to (or `destination`) |
| `card.eject` (`mount`) | Unmounts the card and powers its reader down (UDisks2); the answer arrives as `card.ejected` |
| `card.dismiss` (`mount`) | Hides the bar for that card |

The cards on offer are in the UI state as `cards` (mount, name, layout, copy job).

The destination must be an existing folder that is not on the card. The copy runs as a job: watch
it, or cancel it, like an export.

## Options, and when to pick which

> id: cards.options | does: copy first, import from the card, or copy outside FilmCraft: the trade-offs | cmd: card.offload, file.import | resolve: Clone tool vs importing from the card | premiere: Ingest vs importing from the card | when: choosing how footage comes in | see: cards.copy-details

- **Copy, then edit from the copy (this feature):** the safe default. The card can be formatted
  as soon as the copy is verified and you have a second backup. Costs the time of one full copy
  (a 128 GB card over USB 3 takes roughly 5–15 minutes).
- **Copy to two drives:** professionals keep two verified copies before formatting a card. Run the
  copy twice with different destinations, or back up the first copy's folder with your usual
  backup tool (the checksum list lets you verify that copy too).
- **Import straight from the card** (File ▸ Import, `file.import`): quickest for a look at a few clips, but the
  project then points at the card. Use Project Settings ▸ Ingest Settings (`project.ingestSettings`) ▸ Copy if you want
  FilmCraft to copy the clips you import (only those clips, not the whole card).
- **A dedicated offload tool** (one that writes MHL checksum files): if your workflow hands cards
  to a DIT or a post house that expects MHL, use their tool; FilmCraft's checksum list is a plain
  text file, not MHL.
