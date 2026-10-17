// Quantized matrix (Q4_K, Q6_K, Q8_0) × a batch of f32 activation vectors (AI3.3b, prompt prefill).
// y[t * rows + r] = Σ_c W[r][c] · x[t * cols + c].
//
// Tiling: one 256-thread workgroup computes a 128-row × 128-token output tile, walking K in
// BK-column steps (BK = 16 or 32, substituted by the host). Every weight is w = dd·q − mm with
// q an unsigned byte and (dd, mm) constant over a 16-column group of a row: Q4_K q = nibble,
// dd = d·scale, mm = dmin·min; Q6_K q = the 6-bit value (0..63), dd = d·scale, mm = 32·dd (so
// fma(q, dd, −mm) rounds exactly like the CPU's dd·(q − 32)); Q8_0 q = the byte + 128, dd = d, mm = 128·d.
// Per step the workgroup stages in workgroup memory:
//   wq  the q bytes: per 4 columns (kq) and 4 rows a vec4<u32>, component = row, byte = column,
//   wd  dd and mm per 16-column group, 4 rows per vec4,
//   xs  the activations, k-major, 4 tokens per vec4 (32 vec4 per k),
// or, in the F32_TILE variant, the dequantised weights in `ws` instead of wq + wd.
// Each thread accumulates an 8-row × 8-token block (rows 4·ty.. and 64+4·ty.., tokens 4·tx..
// and 64+4·tx..): per 4 columns it reads 2 vec4<u32> of q (broadcast across the wave), per
// column 2 vec4 of activations, dequantises its 8 weights with 8 FMAs and does 64 FMAs.
// Workgroup memory: BK = 16 → 11.5 KiB, BK = 32 → 22.5 KiB (F32_TILE 17 / 33 KiB).
// Latency: with PREFETCH the next step's raw words are loaded into registers (unconditionally,
// from clamped rows / tokens, so nothing waits on them) before the current step computes, and
// only decoded and stored after it.
// Narrow matrices split K across workgroups (z); `reduce` then sums the partials.
// Shape: the host guarantees rows, n ≥ 1, every u32 index in range, and binding sizes.
// No loops over register arrays and no WGSL loops in the inner product (see `compute`).
// Block layouts on the GPU: see `gpu_matvec.wgsl`.

