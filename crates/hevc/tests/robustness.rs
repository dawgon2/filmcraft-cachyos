//! hvcC / length-prefixed input and resilience against damaged streams.

mod common;

use filmcraft_bitstream::annexb_nals;
use filmcraft_hevc::Decoder;

/// Build an hvcC record and length-prefixed samples from an Annex-B stream.
fn to_hvcc(data: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut sets: [Vec<Vec<u8>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut samples = Vec::new();
    for au in common::split_access_units(data) {
        let mut sample = Vec::new();
        for nal in annexb_nals(au) {
            let t = (nal[0] >> 1) & 0x3f;
            match t {
                32..=34 => sets[(t - 32) as usize].push(nal.to_vec()),
                _ => {
                    sample.extend((nal.len() as u32).to_be_bytes());
                    sample.extend_from_slice(nal);
                }
            }
        }
        if !sample.is_empty() {
            samples.push(sample);
        }
    }
    for s in sets.iter_mut() {
        s.dedup();
    }
    let mut hvcc = vec![1u8];
    hvcc.extend([0u8; 20]); // profile / level / misc fields (not used by the decoder)
    hvcc.push(0xfc | 3); // lengthSizeMinusOne = 3
    hvcc.push(3);
    for (k, s) in sets.iter().enumerate() {
        hvcc.push(0x80 | (32 + k as u8));
        hvcc.extend((s.len() as u16).to_be_bytes());
        for n in s {
            hvcc.extend((n.len() as u16).to_be_bytes());
            hvcc.extend_from_slice(n);
        }
    }
    (hvcc, samples)
}

#[test]
fn hvcc_input_matches_reference() {
    for name in ["medium", "intra_main10", "medium_422"] {
        let f = common::fixture(name);
        let Some((hevc, yuv)) = common::ensure(f) else { return };
        let (hvcc, samples) = to_hvcc(&std::fs::read(hevc).unwrap());
        let mut dec = Decoder::from_hvcc(&hvcc).unwrap();
        assert_eq!(dec.nal_length_size(), Some(4));
        let mut pics = Vec::new();
        for (i, s) in samples.iter().enumerate() {
            pics.extend(dec.decode(s, i as i64 * 1000).unwrap());
        }
        pics.extend(dec.flush());
        let reference = std::fs::read(yuv).unwrap();
        common::compare_fixture(&pics, &reference, f).unwrap();
        let mut pts: Vec<i64> = pics.iter().map(|p| p.pts).collect();
        pts.sort();
        assert_eq!(pts, (0..samples.len() as i64).map(|i| i * 1000).collect::<Vec<_>>());
        let p0 = &pics[0];
        assert!(p0.key);
        assert_eq!(p0.bit_depth, f.bit_depth);
        assert_eq!((p0.y_stride, p0.uv_stride), (f.width as usize, f.width as usize / 2));
    }
}

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// Damaged streams must never panic or hang; errors are fine.
#[test]
fn corrupted_streams_do_not_panic() {
    for name in ["medium", "bframes", "intra_main10", "rect_amp", "weightp", "medium_422", "bframes_422", "intra_422", "slices4_422", "weightp_422"] {
        let f = common::fixture(name);
        let Some((hevc, _)) = common::ensure(f) else { return };
        let data = std::fs::read(hevc).unwrap();
        let mut seed = 0x1234_5678_9abc_def0u64 ^ name.len() as u64;
        for round in 0..12 {
            let mut d = data.clone();
            for _ in 0..(1 + round * 3) {
                let pos = 100 + (xorshift(&mut seed) as usize) % (d.len() - 100);
                d[pos] ^= 1 << (xorshift(&mut seed) % 8);
            }
            if round % 4 == 3 {
                d.truncate(d.len() * 2 / 3);
            }
            for threads in [1, 4] {
                let mut dec = Decoder::with_threads(threads);
                for au in common::split_access_units(&d) {
                    let _ = dec.decode(au, 0);
                }
                let _ = dec.flush();
                let _ = dec.take_error();
            }
        }
    }
}

#[test]
fn garbage_input_is_rejected_gracefully() {
    let mut seed = 42u64;
    for len in [0usize, 1, 5, 100, 5000] {
        for _ in 0..20 {
            let mut g: Vec<u8> = (0..len).map(|_| xorshift(&mut seed) as u8).collect();
            if len > 10 {
                g[..4].copy_from_slice(&[0, 0, 0, 1]);
            }
            let mut dec = Decoder::new();
            let _ = dec.decode(&g, 0);
            let _ = dec.flush();
        }
    }
    assert!(Decoder::from_hvcc(&[1, 2, 3]).is_err());
}

/// The SPS of an Annex-B stream.
fn first_sps(data: &[u8]) -> filmcraft_hevc::params::Sps {
    let nal = annexb_nals(data).into_iter().find(|n| n.first().is_some_and(|b| (b >> 1) & 0x3f == 33)).unwrap();
    filmcraft_hevc::params::Sps::parse(&filmcraft_bitstream::unescape_rbsp(&nal[2..])).unwrap()
}

/// 4:2:2 is accepted only without range-extension tools; other chroma formats are refused with
/// `Error::Unsupported`, never a panic.
#[test]
fn chroma_422_support_gate() {
    let f = common::fixture("intra_422");
    let Some((hevc, _)) = common::ensure(f) else { return };
    let sps = first_sps(&std::fs::read(hevc).unwrap());
    assert_eq!((sps.chroma_format_idc, sps.range_extension), (2, false));
    sps.check_supported().unwrap();
    for idc in [0, 3] {
        let mut s = sps.clone();
        s.chroma_format_idc = idc;
        assert!(matches!(s.check_supported(), Err(filmcraft_hevc::Error::Unsupported(_))), "chroma_format_idc {idc}");
    }
    let mut s = sps.clone();
    s.range_ext.implicit_rdpcm = true;
    s.range_ext.cabac_bypass_alignment = true;
    match s.check_supported() {
        Err(filmcraft_hevc::Error::Unsupported(m)) => assert!(m.contains("implicit_rdpcm") && m.contains("cabac_bypass_alignment"), "{m}"),
        other => panic!("{other:?}"),
    }
}

/// Every access unit of 4:2:2 streams cut short at several points: errors are fine, panics and
/// hangs are not (single- and frame-threaded).
#[test]
fn truncated_422_access_units_do_not_panic() {
    for name in ["intra_422", "bframes_422", "odd_422_1918x1078"] {
        let f = common::fixture(name);
        let Some((hevc, _)) = common::ensure(f) else { return };
        let data = std::fs::read(hevc).unwrap();
        let aus = common::split_access_units(&data);
        for frac in [1usize, 3, 5, 7] {
            for threads in [1, 3] {
                let mut dec = Decoder::with_threads(threads);
                for (i, au) in aus.iter().enumerate() {
                    // the first access unit keeps its parameter sets; slice data is cut at frac/8
                    let cut = if i == 0 { au.len() * (8 + frac) / 16 } else { au.len() * frac / 8 };
                    let _ = dec.decode(&au[..cut.max(1)], i as i64);
                }
                let _ = dec.flush();
                let _ = dec.take_error();
            }
        }
    }
}

/// A PPS whose Exp-Golomb fields hold the largest codable values (num_tile_columns_minus1,
/// num_tile_rows_minus1, init_qp_minus26) parses to an error or a value, never an overflow panic
/// (fork U0.2.2: the VA-API front end parses every PPS before the driver sees its tile layout).
#[test]
fn hostile_pps_fields_do_not_overflow() {
    use filmcraft_bitstream::BitWriter;
    // (init_qp_minus26, num_tile_columns_minus1, num_tile_rows_minus1)
    for (qp, cols, rows) in [(i32::MAX, 0, 0), (0, u32::MAX, 0), (0, 0, u32::MAX), (i32::MAX, u32::MAX, u32::MAX)] {
        let mut w = BitWriter::new();
        w.write_ue(0); // pps_pic_parameter_set_id
        w.write_ue(0); // pps_seq_parameter_set_id
        w.write_bits(0, 2); // dependent slices, output flag
        w.write_bits(0, 3); // num_extra_slice_header_bits
        w.write_bits(0, 2); // sign data hiding, cabac_init_present
        w.write_ue(0);
        w.write_ue(0); // num_ref_idx_lX_default_active_minus1
        w.write_se(qp);
        w.write_bits(0, 3); // constrained intra, transform skip, cu_qp_delta
        w.write_se(0);
        w.write_se(0); // cb / cr qp offsets
        w.write_bits(0, 4); // slice chroma qp offsets, weighted pred / bipred, transquant bypass
        w.write_bit(true); // tiles_enabled
        w.write_bit(false); // entropy_coding_sync
        w.write_ue(cols);
        w.write_ue(rows);
        w.write_bit(true); // uniform spacing
        w.write_bits(0, 8);
        w.rbsp_trailing();
        let r = filmcraft_hevc::params::Pps::parse(&w.finish());
        if cols == u32::MAX || rows == u32::MAX {
            assert!(r.is_err(), "too many tiles: {qp} {cols} {rows}");
        }
    }
}
