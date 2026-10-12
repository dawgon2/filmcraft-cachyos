//! Decode an Annex-B H.265 file to raw planar YUV (yuv420p / yuv420p10le, or yuv422p10le for
//! 4:2:2 streams); without an output file the pictures are counted and dropped (benchmarking):
//! `cargo run --release -p filmcraft-hevc --example hevcdec -- in.hevc out.yuv`

use filmcraft_hevc::Decoder;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: hevcdec <in.hevc> [out.yuv]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read input");
    let aus = access_units(&data);
    let t0 = std::time::Instant::now();
    // HEVC_THREADS=n sets the thread count, HEVC_LOOPS=k decodes the file k times (benchmarking).
    let threads = std::env::var("HEVC_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(0usize);
    let loops = std::env::var("HEVC_LOOPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1usize);
    let keep = args.len() > 2;
    let mut pics = Vec::new();
    let mut count = 0usize;
    for _ in 0..loops {
        let mut dec = if threads > 0 { Decoder::with_threads(threads) } else { Decoder::new() };
        // HEVC_DRAFT=1: draft mode (non-reference pictures skip deblocking and SAO).
        dec.set_draft(std::env::var_os("HEVC_DRAFT").is_some());
        pics.clear();
        // one access unit per call (a whole file in one call holds every picture until the end)
        for au in &aus {
            let out = dec.decode(au, 0).expect("decode");
            count += out.len();
            if keep {
                pics.extend(out);
            }
        }
        let out = dec.flush();
        count += out.len();
        if keep {
            pics.extend(out);
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    eprintln!("{} pictures in {:.3}s ({:.1} fps)", count, secs, count as f64 / secs);
    if let Some(out) = args.get(2) {
        let mut buf = Vec::new();
        for p in &pics {
            buf.extend(p.y.to_le_bytes());
            buf.extend(p.u.to_le_bytes());
            buf.extend(p.v.to_le_bytes());
        }
        std::fs::write(out, buf).expect("write output");
    }
}

/// Annex-B access units (7.4.2.4.4): a new one starts at the first VPS / SPS / PPS / AUD /
/// prefix SEI / NAL type 41..=44 or 48..=55 after a slice, or at a slice with
/// first_slice_segment_in_pic_flag. EOS / EOB / filler data (36..=38) and suffix SEI (40) stay in
/// the current access unit.
fn access_units(data: &[u8]) -> Vec<Vec<u8>> {
    let mut aus = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut seen_vcl = false;
    for nal in filmcraft_bitstream::annexb_nals(data) {
        let Some(&b0) = nal.first() else { continue };
        let t = (b0 >> 1) & 0x3f;
        let vcl = t < 32;
        let first_slice = vcl && nal.get(2).is_some_and(|b| b & 0x80 != 0);
        if seen_vcl && (matches!(t, 32..=35 | 39 | 41..=44 | 48..=55) || first_slice) {
            aus.push(std::mem::take(&mut cur));
            seen_vcl = false;
        }
        seen_vcl |= vcl;
        cur.extend_from_slice(&[0, 0, 0, 1]);
        cur.extend_from_slice(nal);
    }
    if !cur.is_empty() {
        aus.push(cur);
    }
    aus
}