struct Params {
    rows: u32,
    blocks: u32,
    n: u32,
    cols: u32,
    // K split: workgroup z covers blocks z·per .. min((z+1)·per, blocks) and, when splits > 1,
    // writes its partial sums at y[z · n · rows ..] (a scratch buffer `reduce` then sums)
    per: u32,
    splits: u32,
    // `reduce`: workgroups along x before y
    stride_x: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> w: array<u32>;
@group(0) @binding(2) var<storage, read> x: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;

const BM: u32 = 128u;
const BN: u32 = 128u;
// K columns per step; the host substitutes this line (16 or 32).
const BK: u32 = 16u;
// Whether the next step's words are loaded into registers during the current step (AMD), or
// after it (Intel, where the extra live registers spill); the host substitutes this line.
const PREFETCH: bool = true;
// 16-column scale groups per step
const NG: u32 = BK / 16u;
// vec4s per k row of a tile (and row quads per tile)
const V: u32 = 32u;
// vec4s of activations a thread fetches per step: BK / 2 values
const NV: u32 = BK / 8u;

// Whether the weights are staged dequantised (f32, `ws`) rather than as bytes (`wq` + `wd`):
// better where int → float conversion is slow (Intel), but 17 KiB at BK = 16. The host
// substitutes these four lines together (F32_TILE true: WQ_K 1, WD_G 1, WS_K BK).
const F32_TILE: bool = false;
const WQ_K: u32 = BK / 4u;
const WD_G: u32 = NG * 2u;
const WS_K: u32 = 1u;

// Two-dimensional so the bounds clamps naga adds fold away: the k index is a constant in the
// unrolled compute, the row / token quad index is loop-invariant.
var<workgroup> wq: array<array<vec4<u32>, V>, WQ_K>;
var<workgroup> wd: array<array<vec4<f32>, V>, WD_G>;
var<workgroup> ws: array<array<vec4<f32>, V>, WS_K>;
var<workgroup> xs: array<array<vec4<f32>, V>, BK>;

// One thread's raw share of a step: 16 columns of one row (row t % 128, group (t / 128) % NG):
// q = the low-bit words, h = Q6_K's high-bit words, s = scale words; BK / 2 activations of one
// token (t % 128, half t / 128) in x0, x1 (and x2, x3 when BK = 32). Separate fields, not an
// array: a loop index into it would keep it in memory.
struct Fetch {
    q: vec4<u32>,
    h: vec4<u32>,
    s: vec4<u32>,
    x0: vec4<f32>,
    x1: vec4<f32>,
    x2: vec4<f32>,
    x3: vec4<f32>,
}

fn fetch_x(f: ptr<function, Fetch>, n0: u32, t: u32, step: u32) {
    let tok = min(n0 + t % 128u, p.n - 1u);
    let i = (tok * p.cols + step * BK + (t / 128u) * (BK / 2u)) / 4u;
    (*f).x0 = x[i];
    (*f).x1 = x[i + 1u];
    if (NV > 2u) {
        (*f).x2 = x[i + 2u];
        (*f).x3 = x[i + 3u];
    }
}

// First column of thread t's 16-column group in step `step`.
fn group_col(t: u32, step: u32) -> u32 {
    return step * BK + ((t / 128u) % NG) * 16u;
}

fn fetch_q4k(row0: u32, n0: u32, t: u32, step: u32) -> Fetch {
    var f: Fetch;
    fetch_x(&f, n0, t, step);
    let row = min(row0 + t % 128u, p.rows - 1u);
    let c0 = group_col(t, step);
    let j = (c0 % 256u) / 32u;
    let base = (row * p.blocks + c0 / 256u) * 38u;
    let q = base + 6u + (j / 2u) * 8u + (c0 % 32u) / 4u;
    f.q = vec4<u32>(w[q], w[q + 1u], w[q + 2u], w[q + 3u]);
    f.s = vec4<u32>(w[base], w[base + 1u], w[base + 2u + j / 4u], w[base + 4u + j / 4u]);
    return f;
}

fn fetch_q6k(row0: u32, n0: u32, t: u32, step: u32) -> Fetch {
    var f: Fetch;
    fetch_x(&f, n0, t, step);
    let row = min(row0 + t % 128u, p.rows - 1u);
    let c0 = group_col(t, step);
    let g = (c0 % 256u) / 32u;
    let off = c0 % 32u;
    let hf = g / 4u;
    let base = (row * p.blocks + c0 / 256u) * 53u;
    let lo = base + (64u * hf + 32u * (g % 2u) + off) / 4u;
    let hi = base + (128u + 32u * hf + off) / 4u;
    let si = g * 2u + off / 16u;
    f.q = vec4<u32>(w[lo], w[lo + 1u], w[lo + 2u], w[lo + 3u]);
    f.h = vec4<u32>(w[hi], w[hi + 1u], w[hi + 2u], w[hi + 3u]);
    f.s = vec4<u32>(w[base + 52u], w[base + 48u + si / 4u], 0u, 0u);
    return f;
}

// Q8_0: 9 words per 32-value block (d, then the i8 quants); the row's blocks are consecutive, so
// `blocks` (256-column steps, as for the K-quants) × 8 blocks per row. Thread t's 16 columns
// are half a block: 4 words of quants, and the block's d.
fn fetch_q8_0(row0: u32, n0: u32, t: u32, step: u32) -> Fetch {
    var f: Fetch;
    fetch_x(&f, n0, t, step);
    let row = min(row0 + t % 128u, p.rows - 1u);
    let c0 = group_col(t, step);
    let base = (row * p.blocks * 8u + c0 / 32u) * 9u;
    let q = base + 1u + (c0 % 32u) / 4u;
    f.q = vec4<u32>(w[q], w[q + 1u], w[q + 2u], w[q + 3u]);
    f.s = vec4<u32>(w[base], 0u, 0u, 0u);
    return f;
}

fn column(q: vec4<u32>, e: u32) -> vec4<f32> {
    return vec4<f32>((q >> vec4<u32>(e * 8u)) & vec4<u32>(0xFFu));
}

// The four dequantised columns k..k+3 of one row (row quad o, lane e) from one word of bytes.
fn store_ws4(k: u32, o: u32, e: u32, q: u32, dd: f32, mm: f32) {
    let v = fma(vec4<f32>((vec4<u32>(q) >> vec4<u32>(0u, 8u, 16u, 24u)) & vec4<u32>(0xFFu)), vec4<f32>(dd), vec4<f32>(-mm));
    ws[k][o][e] = v.x;
    ws[k + 1u][o][e] = v.y;
    ws[k + 2u][o][e] = v.z;
    ws[k + 3u][o][e] = v.w;
}

fn store_w(t: u32, q: vec4<u32>, dd: f32, mm: f32) {
    let r = t % 128u;
    let g = t / 128u;
    if (g < NG) {
        let o = r / 4u;
        let e = r % 4u;
        if (F32_TILE) {
            let k = g * 16u;
            store_ws4(k, o, e, q.x, dd, mm);
            store_ws4(k + 4u, o, e, q.y, dd, mm);
            store_ws4(k + 8u, o, e, q.z, dd, mm);
            store_ws4(k + 12u, o, e, q.w, dd, mm);
        } else {
            let kq = g * 4u;
            wq[kq][o][e] = q.x;
            wq[kq + 1u][o][e] = q.y;
            wq[kq + 2u][o][e] = q.z;
            wq[kq + 3u][o][e] = q.w;
            wd[g * 2u][o][e] = dd;
            wd[g * 2u + 1u][o][e] = mm;
        }
    }
}

// Columns k..k+3 of token quad o, lane e.
fn store_x4(k: u32, o: u32, e: u32, v: vec4<f32>) {
    xs[k][o][e] = v.x;
    xs[k + 1u][o][e] = v.y;
    xs[k + 2u][o][e] = v.z;
    xs[k + 3u][o][e] = v.w;
}

fn store_x(f: Fetch, t: u32) {
    let r = t % 128u;
    let o = r / 4u;
    let e = r % 4u;
    let k = (t / 128u) * (BK / 2u);
    store_x4(k, o, e, f.x0);
    store_x4(k + 4u, o, e, f.x1);
    if (NV > 2u) {
        store_x4(k + 8u, o, e, f.x2);
        store_x4(k + 12u, o, e, f.x3);
    }
}

fn store_q4k(f: Fetch, t: u32, step: u32) {
    let j = (group_col(t, step) % 256u) / 32u;
    let q = (f.q >> vec4<u32>((j % 2u) * 4u)) & vec4<u32>(0x0F0F0F0Fu);
    let sh = (j % 4u) * 8u;
    let dd = bitcast<f32>(f.s.x) * f32((f.s.z >> sh) & 0xFFu);
    let mm = bitcast<f32>(f.s.y) * f32((f.s.w >> sh) & 0xFFu);
    store_w(t, q, dd, mm);
    store_x(f, t);
}

fn store_q6k(f: Fetch, t: u32, step: u32) {
    let c0 = group_col(t, step);
    let c = ((c0 % 256u) / 32u) % 4u;
    let q = ((f.q >> vec4<u32>((c / 2u) * 4u)) & vec4<u32>(0x0F0F0F0Fu)) | (((f.h >> vec4<u32>(2u * c)) & vec4<u32>(0x03030303u)) << vec4<u32>(4u));
    let si = (((c0 % 256u) / 32u) * 2u + (c0 % 32u) / 16u) % 4u;
    let sc = f32(bitcast<i32>(((f.s.y >> (si * 8u)) & 0xFFu) << 24u) >> 24u);
    let dd = bitcast<f32>(f.s.x) * sc;
    store_w(t, q, dd, 32.0 * dd);
    store_x(f, t);
}

// Q8_0: the signed byte q + 128 is the unsigned byte (q ^ 0x80), dd = d, mm = 128·d (exact), so
// fma(q', d, −128·d) is d·q rounded once, as the CPU's dequantisation.
fn store_q8_0(f: Fetch, t: u32) {
    let d = bitcast<f32>(f.s.x);
    store_w(t, f.q ^ vec4<u32>(0x80808080u), d, 128.0 * d);
    store_x(f, t);
}

struct Acc {
    a: array<vec4<f32>, 8>,
    b: array<vec4<f32>, 8>,
}

// Rows r..r+3 of token t. Written out without loops: a loop index into the accumulators
// would make the compiler keep them in memory.
fn put(base: u32, t: u32, r: u32, v: vec4<f32>) {
    if (t < p.n) {
        let o = base + t * p.rows + r;
        if (r < p.rows) {
            y[o] = v.x;
        }
        if (r + 1u < p.rows) {
            y[o + 1u] = v.y;
        }
        if (r + 2u < p.rows) {
            y[o + 2u] = v.z;
        }
        if (r + 3u < p.rows) {
            y[o + 3u] = v.w;
        }
    }
}

fn write_out(acc: Acc, base: u32, row0: u32, n0: u32, tx: u32, ty: u32) {
    let ta = n0 + 4u * tx;
    let tb = n0 + 64u + 4u * tx;
    let ra = row0 + 4u * ty;
    let rb = row0 + 64u + 4u * ty;
    put(base, ta, ra, acc.a[0]);
    put(base, ta + 1u, ra, acc.a[1]);
    put(base, ta + 2u, ra, acc.a[2]);
    put(base, ta + 3u, ra, acc.a[3]);
    put(base, tb, ra, acc.a[4]);
    put(base, tb + 1u, ra, acc.a[5]);
    put(base, tb + 2u, ra, acc.a[6]);
    put(base, tb + 3u, ra, acc.a[7]);
    put(base, ta, rb, acc.b[0]);
    put(base, ta + 1u, rb, acc.b[1]);
    put(base, ta + 2u, rb, acc.b[2]);
    put(base, ta + 3u, rb, acc.b[3]);
    put(base, tb, rb, acc.b[4]);
    put(base, tb + 1u, rb, acc.b[5]);
    put(base, tb + 2u, rb, acc.b[6]);
    put(base, tb + 3u, rb, acc.b[7]);
}

// One column k: 8 dequantised weights (a0: rows 4·ty.., a1: rows 64+4·ty..) times 8 tokens.
fn kstep(acc: ptr<function, Acc>, a0: vec4<f32>, a1: vec4<f32>, k: u32, tx: u32) {
    let b0 = xs[k][tx];
    let b1 = xs[k][16u + tx];
    (*acc).a[0] = fma(a0, vec4<f32>(b0.x), (*acc).a[0]);
    (*acc).a[1] = fma(a0, vec4<f32>(b0.y), (*acc).a[1]);
    (*acc).a[2] = fma(a0, vec4<f32>(b0.z), (*acc).a[2]);
    (*acc).a[3] = fma(a0, vec4<f32>(b0.w), (*acc).a[3]);
    (*acc).a[4] = fma(a0, vec4<f32>(b1.x), (*acc).a[4]);
    (*acc).a[5] = fma(a0, vec4<f32>(b1.y), (*acc).a[5]);
    (*acc).a[6] = fma(a0, vec4<f32>(b1.z), (*acc).a[6]);
    (*acc).a[7] = fma(a0, vec4<f32>(b1.w), (*acc).a[7]);
    (*acc).b[0] = fma(a1, vec4<f32>(b0.x), (*acc).b[0]);
    (*acc).b[1] = fma(a1, vec4<f32>(b0.y), (*acc).b[1]);
    (*acc).b[2] = fma(a1, vec4<f32>(b0.z), (*acc).b[2]);
    (*acc).b[3] = fma(a1, vec4<f32>(b0.w), (*acc).b[3]);
    (*acc).b[4] = fma(a1, vec4<f32>(b1.x), (*acc).b[4]);
    (*acc).b[5] = fma(a1, vec4<f32>(b1.y), (*acc).b[5]);
    (*acc).b[6] = fma(a1, vec4<f32>(b1.z), (*acc).b[6]);
    (*acc).b[7] = fma(a1, vec4<f32>(b1.w), (*acc).b[7]);
}

// The four columns of quad kq.
fn quad(acc: ptr<function, Acc>, kq: u32, tx: u32, ty: u32) {
    let k = kq * 4u;
    let g = kq / 4u;
    let dda = wd[g * 2u][ty];
    let ddb = wd[g * 2u][16u + ty];
    let mma = -wd[g * 2u + 1u][ty];
    let mmb = -wd[g * 2u + 1u][16u + ty];
    let qa = wq[kq][ty];
    let qb = wq[kq][16u + ty];
    kstep(acc, fma(column(qa, 0u), dda, mma), fma(column(qb, 0u), ddb, mmb), k, tx);
    kstep(acc, fma(column(qa, 1u), dda, mma), fma(column(qb, 1u), ddb, mmb), k + 1u, tx);
    kstep(acc, fma(column(qa, 2u), dda, mma), fma(column(qb, 2u), ddb, mmb), k + 2u, tx);
    kstep(acc, fma(column(qa, 3u), dda, mma), fma(column(qb, 3u), ddb, mmb), k + 3u, tx);
}

// A loop over the quads with the four columns written out in `quad`: a WGSL loop gets naga's
// loop guard, which splits every iteration the driver unrolls into its own basic block and
// serialises the workgroup-memory reads; unrolling everything by hand instead makes the
// driver hoist all reads and run out of registers. kq is uniform, so its bounds clamps are
// scalar.
fn compute(acc: ptr<function, Acc>, tx: u32, ty: u32) {
    if (F32_TILE) {
        if (PREFETCH) {
            // a register-rich GPU (AMD): four columns per iteration
            for (var k = 0u; k < BK; k += 4u) {
                kstep(acc, ws[k][ty], ws[k][16u + ty], k, tx);
                kstep(acc, ws[k + 1u][ty], ws[k + 1u][16u + ty], k + 1u, tx);
                kstep(acc, ws[k + 2u][ty], ws[k + 2u][16u + ty], k + 2u, tx);
                kstep(acc, ws[k + 3u][ty], ws[k + 3u][16u + ty], k + 3u, tx);
            }
        } else {
            // one column per iteration: fewer live registers (Intel spills more)
            for (var k = 0u; k < BK; k++) {
                kstep(acc, ws[k][ty], ws[k][16u + ty], k, tx);
            }
        }
        return;
    }
    for (var kq = 0u; kq < BK / 4u; kq++) {
        quad(acc, kq, tx, ty);
    }
}

@compute @workgroup_size(256)
fn matmul_q4k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row0 = wid.x * BM;
    let n0 = wid.y * BN;
    let b0 = wid.z * p.per;
    let s0 = b0 * (256u / BK);
    let s1 = min(b0 + p.per, p.blocks) * (256u / BK);
    var acc: Acc;
    var f = fetch_q4k(row0, n0, t, s0);
    for (var s = s0; s < s1; s++) {
        if (!PREFETCH) {
            f = fetch_q4k(row0, n0, t, s);
        }
        store_q4k(f, t, s);
        workgroupBarrier();
        if (PREFETCH) {
            // the last step re-fetches its own data rather than branching
            f = fetch_q4k(row0, n0, t, min(s + 1u, s1 - 1u));
        }
        compute(&acc, t % 16u, t / 16u);
        workgroupBarrier();
    }
    write_out(acc, wid.z * p.n * p.rows, row0, n0, t % 16u, t / 16u);
}

