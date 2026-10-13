# Linux integration plan (fork)

Written 2026-10-12 (owner, live): "a phase for native Linux UI integration (tailored for KDE
Plasma and Wayland), and then a phase for native overall Linux integration. Both after phase
AI1." Order in [`FORK-PLAN.md`](FORK-PLAN.md) §1a.6: … C2 → AI1 → **L1 → L2** → A2 → G3 → 7 …
Tags `linux-1`, `linux-2`; builds per §1a.9 (or grouped per §1a.41 if a phase turns out small).

## 0. Ground rules

- **Tailored for KDE Plasma 6 on Wayland** (the owner's desktop: CachyOS), **but never KDE-only.**
  GNOME, other Wayland compositors and X11 must keep working; every integration detects what the
  desktop offers and falls back quietly (no errors, no missing UI) when it doesn't.
- **Pure Rust.** D-Bus through `zbus` (already a dependency of `apps/filmcraft`, used by
  `src/appearance.rs` for the portal colour scheme); portals called over D-Bus (or `ashpd` if it
  saves real code; MIT, check it builds pure-Rust). Any `unsafe` (e.g. a Wayland protocol winit
  doesn't expose) only in `crates/platform` (ADR 0001). New crates: MIT / Apache / BSD only.
- **Layering** (`cargo xtask layers`): desktop integration lives in L5 (`apps/filmcraft`,
  `crates/ui-egui` shell) or behind a trait in `crates/platform`; nothing below L5 learns about
  D-Bus, Wayland or KDE.
- **No KDE / Breeze assets** copied in (Breeze icons are LGPL; §1 asset rules): read the user's
  settings at run time, never bundle someone else's theme.
- **Upstream-contributable by default.** This is generic Linux work: write it so it can go back
  upstream (§1a.25); anything fork-only follows the §1a.36 pattern.
- **Both workflows** (Premiere / Resolve) tested, every new behaviour a command where it is an
  action, settings serde (agent-drivable), guide pages updated (§1a.21, tag [FilmCraft] / [Fork]).
- First step of each phase: **audit** what upstream and the fork already have (portal theme,
  `packaging/linux/` desktop file, metainfo, MIME XML, Flatpak manifest, nfpm, `file_filters.rs`)
  and check upstream `main` for newer work; don't rebuild what exists. gaps.md G9 lists known
  Linux bugs (Wayland title bar #433, playback stutter #617, case-sensitive extensions #593):
  fix the ones in scope.

## Phase L1: native Linux UI on KDE Plasma and Wayland (≈ 20–40 h)

1. **Wayland first.** Run as a native Wayland client by default (not XWayland); app id
   `ai.storyteller.filmcraft` matching the desktop file so KWin shows the right icon, groups
   windows and applies window rules. X11 stays as the fallback.
2. **Scaling and text.** Fractional scaling (`wp_fractional_scale`) and per-monitor scale on
   mixed-DPI setups: crisp UI and text at 125 % / 150 %, correct when a window moves between
   monitors; the Program monitor maps image pixels 1:1 at 100 % zoom on any scale.
3. **Window decorations.** Server-side decorations from KWin (`xdg-decoration`) so the title bar
   is Plasma's own (fixes #433 class bugs); client-side fallback for compositors without it.
   Window size / position / maximised state restored per monitor.
4. **Follow the desktop's look.** Light / dark from the portal (exists; audit live switching),
   **accent colour** (portal `org.freedesktop.appearance accent-color`) used for selection /
   highlights in both workflows' themes, cursor theme and size, double-click interval, reduced
   motion, and the system UI font size as a scale hint (fonts themselves stay per the fonts
   rule). A Settings ▸ Appearance "Follow system" toggle per item.
5. **Native dialogs.** Open / Save / Import / folder pickers through the XDG file-chooser portal
   (KDE's own dialog on Plasma: `rfd` with its portal backend), remembering last folders.
6. **Input.** Drag and drop from Dolphin into the Project / Media Pool, Timeline and Source
   (Wayland DnD, `text/uri-list`); clipboard of paths and timecodes; high-resolution touchpad
   scrolling and pinch-zoom on the timeline / viewers; IME (`text-input-v3`, fcitx5) in text
   fields and titles.
7. **Plasma niceties.** Export / render progress on the **taskbar entry** (Unity LauncherEntry
   D-Bus, which Plasma's task manager shows) and in Plasma's **job tracker** where available;
   a notification when an export / proxy job finishes (portal or org.freedesktop.Notifications)
   with "Show in folder"; **idle inhibit** (no screen lock / sleep) during playback and export
   (portal `Inhibit` or `idle-inhibit`); single instance with `xdg-activation` so opening a
   project focuses the running window.
8. **Global shortcuts (optional, off by default):** the GlobalShortcuts portal for play / pause
   and marker add while another window has focus.
9. **HDR output on KWin (investigate only):** record whether wgpu / winit can present to KWin's
   HDR (`wp_color_management`) without `unsafe` outside `crates/platform`; write the findings
   and a sized proposal to OWNER-QUESTIONS, don't build it in L1.

**Accept:** on this PC (Plasma 6, Wayland, the owner's monitors), native Wayland window with
KWin decorations and the right icon; 125 % / 150 % scale crisp; accent colour and light / dark
follow live; KDE file dialog; drag from Dolphin works into all three targets; taskbar progress
+ completion notification on an export; screen doesn't lock during a 10-minute playback; the
same build still runs under X11 and a non-KDE session (GNOME or a nested compositor) without
errors. UI tests for every new setting and command; screenshots in both workflows.

## Phase L2: native Linux integration overall (≈ 25–45 h)

1. **Standard locations.** XDG base directories: config / data / cache / state honour
   `XDG_*_HOME`; render cache and proxies default to the cache dir (with the existing
   claim-folder setting); a migration for existing data, never deleting the user's files.
2. **File associations and Dolphin.** MIME types for project files (audit
   `ai.storyteller.filmcraft.mime.xml`), "Open with FilmCraft" for the camera formats we
   decode, desktop-file actions (New Project, Project Manager), recent projects published to
   the desktop's recent-files list; **thumbnails** of project files and supported media in
   Dolphin / file managers: Rust renders them (`filmcraft-cli thumbnail`), a freedesktop
   `.thumbnailer` serves GNOME-family managers, and **a fork-only C++ KIO shim in
   `fork/kde-thumbnailer/` serves Dolphin** (owner, 2026-10-12, FORK-PLAN §1a.46).
3. **Media controls.** MPRIS so Plasma's media widget, lock screen and keyboard media keys play
   / pause / step the Program monitor (off when nothing is open).
4. **Cards and drives.** Detect a camera card or drive being mounted (udisks2 over D-Bus) and
   offer an import with verified copy; safe "eject when done"; card-pulled-mid-copy stays a
   handled error (overlaps Phase 7.3: share the tests).
5. **Trash, not delete.** Anything FilmCraft removes on the user's behalf (render cache cleanup
   excepted) goes to the trash via the portal / freedesktop trash spec.
6. **GPU choice on hybrid systems.** Detect the GPUs (here: RX 6700 XT + Intel UHD 750) and pick
   the discrete one for rendering by default, honouring `DRI_PRIME` / switcheroo-control when
   set, with a Settings choice; the Intel media engine for 4:2:2 decode stays as Phase 6 set it.
7. **Power and background work.** Inhibit suspend during exports (logind / portal); a
   low-priority mode for background renders (nice / ioprio on our own threads; no root);
   optional GameMode awareness.
8. **Logging and crash reports.** Logs to the journal when run from the desktop (still to a file
   too); crash reports (the existing panic hook) stored under the state dir with a "Report an
   issue" action (#642).
9. **Packaging for this distro family.** An Arch / CachyOS `PKGBUILD` (installs the binary,
   desktop file, icons in all hicolor sizes, MIME XML, metainfo), the existing Flatpak manifest
   updated to the portals L1 / L2 use (and checked it still builds), AppStream metainfo valid
   (`appstreamcli validate`), `desktop-file-validate` clean. The fork's own launcher
   (`filmcraft-fork.desktop`, §1a.9) keeps working alongside.
10. **KDE Connect (owner, 2026-10-12, FORK-PLAN §1a.50).** Talk to the running KDE Connect daemon
    over D-Bus (zbus); never implement or copy the protocol (every implementation is GPL).
    (a) **Phone → FilmCraft:** watch KDE Connect's download folder (from its config / D-Bus) and
    offer "Import N new clips from <device>" or drop them into an "Inbox" bin (setting); (b)
    **FilmCraft → phone:** "Send to phone" after an export and on a selected file (the share
    plugin's D-Bus call), with the paired devices listed; (c) the phone's media remote controls
    playback through item 3's MPRIS (check it works end to end). Quietly off without KDE Connect
    (GNOME, not installed, no paired device). Commands `kdeconnect.*`. Test against a fake D-Bus
    service in CI; a real phone check on the Dell when the owner has one paired.

**Accept:** fresh user account: install from the PKGBUILD, open a project by double-clicking it
in Dolphin, thumbnails visible, media keys control playback, inserting a card offers an import,
export doesn't let the PC sleep, logs in `journalctl --user`, a file shared from a paired phone
shows up as an import offer and "Send to phone" delivers an export (KDE Connect); Flatpak build runs with the same
features through portals; all validators clean; everything still works on X11 / non-KDE with
the integrations quietly off.

## 1. Not in scope

KDE-only features with no fallback, Plasma widgets in C++ (the KIO thumbnail shim of L2.2 is the one fork-only exception), HDR output implementation (L1.9
only investigates), Snap packaging, system-wide (root) installs from the app.
