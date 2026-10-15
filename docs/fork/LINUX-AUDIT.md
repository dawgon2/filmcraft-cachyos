# LINUX-AUDIT: what L1 / L2 already have (read-only survey)

Written 2026-10-12 ahead of [`LINUX-PLAN.md`](../../LINUX-PLAN.md) L1 / L2 (FORK-PLAN §1a.42), as the
"audit first" step of both phases. Nothing was built or run; every claim comes from the fork's
sources, `upstream/main` (`0879bd7a`, fetched 2026-10-12) and the crate sources in
`~/.cargo/registry/src/index.crates.io-*/`. The plans themselves are not edited here.

Legend: **Have** (works today), **Partly**, **Missing**. Hours are a fresh estimate per item for an
agent session (code + tests + guide page), not the plan's ranges.

## 0. The ground this stands on

| Thing | State | Evidence |
|---|---|---|
| Window stack | eframe 0.36.2 → egui-winit 0.36.2 → winit 0.30.13 → smithay-client-toolkit 0.19.2, wayland-protocols 0.32.13 | `Cargo.lock`; `apps/filmcraft/Cargo.toml` enables eframe `wayland` + `x11` |
| winit features in use | egui-winit's `wayland` = `winit/wayland` only (`egui-winit-0.36.2/Cargo.toml:79`). **`wayland-csd-adwaita` is off** (workspace `eframe = { default-features = false }`), so there is no themed client-side title bar | `Cargo.toml:74`; no `sctk-adwaita` in `Cargo.lock` |
| App id | `with_app_id("ai.storyteller.filmcraft")`, matches the desktop file | `apps/filmcraft/src/main.rs:49,135` |
| D-Bus | `zbus 5.19` (MIT) already a Linux dependency of `apps/filmcraft` (used by `appearance.rs`); also pulled in by accesskit_unix | `apps/filmcraft/Cargo.toml` `[target.'cfg(target_os = "linux")']` |
| rfd 0.17.2 | default features `xdg-portal` + `wayland` are on. The portal backend is rfd's own: it `dlopen`s `libdbus-1.so.3` (not zbus, not ashpd), exports the parent window with `zxdg_exporter_v2`, falls back to zenity | `rfd-0.17.2/Cargo.toml` `[features] default`; `src/backend/xdg_desktop_portal/portal/ffi.rs:199`; `window_identifier/wayland.rs` |
| Data dir | `$FILMCRAFT_DATA_DIR` → portable → `$XDG_DATA_HOME/filmcraft` → `~/.local/share/filmcraft`; everything (Logs, Recovery, Proxies, Media Cache/Previews, prefs.json, models) lives under it | `crates/engine/src/autosave.rs:529`; `crates/engine/src/settings.rs:1459` (`media_cache_dir`) |
| Packaging | desktop file, metainfo template, **empty** MIME XML, hicolor icons 16–512 + scalable (with `.attribution`), nfpm deb/rpm, two Flatpak manifests, `install.sh`, `postinst.sh`, `package.sh` runs `desktop-file-validate` and `appstreamcli validate` when present | `packaging/linux/*`, `assets/app-icon/hicolor/` |
| Fork vs upstream in these files | `appearance.rs`, `dev_icon.rs`, `file_filters.rs`, `crash.rs`, `packaging/linux/*` are **identical** to upstream/main (empty diff) | `git diff --stat HEAD upstream/main -- …` |
| Upstream has, fork lacks | **#260 off-thread portal dialogs** (`crates/ui-egui/src/native_dialogs.rs`, `hooks.run_dialog`, commit `059376af`); Save dialogs opening in the project's folder (`set_directory(parent)`); `graphics.rs` start-up ladder with `--graphics` and `native_adapter_selector` (#820). All arrive with U0.2; L1.5 and L2.6 should build on them, not re-do them | `git show upstream/main:apps/filmcraft/src/main.rs:300-305,440+` |
| This PC (for the acceptance runs) | Plasma 6.7.5 / KWin 6.7.5 Wayland; `xdg-desktop-portal 1.22.1`, `-kde 6.7.5`, `-gtk 1.15.3`; `udisks2 2.11.2`; `zenity`, `appstream 1.2.1`, `desktop-file-utils`, `ffmpegthumbs`, `kdegraphics-thumbnailers` installed; **no** `gamemode`, **no** `switcheroo-control`; `XCURSOR_THEME` / `XCURSOR_SIZE` unset in the agent's shell | `pacman -Q`, `kwin_wayland --version` |

