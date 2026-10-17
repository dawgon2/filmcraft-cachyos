// Kernels of the Qwen3-VL vision encoder on the GPU (AI4.2, `gpu_vision.rs`). Each `//// name`
// section is compiled as its own module with entry point `main`; bindings are numbered in the
// order the host passes buffers. Every tensor is f32, tokens are rows. The host guarantees that
// every u32 index is in range and that the buffers are large enough; the kernels still clamp
// their loads and guard their stores so a wrong shape reads garbage, never out of bounds.
// Constants named in capitals with a `// substituted` note are replaced by the host.

//// linear
// y[t · out + o] = (acc ? y : 0) + act(Σ_i x[t · inp + i] · w[o · inp + i] + b[o]) for `n` tokens,
// with w `out × inp` row-major (as the CPU keeps it) and act 0 none, 1 GELU (tanh), 2 GELU (erf).
// Tiling: a 256-thread workgroup computes a 64-token × 128-output tile, walking the inner size
// in 16-column steps staged in workgroup memory k-major as vec4 quads of tokens / outputs; each
// thread accumulates 4 tokens × 8 outputs (output quads tn and 16 + tn) from one token quad and
// two output quads per column: 3 vec4 reads per 32 multiply-adds. Workgroups:
// (⌈out / 128⌉, ⌈n / 64⌉).
struct P { n: u32, inp: u32, out: u32, act: u32, acc: u32, _a: u32, _b: u32, _c: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> x: array<f32>;
@group(0) @binding(2) var<storage, read> w: array<f32>;
@group(0) @binding(3) var<storage, read> b: array<f32>;
@group(0) @binding(4) var<storage, read_write> y: array<f32>;

const BM: u32 = 64u;
const BN: u32 = 128u;
const BK: u32 = 16u;
// Two-dimensional so the bounds clamps naga adds fold away (the k index is loop-invariant).
var<workgroup> xs: array<array<vec4<f32>, 16>, BK>; // [k][token quad]
var<workgroup> ws: array<array<vec4<f32>, 32>, BK>; // [k][output quad]

// Abramowitz & Stegun 7.1.26, as the CPU's `erf` (|error| < 1.5e-7).
fn erf(v: f32) -> f32 {
    let s = sign(v);
    let a = abs(v);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let poly = ((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592;
    return s * (1.0 - poly * t * exp(-a * a));
}

fn activate(v: f32) -> f32 {
    if (p.act == 1u) {
        return 0.5 * v * (1.0 + tanh(0.7978846 * (v + 0.044715 * v * v * v)));
    }
    if (p.act == 2u) {
        return 0.5 * v * (1.0 + erf(v * 0.70710677));
    }
    return v;
}

fn put(t: u32, o: u32, v: f32) {
    if (t < p.n && o < p.out) {
        let at = t * p.out + o;
        let r = activate(v + b[o]);
        if (p.acc != 0u) {
            y[at] = y[at] + r;
        } else {
            y[at] = r;
        }
    }
}

// A thread's 4 tokens × 8 outputs: tokens by field, the 4 outputs of quad tn (a) and of quad
// 16 + tn (b) by component.
struct Acc {
    a0: vec4<f32>, a1: vec4<f32>, a2: vec4<f32>, a3: vec4<f32>,
    b0: vec4<f32>, b1: vec4<f32>, b2: vec4<f32>, b3: vec4<f32>,
}

// One thread's share of a step's loads: columns k0 + xk .. + 4 of its token row and
// k0 + wk .. + 8 of its output row, zero past the inner size.
struct Fetch {
    x: vec4<f32>,
    w0: vec4<f32>,
    w1: vec4<f32>,
}

fn load_x(base: u32, k: u32) -> f32 {
    return select(0.0, x[base + min(k, p.inp - 1u)], k < p.inp);
}

fn load_w(base: u32, k: u32) -> f32 {
    return select(0.0, w[base + min(k, p.inp - 1u)], k < p.inp);
}

fn fetch(xbase: u32, wbase: u32, k0: u32, xk: u32, wk: u32) -> Fetch {
    var f: Fetch;
    let kx = k0 + xk;
    let kw = k0 + wk;
    f.x = vec4<f32>(load_x(xbase, kx), load_x(xbase, kx + 1u), load_x(xbase, kx + 2u), load_x(xbase, kx + 3u));
    f.w0 = vec4<f32>(load_w(wbase, kw), load_w(wbase, kw + 1u), load_w(wbase, kw + 2u), load_w(wbase, kw + 3u));
    f.w1 = vec4<f32>(load_w(wbase, kw + 4u), load_w(wbase, kw + 5u), load_w(wbase, kw + 6u), load_w(wbase, kw + 7u));
    return f;
}

// Column k of the staged step into the accumulators.
fn step(k: u32, tm: u32, tn: u32, acc: ptr<function, Acc>) {
    let xv = xs[k][tm];
    let w0 = ws[k][tn];
    let w1 = ws[k][16u + tn];
    (*acc).a0 += xv.x * w0;
    (*acc).a1 += xv.y * w0;
    (*acc).a2 += xv.z * w0;
    (*acc).a3 += xv.w * w0;
    (*acc).b0 += xv.x * w1;
    (*acc).b1 += xv.y * w1;
    (*acc).b2 += xv.z * w1;
    (*acc).b3 += xv.w * w1;
}

fn put4(t: u32, o: u32, v: vec4<f32>) {
    put(t, o, v.x);
    put(t, o + 1u, v.y);
    put(t, o + 2u, v.z);
    put(t, o + 3u, v.w);
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let o0 = wid.x * BN;
    let t0 = wid.y * BM;
    let tm = t / 16u;
    let tn = t % 16u;
    // this thread's share of a step's loads: 4 consecutive columns of one token (row xr, columns
    // from xk) and 8 consecutive columns of one output (row wr, columns from wk)
    let xr = t / 4u;
    let xk = (t % 4u) * 4u;
    let wr = t / 2u;
    let wk = (t % 2u) * 8u;
    let xbase = min(t0 + xr, p.n - 1u) * p.inp;
    let wbase = min(o0 + wr, p.out - 1u) * p.inp;
    let xq = xr / 4u;
    let xe = xr % 4u;
    let wq = wr / 4u;
    let we = wr % 4u;
    var acc: Acc;
    acc.a0 = vec4<f32>(0.0);
    acc.a1 = vec4<f32>(0.0);
    acc.a2 = vec4<f32>(0.0);
    acc.a3 = vec4<f32>(0.0);
    acc.b0 = vec4<f32>(0.0);
    acc.b1 = vec4<f32>(0.0);
    acc.b2 = vec4<f32>(0.0);
    acc.b3 = vec4<f32>(0.0);
    // the next step's loads are issued (into registers) before the current step computes, so
    // memory latency overlaps the arithmetic; past the end they read clamped columns nobody uses
    var f = fetch(xbase, wbase, 0u, xk, wk);
    for (var k0 = 0u; k0 < p.inp; k0 += BK) {
        xs[xk][xq][xe] = f.x.x;
        xs[xk + 1u][xq][xe] = f.x.y;
        xs[xk + 2u][xq][xe] = f.x.z;
        xs[xk + 3u][xq][xe] = f.x.w;
        ws[wk][wq][we] = f.w0.x;
        ws[wk + 1u][wq][we] = f.w0.y;
        ws[wk + 2u][wq][we] = f.w0.z;
        ws[wk + 3u][wq][we] = f.w0.w;
        ws[wk + 4u][wq][we] = f.w1.x;
        ws[wk + 5u][wq][we] = f.w1.y;
        ws[wk + 6u][wq][we] = f.w1.z;
        ws[wk + 7u][wq][we] = f.w1.w;
        workgroupBarrier();
        f = fetch(xbase, wbase, k0 + BK, xk, wk);
        // the inner product over the step's 16 columns, unrolled (constant k: the staged
        // addresses are immediate offsets and nothing waits on a loop counter)
        step(0u, tm, tn, &acc);
        step(1u, tm, tn, &acc);
        step(2u, tm, tn, &acc);
        step(3u, tm, tn, &acc);
        step(4u, tm, tn, &acc);
        step(5u, tm, tn, &acc);
        step(6u, tm, tn, &acc);
        step(7u, tm, tn, &acc);
        step(8u, tm, tn, &acc);
        step(9u, tm, tn, &acc);
        step(10u, tm, tn, &acc);
        step(11u, tm, tn, &acc);
        step(12u, tm, tn, &acc);
        step(13u, tm, tn, &acc);
        step(14u, tm, tn, &acc);
        step(15u, tm, tn, &acc);
        workgroupBarrier();
    }
    let a0 = acc.a0;
    let a1 = acc.a1;
    let a2 = acc.a2;
    let a3 = acc.a3;
    let b0 = acc.b0;
    let b1 = acc.b1;
    let b2 = acc.b2;
    let b3 = acc.b3;
    let tt = t0 + tm * 4u;
    let oa = o0 + tn * 4u;
    let ob = oa + 64u;
    put4(tt, oa, a0);
    put4(tt + 1u, oa, a1);
    put4(tt + 2u, oa, a2);
    put4(tt + 3u, oa, a3);
    put4(tt, ob, b0);
    put4(tt + 1u, ob, b1);
    put4(tt + 2u, ob, b2);
    put4(tt + 3u, ob, b3);
}

//// layer_norm
// y = (x − mean) / sqrt(var + eps) · w + b over each `dim`-wide row (two passes, as the CPU's
// `Norm`). One workgroup per row; rows wrap to y past `stride_x` workgroups.
struct P { dim: u32, eps: f32, stride_x: u32, rows: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> x: array<f32>;
@group(0) @binding(2) var<storage, read> w: array<f32>;
@group(0) @binding(3) var<storage, read> b: array<f32>;
@group(0) @binding(4) var<storage, read_write> y: array<f32>;
var<workgroup> part: array<f32, 256>;

fn reduce(t: u32, v: f32) -> f32 {
    part[t] = v;
    workgroupBarrier();
    for (var h = 128u; h > 0u; h >>= 1u) {
        if (t < h) {
            part[t] += part[t + h];
        }
        workgroupBarrier();
    }
    let total = part[0];
    workgroupBarrier();
    return total;
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row = wid.x + wid.y * p.stride_x;
    if (row >= p.rows) {
        return;
    }
    let base = row * p.dim;
    let inv_dim = 1.0 / f32(max(p.dim, 1u));
    var s = 0.0;
    for (var i = t; i < p.dim; i += 256u) {
        s += x[base + i];
    }
    let mean = reduce(t, s) * inv_dim;
    var q = 0.0;
    for (var i = t; i < p.dim; i += 256u) {
        let d = x[base + i] - mean;
        q += d * d;
    }
    let inv = 1.0 / sqrt(reduce(t, q) * inv_dim + p.eps);
    for (var i = t; i < p.dim; i += 256u) {
        y[base + i] = (x[base + i] - mean) * inv * w[i] + b[i];
    }
}

//// rotary
// 2-D rotary embedding of q and k in place in the `[q | k | v]` rows of `qkv` (each part `h`
// wide, heads of `hd`): v = v · cos + rotate_half(v) · sin with rotate_half(v) = [−v_hi, v_lo],
// from per-token tables `cos` / `sin` of `hd` values. One thread per (token, part, head, j < hd/2):
// n · h threads, wrapping to y past `stride_x` workgroups of 256.
struct P { n: u32, h: u32, hd: u32, stride_x: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> cs: array<f32>;
@group(0) @binding(2) var<storage, read> sn: array<f32>;
@group(0) @binding(3) var<storage, read_write> v: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * p.stride_x * 256u;
    if (i >= p.n * p.h) {
        return;
    }
    let half = p.hd / 2u;
    let tok = i / p.h;
    let r = i % p.h;
    let part = r / (p.h / 2u);
    let rem = r % (p.h / 2u);
    let head = rem / half;
    let j = rem % half;
    let base = tok * 3u * p.h + part * p.h + head * p.hd;
    let lo = v[base + j];
    let hi = v[base + j + half];
    let ta = tok * p.hd + j;
    let tb = ta + half;
    v[base + j] = lo * cs[ta] - hi * sn[ta];
    v[base + j + half] = hi * cs[tb] + lo * sn[tb];
}

//// attention
// Full (bidirectional) attention over one picture's `n` tokens, one head and QB queries per
// workgroup, keys in blocks of KB with a running (online) softmax, so memory is O(QB · KB) per
// workgroup whatever `n` is. `qkv` rows are `[q | k | v]`, each `h` wide; `out` rows are `h`.
// Per key block: 256 threads stage the K block transposed ([d][k], conflict-free dots) and the
// V block as vec4 quads of dims ([k][quad]), both fetched into registers one block ahead so the
// loads overlap the arithmetic; thread t scores queries 4 · (t / 16) .. + 4 against keys t % 16
// and t % 16 + 16; threads 0..QB update their query's max / sum and turn the scores into
// probabilities; then thread t accumulates dim quad t % QPR of query rows t / QPR + i · R (one V
// read per key for all of them). Accumulators and fetches are struct fields, never arrays
// indexed in a loop (those end up in memory). Workgroups: (heads, ⌈n / QB⌉). HD is substituted
// (a multiple of 4, at most 128; workgroup memory (QB + 2 · KB) · HD + QB · KB f32).
struct P { n: u32, h: u32, heads: u32, scale: f32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> qkv: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

const HD: u32 = 64u; // substituted
const QB: u32 = 64u;
const KB: u32 = 32u;
// dim quads per row, query rows per pass of the 256 threads, passes to cover QB rows (≤ 8)
const QPR: u32 = HD / 4u;
const R: u32 = 256u / QPR;
const ITEMS: u32 = (QB + R - 1u) / R;
// quads of one K (or V) block, and per thread (≤ 4)
const KVQ: u32 = KB * QPR;
const KVI: u32 = (KVQ + 255u) / 256u;
const LOW: f32 = -1.0e30;

var<workgroup> qs: array<array<vec4<f32>, QPR>, QB>; // [q][dim quad]
var<workgroup> ks: array<f32, HD * KB>; // [d][k]
var<workgroup> vs: array<array<vec4<f32>, QPR>, KB>; // [k][dim quad]
var<workgroup> sc: array<f32, QB * KB>; // [q][k]
var<workgroup> mrow: array<f32, QB>;
var<workgroup> lrow: array<f32, QB>;
var<workgroup> crow: array<f32, QB>;

// This thread's P · V rows: row i is query qr + i · R.
struct Acc {
    r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, r3: vec4<f32>,
    r4: vec4<f32>, r5: vec4<f32>, r6: vec4<f32>, r7: vec4<f32>,
}

// This thread's share of a K and V block: quad t + i · 256 of each (key (t + i · 256) / QPR,
// dim quad (t + i · 256) % QPR).
struct Kv {
    k0: vec4<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>,
    v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>,
}

// Dims 4 · quad .. + 4 of token `tok`'s part at `off` (zero for a key past the picture).
fn quad(tok: u32, off: u32, q: u32, valid: bool) -> vec4<f32> {
    let at = min(tok, p.n - 1u) * 3u * p.h + off + q * 4u;
    return select(vec4<f32>(0.0), vec4<f32>(qkv[at], qkv[at + 1u], qkv[at + 2u], qkv[at + 3u]), valid);
}

fn fetch_kv(k0: u32, t: u32, koff: u32, voff: u32) -> Kv {
    var f: Kv;
    let i0 = t;
    let i1 = t + 256u;
    let i2 = t + 512u;
    let i3 = t + 768u;
    f.k0 = quad(k0 + i0 / QPR, koff, i0 % QPR, i0 < KVQ && k0 + i0 / QPR < p.n);
    f.v0 = quad(k0 + i0 / QPR, voff, i0 % QPR, i0 < KVQ && k0 + i0 / QPR < p.n);
    if (KVI > 1u) {
        f.k1 = quad(k0 + i1 / QPR, koff, i1 % QPR, i1 < KVQ && k0 + i1 / QPR < p.n);
        f.v1 = quad(k0 + i1 / QPR, voff, i1 % QPR, i1 < KVQ && k0 + i1 / QPR < p.n);
    }
    if (KVI > 2u) {
        f.k2 = quad(k0 + i2 / QPR, koff, i2 % QPR, i2 < KVQ && k0 + i2 / QPR < p.n);
        f.v2 = quad(k0 + i2 / QPR, voff, i2 % QPR, i2 < KVQ && k0 + i2 / QPR < p.n);
    }
    if (KVI > 3u) {
        f.k3 = quad(k0 + i3 / QPR, koff, i3 % QPR, i3 < KVQ && k0 + i3 / QPR < p.n);
        f.v3 = quad(k0 + i3 / QPR, voff, i3 % QPR, i3 < KVQ && k0 + i3 / QPR < p.n);
    }
    return f;
}

// Quad `i` of the block (key i / QPR, dims 4 · (i % QPR) ..) into the staged K and V.
fn stage(i: u32, k: vec4<f32>, v: vec4<f32>) {
    if (i < KVQ) {
        let key = i / QPR;
        let d = (i % QPR) * 4u;
        ks[d * KB + key] = k.x;
        ks[(d + 1u) * KB + key] = k.y;
        ks[(d + 2u) * KB + key] = k.z;
        ks[(d + 3u) * KB + key] = k.w;
        vs[key][i % QPR] = v;
    }
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let head = wid.x;
    let q0 = wid.y * QB;
    let stride = 3u * p.h;
    let qoff = head * HD;
    let koff = p.h + head * HD;
    let voff = 2u * p.h + head * HD;
    for (var i = t; i < QB * HD; i += 256u) {
        let q = i / HD;
        let d = i % HD;
        let tok = min(q0 + q, p.n - 1u);
        qs[q][d / 4u][d % 4u] = qkv[tok * stride + qoff + d];
    }
    if (t < QB) {
        mrow[t] = LOW;
        lrow[t] = 0.0;
        crow[t] = 1.0;
    }
    var acc: Acc;
    acc.r0 = vec4<f32>(0.0);
    acc.r1 = vec4<f32>(0.0);
    acc.r2 = vec4<f32>(0.0);
    acc.r3 = vec4<f32>(0.0);
    acc.r4 = vec4<f32>(0.0);
    acc.r5 = vec4<f32>(0.0);
    acc.r6 = vec4<f32>(0.0);
    acc.r7 = vec4<f32>(0.0);
    // the scores micro tile: queries sq .. sq + 4, keys kq and kq + 16
    let sq = (t / 16u) * 4u;
    let kq = t % 16u;
    // the P · V share: dim quad dq of query rows qr, qr + R, … (row i exists when qr + i · R < QB,
    // which only the last row can miss)
    let dq = t % QPR;
    let qr = t / QPR;
    let busy = t < R * QPR;
    let nk = (p.n + KB - 1u) / KB;
    var kv = fetch_kv(0u, t, koff, voff);
    for (var kb = 0u; kb < nk; kb++) {
        let k0 = kb * KB;
        workgroupBarrier();
        stage(t, kv.k0, kv.v0);
        if (KVI > 1u) { stage(t + 256u, kv.k1, kv.v1); }
        if (KVI > 2u) { stage(t + 512u, kv.k2, kv.v2); }
        if (KVI > 3u) { stage(t + 768u, kv.k3, kv.v3); }
        workgroupBarrier();
        kv = fetch_kv(k0 + KB, t, koff, voff);
        var s0 = vec4<f32>(0.0);
        var s1 = vec4<f32>(0.0);
        // a dim quad per iteration: 4 vec4 reads of Q (one per query), 4 + 4 scalar reads of the
        // two keys' dims, 8 dot products
        for (var d4 = 0u; d4 < QPR; d4++) {
            let ka = d4 * 4u * KB + kq;
            let qv0 = qs[sq][d4];
            let qv1 = qs[sq + 1u][d4];
            let qv2 = qs[sq + 2u][d4];
            let qv3 = qs[sq + 3u][d4];
            let k0v = vec4<f32>(ks[ka], ks[ka + KB], ks[ka + 2u * KB], ks[ka + 3u * KB]);
            let k1v = vec4<f32>(ks[ka + 16u], ks[ka + KB + 16u], ks[ka + 2u * KB + 16u], ks[ka + 3u * KB + 16u]);
            s0 += vec4<f32>(dot(qv0, k0v), dot(qv1, k0v), dot(qv2, k0v), dot(qv3, k0v));
            s1 += vec4<f32>(dot(qv0, k1v), dot(qv1, k1v), dot(qv2, k1v), dot(qv3, k1v));
        }
        let v0 = k0 + kq < p.n;
        let v1 = k0 + kq + 16u < p.n;
        sc[sq * KB + kq] = select(LOW, s0.x * p.scale, v0);
        sc[(sq + 1u) * KB + kq] = select(LOW, s0.y * p.scale, v0);
        sc[(sq + 2u) * KB + kq] = select(LOW, s0.z * p.scale, v0);
        sc[(sq + 3u) * KB + kq] = select(LOW, s0.w * p.scale, v0);
        sc[sq * KB + kq + 16u] = select(LOW, s1.x * p.scale, v1);
        sc[(sq + 1u) * KB + kq + 16u] = select(LOW, s1.y * p.scale, v1);
        sc[(sq + 2u) * KB + kq + 16u] = select(LOW, s1.z * p.scale, v1);
        sc[(sq + 3u) * KB + kq + 16u] = select(LOW, s1.w * p.scale, v1);
        workgroupBarrier();
        if (t < QB) {
            var mx = mrow[t];
            for (var k = 0u; k < KB; k++) {
                mx = max(mx, sc[t * KB + k]);
            }
            let c = exp(mrow[t] - mx);
            var sum = 0.0;
            for (var k = 0u; k < KB; k++) {
                let pk = exp(sc[t * KB + k] - mx);
                sc[t * KB + k] = pk;
                sum += pk;
            }
            lrow[t] = lrow[t] * c + sum;
            mrow[t] = mx;
            crow[t] = c;
        }
        workgroupBarrier();
        if (busy) {
            // a row past QB (only the last one can be) reads a clamped row nobody emits
            let q1 = min(qr + R, QB - 1u);
            let q2 = min(qr + 2u * R, QB - 1u);
            let q3 = min(qr + 3u * R, QB - 1u);
            let q4 = min(qr + 4u * R, QB - 1u);
            let q5 = min(qr + 5u * R, QB - 1u);
            let q6 = min(qr + 6u * R, QB - 1u);
            let q7 = min(qr + 7u * R, QB - 1u);
            acc.r0 *= crow[qr];
            if (ITEMS > 1u) { acc.r1 *= crow[q1]; }
            if (ITEMS > 2u) { acc.r2 *= crow[q2]; }
            if (ITEMS > 3u) { acc.r3 *= crow[q3]; }
            if (ITEMS > 4u) { acc.r4 *= crow[q4]; }
            if (ITEMS > 5u) { acc.r5 *= crow[q5]; }
            if (ITEMS > 6u) { acc.r6 *= crow[q6]; }
            if (ITEMS > 7u) { acc.r7 *= crow[q7]; }
            for (var k = 0u; k < KB; k++) {
                let vv = vs[k][dq];
                acc.r0 += sc[qr * KB + k] * vv;
                if (ITEMS > 1u) { acc.r1 += sc[q1 * KB + k] * vv; }
                if (ITEMS > 2u) { acc.r2 += sc[q2 * KB + k] * vv; }
                if (ITEMS > 3u) { acc.r3 += sc[q3 * KB + k] * vv; }
                if (ITEMS > 4u) { acc.r4 += sc[q4 * KB + k] * vv; }
                if (ITEMS > 5u) { acc.r5 += sc[q5 * KB + k] * vv; }
                if (ITEMS > 6u) { acc.r6 += sc[q6 * KB + k] * vv; }
                if (ITEMS > 7u) { acc.r7 += sc[q7 * KB + k] * vv; }
            }
        }
    }
    if (busy) {
        emit(q0, qr, head, dq, acc.r0);
        if (ITEMS > 1u) { emit(q0, qr + R, head, dq, acc.r1); }
        if (ITEMS > 2u) { emit(q0, qr + 2u * R, head, dq, acc.r2); }
        if (ITEMS > 3u) { emit(q0, qr + 3u * R, head, dq, acc.r3); }
        if (ITEMS > 4u) { emit(q0, qr + 4u * R, head, dq, acc.r4); }
        if (ITEMS > 5u) { emit(q0, qr + 5u * R, head, dq, acc.r5); }
        if (ITEMS > 6u) { emit(q0, qr + 6u * R, head, dq, acc.r6); }
        if (ITEMS > 7u) { emit(q0, qr + 7u * R, head, dq, acc.r7); }
    }
}

// Query row q's dim quad, normalised, into the output (nothing past the picture or the block).
fn emit(q0: u32, q: u32, head: u32, dq: u32, a: vec4<f32>) {
    let tok = q0 + q;
    if (q < QB && tok < p.n) {
        let r = a / max(lrow[q], 1.0e-30);
        let at = tok * p.h + head * HD + dq * 4u;
        out[at] = r.x;
        out[at + 1u] = r.y;
        out[at + 2u] = r.z;
        out[at + 3u] = r.w;
    }
}

//// add
// x[i] += y[i] for `n` values; threads wrap to y past `stride_x` workgroups of 256.
struct P { n: u32, stride_x: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> x: array<f32>;
@group(0) @binding(2) var<storage, read> y: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * p.stride_x * 256u;
    if (i < p.n) {
        x[i] = x[i] + y[i];
    }
}
