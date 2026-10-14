//! Look packs (fork Y3.5): export → import round trip, renaming instead of overwriting, and
//! hostile archives refused with a message.

use std::io::Write;

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;
use crate::packs::{MAX_ENTRIES, MAX_ENTRY, read_pack};

const LUT: &str = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";

fn demo(dir: &str) -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.prefs_path = Some(tmp_dir(dir).join("prefs.json"));
    s
}

/// A session with one user preset ("Warm Look"), one project LUT ("Teal") and one installed
/// template ("Lower").
fn author(dir: &str) -> Session {
    let mut s = demo(dir);
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    s.execute("effects.apply", json!({"effect": "black_white"})).unwrap();
    let n = s.active_sequence().unwrap().find_item(clip).unwrap().1.effects.len();
    s.execute("presets.save", json!({"clip": clip.0, "effects": [n - 1], "name": "Warm Look"})).unwrap();
    let lut = tmp_dir(dir).join("Teal.cube");
    std::fs::write(&lut, LUT).unwrap();
    s.execute("lut.import", json!({"path": lut.to_string_lossy()})).unwrap();
    s.execute("playhead.set", json!({"seconds": 600})).unwrap();
    let c = s.execute("graphics.newText", json!({"text": "Name", "position": [200, 900]})).unwrap()["clip"].as_u64().unwrap();
    let t = s.execute("graphics.template.export", json!({"clip": c, "name": "Lower", "controls": [{"layer": 0, "param": "text", "name": "Name"}]})).unwrap();
    s.execute("graphics.template.install", json!({"path": t["path"]})).unwrap();
    s
}

