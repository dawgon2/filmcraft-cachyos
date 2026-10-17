// Quantized matrix × f32 vector (AI3.3b; AI4.2: four rows per workgroup, coalesced). The one-token
// path of `gpu_forward.rs` is bound by these kernels' memory bandwidth, so they are built to read
// every weight word exactly once, with neighbouring threads reading neighbouring words:
// - one 64-thread workgroup computes ROWS (4) consecutive output rows; each thread walks the
//   rows' "units" (a few u32 of quants: Q8_0 2 words / 8 values, Q4_K 1 word / 8 values, Q6_K
//   3 words / 16 values, F32 4 words) at stride 64, so a wave's loads of one row are contiguous;
// - the activations are read as vec4 once per unit and reused for the 4 rows;
// - per-block scales are re-read by the threads sharing a block (cache hits, not memory traffic);
// - no subgroup operations (one workgroup-memory tree sum at the end), no loops over register
//   arrays (each row is a function call; the 4 sums live in a vec4).
// Workgroup z picks one of several token-major vectors (x at z · cols, y at z · rows): prefill of
// weights without a batched matmul kernel (Q8_0 of other widths, F32) runs as a matvec per token
// in one dispatch. Rows past `rows` (the last workgroup) read the last row and are not written.
// The host guarantees rows ≥ 1, cols a multiple of 4 (32 for the quantized kinds), every u32
// index in range and the binding sizes (`gpu.rs`); `gpu_matvec_f32.wgsl` covers F32 matrices of
// other widths.
//
// Block layouts on the GPU (the matmul kernels in `gpu_matmul.wgsl` read the same):
// Q4_K: 38 u32 (152 bytes; the GGUF block is 144 with packed scales): d, dmin (f32 bits), the
//   eight 6-bit scales unpacked to bytes (2 words), the eight mins (2 words), the 128 nibble bytes
//   (32 words) in ggml order: sub-blocks 2p and 2p+1 share bytes 32p..32p+31, low / high nibble.
// Q6_K: 53 u32 (212 bytes; the GGUF block is 210 with an f16 d): ql (32 words), qh (16 words), the
//   16 i8 scales (4 words), d (f32 bits). Sub-block g (32 values) of half h = g / 4, quarter
//   c = g % 4: low bits from ql byte 64h + 32(c % 2) + l, nibble c / 2; high bits from qh byte
//   32h + l, bits 2c..2c+1; value − 32; scale 8h + 2c + l / 16 (ggml order).
// Q8_0: 9 u32 per 32 values: d (f32 bits), then the 32 i8 quants (8 words). `blocks` is the
//   number of 32-value blocks per row.
// F32: plain row-major f32 bits; `blocks` is the number of columns.

