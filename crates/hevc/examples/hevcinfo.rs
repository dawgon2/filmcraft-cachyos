//! Print the SPS / PPS of an Annex-B H.265 file (chroma format, bit depth, range-extension tools):
//! `cargo run -p filmcraft-hevc --example hevcinfo -- in.hevc`

use filmcraft_bitstream::{annexb_nals, unescape_rbsp};
use filmcraft_hevc::params::{Pps, Sps};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: hevcinfo <in.hevc>");
        std::process::exit(2);
    };
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("read {path}: {e}");
            std::process::exit(1);
        }
    };
    let (mut nsps, mut npps) = (0, 0);
    for nal in annexb_nals(&data) {
        let Some(&b0) = nal.first() else { continue };
        let t = (b0 >> 1) & 0x3f;
        let rbsp = unescape_rbsp(nal.get(2..).unwrap_or_default());
        match t {
            33 if nsps < 4 => {
                nsps += 1;
                match Sps::parse(&rbsp) {
                    Ok(s) => {
                        println!(
                            "SPS {}: profile_idc {} {}x{} chroma_format_idc {} bit depth {}/{} ctb {} sps_extension_flags {:#04x}",
                            s.id,
                            s.ptl.profile_idc,
                            s.width,
                            s.height,
                            s.chroma_format_idc,
                            s.bit_depth_luma,
                            s.bit_depth_chroma,
                            1u32 << s.log2_ctb,
                            s.extension_flags
                        );
                        println!("  range extension: {:?}", s.range_ext);
                        println!("  supported: {:?}", s.check_supported());
                    }
                    Err(e) => println!("SPS: {e}"),
                }
            }
            34 if npps < 4 => {
                npps += 1;
                match Pps::parse(&rbsp) {
                    Ok(p) => {
                        println!(
                            "PPS {}: tiles {} wpp {} transform_skip {} sign_hiding {} cu_qp_delta {} pps_extension_flags {:#04x}",
                            p.id, p.tiles_enabled, p.entropy_coding_sync, p.transform_skip, p.sign_data_hiding, p.cu_qp_delta_enabled, p.extension_flags
                        );
                        println!("  range extension: {:?}", p.range_ext);
                        println!("  supported: {:?}", p.check_supported());
                    }
                    Err(e) => println!("PPS: {e}"),
                }
            }
            _ => {}
        }
        if nsps >= 4 && npps >= 4 {
            break;
        }
    }
}