#[test]
fn a_pack_round_trips_and_never_overwrites() {
    let mut a = author("pack-a");
    let path = tmp_dir("pack-a").join("look.fcpack").to_string_lossy().to_string();
    let r = a.execute("packs.export", json!({"path": path, "name": "Teal & Warm"})).unwrap();
    assert_eq!((r["presets"].as_u64(), r["luts"].as_u64(), r["templates"].as_u64()), (Some(1), Some(1), Some(1)), "{r}");

    let mut b = demo("pack-b");
    let r = b.execute("packs.import", json!({"path": path})).unwrap();
    assert_eq!(r["name"], "Teal & Warm");
    assert_eq!(r["presets"], json!(["Warm Look"]));
    let want = a.presets.user.iter().find(|p| p.name == "Warm Look").unwrap();
    let got = b.presets.user.iter().find(|p| p.name == "Warm Look").unwrap();
    assert_eq!(serde_json::to_value(want).unwrap(), serde_json::to_value(got).unwrap(), "the preset is unchanged");
    assert_eq!(b.project.luts.len(), 1);
    assert_eq!(&*b.project.luts[0].text, LUT);
    assert_eq!(b.project.luts[0].name, "Teal");
    let lib = crate::graphic_templates::library(&b);
    let t = lib.iter().find(|e| e.path.is_some() && e.template.name == "Lower").expect("template installed");
    let orig = crate::graphic_templates::library(&a).into_iter().find(|e| e.path.is_some()).unwrap();
    assert_eq!(t.template.graphic, orig.template.graphic, "the template's graphic is unchanged");

    // again: the preset and the template get " (2)", the identical LUT is reused
    let r = b.execute("packs.import", json!({"path": path})).unwrap();
    assert_eq!(r["presets"], json!(["Warm Look (2)"]));
    assert_eq!(r["luts"][0]["reused"], true);
    assert_eq!(b.project.luts.len(), 1);
    let names: Vec<String> = crate::graphic_templates::library(&b).into_iter().filter(|e| e.path.is_some()).map(|e| e.template.name).collect();
    assert!(names.contains(&"Lower".to_string()) && names.contains(&"Lower (2)".to_string()), "{names:?}");

    // choosing what goes in; unknown names are refused
    let p2 = tmp_dir("pack-a").join("presets-only.fcpack").to_string_lossy().to_string();
    let r = a.execute("packs.export", json!({"path": p2, "presets": ["Warm Look"], "luts": [], "templates": []})).unwrap();
    assert_eq!((r["presets"].as_u64(), r["luts"].as_u64(), r["templates"].as_u64()), (Some(1), Some(0), Some(0)));
    assert!(a.execute("packs.export", json!({"path": p2, "presets": ["Nope"]})).is_err());
    assert!(a.execute("packs.export", json!({"path": p2, "luts": ["Nope"]})).is_err());
    assert!(a.execute("packs.export", json!({"path": p2, "presets": [], "luts": [], "templates": []})).is_err(), "an empty pack");
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in entries {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const MANIFEST: &[u8] = br#"{"format":"filmcraft.pack","version":1,"name":"x"}"#;

#[test]
fn hostile_packs_are_refused_with_a_message() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("garbage", b"not a zip at all".to_vec()),
        ("no manifest", zip_of(&[("luts/a.cube", LUT.as_bytes())])),
        ("wrong format", zip_of(&[("pack.json", br#"{"format":"other","version":1}"#)])),
        ("newer version", zip_of(&[("pack.json", br#"{"format":"filmcraft.pack","version":9}"#)])),
        ("parent path", zip_of(&[("pack.json", MANIFEST), ("luts/../../evil.cube", LUT.as_bytes())])),
        ("absolute path", zip_of(&[("pack.json", MANIFEST), ("/etc/evil.cube", LUT.as_bytes())])),
        ("backslash", zip_of(&[("pack.json", MANIFEST), ("luts\\evil.cube", LUT.as_bytes())])),
        ("nested", zip_of(&[("pack.json", MANIFEST), ("luts/sub/evil.cube", LUT.as_bytes())])),
        ("mogrt", zip_of(&[("pack.json", MANIFEST), ("templates/x.mogrt", b"PK")])),
        ("unknown file", zip_of(&[("pack.json", MANIFEST), ("run.sh", b"echo")])),
        ("lut type", zip_of(&[("pack.json", MANIFEST), ("luts/a.png", b"x")])),
        ("lut not text", zip_of(&[("pack.json", MANIFEST), ("luts/a.cube", &[0xff, 0xfe, 0x00])])),
        ("bad presets", zip_of(&[("pack.json", MANIFEST), ("presets.json", b"{}")])),
        ("bad manifest", zip_of(&[("pack.json", b"[1,2")])),
    ];
    for (what, bytes) in cases {
        let e = read_pack(&bytes).expect_err(what);
        assert!(!e.is_empty(), "{what}");
    }
    // too many entries
    let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("luts/{i}.cube")).collect();
    let many: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), b"" as &[u8])).collect();
    assert!(read_pack(&zip_of(&many)).unwrap_err().contains("entries"));
    // an entry larger than the cap (zeros: tiny packed)
    let big = vec![0u8; (MAX_ENTRY + 1) as usize];
    assert!(read_pack(&zip_of(&[("pack.json", MANIFEST), ("luts/big.cube", &big)])).unwrap_err().contains("too large"));
    // a LUT that doesn't parse is refused at import, and nothing half-imported stays
    let bad = zip_of(&[("pack.json", MANIFEST), ("luts/bad.cube", b"LUT_3D_SIZE 99999\n")]);
    let path = tmp_dir("pack-bad").join("bad.fcpack");
    std::fs::write(&path, bad).unwrap();
    let mut s = demo("pack-bad");
    assert!(s.execute("packs.import", json!({"path": path.to_string_lossy()})).is_err());
    assert!(s.project.luts.is_empty());
}

/// `lut.importFolder` (fork Y3.5b): every LUT in a folder, one undo step, broken files skipped.
#[test]
fn a_lut_folder_imports_as_one_undo_step() {
    let dir = tmp_dir("lut-folder");
    std::fs::write(dir.join("a Teal.cube"), LUT).unwrap();
    std::fs::write(dir.join("b Warm.cube"), LUT.replace("1 1 1\n", "1 0.9 0.8\n")).unwrap();
    std::fs::write(dir.join("c broken.cube"), "LUT_3D_SIZE 3\n0 0 0\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a LUT").unwrap();
    let mut s = demo("lut-folder");
    let undo0 = s.history.undo.len();
    let r = s.execute("lut.importFolder", json!({"path": dir.to_string_lossy()})).unwrap();
    assert_eq!(r["added"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "{r}");
    let names: Vec<&str> = s.project.luts.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["a Teal", "b Warm"]);
    assert_eq!(s.history.undo.len(), undo0 + 1, "one undo step");
    s.undo();
    assert!(s.project.luts.is_empty());
    assert!(s.execute("lut.importFolder", json!({"path": tmp_dir("lut-folder-empty").to_string_lossy()})).is_err());
    assert!(s.execute("lut.importFolder", json!({"path": "/no/such/folder"})).is_err());
}