### G9 issues (gaps.md) — current status

| Issue | State on GitHub | Notes for L1 / L2 |
|---|---|---|
| #433 Wayland title bar | **open** | GNOME Wayland (Mutter has no `xdg-decoration`), so winit draws its plain fallback frame. On KWin the app already gets server-side decorations (winit asks `RequestServer` when `decorations` is true, `winit-0.30.13/src/platform_impl/linux/wayland/window/mod.rs:99-108`). Fix for GNOME is the `wayland-csd-adwaita` feature (sctk-adwaita, MIT) or an in-app title bar; L1.3 |
| #593 case-sensitive extensions | **closed** 2026-10-10 | fixed by `file_filters.rs` (bracket globs); nothing to do |
| #617 Linux Mint playback stutter | **open** | NVIDIA RTX 3070 + Cinnamon (X11), 1 s start/stop; audio-device class (gaps.md G13 lists it with #336/#410). Not a KDE/Wayland item: out of L1/L2 scope unless the Phase 7 soak reproduces it |
| #642 "Report an issue" link | **closed** 2026-10-10 | `open::that_detached` in `main.rs:247`; L2.8's "Report an issue" action only needs to attach the crash log path |

## 1. Phase L1, item by item

### L1.1 Wayland first, app id — **Have**
- Evidence: eframe `wayland` feature on; winit 0.30 prefers Wayland when `WAYLAND_DISPLAY` is set and falls back to X11; `with_app_id` set; desktop file `StartupWMClass=ai.storyteller.filmcraft`; `dev_icon.rs` installs a user-level desktop entry for source-tree runs (opt-in `FILMCRAFT_DEV_DESKTOP_ENTRY=1`).
- Gap: the fork launcher `~/.local/share/applications/filmcraft-fork.desktop` is not named after the app id, so KWin may match the window to the packaged `ai.storyteller.filmcraft.desktop` (if installed) or show the generic icon; Plasma does also match `StartupWMClass`, so verify once. A `WINIT_UNIX_BACKEND` note in the guide covers the X11 fallback.
- Route: verify with `ui.screenshot` + `kdotool`/`qdbus org.kde.KWin` that the surface's app id is right; rename or add the fork launcher per §1a.9 if needed. **1 h.**

### L1.2 Fractional scaling and per-monitor scale — **Have (verify)**
- Evidence: winit 0.30.13 binds `wp_fractional_scale_v1` and `wp_viewporter` (`types/wp_fractional_scaling.rs`, `types/wp_viewporter.rs`); egui-winit forwards `ScaleFactorChanged` (`egui-winit-0.36.2/src/lib.rs:324`); the UI reads `native_pixels_per_point` each frame and re-applies Settings ▸ Appearance ▸ UI Scale `auto` (`crates/ui-egui/src/lib.rs:742-759`).
- Gap: nothing proves the Program monitor maps 1:1 at 100 % zoom on 125 % / 150 %; "X11 scale (#457)" is listed in `docs/ui-parity.md:30`.
- Route: a UI test that sets `pixels_per_point` 1.25 / 1.5 and checks the monitor's image rect is an integer multiple of the frame, plus a screenshot pass on this PC's monitors. **3 h.**

