// @effect "VHS"
// @category "Stylize"
// @param amount: float = 0.6 [0.0, 1.0] "Amount"
// @param noise: float = 0.3 [0.0, 1.0] "Noise"
// @param scanlines: bool = true "Scanlines"
//
// FilmCraft example effect (original work, MIT OR Apache-2.0). An old tape look: the picture
// wobbles sideways, colour bleeds to the right of edges (tape stores colour at a lower
// resolution than brightness), the image is a little washed out, with grain and dark scanlines.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

fn effect(px: vec2<f32>) -> vec4<f32> {
    let wobble = sin(px.y * 0.05 + time * 3.0) * 1.5 * P.amount;
    let p = px + vec2(wobble, 0.0);
    let c = sample(p);
    // colour (not brightness) averaged over a few pixels, mostly from the left
    var chroma = vec3(0.0);
    for (var i = -2; i <= 6; i++) {
        let s = sample(p - vec2(f32(i) * 2.0 * P.amount, 0.0)).rgb;
        chroma += s - vec3(luma(s));
    }
    chroma /= 9.0;
    var o = vec3(luma(c.rgb)) + chroma * (1.0 - 0.3 * P.amount);
    // washed-out blacks and soft whites
    o = mix(o, o * 0.9 + 0.06, P.amount);
    o += (hash2(floor(px) + fract(time) * 97.0) - 0.5) * 0.2 * P.noise;
    if P.scanlines && (u32(max(px.y, 0.0)) % 2u) == 1u {
        o *= 1.0 - 0.15 * P.amount;
    }
    return vec4(o, c.a);
}
