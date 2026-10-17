//! STORAGE-PLAN S1 **Accept** on a real network share: an sshfs mount of the Dell
//! (`dell:~/fc-share`), which FilmCraft classifies as `fuse.sshfs` (a network share).
//!
//! Ignored (needs the network, ssh access to the Dell and ffmpeg as a fixture generator). One
//! command runs the whole acceptance; the test mounts, kills and remounts the share itself:
//!
//! ```sh
//! FILMCRAFT_S1_SHARE=~/fc-share cargo test --release -p filmcraft-engine \
//!     --test s1_real_share -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Optional: `FILMCRAFT_S1_REMOTE` (default `dell:~/fc-share`),
//! `FILMCRAFT_S1_FIXTURES` (a folder holding `a_6k2_hevc10.mp4` / `b_1080_h264.mp4`; made with
//! ffmpeg under `target/fixtures/s1share` when missing).
//!
//! Scenarios (every step bounded by a watchdog: a hang fails the test instead of waiting):
//! 1. media + project on the share: import, sequences, Save As on the share, reopen, decode;
//! 2. local copies (`mediaCache.networkCopies` auto): the 6.2K clip is copied and decodes
//!    bit-exact like the same file on a local disk;
//! 3. the network is pulled mid-playback (the sshfs process is killed): every frame returns within
//!    [`FRAME_BOUND`], the clips go offline, the copied clip keeps decoding from its copy;
//! 4. a save on the dead share fails within [`SAVE_BOUND`] and its local copy opens with the edit;
//! 5. reconnect (remount): the clips come back online by themselves, decode, and saves work again;
//! 6. a frozen server (the sshfs's ssh stopped, so requests hang instead of failing): frames and a
//!    save still return within their bounds.
//!
//! The share is left as it was found (mounted, or unmounted cleanly) and the test's folder on it
//! is removed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use filmcraft_engine::Session;
use filmcraft_project::ItemId;
use serde_json::{Value, json};

/// The share time limit (`filmcraft_media::deadline::NETWORK_TIMEOUT`, 8 s) plus 2 s margin.
const FRAME_BOUND: Duration = Duration::from_secs(10);
/// A save does a local copy, a write and a read-back: two share calls' worth plus margin.
const SAVE_BOUND: Duration = Duration::from_secs(20);
/// Clips back online after the share returns (the UI looks every few seconds; a share marked
/// down after a timeout is skipped for `netfs::DOWN_FOR` = 30 s).
const ONLINE_BOUND: Duration = Duration::from_secs(60);
const MOUNT_BOUND: Duration = Duration::from_secs(30);
const COPY_BOUND: Duration = Duration::from_secs(300);

const DEFAULT_REMOTE: &str = "dell:~/fc-share";
const CLIP_6K: &str = "a_6k2_hevc10.mp4";
const CLIP_HD: &str = "b_1080_h264.mp4";
/// Preview scale of the comparisons (the decoder still decodes the full 6240×4160 frame).
const SCALE: f32 = 0.25;

// ------------------------------------------------------------------------------------------
// The share: mount, kill, freeze, unmount
// ------------------------------------------------------------------------------------------

struct Share {
    mount: PathBuf,
    remote: String,
    was_mounted: bool,
    /// The test's own folder on the share (removed at the end).
    work: PathBuf,
}

/// Whether `mount` is an sshfs mount point right now (fresh read of the mount table).
fn is_sshfs_mount(mount: &Path) -> bool {
    let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let want = mount.to_string_lossy().replace(' ', "\\040");
    table.lines().any(|l| {
        let f: Vec<&str> = l.split(' ').collect();
        let fs = l.split(" - ").nth(1).and_then(|r| r.split(' ').next());
        f.get(4) == Some(&want.as_str()) && fs == Some("fuse.sshfs")
    })
}