### L1.3 Server-side decorations, restore size / position — **Partly**
- Evidence: SSD already on KWin (see #433 above). `with_titlebar_shown(false)` / `with_title_shown(false)` in `main.rs:129-130` are macOS-only and harmless. eframe's `persistence` is **off** (no `persist_window`), so size / maximised state are not restored; Wayland cannot restore position at all (compositor-owned), so "per monitor" means size + maximised only.
- Route: (a) add `winit/wayland-csd-adwaita` behind eframe (eframe has no feature for it: add `winit = { version = "0.30", default-features = false, features = ["wayland-csd-adwaita"] }` to `apps/filmcraft` on Linux, same pattern as the macOS block) — fixes #433 on GNOME; sctk-adwaita MIT + ab_glyph Apache-2.0; (b) persist `inner_size` + `maximized` in prefs (serde, agent-drivable) and apply via `ViewportCommand` on start. **4 h.** Risk: none; no `unsafe`.

### L1.4 Follow the desktop's look — **Partly**
- Have: light / dark from the portal with live `SettingChanged` following (`apps/filmcraft/src/appearance.rs:90-150`, `gsettings` fallback); `highlight_color` pref exists (`crates/engine/src/settings.rs:128`, `appearance.highlightColor` Kind::Color) and the Resolve theme has its own `accent` (`crates/ui-egui/src/theme.rs:68,274`).
- Missing: reading `org.freedesktop.appearance accent-color` (portal Settings ≥ 1.17, a `(ddd)` tuple; `appearance.rs` already has the proxy and signal loop — extend `portal_code` decoding), the per-item "Follow system" toggles, reduced motion (portal has no key; KDE exposes `org.kde.kdeglobals.KDE/AnimationDurationFactor` through the same `Read` namespace), cursor theme / size (winit uses `wp_cursor_shape_v1` when KWin offers it, else `XCURSOR_THEME`/`XCURSOR_SIZE` — `smithay-client-toolkit-0.19.2/src/seat/pointer/mod.rs:603`; nothing to do but document), double-click interval (egui has no setting; skip), system font size as a scale hint (`org.gnome.desktop.interface text-scaling-factor` via portal on GNOME; KDE: `kdeglobals General/font` — read-only hint for `ui_scale` `auto`).
- Route: extend `appearance.rs` into a small `portal_settings` worker publishing a struct `{theme, accent, animation_factor, text_scale}`; prefs `appearance.followSystemAccent` / `…Motion` / `…TextScale`; theme applies accent to both workflows' `Tokens`. **6 h.** Risk: none; pure zbus.

### L1.5 Native dialogs through the portal — **Have (needs #260)**
- Evidence: rfd's portal backend is the default and on Plasma gives KDE's dialog (`xdg-desktop-portal-kde` installed); `file_filters.rs` handles case; `rfd::FileDialog::set_parent` is never called (dialogs are not modal to the window); `set_directory` only in `pick_folder_at` (`main.rs:242`); "remember last folder" absent.
- Risk already known upstream: every picker is called **synchronously on the UI thread** (`main.rs:210-270`) — the portal call blocks the frame loop (#260). Upstream fixed it (`native_dialogs.rs`); the fork gets it in U0.2.
- Route: after U0.2, add `set_parent(frame.window_handle())` (eframe exposes `cc.window_handle()`; rfd then exports the surface with `zxdg_exporter_v2` so KWin stacks the dialog as a child), a `dialogs.lastFolder.<kind>` pref, and `set_directory` for Open / Import / Save. **3 h** (if U0.2 has landed; 8 h if it has to be back-ported first).

### L1.6 Input: DnD, clipboard, touchpad, IME — **Partly; DnD Missing**
- DnD: egui side exists (`crates/ui-egui/src/lib.rs:1447` `handle_drops`, Timeline drop placement) and works on X11 (winit `event_processor.rs` XDnD). **winit 0.30.13 has no Wayland drag-and-drop at all** — no `DroppedFile` in `platform_impl/linux/wayland/` (winit tracks it under #720; `FEATURES.md:214`). sctk 0.19 ships `data_device_manager`, but a second client connection cannot receive drops for winit's surface, so this needs winit support (0.31 / egui 0.37+) or a winit fork. Also `drop_pointer` relies on `cursor_screen_position`, which is `None` off Windows (`crates/platform/src/cursor.rs:25`), so the Timeline drop preview on Linux already depends on egui's pointer state only.
- Clipboard: `arboard 3.6.1` without `wayland-data-control` (`Cargo.lock`: x11rb only) — `clipboard_media` (`main.rs:296`) reads file lists through XWayland's clipboard bridge; enable the feature (`wl-clipboard-rs`, MIT/Apache-2.0; KWin supports `wlr-data-control`) for a native path. Text goes through egui-winit's smithay-clipboard already. Timecode copy exists.
- Touchpad: winit Wayland emits `PixelDelta` for finger scrolling and `LineDelta` for wheels (`seat/pointer/mod.rs:180-230`); timeline / monitor already handle both and a pinch (`timeline.rs:1939`, `monitor_view.rs:395`). **Pinch on Wayland: Missing** — winit 0.30 has no `zwp_pointer_gestures_v1` on Wayland (`PinchGesture` only on macOS / Windows). Ctrl+wheel zoom works.
- IME: winit binds `zwp_text_input_v3` (`seat/text_input`), egui-winit handles `Ime::Preedit/Commit` and `set_ime_allowed` (`lib.rs:718-760,1153`); fcitx5 should work — verify in a title field.
- Route: ship the clipboard feature flag + IME / touchpad verification tests now (**2 h**); write Wayland DnD and pinch as a **blocked** item with the winit version they need, and offer the fallbacks that exist: Open With from Dolphin (L2.2 + single instance, L1.7), Paste of files, Import dialog. If the owner wants DnD regardless: a winit fork adding sctk `DataDevice` handling to the Wayland `WinitState` (~300 lines of winit code; no `unsafe`, but a git dependency on egui-winit → winit, which is a lockfile burden for upstream contributions). **Estimate for the fork route: 12 h**; not recommended in L1.

### L1.7 Plasma niceties — **Missing**
- Taskbar progress: Unity LauncherEntry is a D-Bus **signal** (`com.canonical.Unity.LauncherEntry.Update` on the session bus, `app-uri: application://ai.storyteller.filmcraft.desktop`, `{progress: d, progress-visible: b}`); Plasma's task manager listens. ~40 lines of zbus; progress source is `job.progress.fraction()` (`crates/ui-egui/src/lib.rs:1883-1890`).
- Plasma job tracker (`org.kde.JobViewServer` / `JobViewV2`): KDE-only API; ~80 lines; off when the name is absent.
- Notifications: `org.freedesktop.Notifications.Notify` with an action ("Show in folder" → existing `open_path(reveal)` in `main.rs:309`) and `ActionInvoked` signal; or the Notification portal (no actions callback without a request handle; prefer the fd.o interface, portal only inside Flatpak). ~120 lines of zbus, or `notify-rust` 4.x (MIT/Apache-2.0, zbus backend with feature `z`).
- Idle inhibit: easiest is the portal `org.freedesktop.portal.Inhibit.Inhibit` (flags 4 idle, 8 suspend; a request object kept open while playing / exporting) — works everywhere incl. Flatpak; `zwp_idle_inhibit_manager_v1` would need winit's surface. ~60 lines.
- Single instance + `xdg-activation`: Missing (no lock, `main.rs` always opens a new window). Route: own a D-Bus name `ai.storyteller.filmcraft` (zbus `request_name`); a second launch calls `Open(paths, activation_token)` on it and exits; the running app imports / opens and raises itself with `request_user_attention` (winit uses `xdg_activation` for it, `window/mod.rs:520`; eframe `ViewportCommand::RequestUserAttention`). The launching instance gets the token from `XDG_ACTIVATION_TOKEN` env (desktop file `StartupNotify=true` is already set) — `winit::platform::startup_notify` exposes `read_token_from_env`. A true focus hand-over needs `xdg_activation_v1.activate(token, surface)`, which winit does **not** expose for a token from another process; `request_user_attention` makes the taskbar entry flash instead, which Plasma then raises on click. Good enough; document.
- **9 h** total (progress 1.5, job tracker 1.5, notification 2, inhibit 1, single instance 3). Risk: none; all session-bus D-Bus in L5.

### L1.8 GlobalShortcuts portal (optional, off) — **Missing**
- `org.freedesktop.portal.GlobalShortcuts` (portal ≥ 1.17; KDE backend present): `CreateSession`, `BindShortcuts`, `Activated` signal. ~150 lines zbus or `ashpd` (MIT) which wraps it. Commands exist (`playback.toggle`, marker add). **3 h.** Keep off by default; GNOME ≥ 45 also supports it.

### L1.9 HDR on KWin (investigate only) — note now, no build
- wgpu-hal 30 Vulkan enables `VK_EXT_swapchain_colorspace` and maps `EXTENDED_SRGB_LINEAR` / `HDR10` colour spaces (`wgpu-hal-30.0.1/src/vulkan/instance.rs:280`, `conv.rs:186-189`), so an `Rgba16Float` extended-sRGB surface is possible from Rust alone; on Wayland the Mesa WSI, not the app, speaks `wp_color_management_v1` (in `wayland-protocols-0.32.13/protocols/staging/color-management`), and RADV only does so when `wp_color_management` is advertised and the driver has that support (Mesa 25.x, KWin 6.2+ with HDR enabled on the output). egui / eframe 0.36 create the surface with an sRGB-ish 8-bit format and know nothing about HDR, so the Program monitor would need its own `wgpu::Surface` or an eframe change. No `unsafe` needed anywhere; the work is an eframe surface-format option + a tone-mapped UI around an HDR picture. Write it up in OWNER-QUESTIONS with a 20–30 h size. **1 h** for the write-up.

## 2. Phase L2, item by item

### L2.1 XDG base directories — **Partly**
- Have: data dir follows `XDG_DATA_HOME`; `dev_icon.rs` too. Missing: config (`prefs.json` sits in the data dir), cache (`Media Cache`, Previews, Proxies, model downloads under data), state (logs, crash logs, graphics-start state); no `XDG_CONFIG_HOME` / `XDG_CACHE_HOME` / `XDG_STATE_HOME` anywhere (`grep` of engine / app). `cache.claimFolder` done 2026-10-15 (Settings ▸ Media Cache ▸ Render Cache ▸ Claim Files of No Known Project).
- Route: one `filmcraft_engine::dirs` module (no new crate; `dirs 6.0` is already in the tree if wanted, MIT/Apache-2.0) returning `{data, config, cache, state}` with the same `FILMCRAFT_DATA_DIR` / portable overrides collapsing all four; `media_cache_dir` default → `$XDG_CACHE_HOME/filmcraft`; logs + crash logs + graphics state → state dir; a one-time migration that **moves** `prefs.json` and leaves caches (rebuilt) — never deletes; `Help ▸ Reveal Log Files` follows. Mind the fork launcher's `--data-dir ~/.local/share/filmcraft-fork` (§1a.9): the override must keep everything under that one folder. **5 h.** Risk: tests that assume `<data>/Logs`.

### L2.2 File associations, Dolphin, thumbnails — **Partly**
- Have: desktop file with `%F`, `Exec` args handled (`args.rs:35,70`, upstream #850 made them case-insensitive — in U0.2); hicolor icons all sizes; `postinst.sh` / `install.sh` run `update-mime-database`; Flatpak installs the MIME file. **MIME XML is empty** (`packaging/linux/ai.storyteller.filmcraft.mime.xml`, also empty upstream) — so no `.fcproj` type, no "Open with FilmCraft" on project files.
- Missing: `application/x-filmcraft-project` (glob `*.fcproj`, `*.FCPROJ`, magic on the JSON head, `sub-class-of application/json`), `MimeType=` line in the desktop file listing the project type and the camera / container types we decode (`video/mp4`, `video/quicktime`, `video/x-matroska`, `video/MP2T`, `image/x-adobe-dng`, …); desktop `Actions=` (New Project, Project Manager → `filmcraft --empty`, `--project-manager` flag to add); recent files: write `~/.local/share/recently-used.xbel` (freedesktop spec; KDE reads it since Plasma 5.x alongside its own `kactivitymanagerd` list) when a project is opened / saved — ~80 lines of XML handling, no crate, or the Recent Files portal is not a thing (skip portals).
- Thumbnails: freedesktop thumbnailer spec = a `*.thumbnailer` file under `share/thumbnailers/` with `Exec=filmcraft-cli thumbnail -s %s %u %o`, used by GNOME / Thunar / Nemo; **Plasma's Dolphin does not read `.thumbnailer` files** — it needs a KIO `ThumbCreator` plugin (C++), out of scope per the plan. On Plasma we rely on `ffmpegthumbs` for media (installed) and write down that `.fcproj` thumbnails need a KIO plugin; a cheap substitute: embed a PNG poster in the project folder's `.directory`? No — just document. The `filmcraft-cli thumbnail` subcommand (render frame 0 / poster at 256 px) is cheap because `render_frame` exists.
- **6 h** (MIME + desktop actions 2, recent files 1.5, thumbnailer CLI + file 2.5).

### L2.3 MPRIS — **Missing**
- Nothing on D-Bus today. Route: own `org.mpris.MediaPlayer2.filmcraft` with `org.mpris.MediaPlayer2` + `.Player` (PlayPause, Play, Pause, Next/Previous → step frame, Seek, `PlaybackStatus`, `Metadata` = project / sequence name, `Position`), via zbus `#[interface]` (~250 lines) or `mpris-server` 0.8 (MIT, zbus-based, ~80 lines). Media keys on Plasma reach it through the compositor's MPRIS handling; off (name released) when no project is open or Settings ▸ Playback ▸ Media keys is off. Playback commands exist; `Position` must be ticks → µs. **4 h.** Keep the name off when `--control` drives the app for tests? No: tests can read the interface through zbus too.

### L2.4 Cards and drives (udisks2) — **Missing**
- Nothing listens to mounts; `media_watch.rs` only re-stats known files. Phase 7.3 (card pulled mid-copy) is still ahead. Route: `org.freedesktop.UDisks2` `ObjectManager` `InterfacesAdded` + `org.freedesktop.UDisks2.Filesystem.MountPoints` on the system bus (zbus `fdo::ObjectManagerProxy`; ~150 lines, or the `udisks2` crate, MIT, zbus-based). Detect a camera layout (`DCIM/`, `PRIVATE/`, `CLIP/`, `XDROOT`) and show an "Import card" banner (a command `card.import { mount, dest, verify: true }` that copies with a size + xxhash/CRC check — needs a small hash in pure Rust: `crc32fast` (MIT/Apache-2.0; `xxhash-rust` is BSL-1.0, which the plan's MIT / Apache / BSD rule excludes); then `Filesystem.Unmount` + `Drive.PowerOff` for "eject when done"). Fallback when udisks is absent: watch `/run/media/$USER` with `notify`-style polling — just poll the directory every few seconds, no crate. **8 h.** Risk: system-bus policy on Flatpak (needs `--system-talk-name=org.freedesktop.UDisks2`).

### L2.5 Trash, not delete — **Have (nothing to do)**
- Audit of `remove_file` / `remove_dir_all` outside tests: proxies' partial files, previews, model downloads (`ai_explain.rs:766`), media-cache cleanup — all FilmCraft-owned caches, exempt by the plan. No user-file deletion exists (bins only remove items from the project). Route: a `platform::trash` shim is not needed; if a future feature deletes user files, use the `trash` crate (MIT/Apache-2.0, freedesktop trash spec, pure Rust) then. **0.5 h** to record this and add a `cargo xtask` grep guard? Skip the guard; **0.5 h** doc only.

### L2.6 GPU choice on hybrid systems — **Partly**
- Have: the interop path asks `PowerPreference::HighPerformance` (env `WGPU_POWER_PREF` overrides) on a Vulkan-only instance (`crates/ui-egui/src/lib.rs:483`), the export renderer too (`crates/gpu/src/lib.rs:1366`); `MESA_VK_DEVICE_SELECT` and `DRI_PRIME` are honoured by Mesa underneath. VA-API picks the render node itself (Phase 6: AMD renderD129 for decode, Intel for 4:2:2) — independent of the wgpu adapter.
- Missing: a Settings choice (`graphics.device`: Auto / adapter name), honouring `DRI_PRIME` when set by preferring the adapter Mesa reports first, switcheroo-control (`net.hadess.SwitcherooControl` on the system bus — not installed here; read-only hint, optional). Upstream's `graphics.rs` (#820, in U0.2) adds `native_adapter_selector` plumbing for eframe's own path; the fork's `interop_setup` uses `WgpuSetup::Existing` and must apply the same choice itself (`instance.enumerate_adapters(VULKAN)` → match name / `device_id`).
- Route: after U0.2, one `pick_adapter(prefs, adapters)` function shared by `interop_setup`, `graphics.rs` and the export renderer; Settings ▸ Playback ▸ Graphics device; `graphics.listDevices` command for agents. **4 h.** Risk: device loss when switching → restart required (as hardware decoding does).

### L2.7 Power and background work — **Partly**
- Have: `app_nap.rs` (macOS only). Missing on Linux: suspend inhibit (same portal `Inhibit` request as L1.7 with flag 8, or `org.freedesktop.login1.Manager.Inhibit` on the system bus — portal is enough), low-priority renders, GameMode.
- Route: thread priority with **rustix** (already locked, `rustix-1.1.5/src/process/priority.rs:130` `setpriority_process(Some(tid), 10)` — a Linux TID as `Pid` is per-thread; safe wrapper, no `libc` `unsafe`); ioprio has no rustix wrapper (`ioprio_set` syscall → would need `unsafe` in `crates/platform`; skip, `nice` 10 is enough with CFS). GameMode: `com.feralinteractive.GameMode` on the session bus is for games to *register*; FilmCraft should only *read* whether it's active and then not lower priority — optional, **drop** unless the owner uses it (not installed here). **3 h.**

### L2.8 Logging and crash reports — **Partly**
- Have: `logging.rs` (stderr + `<data>/Logs/filmcraft.log`, rotation), `crash.rs` panic hook → `crash-<day>.log` + in-app error window, Help ▸ Report an Issue opens the tracker (#642 closed).
- Journal: apps launched from Plasma / `systemd-run --user --scope` already have stderr captured by `journalctl --user` (identifier = binary name) — so "logs to the journal" is **Have** by accident, minus levels. Proper fields need the journal socket (`/run/systemd/journal/socket`, datagram with `KEY=VALUE\n` — pure Rust, ~60 lines with `std::os::unix::net::UnixDatagram`, no crate; `libsystemd` / `systemd-journal-logger` (MIT/Apache-2.0) do the same with memfd for large records). Add it as a second `log` sink only when the socket exists and not under Flatpak (no socket there).
- Crash reports: move crash logs to the state dir (L2.1); the error window gets "Report an issue" that opens the tracker **and** reveals the crash log (never uploads). **3 h.**

### L2.9 Packaging for Arch / CachyOS, Flatpak, validators — **Partly**
- Have: nfpm deb/rpm, Flatpak manifests (runtime 25.08, `--socket=wayland`, `fallback-x11`, `--device=dri`, `--filesystem=home` …), `package.sh` validation steps, `install.sh` for tarballs. Missing: a `PKGBUILD` (`packaging/linux/arch/PKGBUILD`: `cargo build --release --locked -p filmcraft -p filmcraft-cli`, install desktop / icons / MIME / metainfo / thumbnailer, `depends=(gcc-libs glibc libxkbcommon vulkan-icd-loader alsa-lib)`, `optdepends=(xdg-desktop-portal-kde: native dialogs, libva-mesa-driver, ffmpeg: fork-only paths)`, `-git` variant optional); Flatpak finish-args for the new portals (none needed for Settings / FileChooser / Inhibit / Notification / GlobalShortcuts — they are portals; **MPRIS** needs `--own-name=org.mpris.MediaPlayer2.filmcraft`, **udisks** `--system-talk-name=org.freedesktop.UDisks2`, LauncherEntry needs `--talk-name` nothing (it's a broadcast signal; works), journal: no socket in the sandbox); metainfo `releases` entries for the fork tags and the new `screenshots`; `desktop-file-validate` on `MimeType=` / `Actions=` lines. `appstreamcli` 1.2 and `desktop-file-validate` are on this PC. **5 h** (PKGBUILD 2, Flatpak 1, validators + metainfo 2). Risk: `namcap` warnings on the big binary; none technical.

## 3. Crates (all pure Rust; licenses from their manifests or crates.io)

| Need | Crate | License | Already on disk? | Verdict |
|---|---|---|---|---|
| D-Bus (portals, MPRIS, udisks, LauncherEntry, Notifications) | `zbus 5.19` | MIT | yes (locked) | use for everything; its `fdo` module has `ObjectManager` and `Properties` proxies |
| Portal convenience | `ashpd` 0.11 | MIT | no | optional; only saves code for GlobalShortcuts / Notification request handling (~200 lines). Pulls `zbus` + `futures`; fine but not needed. Decide in L1.7: zbus-only first |
| Notifications | `notify-rust` 4 | MIT OR Apache-2.0 | no | not needed; one `Notify` call via zbus |
| MPRIS | `mpris-server` 0.8 | MIT (confirm on crates.io before adding; not on disk) | no | optional; saves ~150 lines of interface boilerplate; or zbus `#[interface]` by hand |
| udisks2 | `udisks2` (zbus proxies) | MIT (confirm; not on disk) | no | optional; two proxies by hand are enough |
| Trash | `trash` 5 | MIT OR Apache-2.0 | no | not needed now (L2.5) |
| XDG dirs | `dirs 6` | MIT OR Apache-2.0 | yes (transitive) | optional; four `env` reads are simpler and keep the `FILMCRAFT_DATA_DIR` semantics |
| Journal | `libsystemd` / `systemd-journal-logger` | MIT OR Apache-2.0 | no | not needed; `UnixDatagram` to the journal socket |
| Thread priority | `rustix 1.1` (`process` feature) | Apache-2.0 WITH LLVM-exception OR MIT | yes (locked) | use; avoids `libc` `unsafe` |
| Wayland clipboard | `wl-clipboard-rs` via `arboard/wayland-data-control` | MIT OR Apache-2.0 | no | one feature flag |
| GNOME title bar | `sctk-adwaita` via `winit/wayland-csd-adwaita` | MIT | no | one feature flag (+ `ab_glyph`, Apache-2.0) |
| Wayland DnD / pinch | none: needs winit ≥ 0.31 or a fork | — | — | blocked; see L1.6 |

No GPL / LGPL anywhere in the list. None needs `unsafe`; nothing new goes into `crates/platform`
except an optional `ioprio_set` if the owner insists on I/O priority (recommend not).

## 4. Recommended build order and changes to the plan

**Do L2.1 (XDG dirs) first**, before anything in L1 that writes state (window geometry, last
folders, crash logs): it moves files once and everything after lands in the right place. Then:

1. **L1 quick wins (one session, ~10 h):** L1.3 feature flag + geometry persistence → L1.4 accent /
   motion / text-scale from the portal → L1.6 clipboard feature + IME / touchpad tests → L1.1 /
   L1.2 verification screenshots on this PC.
2. **L1.7 D-Bus session (~9 h):** inhibit (shared with L2.7) → LauncherEntry progress →
   notification with "Show in folder" → single instance / activation → KDE job tracker last
   (KDE-only, drop if time runs out). L1.8 GlobalShortcuts right after if the owner wants it
   (3 h, otherwise defer to a later phase).
3. **L1.5 after U0.2** (the fork order has U0.2 before L1, so this holds): parent window, last
   folders. L1.9 is a 1 h write-up at any point.
4. **L2 (≈ 35 h):** L2.2 MIME / actions / recent / thumbnailer → L2.3 MPRIS → L2.8 journal + crash
   report action → L2.6 GPU choice (after U0.2's `graphics.rs`) → L2.7 priorities → L2.4 udisks
   (largest; shares the verified-copy tests with Phase 7.3, so write the copy routine in
   `crates/engine` with its tests first) → L2.9 PKGBUILD / Flatpak / validators last, since every
   earlier item changes what they install.

**Drop or move:**
- **Wayland drag-and-drop and pinch (L1.6):** blocked by winit 0.30; record in OWNER-QUESTIONS
  with the two routes (wait for egui's winit bump vs. a winit fork, 12 h) and ship the fallbacks
  (Open With, Paste, Import dialog, single-instance file hand-off). Don't put it in L1's acceptance
  ("drag from Dolphin works into all three targets" cannot pass on Wayland today; it does on X11).
- **L1.4 double-click interval:** egui has no such setting; drop.
- **L2.5 trash:** nothing deletes user files; reduce to a one-line rule in docs/architecture.
- **L2.7 GameMode:** not installed, no use case for an editor; drop.
- **L2.2 Dolphin thumbnails of `.fcproj`:** needs a KIO C++ plugin; write the `.thumbnailer` for
  GNOME-family managers and document the Plasma limit (plan already allows this).
- **#617** is not a Wayland / KDE issue (NVIDIA + Cinnamon, audio-device class); leave it to the
  Phase 7 soak, out of L1 / L2.
- **Plasma job tracker (L1.7)** is KDE-only with no fallback value beyond the LauncherEntry
  progress; make it the last, optional item.

**Revised totals:** L1 ≈ 22 h without DnD / pinch (plan said 20–40); L2 ≈ 35 h (plan said 25–45).
Both fit the §1a.41 pattern of grouped acceptance if L1 turns out small.
