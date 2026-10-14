// @effect "Light Leak"
// @category "Stylize"
// @param tint: color = #ff7a2e "Color"
// @param intensity: float = 0.6 [0.0, 1.0] "Intensity"
// @param position: point = auto "Position"
// @param radius: float = 0.6 [0.05, 2.0] "Size"
// @param drift: float = 0.2 [0.0, 2.0] "Drift"
//
// FilmCraft example effect (original work, MIT OR Apache-2.0). A soft warm glow, as when light
// leaks into a film camera, screened over the picture; it drifts slowly over time.
fn effect(px: vec2<f32>) -> vec4<f32> {
    let c = sample(px);
    let d = max(length(size), 1.0);
    let centre = P.position + vec2(sin(time * 0.7), cos(time * 0.5)) * P.drift * d * 0.1;
    let r = length(px - centre) / (max(P.radius, 0.01) * d);
    let glow = exp(-r * r * 2.0) * P.intensity;
    // screen: brightens, never darkens
    let o = vec3(1.0) - (vec3(1.0) - c.rgb) * (vec3(1.0) - P.tint * glow);
    return vec4(o, c.a);
}
