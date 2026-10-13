//! Card offload (LINUX-PLAN L2.4): card detection, the capped walk and the verified copy, with
//! hostile cards (link loops, huge trees, files that change or vanish while copied).

use std::path::Path;

use crate::card_offload::{CopyError, copy_verified, crc_of, detect, list, list_capped};
use crate::media_test_util::tmp_dir;

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn no_parts(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().ends_with(".filmcraft-part") {
                return false;
            }
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            }
        }
    }
    true
}

#[test]
fn camera_cards_are_told_from_other_drives() {
    let root = tmp_dir("card-detect");
    let card = |name: &str, dirs: &[&str]| {
        let m = root.join(name);
        std::fs::create_dir_all(&m).unwrap();
        for d in dirs {
            std::fs::create_dir_all(m.join(d)).unwrap();
        }
        m
    };
    assert_eq!(detect(&card("fuji", &["DCIM/100_FUJI"])), Some("dcim"));
    assert_eq!(detect(&card("sony", &["private/M4ROOT/CLIP"])), Some("private"), "any case");
    assert_eq!(detect(&card("both", &["PRIVATE", "DCIM"])), Some("dcim"), "the first marker wins");
    assert_eq!(detect(&card("xdcam", &["XDROOT/Clip"])), Some("xdcam"));
    assert_eq!(detect(&card("usb", &["Documents", "Music"])), None);
    let fake = card("fake", &[]);
    write(&fake.join("DCIM"), b"a file, not a folder");
    assert_eq!(detect(&fake), None);
    assert_eq!(detect(&root.join("missing")), None);
}

#[test]
fn the_walk_skips_hidden_files_and_links_and_is_capped() {
    let card = tmp_dir("card-walk");
    write(&card.join("DCIM/100_FUJI/DSCF0001.MOV"), b"movie");
    write(&card.join("DCIM/100_FUJI/DSCF0002.JPG"), b"jpg");
    write(&card.join("PRIVATE/M4ROOT/CLIP/C0001M01.XML"), b"<xml/>");
    write(&card.join(".Trashes/x"), b"x");
    write(&card.join("._DSCF0001.MOV"), b"x");
    write(&card.join("System Volume Information/IndexerVolumeGuid"), b"x");
    #[cfg(unix)]
    {
        // a loop and a link out of the card are never followed
        std::os::unix::fs::symlink(&card, card.join("DCIM/loop")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", card.join("DCIM/out")).unwrap();
    }
    let l = list(&card).unwrap();
    let rels: Vec<String> = l.files.iter().map(|f| f.rel.to_string_lossy().into_owned()).collect();
    assert_eq!(rels, ["DCIM/100_FUJI/DSCF0001.MOV", "DCIM/100_FUJI/DSCF0002.JPG", "PRIVATE/M4ROOT/CLIP/C0001M01.XML"]);
    assert_eq!(l.bytes, 5 + 3 + 6);
    assert!(!l.truncated);

    let capped = list_capped(&card, 1, 1000).unwrap();
    assert_eq!(capped.files.len(), 1);
    assert!(capped.truncated);
    assert!(list_capped(&card, 1000, 2).unwrap().truncated);

    // deeper than MAX_DEPTH: listed down to the cap, flagged
    let deep = tmp_dir("card-deep");
    let mut p = deep.join("DCIM");
    for i in 0..(crate::card_offload::MAX_DEPTH + 5) {
        p = p.join(format!("d{i}"));
    }
    write(&p.join("f.mov"), b"x");
    let l = list(&deep).unwrap();
    assert!(l.truncated && l.files.is_empty());
    assert!(list(&deep.join("missing")).is_err());
}

#[test]
fn a_copy_is_verified_and_leaves_nothing_half_written() {
    let root = tmp_dir("card-copy");
    let src = root.join("card/DCIM/A001.MOV");
    let bytes: Vec<u8> = (0..(3u32 << 20) + 17).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect();
    write(&src, &bytes);
    let size = bytes.len() as u64;
    let dst = root.join("drive/Shoot/DCIM/A001.MOV");

    let mut calls = 0;
    let c = copy_verified(&src, size, &dst, &mut |_| {
        calls += 1;
        true
    })
    .unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), bytes);
    assert_eq!(c.crc, crc32fast::hash(&bytes));
    assert_eq!((c.bytes, c.already), (size, false));
    assert!(calls >= 3, "progress per chunk");
    assert!(no_parts(&root.join("drive")));

    // run again: the identical file is kept
    let again = copy_verified(&src, size, &dst, &mut |_| true).unwrap();
    assert_eq!((again.bytes, again.already, again.crc), (0, true, c.crc));
    // the check of an identical file reports progress and can be cancelled (both halves)
    let mut seen = Vec::new();
    copy_verified(&src, size, &dst, &mut |d| {
        seen.push(d);
        true
    })
    .unwrap();
    assert!(seen.len() >= 6 && seen.iter().all(|d| *d <= size) && seen.last() == Some(&size), "{seen:?}");
    assert_eq!(copy_verified(&src, size, &dst, &mut |_| false), Err(CopyError::Cancelled));
    assert_eq!(copy_verified(&src, size, &dst, &mut |d| d < size / 2 + 1), Err(CopyError::Cancelled), "cancelled in the read-back");
    assert_eq!(std::fs::read(&dst).unwrap(), bytes, "left alone");

    // a different file of the same name and size is never overwritten
    let other = root.join("card/DCIM/B001.MOV");
    let mut changed = bytes.clone();
    if let Some(b) = changed.last_mut() {
        *b ^= 0xFF;
    }
    write(&other, &changed);
    assert_eq!(copy_verified(&other, size, &dst, &mut |_| true), Err(CopyError::Exists));
    assert_eq!(std::fs::read(&dst).unwrap(), bytes, "left alone");

    // cancelled half-way: nothing at the destination
    let dst2 = root.join("drive/Shoot/DCIM/C001.MOV");
    assert_eq!(copy_verified(&src, size, &dst2, &mut |done| done < (1 << 20)), Err(CopyError::Cancelled));
    assert!(!dst2.exists());
    assert!(no_parts(&root.join("drive")));
}