@compute @workgroup_size(256)
fn matmul_q6k(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row0 = wid.x * BM;
    let n0 = wid.y * BN;
    let b0 = wid.z * p.per;
    let s0 = b0 * (256u / BK);
    let s1 = min(b0 + p.per, p.blocks) * (256u / BK);
    var acc: Acc;
    var f = fetch_q6k(row0, n0, t, s0);
    for (var s = s0; s < s1; s++) {
        if (!PREFETCH) {
            f = fetch_q6k(row0, n0, t, s);
        }
        store_q6k(f, t, s);
        workgroupBarrier();
        if (PREFETCH) {
            f = fetch_q6k(row0, n0, t, min(s + 1u, s1 - 1u));
        }
        compute(&acc, t % 16u, t / 16u);
        workgroupBarrier();
    }
    write_out(acc, wid.z * p.n * p.rows, row0, n0, t % 16u, t / 16u);
}

@compute @workgroup_size(256)
fn matmul_q8_0(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row0 = wid.x * BM;
    let n0 = wid.y * BN;
    let b0 = wid.z * p.per;
    let s0 = b0 * (256u / BK);
    let s1 = min(b0 + p.per, p.blocks) * (256u / BK);
    var acc: Acc;
    var f = fetch_q8_0(row0, n0, t, s0);
    for (var s = s0; s < s1; s++) {
        if (!PREFETCH) {
            f = fetch_q8_0(row0, n0, t, s);
        }
        store_q8_0(f, t);
        workgroupBarrier();
        if (PREFETCH) {
            f = fetch_q8_0(row0, n0, t, min(s + 1u, s1 - 1u));
        }
        compute(&acc, t % 16u, t / 16u);
        workgroupBarrier();
    }
    write_out(acc, wid.z * p.n * p.rows, row0, n0, t % 16u, t / 16u);
}

// y[i] = Σ_z partial[z · n · rows + i] (the partials bound as `w`, f32 bits).
@compute @workgroup_size(256)
fn reduce(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let len = p.n * p.rows;
    let i = (wid.x + wid.y * p.stride_x) * 256u + t;
    if (i < len) {
        var sum = 0.0;
        for (var z = 0u; z < p.splits; z++) {
            sum += bitcast<f32>(w[z * len + i]);
        }
        y[i] = sum;
    }
}
