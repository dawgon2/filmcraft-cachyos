// @effect "Glitch"
// @category "Stylize"
// @param strength: float = 0.5 [0.0, 1.0] "Strength"
// @param block: float = 24.0 [2.0, 200.0] "Band height (px)"
// @param speed: float = 8.0 [0.0, 60.0] "Changes per second"
//
// FilmCraft example effect (original work, MIT OR Apache-2.0). Horizontal bands jump sideways and
// split into red and blue copies; which bands jump changes `speed` times a second.
fn hash1(n: f32) -> f32 {
    return fract(sin(n * 12.9898) * 43758.5453);
}

fn effect(px: vec2<f32>) -> vec4<f32> {
    let band = floor(px.y / max(P.block, 1.0));
    let tick = floor(time * P.speed);
    var shift = 0.0;
    if hash1(band * 7.13 + tick * 1.71) < P.strength * 0.5 {
        shift = (hash1(band + tick * 3.31) - 0.5) * size.x * 0.2 * P.strength;
    }
    let split = select(0.0, 2.0 + P.strength * 6.0, shift != 0.0);
    let p = vec2(px.x + shift, px.y);
    let c = sample(p);
    let r = sample(p + vec2(split, 0.0));
    let b = sample(p - vec2(split, 0.0));
    return vec4(r.r, c.g, b.b, c.a);
}