/// A card pulled out (or a file cut short) mid-copy: an error, never a short file under the
/// real name and never a panic.
#[test]
fn a_source_that_changes_or_vanishes_is_an_error() {
    let root = tmp_dir("card-pulled");
    let src = root.join("card/A.MOV");
    write(&src, &[7u8; 4096]);
    let drive = root.join("drive");

    // listed at 8192 bytes, only 4096 there now
    let r = copy_verified(&src, 8192, &drive.join("A.MOV"), &mut |_| true);
    assert!(matches!(r, Err(CopyError::SourceChanged(_))), "{r:?}");
    // listed smaller than it is (it grew)
    let r = copy_verified(&src, 100, &drive.join("B.MOV"), &mut |_| true);
    assert!(matches!(r, Err(CopyError::SourceChanged(_))), "{r:?}");
    // gone
    let r = copy_verified(&root.join("card/missing.MOV"), 10, &drive.join("C.MOV"), &mut |_| true);
    assert!(matches!(r, Err(CopyError::SourceChanged(_))), "{r:?}");
    // the file shrinks while it's being copied
    let big = root.join("card/BIG.MOV");
    write(&big, &vec![1u8; 3 << 20]);
    let r = copy_verified(&big, 3 << 20, &drive.join("BIG.MOV"), &mut |done| {
        if done <= 1 << 20 {
            let f = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
            f.set_len(1 << 20).unwrap();
        }
        true
    });
    assert!(matches!(r, Err(CopyError::SourceChanged(_))), "{r:?}");
    for n in ["A.MOV", "B.MOV", "C.MOV", "BIG.MOV"] {
        assert!(!drive.join(n).exists(), "{n}");
    }
    assert!(no_parts(&drive));
    // CRC-32 (IEEE) check value
    write(&root.join("check"), b"123456789");
    assert_eq!(crc_of(&root.join("check"), 9, &mut |_| true), Ok(0xCBF4_3926));
    // a hostile size: no huge allocation, just an error
    assert!(matches!(crc_of(&src, u64::MAX, &mut |_| true), Err(CopyError::SourceChanged(_))));
}

#[test]
fn a_destination_that_cannot_be_written_is_an_error() {
    let root = tmp_dir("card-readonly");
    let src = root.join("A.MOV");
    write(&src, b"abc");
    // the destination's folder is a file
    write(&root.join("drive"), b"not a folder");
    let r = copy_verified(&src, 3, &root.join("drive/A.MOV"), &mut |_| true);
    assert!(matches!(r, Err(CopyError::Io(_))), "{r:?}");
    // a folder already at the destination name
    std::fs::create_dir_all(root.join("d2/A.MOV")).unwrap();
    assert_eq!(copy_verified(&src, 3, &root.join("d2/A.MOV"), &mut |_| true), Err(CopyError::Exists));
}

fn bin_items(s: &crate::Session, name: &str) -> Option<usize> {
    s.project.root.children.iter().find_map(|e| match e {
        filmcraft_project::BinEntry::Bin(b) if b.name == name => Some(b.children.len()),
        _ => None,
    })
}

