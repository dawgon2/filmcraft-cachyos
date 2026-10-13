# STORAGE-PLAN.md: projects and media on a home server

Written 2026-10-12 (live session, with the owner). Order: FORK-PLAN §1a.6 / §1a.50 / §1a.51 (S1 → S2 after
C2, its AI2 training and AUDIO-PLAN A2, before G3; so Phase 7's soak covers network drops). Every FORK-PLAN rule
applies (never crash, `Tick` time, everything is a command, every widget has an automation id,
layering: network code stays below the UI and never in L0, `cargo xtask ci` per phase, a build
and a `cachyos` publish per phase). Generic, not fork-only: written to be upstream-contributable.
Not in scope: multi-user collaboration (several people editing one project at once, ≈ 100+ h).

## Phase S1: network shares done right (≈ 10–20 h)

SMB / NFS shares mounted by Plasma (kio-fuse), fstab or autofs already look like local folders;
this phase makes FilmCraft safe and smooth on them.
1. **Detect network paths** (statfs filesystem type: cifs, nfs, fuse.kio-fuse, sshfs…) and show
   it in the Media panel / Project Settings.
2. **Local media cache:** clips on a network path can be cached locally (whole file in the
   background, or just proxies / render cache, which default to local disk anyway), with a size
   cap and the existing claim-folder rules; playback reads the cache when it's complete.
3. **Server drops:** every read on a network path has a timeout off the UI thread; a dropped
   share turns its clips **offline** (the existing offline badge), never a hang or crash; when
   the share returns they relink by identity automatically.
4. **Safe saves:** write to a temp file on the same share, fsync, rename; verify by re-reading;
   always keep a local backup copy of the last good save (and autosaves locally), because rename
   over SMB isn't always atomic. A save that fails says so and keeps the local copy.

**Accept:** with a local Samba / NFS server in a container or VM (or the Dell as the server): play
and edit a project stored on the share; pull the network mid-playback and mid-save — UI stays
responsive, clips go offline, no data lost (the local backup opens), reconnect relinks; 6.2K
playback from the cache matches local playback (synthetic). Tag `storage-1`.

## Phase S2: built-in remote locations (≈ 30–60 h)

Connect without mounting anything.
1. **Backends** behind one `RemoteStore` trait (list, stat, read range, write, rename, lock):
   **WebDAV** (Nextcloud, ownCloud, Synology), **SFTP** (any Linux server; pure-Rust `russh`,
   Apache-2.0) and **S3-compatible** (MinIO, Garage; signed requests over the workspace's
   rustls / ureq). Pure Rust, no system libraries required.
2. **Settings ▸ Remote locations:** add / test / remove a server; **passwords and keys in the
   system keyring** (Secret Service / KWallet over zbus, a file-backed fallback only if the user
   allows it), never in project files or logs.
3. **Media panel:** browse a remote location like a folder; import downloads into the local
   media cache (S1) with progress and resume; or link remote media and stream with read-ahead
   when the connection is fast enough.
4. **Projects on the server:** open / save / Save As to a remote location; a **lock file**
   (holder, machine, time) so two machines can't overwrite each other (read-only open or take
   over, with a warning); conflict copies instead of overwrites; the last N versions kept on the
   server; local backup always (S1.4).
5. Commands `remote.*` (add, test, list, import, open, save, unlock) for CLI / MCP.

**Accept:** against local test servers (Nextcloud or another WebDAV server, OpenSSH, MinIO, in
containers): browse, import, open and save on each; a second "machine" (another process) sees the
lock; kill the connection mid-transfer → resume or clean error, never a corrupt project; no
credential appears in any file under the project or log dir (checked by a test). Tag
`storage-2`.
