// Element kernels for the GPU forward pass (AI3.3b). Each `//// name` section is compiled as
// its own module with entry point `main`; its bindings are numbered in the order the host passes
// buffers (see `gpu.rs`, `Op`). `S` is the step uniform: the (first) new token's position and
// the context length after the step. Kernels that take a batch of tokens (prefill) read token
// `b` at `b · width` and run it at position `pos + b`; one token is the batch of one.

//// rms_norm
// y = x / sqrt(mean(x²) + eps) · w (divide, as the CPU's `rms_norm`). One workgroup per token.
struct P { n: u32, eps: f32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> x: array<f32>;
@group(0) @binding(2) var<storage, read> w: array<f32>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;
var<workgroup> part: array<f32, 256>;

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let base = wid.x * p.n;
    var s = 0.0;
    for (var i = t; i < p.n; i += 256u) {
        let v = x[base + i];
        s += v * v;
    }
    part[t] = s;
    workgroupBarrier();
    for (var h = 128u; h > 0u; h >>= 1u) {
        if (t < h) {
            part[t] += part[t + h];
        }
        workgroupBarrier();
    }
    let m = sqrt(part[0] / f32(max(p.n, 1u)) + p.eps);
    for (var i = t; i < p.n; i += 256u) {
        y[base + i] = x[base + i] / m * w[i];
    }
}

//// head_norm
// Qwen3's q / k norm: RMSNorm of every head in place with one `head_dim` weight vector (the
// CPU's `rms_norm` per head). Workgroup (head, token): `n_heads` heads of `head_dim` per token.
struct P { head_dim: u32, eps: f32, n_heads: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> w: array<f32>;
@group(0) @binding(2) var<storage, read_write> v: array<f32>;
var<workgroup> part: array<f32, 128>;

@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let n = min(p.head_dim, 512u);
    let base = (wid.y * p.n_heads + wid.x) * p.head_dim;
    var s = 0.0;
    for (var i = t; i < n; i += 128u) {
        let x = v[base + i];
        s += x * x;
    }
    part[t] = s;
    workgroupBarrier();
    for (var h = 64u; h > 0u; h >>= 1u) {
        if (t < h) {
            part[t] += part[t + h];
        }
        workgroupBarrier();
    }
    let m = sqrt(part[0] / f32(max(n, 1u)) + p.eps);
    for (var i = t; i < n; i += 128u) {
        v[base + i] = v[base + i] / m * w[i];
    }
}

//// rope
// Rotary embedding of `n_heads` heads in place, at the step's position, from the CPU's cos / sin
// table (`max_pos × head_dim/2` cos values, then as many sin values from `sin_at`).
struct P { n_heads: u32, head_dim: u32, interleaved: u32, sin_at: u32 }
struct S { pos: u32, n_ctx: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: S;
@group(0) @binding(2) var<storage, read> table: array<f32>;
@group(0) @binding(3) var<storage, read_write> v: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let half = p.head_dim / 2u;
    let i = gid.x;
    if (half == 0u || i >= p.n_heads * half) {
        return;
    }
    let head = i / half;
    let j = i % half;
    let pos = s.pos + gid.y;
    let c = table[pos * half + j];
    let sn = table[p.sin_at + pos * half + j];
    let base = gid.y * p.n_heads * p.head_dim;
    var a = base + head * p.head_dim + j;
    var b = a + half;
    if (p.interleaved != 0u) {
        a = base + head * p.head_dim + 2u * j;
        b = a + 1u;
    }
    let u = v[a];
    let w = v[b];
    v[a] = u * c - w * sn;
    v[b] = u * sn + w * c;
}

//// kv_store
// Appends the new token's key and value rows to the layer's caches at the step's position.
struct P { kv_dim: u32, max_pos: u32, _a: u32, _b: u32 }
struct S { pos: u32, n_ctx: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: S;
@group(0) @binding(2) var<storage, read> k: array<f32>;
@group(0) @binding(3) var<storage, read> v: array<f32>;
@group(0) @binding(4) var<storage, read_write> kc: array<f32>;
@group(0) @binding(5) var<storage, read_write> vc: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let pos = s.pos + gid.y;
    if (i >= p.kv_dim || pos >= p.max_pos) {
        return;
    }
    let at = pos * p.kv_dim + i;
    let src = gid.y * p.kv_dim + i;
    kc[at] = k[src];
    vc[at] = v[src];
}

