//! The accelerated decoder's bitstream side (`filmcraft_hevc::accel`) outputs the same pictures in
//! the same order as the software decoder, on every conformance fixture (skipped when the fixtures
//! have not been generated: `cargo xtask fixtures hevc`).

use filmcraft_hevc::accel::{AccelDecoder, AccelPicture, Accelerator, RefUse};
use filmcraft_hevc::params::Sps;
use filmcraft_hevc::{Decoder, Result};

/// Surfaces are numbers; decoding checks what a hardware decoder relies on.
#[derive(Default)]
struct Fake {
    next: u64,
    pictures: usize,
}

impl Accelerator for Fake {
    type Surface = u64;
    fn new_surface(&mut self, _sps: &Sps) -> Result<u64> {
        self.next += 1;
        Ok(self.next)
    }
    fn decode(&mut self, pic: &AccelPicture<'_, u64>) -> Result<()> {
        self.pictures += 1;
        assert!(pic.refs.len() <= 16, "DPB holds {} references", pic.refs.len());
        assert!(!pic.refs.iter().any(|r| r.surface == pic.target), "the target is not its own reference");
        for s in &pic.slices {
            assert!(s.data_byte_offset >= 2 && s.data_byte_offset <= s.nal.len(), "slice data offset {} of {}", s.data_byte_offset, s.nal.len());
            for (l, list) in s.ref_lists.iter().enumerate() {
                assert_eq!(list.len(), if s.header.is_intra() || (l == 1 && !s.header.is_b()) { 0 } else { s.header.num_ref_idx[l] as usize });
                for &i in list {
                    assert!(pic.refs[i].usage != RefUse::Foll, "a list entry uses a picture marked unused by the current picture");
                }
            }
        }
        Ok(())
    }
}

#[test]
fn accelerated_output_order_matches_the_software_decoder() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/hevc");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no fixtures in {}; skipped", dir.display());
        return;
    };
    let mut checked = 0;
    let mut names: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hevc")).collect();
    names.sort();
    for path in names {
        let data = std::fs::read(&path).expect("read fixture");
        let mut sw = Decoder::with_threads(1);
        let mut want: Vec<i32> = sw.decode(&data, 0).unwrap_or_else(|e| panic!("{}: {e}", path.display())).iter().map(|p| p.poc).collect();
        want.extend(sw.flush().iter().map(|p| p.poc));
        let mut hw = AccelDecoder::new(Fake::default());
        let mut got: Vec<i32> = hw.decode(&data, 0).unwrap_or_else(|e| panic!("{}: {e}", path.display())).iter().map(|o| o.poc).collect();
        got.extend(hw.flush().expect("flush").iter().map(|o| o.poc));
        assert_eq!(got, want, "{}: output POCs", path.display());
        assert!(hw.accel.pictures >= want.len(), "{}: every output picture was decoded", path.display());
        checked += 1;
    }
    eprintln!("{checked} fixtures checked");
}
