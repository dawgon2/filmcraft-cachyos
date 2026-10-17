// F32 matrix × f32 vector for widths that are not a multiple of 4 (`gpu_matvec.wgsl` reads the
// activations as vec4): one 64-thread workgroup per output row, then a tree sum. Bindings and
// parameters as `gpu_matvec.wgsl` (`blocks` is the number of columns).

struct Params {
    rows: u32,
    blocks: u32,
    stride_x: u32,
    cols: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> w: array<u32>;
@group(0) @binding(2) var<storage, read> x: array<f32>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;

var<workgroup> partial: array<f32, 64>;

@compute @workgroup_size(64)
fn matvec_f32_scalar(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let row = wid.x + wid.y * p.stride_x;
    let xb = wid.z * p.cols;
    var acc = 0.0;
    if (row < p.rows) {
        let base = row * p.blocks;
        for (var c = t; c < p.blocks; c += 64u) {
            acc += bitcast<f32>(w[base + c]) * x[xb + c];
        }
    }
    partial[t] = acc;
    workgroupBarrier();
    for (var h = 32u; h > 0u; h >>= 1u) {
        if (t < h) {
            partial[t] += partial[t + h];
        }
        workgroupBarrier();
    }
    if (t == 0u && row < p.rows) {
        y[wid.z * p.rows + row] = partial[0];
    }
}