//// attention
// One new token's grouped-query attention over the first `n_ctx` cached positions: one workgroup
// per query head (head h reads KV head h / (n_head / n_kv_head)); scores = q·k / sqrt(head_dim),
// softmax with the maximum subtracted, out = Σ softmax · v. `scores` is `n_head × max_pos`.
struct P { n_head: u32, n_kv_head: u32, head_dim: u32, max_pos: u32, kv_dim: u32, scale: f32, _a: u32, _b: u32 }
struct S { pos: u32, n_ctx: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: S;
@group(0) @binding(2) var<storage, read> q: array<f32>;
@group(0) @binding(3) var<storage, read> kc: array<f32>;
@group(0) @binding(4) var<storage, read> vc: array<f32>;
@group(0) @binding(5) var<storage, read_write> scores: array<f32>;
@group(0) @binding(6) var<storage, read_write> out: array<f32>;
var<workgroup> qh: array<f32, 512>;
var<workgroup> red: array<f32, 256>;

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let h = wid.x;
    let grp = max(p.n_head / max(p.n_kv_head, 1u), 1u);
    let kv_off = (h / grp) * p.head_dim;
    let n = min(s.n_ctx, p.max_pos);
    let sb = h * p.max_pos;
    for (var i = t; i < min(p.head_dim, 512u); i += 256u) {
        qh[i] = q[h * p.head_dim + i];
    }
    workgroupBarrier();
    var mx = -3.0e38;
    for (var j = t; j < n; j += 256u) {
        let at = j * p.kv_dim + kv_off;
        var d = 0.0;
        for (var i = 0u; i < min(p.head_dim, 512u); i++) {
            d += qh[i] * kc[at + i];
        }
        let sc = d / p.scale;
        scores[sb + j] = sc;
        mx = max(mx, sc);
    }
    red[t] = mx;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k >>= 1u) {
        if (t < k) {
            red[t] = max(red[t], red[t + k]);
        }
        workgroupBarrier();
    }
    let m = red[0];
    workgroupBarrier();
    var sum = 0.0;
    for (var j = t; j < n; j += 256u) {
        let e = exp(scores[sb + j] - m);
        scores[sb + j] = e;
        sum += e;
    }
    red[t] = sum;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k >>= 1u) {
        if (t < k) {
            red[t] += red[t + k];
        }
        workgroupBarrier();
    }
    let total = max(red[0], 1.0e-30);
    storageBarrier();
    workgroupBarrier();
    for (var i = t; i < p.head_dim; i += 256u) {
        var o = 0.0;
        for (var j = 0u; j < n; j++) {
            o += (scores[sb + j] / total) * vc[j * p.kv_dim + kv_off + i];
        }
        if (h < p.n_head) {
            out[h * p.head_dim + i] = o;
        }
    }
}

//// silu_mul
// g = silu(g) · u. Threads past x wrap to y (`groups_2d` on the host).
struct P { n: u32, _a: u32, _b: u32, _c: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> g: array<f32>;
@group(0) @binding(2) var<storage, read> u: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let i = gid.x + gid.y * nwg.x * 64u;
    if (i < p.n) {
        let x = g[i];
        g[i] = x / (1.0 + exp(-x)) * u[i];
    }
}

//// add
// x += y, or x[i] += y[i % period] (a bias on every token of a batch). Threads wrap as in silu_mul.
struct P { n: u32, period: u32, _b: u32, _c: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> x: array<f32>;
@group(0) @binding(2) var<storage, read> y: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let i = gid.x + gid.y * nwg.x * 64u;
    if (i < p.n) {
        x[i] += y[select(i, i % max(p.period, 1u), p.period != 0u)];
    }
}