struct Params {
    rows: u32,
    blocks: u32,
    stride_x: u32,
    cols: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> w: array<u32>;
@group(0) @binding(2) var<storage, read> x: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;

const WG: u32 = 64u;
const ROWS: u32 = 4u;

var<workgroup> partial: array<vec4<f32>, 64>;

// The first of the workgroup's rows and the other three (clamped to the last row).
fn rows_of(wid: vec3<u32>) -> vec4<u32> {
    let r0 = (wid.x + wid.y * p.stride_x) * ROWS;
    let last = p.rows - 1u;
    return min(vec4<u32>(r0, r0 + 1u, r0 + 2u, r0 + 3u), vec4<u32>(last));
}

// Sums `acc` over the workgroup and writes the rows that exist.
fn finish(wid: vec3<u32>, t: u32, acc: vec4<f32>) {
    partial[t] = acc;
    workgroupBarrier();
    for (var h = WG / 2u; h > 0u; h >>= 1u) {
        if (t < h) {
            partial[t] += partial[t + h];
        }
        workgroupBarrier();
    }
    let row = (wid.x + wid.y * p.stride_x) * ROWS + t;
    if (t < ROWS && row < p.rows) {
        y[wid.z * p.rows + row] = partial[0][t];
    }
}

// The four signed bytes of `v` as f32.
fn i8x4(v: u32) -> vec4<f32> {
    return vec4<f32>(bitcast<vec4<i32>>(vec4<u32>(v) << vec4<u32>(24u, 16u, 8u, 0u)) >> vec4<u32>(24u));
}

// Q8_0, unit = a quarter block (8 values): block b = u / 4, quarter q = u % 4.
fn q8_0_row(row: u32, b: u32, q: u32, x0: vec4<f32>, x1: vec4<f32>) -> f32 {
    let base = (row * p.blocks + b) * 9u;
    let qw = base + 1u + 2u * q;
    return bitcast<f32>(w[base]) * (dot(i8x4(w[qw]), x0) + dot(i8x4(w[qw + 1u]), x1));
}

@compute @workgroup_size(64)
fn matvec_q8_0(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = rows_of(wid);
    let xb = wid.z * (p.cols / 4u);
    let units = p.blocks * 4u;
    var acc = vec4<f32>(0.0);
    for (var u = t; u < units; u += WG) {
        let b = u / 4u;
        let q = u % 4u;
        let xi = xb + b * 8u + q * 2u;
        let x0 = x[xi];
        let x1 = x[xi + 1u];
        acc += vec4<f32>(q8_0_row(r.x, b, q, x0, x1), q8_0_row(r.y, b, q, x0, x1), q8_0_row(r.z, b, q, x0, x1), q8_0_row(r.w, b, q, x0, x1));
    }
    finish(wid, t, acc);
}

// Q4_K, unit = one nibble word (8 values: 4 of sub-block 2p, 4 of 2p + 1): block b = u / 32,
// word i = u % 32, p = i / 8. `sx` holds the sums of x0 and x1 (the mins' terms).
fn q4k_row(row: u32, b: u32, i: u32, x0: vec4<f32>, x1: vec4<f32>, sx: vec2<f32>) -> f32 {
    let base = (row * p.blocks + b) * 38u;
    let pp = i / 8u;
    let shift = (pp % 2u) * 16u;
    let scw = w[base + 2u + pp / 2u] >> shift;
    let mw = w[base + 4u + pp / 2u] >> shift;
    let sc = vec2<f32>(f32(scw & 0xFFu), f32((scw >> 8u) & 0xFFu));
    let m = vec2<f32>(f32(mw & 0xFFu), f32((mw >> 8u) & 0xFFu));
    let qv = vec4<u32>(w[base + 6u + i]) >> vec4<u32>(0u, 8u, 16u, 24u);
    let lo = vec4<f32>(qv & vec4<u32>(0xFu));
    let hi = vec4<f32>((qv >> vec4<u32>(4u)) & vec4<u32>(0xFu));
    let d = bitcast<f32>(w[base]);
    let dmin = bitcast<f32>(w[base + 1u]);
    return d * (sc.x * dot(lo, x0) + sc.y * dot(hi, x1)) - dmin * dot(m, sx);
}

@compute @workgroup_size(64)
fn matvec_q4k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = rows_of(wid);
    let xb = wid.z * (p.cols / 4u);
    let units = p.blocks * 32u;
    var acc = vec4<f32>(0.0);
    for (var u = t; u < units; u += WG) {
        let b = u / 32u;
        let i = u % 32u;
        // values b·256 + 64p + 4k (sub-block 2p) and 32 later (2p + 1), k = i % 8
        let xi = xb + b * 64u + (i / 8u) * 16u + i % 8u;
        let x0 = x[xi];
        let x1 = x[xi + 8u];
        let one = vec4<f32>(1.0);
        let sx = vec2<f32>(dot(x0, one), dot(x1, one));
        acc += vec4<f32>(q4k_row(r.x, b, i, x0, x1, sx), q4k_row(r.y, b, i, x0, x1, sx), q4k_row(r.z, b, i, x0, x1, sx), q4k_row(r.w, b, i, x0, x1, sx));
    }
    finish(wid, t, acc);
}

