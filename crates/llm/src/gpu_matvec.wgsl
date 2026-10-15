// K-quant matrix × f32 vector (AI3.3b, GPU inference step 1). One workgroup per output row,
// 64 threads over its 32-value sub-blocks, then a tree sum.
// Q4_K (`matvec_q4k`):
// Each Q4_K block is 38 u32 (152 bytes; the GGUF block is 144 with packed scales): d, dmin (f32 bits), the eight
// 6-bit scales unpacked to bytes (2 words), the eight mins (2 words), the 128 nibble bytes
// (32 words) in ggml order: sub-blocks 2p and 2p+1 share bytes 32p..32p+31, low / high nibble.

struct Params {
    rows: u32,
    blocks: u32,
    stride_x: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> w: array<u32>;
@group(0) @binding(2) var<storage, read> x: array<f32>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;

var<workgroup> partial: array<f32, 64>;

fn byte_at(word: u32, i: u32) -> u32 {
    return (w[word + i / 4u] >> ((i % 4u) * 8u)) & 0xFFu;
}

@compute @workgroup_size(64)
fn matvec_q4k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row = wid.x + wid.y * p.stride_x;
    var acc = 0.0;
    if (row < p.rows) {
        let n = p.blocks * 8u;
        for (var i = t; i < n; i += 64u) {
            let b = i / 8u;
            let j = i % 8u;
            let base = (row * p.blocks + b) * 38u;
            let d = bitcast<f32>(w[base]);
            let dmin = bitcast<f32>(w[base + 1u]);
            let sc = f32(byte_at(base + 2u, j));
            let m = f32(byte_at(base + 4u, j));
            let q = base + 6u + (j / 2u) * 8u;
            let shift = (j % 2u) * 4u;
            let xo = b * 256u + j * 32u;
            var s = 0.0;
            var sx = 0.0;
            for (var k = 0u; k < 8u; k++) {
                let word = w[q + k];
                for (var e = 0u; e < 4u; e++) {
                    let v = f32((word >> (e * 8u + shift)) & 0xFu);
                    let xv = x[xo + k * 4u + e];
                    s += v * xv;
                    sx += xv;
                }
            }
            acc += d * sc * s - dmin * m * sx;
        }
    }
    finish(row, t, acc);
}

fn finish(row: u32, t: u32, acc: f32) {
    partial[t] = acc;
    workgroupBarrier();
    for (var h = 32u; h > 0u; h >>= 1u) {
        if (t < h) {
            partial[t] += partial[t + h];
        }
        workgroupBarrier();
    }
    if (t == 0u && row < p.rows) {
        y[row] = partial[0];
    }
}

// Q6_K (`matvec_q6k`): 53 u32 (212 bytes; the GGUF block is 210 with an f16 d): ql (32 words),
// qh (16 words), the 16 i8 scales (4 words), d (f32 bits). Sub-block g (32 values) of half
// h = g / 4, quarter c = g % 4: low bits from ql byte 64h + 32(c % 2) + l, nibble c / 2; high
// bits from qh byte 32h + l, bits 2c..2c+1; value − 32; scale g·2 + l / 16 (ggml order).
@compute @workgroup_size(64)
fn matvec_q6k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row = wid.x + wid.y * p.stride_x;
    var acc = 0.0;
    if (row < p.rows) {
        let n = p.blocks * 8u;
        for (var i = t; i < n; i += 64u) {
            let b = i / 8u;
            let g = i % 8u;
            let base = (row * p.blocks + b) * 53u;
            let d = bitcast<f32>(w[base + 52u]);
            let h = g / 4u;
            let c = g % 4u;
            let lo = 64u * h + 32u * (c % 2u);
            let lo_shift = (c / 2u) * 4u;
            let hi = 128u + 32u * h;
            let xo = b * 256u + g * 32u;
            var s0 = 0.0;
            var s1 = 0.0;
            for (var l = 0u; l < 32u; l++) {
                let ql = (byte_at(base, lo + l) >> lo_shift) & 0xFu;
                let qh = (byte_at(base, hi + l) >> (2u * c)) & 3u;
                let v = f32(i32(ql | (qh << 4u)) - 32) * x[xo + l];
                if (l < 16u) { s0 += v; } else { s1 += v; }
            }
            let sc0 = f32(bitcast<i32>(byte_at(base, 192u + g * 2u) << 24u) >> 24u);
            let sc1 = f32(bitcast<i32>(byte_at(base, 193u + g * 2u) << 24u) >> 24u);
            acc += d * (sc0 * s0 + sc1 * s1);
        }
    }
    finish(row, t, acc);
}