//// attention_prefill
// Causal grouped-query attention of a batch of new tokens whose keys / values are already in the
// caches: workgroup (h, b) is query head h of token b (at position pos + b), attending to cached
// positions 0 ..= pos + b. The positions run in tiles of 128 (one per thread) with an online
// softmax: per tile the scores' maximum m, then o = o · exp(m_old − m) + Σ exp(score − m) · v and
// l likewise; out = o / l. No scores buffer, so any context length. Head dims up to 512 (four
// output values per thread).
struct P { n_head: u32, n_kv_head: u32, head_dim: u32, max_pos: u32, kv_dim: u32, scale: f32, q_stride: u32, _b: u32 }
struct S { pos: u32, n_ctx: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: S;
@group(0) @binding(2) var<storage, read> q: array<f32>;
@group(0) @binding(3) var<storage, read> kc: array<f32>;
@group(0) @binding(4) var<storage, read> vc: array<f32>;
@group(0) @binding(5) var<storage, read_write> out: array<f32>;
var<workgroup> qh: array<f32, 512>;
var<workgroup> pr: array<f32, 128>;
var<workgroup> red: array<f32, 128>;

@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let h = wid.x;
    let hd = min(p.head_dim, 512u);
    let grp = max(p.n_head / max(p.n_kv_head, 1u), 1u);
    let kv_off = (h / grp) * p.head_dim;
    let qb = wid.y * p.q_stride + h * p.head_dim;
    for (var i = t; i < hd; i += 128u) {
        qh[i] = q[qb + i];
    }
    workgroupBarrier();
    let n = min(s.pos + wid.y + 1u, p.max_pos);
    var m = -3.0e38;
    var l = 0.0;
    var o0 = 0.0;
    var o1 = 0.0;
    var o2 = 0.0;
    var o3 = 0.0;
    for (var start = 0u; start < n; start += 128u) {
        let j = start + t;
        var sc = -3.0e38;
        if (j < n) {
            let at = j * p.kv_dim + kv_off;
            var d = 0.0;
            for (var i = 0u; i < hd; i++) {
                d += qh[i] * kc[at + i];
            }
            sc = d / p.scale;
        }
        red[t] = sc;
        workgroupBarrier();
        for (var k = 64u; k > 0u; k >>= 1u) {
            if (t < k) {
                red[t] = max(red[t], red[t + k]);
            }
            workgroupBarrier();
        }
        let m_new = max(m, red[0]);
        workgroupBarrier();
        var e = 0.0;
        if (j < n) {
            e = exp(sc - m_new);
        }
        pr[t] = e;
        red[t] = e;
        workgroupBarrier();
        for (var k = 64u; k > 0u; k >>= 1u) {
            if (t < k) {
                red[t] += red[t + k];
            }
            workgroupBarrier();
        }
        let corr = exp(m - m_new);
        l = l * corr + red[0];
        m = m_new;
        let cnt = min(128u, n - start);
        o0 *= corr;
        o1 *= corr;
        o2 *= corr;
        o3 *= corr;
        for (var jj = 0u; jj < cnt; jj++) {
            let w = pr[jj];
            let vb = (start + jj) * p.kv_dim + kv_off + t;
            if (t < hd) { o0 += w * vc[vb]; }
            if (t + 128u < hd) { o1 += w * vc[vb + 128u]; }
            if (t + 256u < hd) { o2 += w * vc[vb + 256u]; }
            if (t + 384u < hd) { o3 += w * vc[vb + 384u]; }
        }
        workgroupBarrier();
    }
    if (h < p.n_head) {
        let inv = 1.0 / max(l, 1.0e-30);
        if (t < hd) { out[qb + t] = o0 * inv; }
        if (t + 128u < hd) { out[qb + t + 128u] = o1 * inv; }
        if (t + 256u < hd) { out[qb + t + 256u] = o2 * inv; }
        if (t + 384u < hd) { out[qb + t + 384u] = o3 * inv; }
    }
}