// Q6_K, unit = 4 positions l = 4k..4k+3 of half h for all four quarters (16 values): block
// b = u / 16, i = u % 16, h = i / 8, k = i % 8. Reads ql words 16h + k and 16h + 8 + k, qh word
// 32 + 8h + k: the 16 units of a block cover its 48 quant words once.
fn q6k_row(row: u32, b: u32, i: u32, xv: array<vec4<f32>, 4>) -> f32 {
    let base = (row * p.blocks + b) * 53u;
    let h = i / 8u;
    let k = i % 8u;
    let sh = vec4<u32>(0u, 8u, 16u, 24u);
    let a = (vec4<u32>(w[base + 16u * h + k]) >> sh) & vec4<u32>(0xFFu);
    let bq = (vec4<u32>(w[base + 16u * h + 8u + k]) >> sh) & vec4<u32>(0xFFu);
    let hh = (vec4<u32>(w[base + 32u + 8u * h + k]) >> sh) & vec4<u32>(0xFFu);
    let m4 = vec4<u32>(0xFu);
    let m2 = vec4<u32>(3u);
    let off = vec4<f32>(32.0);
    let q0 = vec4<f32>((a & m4) | ((hh & m2) << vec4<u32>(4u))) - off;
    let q1 = vec4<f32>((bq & m4) | (((hh >> vec4<u32>(2u)) & m2) << vec4<u32>(4u))) - off;
    let q2 = vec4<f32>((a >> vec4<u32>(4u)) | (((hh >> vec4<u32>(4u)) & m2) << vec4<u32>(4u))) - off;
    let q3 = vec4<f32>((bq >> vec4<u32>(4u)) | (((hh >> vec4<u32>(6u)) & m2) << vec4<u32>(4u))) - off;
    // scales 8h + j + 2c, j = k / 4: bytes j, j + 2 of word 48 + 2h (c = 0, 1) and of 49 + 2h
    let j8 = (k / 4u) * 8u;
    let s01 = w[base + 48u + 2u * h];
    let s23 = w[base + 49u + 2u * h];
    let sv = vec4<u32>(s01 >> j8, s01 >> (j8 + 16u), s23 >> j8, s23 >> (j8 + 16u));
    let sc = vec4<f32>(bitcast<vec4<i32>>(sv << vec4<u32>(24u)) >> vec4<u32>(24u));
    let s = vec4<f32>(dot(q0, xv[0]), dot(q1, xv[1]), dot(q2, xv[2]), dot(q3, xv[3]));
    return bitcast<f32>(w[base + 52u]) * dot(sc, s);
}

@compute @workgroup_size(64)
fn matvec_q6k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = rows_of(wid);
    let xb = wid.z * (p.cols / 4u);
    let units = p.blocks * 16u;
    var acc = vec4<f32>(0.0);
    for (var u = t; u < units; u += WG) {
        let b = u / 16u;
        let i = u % 16u;
        // values b·256 + 128h + 32c + 4k for quarter c
        let xi = xb + b * 64u + (i / 8u) * 32u + i % 8u;
        let xv = array<vec4<f32>, 4>(x[xi], x[xi + 8u], x[xi + 16u], x[xi + 24u]);
        acc += vec4<f32>(q6k_row(r.x, b, i, xv), q6k_row(r.y, b, i, xv), q6k_row(r.z, b, i, xv), q6k_row(r.w, b, i, xv));
    }
    finish(wid, t, acc);
}

// F32, unit = 4 columns.
fn f32_row(row: u32, c4: u32, xv: vec4<f32>) -> f32 {
    let o = row * p.cols + c4 * 4u;
    return dot(bitcast<vec4<f32>>(vec4<u32>(w[o], w[o + 1u], w[o + 2u], w[o + 3u])), xv);
}

@compute @workgroup_size(64)
fn matvec_f32(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = rows_of(wid);
    let n4 = p.cols / 4u;
    let xb = wid.z * n4;
    var acc = vec4<f32>(0.0);
    for (var c = t; c < n4; c += WG) {
        let xv = x[xb + c];
        acc += vec4<f32>(f32_row(r.x, c, xv), f32_row(r.y, c, xv), f32_row(r.z, c, xv), f32_row(r.w, c, xv));
    }
    finish(wid, t, acc);
}
