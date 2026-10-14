// @effect "Chromatic Aberration"
// @category "Stylize"
// @param amount: float = 4.0 [0.0, 40.0] "Amount (px at the corners)"
// @param center: point = auto "Center"
//
// FilmCraft example effect (original work, MIT OR Apache-2.0). A cheap lens focuses red, green
// and blue at slightly different sizes: red is read a little further from the centre and blue a
// little closer, so edges get coloured fringes that grow towards the corners.
fn effect(px: vec2<f32>) -> vec4<f32> {
    let d = px - P.center;
    // `amount` pixels of shift at a corner of the frame
    let k = P.amount / max(0.5 * length(size), 1.0);
    let r = sample(px + d * k);
    let g = sample(px);
    let b = sample(px - d * k);
    return vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
}