//// attention_prefill8
// `attention_prefill` with 8 query tokens per workgroup (one head): every key / value loaded is
// used by 8 queries, and loads are vec4. Workgroup (h, block): tokens 8·block .. 8·block + 7 of
// the batch (the batch length is n_ctx − pos). Head dims ≤ 256 and a multiple of 4 (the host
// picks `attention_prefill` otherwise). Rows 0..3 / 4..7 of the block are the components of
// two vec4s. Per tile of 128 positions (one per thread): scores into `pr`, a max tree over it in
// place, then exp(score − m) into `pr` for the value pass; each thread keeps partial sums of l
// (all threads share m), summed once at the end. Workgroup memory: 12 KiB.
struct P { n_head: u32, n_kv_head: u32, head_dim: u32, max_pos: u32, kv_dim: u32, scale: f32, q_stride: u32, _b: u32 }
struct S { pos: u32, n_ctx: u32, _a: u32, _b: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: S;
@group(0) @binding(2) var<storage, read> q: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> kc: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> vc: array<f32>;
@group(0) @binding(5) var<storage, read_write> out: array<f32>;
var<workgroup> qs: array<array<vec4<f32>, 64>, 8>;
var<workgroup> pa: array<vec4<f32>, 128>;
var<workgroup> pb: array<vec4<f32>, 128>;

fn put(row: u32, b0: u32, nb: u32, h: u32, dim: u32, v: f32) {
    if (b0 + row < nb && dim < min(p.head_dim, 256u)) {
        out[(b0 + row) * p.q_stride + h * p.head_dim + dim] = v;
    }
}

@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let h = wid.x;
    let hd = min(p.head_dim, 256u);
    let hd4 = hd / 4u;
    let grp = max(p.n_head / max(p.n_kv_head, 1u), 1u);
    let kv_off = (h / grp) * p.head_dim;
    let nb = s.n_ctx - min(s.pos, s.n_ctx);
    let b0 = wid.y * 8u;
    for (var i = t; i < 8u * hd4; i += 128u) {
        let r = i / hd4;
        let d = i % hd4;
        var v = vec4<f32>(0.0);
        if (b0 + r < nb) {
            v = q[((b0 + r) * p.q_stride + h * p.head_dim) / 4u + d];
        }
        qs[r][d] = v;
    }
    workgroupBarrier();
    // positions row r may see: 0 .. lim[r] − 1
    let first = s.pos + b0 + 1u;
    let lim_a = min(vec4<u32>(first, first + 1u, first + 2u, first + 3u), vec4<u32>(p.max_pos));
    let lim_b = min(vec4<u32>(first + 4u, first + 5u, first + 6u, first + 7u), vec4<u32>(p.max_pos));
    let n = min(s.pos + min(b0 + 8u, nb), p.max_pos);
    var ma = vec4<f32>(-3.0e38);
    var mb = vec4<f32>(-3.0e38);
    var la = vec4<f32>(0.0);
    var lb = vec4<f32>(0.0);
    var oa0 = vec4<f32>(0.0);
    var ob0 = vec4<f32>(0.0);
    var oa1 = vec4<f32>(0.0);
    var ob1 = vec4<f32>(0.0);
    let kv_off4 = kv_off / 4u;
    let kv_dim4 = p.kv_dim / 4u;
    for (var start = 0u; start < n; start += 128u) {
        let j = start + t;
        var sa = vec4<f32>(-3.0e38);
        var sb = vec4<f32>(-3.0e38);
        if (j < n) {
            let at = j * kv_dim4 + kv_off4;
            var da = vec4<f32>(0.0);
            var db = vec4<f32>(0.0);
            for (var d = 0u; d < hd4; d++) {
                let k = kc[at + d];
                da += vec4<f32>(dot(qs[0][d], k), dot(qs[1][d], k), dot(qs[2][d], k), dot(qs[3][d], k));
                db += vec4<f32>(dot(qs[4][d], k), dot(qs[5][d], k), dot(qs[6][d], k), dot(qs[7][d], k));
            }
            let jv = vec4<u32>(j);
            sa = select(vec4<f32>(-3.0e38), da / p.scale, jv < lim_a);
            sb = select(vec4<f32>(-3.0e38), db / p.scale, jv < lim_b);
        }
        pa[t] = sa;
        pb[t] = sb;
        workgroupBarrier();
        for (var k = 64u; k > 0u; k >>= 1u) {
            if (t < k) {
                pa[t] = max(pa[t], pa[t + k]);
                pb[t] = max(pb[t], pb[t + k]);
            }
            workgroupBarrier();
        }
        let na = max(ma, pa[0]);
        let nbm = max(mb, pb[0]);
        workgroupBarrier();
        let ca = exp(ma - na);
        let cb = exp(mb - nbm);
        let jv = vec4<u32>(j);
        let ea = select(vec4<f32>(0.0), exp(sa - na), (jv < lim_a) & vec4<bool>(j < n));
        let eb = select(vec4<f32>(0.0), exp(sb - nbm), (jv < lim_b) & vec4<bool>(j < n));
        pa[t] = ea;
        pb[t] = eb;
        la = la * ca + ea;
        lb = lb * cb + eb;
        ma = na;
        mb = nbm;
        oa0 *= ca;
        ob0 *= cb;
        oa1 *= ca;
        ob1 *= cb;
        workgroupBarrier();
        let cnt = min(128u, n - start);
        let vbase = start * p.kv_dim + kv_off + t;
        let wide = t + 128u < hd;
        for (var jj = 0u; jj < cnt; jj++) {
            let wa = pa[jj];
            let wb = pb[jj];
            let vb = vbase + jj * p.kv_dim;
            if (t < hd) {
                let v0 = vc[vb];
                oa0 += wa * v0;
                ob0 += wb * v0;
            }
            if (wide) {
                let v1 = vc[vb + 128u];
                oa1 += wa * v1;
                ob1 += wb * v1;
            }
        }
        workgroupBarrier();
    }
    // l: the threads' partial sums
    pa[t] = la;
    pb[t] = lb;
    workgroupBarrier();
    for (var k = 64u; k > 0u; k >>= 1u) {
        if (t < k) {
            pa[t] += pa[t + k];
            pb[t] += pb[t + k];
        }
        workgroupBarrier();
    }
    let ia = 1.0 / max(pa[0], vec4<f32>(1.0e-30));
    let ib = 1.0 / max(pb[0], vec4<f32>(1.0e-30));
    if (h < p.n_head) {
        let ra = oa0 * ia;
        let rb = ob0 * ib;
        let sa1 = oa1 * ia;
        let sb1 = ob1 * ib;
        for (var r = 0u; r < 4u; r++) {
            put(r, b0, nb, h, t, ra[r]);
            put(r + 4u, b0, nb, h, t, rb[r]);
            put(r, b0, nb, h, t + 128u, sa1[r]);
            put(r + 4u, b0, nb, h, t + 128u, sb1[r]);
        }
    }
}