#[test]
fn card_commands_refuse_hostile_parameters() {
    use serde_json::json;
    let root = tmp_dir("card-cmd-bad");
    let card = root.join("CARD");
    write(&card.join("DCIM/A.MOV"), b"x");
    let file = root.join("file");
    write(&file, b"x");
    let empty = root.join("EMPTY");
    std::fs::create_dir_all(empty.join("DCIM")).unwrap();
    let drive = root.join("drive");
    std::fs::create_dir_all(&drive).unwrap();
    let mut s = crate::Session::default();
    let (c, d, f, e) = (card.to_string_lossy(), drive.to_string_lossy(), file.to_string_lossy(), empty.to_string_lossy());
    for p in [
        json!({}),
        json!({"mount": 3}),
        json!({"mount": ""}),
        json!({"mount": "CARD"}),
        json!({"mount": f}),
        json!({"mount": "/no/such/card"}),
        json!({"mount": "/a\u{0}b"}),
    ] {
        assert!(s.execute("card.inspect", p.clone()).is_err(), "{p}");
    }
    for p in [
        json!({"mount": c}),
        json!({"mount": c, "destination": "drive"}),
        json!({"mount": c, "destination": f}),
        json!({"mount": c, "destination": c}),
        json!({"mount": c, "destination": card.join("DCIM").to_string_lossy()}),
        // the card's parent with the card's own name: the copy would be the card itself
        json!({"mount": c, "destination": root.to_string_lossy(), "name": "CARD"}),
        json!({"mount": e, "destination": d}),
    ] {
        let mut p = p;
        p["wait"] = json!(true);
        assert!(s.execute("card.offload", p.clone()).is_err(), "{p}");
    }
    assert!(s.jobs.is_empty(), "nothing started");
    assert_eq!(crate::card_offload::clean_name(".."), None);
    assert_eq!(crate::card_offload::clean_name(" \u{7}/ "), None);
    assert_eq!(crate::card_offload::clean_name("../../etc").as_deref(), Some("....etc"));
    assert_eq!(crate::card_offload::clean_name(&"n".repeat(1000)).map(|n| n.chars().count()), Some(120));
}

#[test]
fn a_card_is_copied_verified_and_its_media_imported_into_a_bin() {
    use serde_json::json;
    let root = tmp_dir("card-cmd");
    let card = root.join("FUJI_X-H2S");
    std::fs::create_dir_all(card.join("DCIM/100_FUJI")).unwrap();
    crate::media_test_util::make_movie(&card.join("DCIM/100_FUJI/DSCF0001.MOV"), filmcraft_media::DemoScene::Aurora, 64, 36, 4);
    write(&card.join("DCIM/100_FUJI/DSCF0001.XML"), b"<meta/>");
    let drive = root.join("drive");
    std::fs::create_dir_all(&drive).unwrap();
    let mut s = crate::Session::default();

    let i = s.execute("card.inspect", json!({"mount": card.to_string_lossy()})).unwrap();
    assert_eq!((i["layout"].as_str(), i["files"].as_u64(), i["media"].as_u64()), (Some("dcim"), Some(2), Some(1)), "{i}");

    let r = s.execute("card.offload", json!({"mount": card.to_string_lossy(), "destination": drive.to_string_lossy(), "name": "Day 1", "wait": true})).unwrap();
    assert!(r.get("error").is_none(), "{r}");
    let folder = drive.join("Day 1");
    assert_eq!(std::fs::read(folder.join("DCIM/100_FUJI/DSCF0001.XML")).unwrap(), b"<meta/>");
    assert_eq!(std::fs::read(folder.join("DCIM/100_FUJI/DSCF0001.MOV")).unwrap(), std::fs::read(card.join("DCIM/100_FUJI/DSCF0001.MOV")).unwrap());
    let manifest = std::fs::read_to_string(folder.join(crate::card_offload::MANIFEST)).unwrap();
    assert_eq!(manifest.lines().count(), 2, "{manifest}");
    assert!(no_parts(&folder));
    assert_eq!(bin_items(&s, "Day 1"), Some(1), "only the movie is imported");
    assert!(s.card_jobs.is_empty());

    // run again after an interruption: the verified copies are kept, nothing is overwritten
    let r = s
        .execute(
            "card.offload",
            json!({"mount": card.to_string_lossy(), "destination": drive.to_string_lossy(), "name": "Day 1", "import": false, "wait": true}),
        )
        .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(bin_items(&s, "Day 1"), Some(1), "import: false");

    // default name: the card's folder name and the date
    let r = s.execute("card.offload", json!({"mount": card.to_string_lossy(), "destination": drive.to_string_lossy(), "import": false, "wait": true})).unwrap();
    let folder = r["folder"].as_str().unwrap_or_default().to_string();
    let date = folder.strip_prefix(&*drive.join("FUJI_X-H2S ").to_string_lossy()).unwrap_or_else(|| panic!("{folder}")).to_string();
    assert!(date.len() == 10 && date.as_bytes().get(4) == Some(&b'-') && date.starts_with("20"), "{folder}");
}