/// Runs a command with a time limit; `true` when it exited successfully in time.
fn run_bounded(cmd: &mut Command, limit: Duration) -> bool {
    let Ok(mut child) = cmd.spawn() else { return false };
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if t0.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// `(pid, ppid, args)` of every process (from `ps`, never a `pkill -f` pattern).
fn processes() -> Vec<(u32, u32, String)> {
    let out = Command::new("ps").args(["-eo", "pid=,ppid=,args="]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            Some((pid, ppid, it.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

/// The sshfs process serving `mount` (its args name the mount point exactly).
fn sshfs_pid(mount: &Path) -> Option<u32> {
    let m = mount.to_string_lossy().into_owned();
    processes().into_iter().find(|(_, _, a)| a.split(' ').next().is_some_and(|c| c.ends_with("sshfs")) && a.split(' ').any(|w| w == m)).map(|(p, _, _)| p)
}

/// The ssh process of that sshfs: its child, or (started before sshfs went to the background and
/// so handed to the init process) the latest `ssh … -- <host> sftp` started just before it.
fn ssh_child(sshfs: u32, remote: &str) -> Option<u32> {
    let host = remote.split(':').next().unwrap_or(remote);
    let tail = format!("-- {host} sftp");
    let ssh: Vec<(u32, u32, String)> =
        processes().into_iter().filter(|(_, _, a)| a.split(' ').next().is_some_and(|c| c.ends_with("ssh")) && a.ends_with(&tail)).collect();
    if let Some((p, _, _)) = ssh.iter().find(|(_, pp, _)| *pp == sshfs) {
        return Some(*p);
    }
    ssh.iter().filter(|(p, _, _)| *p < sshfs && sshfs - *p < 50).map(|(p, _, _)| *p).max()
}

fn signal(pid: u32, sig: &str) -> bool {
    run_bounded(Command::new("kill").args([sig, &pid.to_string()]), Duration::from_secs(5))
}

impl Share {
    fn mount_now(&self) -> bool {
        let _ = std::fs::create_dir_all(&self.mount);
        let ok = run_bounded(
            Command::new("sshfs").args(["-o", "reconnect,ServerAliveInterval=5,ServerAliveCountMax=2"]).arg(&self.remote).arg(&self.mount),
            MOUNT_BOUND,
        );
        ok && is_sshfs_mount(&self.mount)
    }

    fn unmount(&self) {
        if !is_sshfs_mount(&self.mount) {
            return;
        }
        if !run_bounded(Command::new("fusermount3").arg("-u").arg(&self.mount), Duration::from_secs(10)) {
            let _ = run_bounded(Command::new("fusermount3").arg("-uz").arg(&self.mount), Duration::from_secs(10));
        }
    }

    /// Pulls the network: kills the sshfs process (its mount stays, every call on it fails).
    fn kill(&self) -> bool {
        sshfs_pid(&self.mount).is_some_and(|p| signal(p, "-KILL"))
    }

    /// Freezes the server: stops sshfs's ssh, so requests on the share hang (no error at all).
    fn freeze(&self) -> Option<u32> {
        let ssh = ssh_child(sshfs_pid(&self.mount)?, &self.remote)?;
        signal(ssh, "-STOP").then_some(ssh)
    }

    /// Unmounts whatever is there (a dead or frozen mount too) and mounts again.
    fn remount(&self) -> bool {
        if let Some(p) = sshfs_pid(&self.mount) {
            if let Some(ssh) = ssh_child(p, &self.remote) {
                let _ = signal(ssh, "-CONT");
                let _ = signal(ssh, "-KILL");
            }
            let _ = signal(p, "-KILL");
        }
        self.unmount();
        self.mount_now()
    }
}

/// Last resort at the end (or after a failed assertion): the test's folder is removed and the
/// share is left as it was found, mounted and working, or cleanly unmounted.
impl Drop for Share {
    fn drop(&mut self) {
        cleanup(self);
    }
}

fn cleanup(s: &Share) {
    // a frozen ssh (scenario 6) is woken first, so nothing below waits on it
    if let Some(ssh) = sshfs_pid(&s.mount).and_then(|p| ssh_child(p, &s.remote)) {
        let _ = signal(ssh, "-CONT");
    }
    let alive = sshfs_pid(&s.mount).is_some_and(|p| ssh_child(p, &s.remote).is_some()) && is_sshfs_mount(&s.mount);
    if !alive || !s.work.exists() {
        let _ = s.remount();
    }
    let t0 = Instant::now();
    let _ = std::fs::remove_dir_all(&s.work);
    eprintln!("[s1] cleanup: removed {} ({:.1} s)", s.work.display(), t0.elapsed().as_secs_f64());
    if !s.was_mounted {
        s.unmount();
    }
}

// ------------------------------------------------------------------------------------------
// The watchdog: a step that runs over its bound fails the test (and frees the share)
// ------------------------------------------------------------------------------------------

struct Watchdog {
    armed: Arc<Mutex<Option<(String, Instant, Duration)>>>,
}

impl Watchdog {
    fn new(mount: PathBuf, remote: String, work: PathBuf, was_mounted: bool) -> Self {
        let armed: Arc<Mutex<Option<(String, Instant, Duration)>>> = Arc::default();
        let a = armed.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(100));
                let over = a.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().filter(|(_, t, d)| t.elapsed() > *d).cloned();
                if let Some((label, _, d)) = over {
                    eprintln!("[s1] FAIL: {label} did not return within {:.0} s (a hang)", d.as_secs_f64());
                    let share = Share { mount: mount.clone(), remote: remote.clone(), was_mounted, work: work.clone() };
                    let _ = share.remount();
                    cleanup(&share);
                    std::mem::forget(share);
                    std::process::exit(101);
                }
            }
        });
        Self { armed }
    }

    /// Runs `f`, failing the whole test if it takes longer than `limit`; returns its result and
    /// how long it took.
    fn bounded<T>(&self, label: &str, limit: Duration, f: impl FnOnce() -> T) -> (T, Duration) {
        let t0 = Instant::now();
        *self.armed.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((label.to_string(), t0, limit));
        let r = f();
        *self.armed.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let took = t0.elapsed();
        assert!(took <= limit, "{label}: {took:?} > {limit:?}");
        (r, took)
    }
}

// ------------------------------------------------------------------------------------------
// Session helpers
// ------------------------------------------------------------------------------------------

fn fixtures() -> PathBuf {
    let dir = std::env::var_os("FILMCRAFT_S1_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/s1share"));
    let _ = std::fs::create_dir_all(&dir);
    let make = |name: &str, args: &[&str]| {
        let out = dir.join(name);
        if out.exists() {
            return;
        }
        eprintln!("[s1] making {} with ffmpeg", out.display());
        let ok = Command::new("nice")
            .args(["-n", "19", "ionice", "-c", "3", "ffmpeg", "-y", "-hide_banner", "-loglevel", "error"])
            .args(args)
            .arg(&out)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg failed for {name}");
    };
    make(
        CLIP_6K,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=6240x4160:rate=24:duration=3",
            "-c:v",
            "libx265",
            "-preset",
            "fast",
            "-pix_fmt",
            "yuv420p10le",
            "-x265-params",
            "log-level=error",
            "-tag:v",
            "hvc1",
        ],
    );
    make(CLIP_HD, &["-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=24:duration=3", "-c:v", "libx264", "-preset", "fast", "-pix_fmt", "yuv420p"]);
    dir
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("filmcraft-s1share-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// A session with a data folder of its own (the local save copies go there) and its media cache
/// (local copies of share clips) in `cache`.
fn session(data: &Path, cache: &Path) -> Session {
    let mut s = Session::default();
    s.prefs_path = Some(data.join("preferences.json"));
    s.prefs.media_cache.location = cache.to_string_lossy().into_owned();
    s.prefs.media_cache.network_copies = "off".into();
    s
}

/// Imports `paths` and makes one sequence per clip; returns (items, sequences) in order.
fn import(s: &mut Session, paths: &[PathBuf]) -> (Vec<ItemId>, Vec<ItemId>) {
    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let r = s.execute("file.import", json!({"paths": paths})).unwrap_or_else(|e| panic!("import: {e}"));
    let items: Vec<ItemId> = r["items"].as_array().map(|a| a.iter().filter_map(Value::as_u64).map(ItemId).collect()).unwrap_or_default();
    assert_eq!(items.len(), paths.len(), "{r}");
    let seqs = items
        .iter()
        .map(|i| {
            let r = s.execute("file.newSequenceFromClip", json!({"items": [i.0]})).unwrap_or_else(|e| panic!("sequence: {e}"));
            ItemId(r["sequence"].as_u64().unwrap_or_default())
        })
        .collect();
    (items, seqs)
}

/// One frame of `seq` as RGBA8 over black (`None`: nothing rendered).
fn frame(s: &mut Session, seq: ItemId, n: i64) -> Option<Vec<u8>> {
    s.state.active_sequence = Some(seq);
    let t = s.sequence_rate().tick_of(n);
    s.render_program_at(SCALE, t).map(|i| i.over_black_rgba8())
}

/// Whether a frame shows picture (testsrc2 is colourful; an offline slate or black isn't).
fn colourful(px: &[u8]) -> bool {
    let distinct: std::collections::HashSet<[u8; 3]> = px.as_chunks::<4>().0.iter().step_by(37).map(|c| [c[0] / 16, c[1] / 16, c[2] / 16]).collect();
    distinct.len() > 12
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// Looks at the media files the way the UI does (`start_media_scan` on a worker, polled) until
/// `want` says the result is the one expected, or `limit` passes. Returns the scans' merged
/// `online` / `offline` ids.
fn scan_until(dog: &Watchdog, s: &mut Session, label: &str, limit: Duration, want: impl Fn(&[u64], &[u64]) -> bool) -> (Vec<u64>, Vec<u64>, Duration) {
    let ((on, off), took) = dog.bounded(label, limit + Duration::from_secs(5), || {
        let (mut on, mut off) = (Vec::new(), Vec::new());
        let t0 = Instant::now();
        while t0.elapsed() < limit {
            s.start_media_scan();
            if let Some(r) = s.poll_media_scan() {
                let r = r.unwrap_or_else(|e| panic!("{label}: {e}"));
                let ids = |k: &str| r[k].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect::<Vec<_>>()).unwrap_or_default();
                on.extend(ids("online"));
                off.extend(ids("offline"));
                if want(&on, &off) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        (on, off)
    });
    (on, off, took)
}

// ------------------------------------------------------------------------------------------
// The acceptance
// ------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs a real share: FILMCRAFT_S1_SHARE=<sshfs mount point> (see the module docs)"]
fn s1_acceptance_on_a_real_share() {
    let Some(mount) = std::env::var_os("FILMCRAFT_S1_SHARE").map(PathBuf::from) else {
        eprintln!("[s1] SKIPPED: FILMCRAFT_S1_SHARE is not set");
        return;
    };
    let remote = std::env::var("FILMCRAFT_S1_REMOTE").unwrap_or_else(|_| DEFAULT_REMOTE.into());
    let was_mounted = is_sshfs_mount(&mount);
    let work = mount.join(format!("s1-acceptance-{}", std::process::id()));
    let share = Share { mount: mount.clone(), remote: remote.clone(), was_mounted, work: work.clone() };
    if !was_mounted {
        let t0 = Instant::now();
        if !share.mount_now() {
            eprintln!("[s1] SKIPPED: {} is not an sshfs mount and mounting {remote} failed", mount.display());
            std::mem::forget(share);
            return;
        }
        eprintln!("[s1] mounted {remote} on {} in {:.2} s", mount.display(), secs(t0.elapsed()));
    }
    let dog = Watchdog::new(mount.clone(), remote.clone(), work.clone(), was_mounted);
    assert!(filmcraft_engine::netfs::classify(&work).is_network(), "Revolution sees the sshfs mount as a network share");

    let fx = fixtures();
    let data = tmp("data");
    let cache = tmp("cache");
    let _ = std::fs::create_dir_all(&work);
    let (on_share, took) = dog.bounded("copy the clips to the share", Duration::from_secs(300), || {
        [CLIP_6K, CLIP_HD]
            .iter()
            .map(|c| {
                let to = work.join(c);
                std::fs::copy(fx.join(c), &to).unwrap_or_else(|e| panic!("copy {c} to the share: {e}"));
                to
            })
            .collect::<Vec<_>>()
    });
    let bytes: u64 = on_share.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    eprintln!("[s1] copied {:.1} MB to the share in {:.2} s", bytes as f64 / 1e6, secs(took));

    // the reference: the same files decoded from a local disk
    let (mut reference, frames) = {
        let mut r = session(&tmp("ref-data"), &tmp("ref-cache"));
        let paths: Vec<PathBuf> = [CLIP_6K, CLIP_HD].iter().map(|c| fx.join(c)).collect();
        let (_, seqs) = import(&mut r, &paths);
        let ((a, b), took) = dog.bounded("local reference frames", Duration::from_secs(300), || {
            let a: Vec<Vec<u8>> = [10, 40, 60].iter().map(|&n| frame(&mut r, seqs[0], n).unwrap_or_default()).collect();
            let b: Vec<Vec<u8>> = (0..72).map(|n| frame(&mut r, seqs[1], n).unwrap_or_default()).collect();
            (a, b)
        });
        assert!(a.iter().chain(&b).all(|f| colourful(f)), "the local reference decodes");
        eprintln!("[s1] local reference: 3 frames of 6.2K + 72 of 1080p in {:.2} s", secs(took));
        (r, (a, b))
    };
    let (ref_6k, ref_hd) = frames;

    // ---- 1. media and project on the share --------------------------------------------------
    let proj = work.join("s1 acceptance.fcproj").to_string_lossy().into_owned();
    let mut s = session(&data, &cache);
    let ((items, seqs), took) = dog.bounded("import from the share", Duration::from_secs(60), || import(&mut s, &on_share));
    eprintln!("[s1] 1. imported 2 clips from the share in {:.2} s", secs(took));
    let storage = s.execute("media.storage", json!({})).unwrap_or_default();
    assert_eq!(storage["networkItems"], json!(2), "{storage}");
    let (r, took) = dog.bounded("save as on the share", SAVE_BOUND, || s.execute("file.saveAs", json!({"path": proj})));
    let r = r.unwrap_or_else(|e| panic!("save as: {e}"));
    let backup = PathBuf::from(r["localCopy"].as_str().unwrap_or_default());
    assert!(backup.starts_with(&data) && backup.exists(), "{r}");
    eprintln!("[s1] 1. saved on the share in {:.2} s (local copy {})", secs(took), backup.display());
    let (r, took) = dog.bounded("reopen from the share", Duration::from_secs(60), || s.execute("file.open", json!({"path": proj})));
    r.unwrap_or_else(|e| panic!("reopen: {e}"));
    assert_eq!(s.path.as_deref(), Some(proj.as_str()));
    let (f, took2) = dog.bounded("decode from the share", Duration::from_secs(120), || {
        (frame(&mut s, seqs[0], 10).unwrap_or_default(), (0..6).map(|n| frame(&mut s, seqs[1], n).unwrap_or_default()).collect::<Vec<_>>())
    });
    assert!(f.0 == ref_6k[0], "6.2K frame 10 from the share equals the local decode");
    for (n, px) in f.1.iter().enumerate() {
        assert!(ref_hd.get(n) == Some(px), "1080p frame {n} from the share equals the local decode");
    }
    eprintln!("[s1] 1. reopened in {:.2} s; 1 frame of 6.2K + 6 of 1080p decoded from the share in {:.2} s, bit-exact", secs(took), secs(took2));

    // ---- 2. local copies ------------------------------------------------------------------------
    s.prefs.media_cache.network_copies = "auto".into();
    s.apply_media_cache();
    let (copied, took) = dog.bounded("copy the 6.2K clip locally", COPY_BOUND + Duration::from_secs(5), || {
        let t0 = Instant::now();
        assert!(s.start_net_copy(), "the copier starts");
        loop {
            if let Some((src, r)) = s.poll_net_copy() {
                break (src, r);
            }
            assert!(t0.elapsed() < COPY_BOUND, "copy not done in {COPY_BOUND:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let copy_path = copied.1.unwrap_or_else(|e| panic!("the copy failed: {e}"));
    assert_eq!(copied.0, on_share[0].to_string_lossy(), "the 6.2K clip is copied first");
    assert!(copy_path.starts_with(&cache), "{}", copy_path.display());
    let size = std::fs::metadata(&copy_path).map(|m| m.len()).unwrap_or_default();
    eprintln!(
        "[s1] 2. local copy of the 6.2K clip: {:.1} MB in {:.2} s ({:.1} MB/s)",
        size as f64 / 1e6,
        secs(took),
        size as f64 / 1e6 / secs(took).max(0.001)
    );
    let (px, took) = dog.bounded("6.2K from the copy", Duration::from_secs(60), || frame(&mut s, seqs[0], 40).unwrap_or_default());
    assert!(px == ref_6k[1], "6.2K playback from the cache matches local playback (frame 40, bit-exact)");
    eprintln!("[s1] 2. 6.2K frame 40 from the local copy in {:.2} s: bit-exact with the local file", secs(took));
    // the 1080p clip stays without a copy for scenario 3 (the copier isn't started again)

    // ---- 3. pull the network mid-playback ----------------------------------------------------------
    let killed = Arc::new(AtomicBool::new(false));
    let killer = {
        let (k, m, r) = (killed.clone(), mount.clone(), remote.clone());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1000));
            let sh = Share { mount: m, remote: r, was_mounted: true, work: PathBuf::new() };
            let ok = sh.kill();
            std::mem::forget(sh);
            k.store(ok, Ordering::SeqCst);
            ok
        })
    };
    let mut worst = Duration::ZERO;
    let (mut good_after, mut bad_after, mut n_frames) = (0, 0, 0);
    let mut first_bad: Option<i64> = None;
    let t_play = Instant::now();
    let mut n = 6;
    // frames 6–47 (48–71 stay undecoded for scenarios 3, 5 and 6)
    while t_play.elapsed() < Duration::from_secs(6) && n < 48 {
        let before_kill = !killed.load(Ordering::SeqCst);
        let (px, took) = dog.bounded("a frame while the network is pulled", FRAME_BOUND, || frame(&mut s, seqs[1], n));
        worst = worst.max(took);
        n_frames += 1;
        if !before_kill {
            if px.as_deref() == ref_hd.get(n as usize).map(Vec::as_slice) {
                good_after += 1;
            } else {
                bad_after += 1;
                first_bad.get_or_insert(n);
            }
        }
        n += 1;
        // real time, 24 fps, so the pull lands mid-playback
        let due = Duration::from_secs_f64(f64::from(u32::try_from(n - 6).unwrap_or(0)) / 24.0);
        if let Some(wait) = due.checked_sub(t_play.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    assert!(killer.join().unwrap_or(false), "the sshfs process was killed");
    eprintln!(
        "[s1] 3. playback while the network was pulled: {n_frames} frames, slowest {:.3} s (bound {:.0} s); after the pull {good_after} still right (read-ahead), {bad_after} not",
        secs(worst),
        secs(FRAME_BOUND)
    );
    let (_, off, took) = scan_until(&dog, &mut s, "clips go offline", Duration::from_secs(20), |_, off| items.iter().all(|i| off.contains(&i.0)));
    assert!(items.iter().all(|i| off.contains(&i.0)), "both clips' files are unreachable: offline ({off:?})");
    assert!(items.iter().all(|i| s.offline.missing.contains(i)));
    eprintln!("[s1] 3. both clips reported offline {:.2} s after the look started", secs(took));
    // a frame never decoded in this session (frames decoded before the pull may still come from
    // the decoded-frame cache)
    let (px, took) = dog.bounded("1080p frame on the dead share", FRAME_BOUND, || frame(&mut s, seqs[1], 60));
    let right = px.as_deref() == ref_hd.get(60).map(Vec::as_slice);
    eprintln!("[s1] 3. the 1080p clip (no copy) answers an uncached frame in {:.3} s (its picture: {right})", secs(took));
    let (px, took) = dog.bounded("6.2K from the copy, share dead", FRAME_BOUND + Duration::from_secs(20), || frame(&mut s, seqs[0], 60).unwrap_or_default());
    assert!(px == ref_6k[2], "the copied clip keeps decoding from its copy (frame 60, bit-exact)");
    eprintln!("[s1] 3. the 6.2K clip keeps playing from its local copy: frame 60 in {:.2} s, bit-exact", secs(took));

    // ---- 4. pull the network mid-save ----------------------------------------------------------------
    s.state.active_sequence = Some(seqs[1]);
    let clips_before = s.project.sequence(seqs[1]).map(|q| q.video_tracks.first().map_or(0, |t| t.items.len())).unwrap_or(0);
    s.execute("sequence.addEdit", json!({"seconds": 1.0})).unwrap_or_else(|e| panic!("add edit: {e}"));
    let (r, took) = dog.bounded("save on the dead share", SAVE_BOUND, || s.execute("file.save", json!({})));
    let e = r.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("kept on this computer"), "the failed save says where it is kept: {e}");
    eprintln!("[s1] 4. save on the dead share failed in {:.3} s: {e}", secs(took));
    let backup = filmcraft_engine::netfs::backup_path(&data, &proj);
    let mut b = session(&tmp("backup-data"), &tmp("backup-cache"));
    let (r, took) = dog.bounded("open the local copy", Duration::from_secs(30), || b.execute("file.open", json!({"path": backup.to_string_lossy()})));
    r.unwrap_or_else(|e| panic!("the local copy opens: {e}"));
    let clips_after = b.project.sequence(seqs[1]).map(|q| q.video_tracks.first().map_or(0, |t| t.items.len())).unwrap_or(0);
    assert_eq!(clips_after, clips_before + 1, "the local copy has the edit made before the failed save");
    eprintln!("[s1] 4. the local copy {} opens ({:.2} s) with the last edit: no work lost", backup.display(), secs(took));
    b.shutdown();

    // ---- 5. reconnect ------------------------------------------------------------------------------------
    let (ok, took) = dog.bounded("remount", MOUNT_BOUND + Duration::from_secs(15), || share.remount());
    assert!(ok, "remounted");
    eprintln!("[s1] 5. remounted in {:.2} s", secs(took));
    let (on, _, took) = scan_until(&dog, &mut s, "clips back online", ONLINE_BOUND, |on, _| items.iter().all(|i| on.contains(&i.0)));
    assert!(items.iter().all(|i| on.contains(&i.0)), "both clips back online by themselves ({on:?})");
    assert!(s.offline.missing.is_empty(), "{:?}", s.offline.missing);
    eprintln!("[s1] 5. both clips back online {:.2} s after the share returned", secs(took));
    let (px, took) = dog.bounded("1080p after reconnect", FRAME_BOUND, || frame(&mut s, seqs[1], 71));
    assert!(px.as_deref() == ref_hd.get(71).map(Vec::as_slice), "the 1080p clip decodes from the share again (bit-exact)");
    eprintln!("[s1] 5. the 1080p clip decodes from the share again ({:.3} s, bit-exact)", secs(took));
    for n in first_bad.into_iter().chain([60]) {
        let px = frame(&mut s, seqs[1], n);
        assert!(px.as_deref() == ref_hd.get(n as usize).map(Vec::as_slice), "frame {n}, wrong while the share was down, is right again");
    }
    eprintln!("[s1] 5. frames {first_bad:?} and 60 (not shown while the share was down) show their picture again");
    let (r, took) = dog.bounded("save after reconnect", SAVE_BOUND, || s.execute("file.save", json!({})));
    let r = r.unwrap_or_else(|e| panic!("save after reconnect: {e}"));
    assert!(r["localCopy"].is_string(), "{r}");
    eprintln!("[s1] 5. saved on the share again in {:.2} s", secs(took));

    // ---- 6. a frozen server: requests hang instead of failing -----------------------------------------
    // a third clip, imported (its index read) but none of its pictures: every frame read goes to
    // the frozen share (the 1080p clip's few MB may be held by the reader already)
    let third = work.join("c_1080_h264.mp4");
    std::fs::copy(fx.join(CLIP_HD), &third).unwrap_or_else(|e| panic!("copy the third clip: {e}"));
    let (c_items, c_seqs) = import(&mut s, &[third]);
    let ssh = share.freeze();
    assert!(ssh.is_some(), "froze sshfs's ssh");
    // first, while nothing has timed out yet: the save's own write waits out the time limit
    let (r, took) = dog.bounded("save on a frozen share", SAVE_BOUND, || s.execute("file.save", json!({})));
    let e = r.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("kept on this computer"), "the save on a frozen share fails and says where it is kept: {e}");
    eprintln!("[s1] 6. save on a frozen share failed in {:.2} s: {e}", secs(took));
    let mut worst = Duration::ZERO;
    let mut shown = 0;
    for n in [20, 40, 60] {
        let (px, took) = dog.bounded("a frame on a frozen share", FRAME_BOUND, || frame(&mut s, c_seqs[0], n));
        worst = worst.max(took);
        shown += usize::from(px.as_deref() == ref_hd.get(n as usize).map(Vec::as_slice));
        eprintln!("[s1] 6. frozen share: frame {n} of the third clip in {:.2} s", secs(took));
    }
    eprintln!("[s1] 6. frozen share: 3 frames never read, slowest {:.2} s (bound {:.0} s), {shown} with their picture", secs(worst), secs(FRAME_BOUND));
    let (_, off, took) = scan_until(&dog, &mut s, "frozen share: clips go offline", Duration::from_secs(30), |_, off| off.contains(&c_items[0].0));
    assert!(off.contains(&c_items[0].0), "the frozen share's clips go offline ({off:?})");
    eprintln!("[s1] 6. frozen share: the clips reported offline after {:.2} s ({} items)", secs(took), off.len());
    let (ok, took) = dog.bounded("remount after the freeze", MOUNT_BOUND + Duration::from_secs(15), || share.remount());
    assert!(ok, "remounted");
    eprintln!("[s1] 6. remounted in {:.2} s", secs(took));

    s.shutdown();
    reference.shutdown();
    for d in [data, cache] {
        let _ = std::fs::remove_dir_all(d);
    }
    drop(share);
}
