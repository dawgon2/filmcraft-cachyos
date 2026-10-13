//! Colour-grading widgets that work on plain data (no project or effect types), shared by the
//! Lumetri panel and any other grader: a curve editor and a colour wheel, and the Color page's
//! Resolve-style wheel, jog bar and number boxes.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::theme::Tokens;

/// Curve editor: drag points, click the curve to add, double-click a point to remove. `pts` is the
/// current curve (x, y in 0..1); `hue` selects the wrapping hue-curve style (no fixed endpoints, hue
/// strip). `drag_id` keys the per-curve drag state and must be unique per curve. Returns the edited
/// point list when the user changed it this frame.
pub fn curve_editor(ui: &mut egui::Ui, pts: &[[f32; 2]], hue: bool, col: Color32, t: &Tokens, drag_id: egui::Id) -> Option<Vec<[f32; 2]>> {
    let w = ui.available_width().min(260.0);
    let h = if hue { w * 0.45 } else { w };
    let (area, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(area, 2.0, Color32::from_rgb(0x16, 0x16, 0x16));
    for i in 1..4 {
        let f = i as f32 / 4.0;
        p.line_segment(
            [pos2(area.min.x + area.width() * f, area.min.y), pos2(area.min.x + area.width() * f, area.max.y)],
            Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2a, 0x2a)),
        );
        p.line_segment(
            [pos2(area.min.x, area.min.y + area.height() * f), pos2(area.max.x, area.min.y + area.height() * f)],
            Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2a, 0x2a)),
        );
    }
    if hue {
        let n = 36;
        for i in 0..n {
            let f0 = i as f32 / n as f32;
            let rgb = filmcraft_color::hsl_to_rgb(f0, 0.8, 0.5);
            let c = Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8);
            p.rect_filled(
                Rect::from_min_max(
                    pos2(area.min.x + area.width() * f0, area.max.y - 6.0),
                    pos2(area.min.x + area.width() * (f0 + 1.0 / n as f32) + 0.5, area.max.y),
                ),
                0.0,
                c,
            );
        }
    }
    let to_screen = |q: [f32; 2]| pos2(area.min.x + q[0] * area.width(), area.max.y - q[1] * area.height());
    let from_screen = |s: Pos2| [((s.x - area.min.x) / area.width().max(1.0)).clamp(0.0, 1.0), ((area.max.y - s.y) / area.height().max(1.0)).clamp(0.0, 1.0)];
    // curve polyline
    let draw_pts: Vec<[f32; 2]> = if hue && pts.is_empty() { vec![[0.0, 0.5], [1.0, 0.5]] } else { pts.to_vec() };
    let lut = if hue && !pts.is_empty() {
        let mut ext = Vec::new();
        for off in [-1.0f32, 0.0, 1.0] {
            for q in pts {
                ext.push([(q[0] + off + 1.0) / 3.0, q[1]]);
            }
        }
        let l = filmcraft_render::effects::curve_lut(&ext, 3 * 128);
        l.get(128..256).map(<[f32]>::to_vec).unwrap_or_default()
    } else {
        filmcraft_render::effects::curve_lut(&draw_pts, 128)
    };
    let line: Vec<Pos2> = lut.iter().enumerate().map(|(i, y)| to_screen([i as f32 / 127.0, y.clamp(0.0, 1.0)])).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.5, col)));
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let mut new_pts = pts.to_vec();
    let mut changed = false;
    for (i, q) in pts.iter().enumerate() {
        let sp = to_screen(*q);
        p.circle_filled(sp, 4.0, if dragging == Some(i) { t.hot_text } else { col });
    }
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let near =
            pts.iter().enumerate().map(|(i, q)| (i, to_screen(*q).distance(pos))).filter(|(_, d)| *d < 9.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
        dragging = near.or_else(|| {
            new_pts.push(from_screen(pos));
            changed = true;
            Some(new_pts.len() - 1)
        });
    }
    if resp.dragged()
        && let (Some(i), Some(pos)) = (dragging, resp.interact_pointer_pos())
        && i < new_pts.len()
    {
        new_pts[i] = from_screen(pos);
        changed = true;
    }
    if resp.drag_stopped() {
        dragging = None;
    }
    if resp.double_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(i) = pts.iter().position(|q| to_screen(*q).distance(pos) < 9.0)
    {
        let endpoint = !hue && (i == 0 || i + 1 == pts.len());
        if !endpoint {
            new_pts.remove(i);
            changed = true;
        }
    } else if resp.double_clicked() && !hue {
        new_pts = vec![[0.0, 0.0], [1.0, 1.0]];
        changed = true;
    }
    ui.data_mut(|d| {
        if let Some(i) = dragging {
            d.insert_temp(drag_id, i);
        } else {
            d.remove::<usize>(drag_id);
        }
    });
    ui.add_space(6.0);
    changed.then_some(new_pts)
}

/// What the user changed on a [`color_wheel`] this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelChange {
    /// New puck position (x, y in -1..1, y up): dragging/clicking the ring area, or `[0, 0]` on double-click.
    pub puck: Option<[f32; 2]>,
    /// New lightness (-100..100) from the vertical slider.
    pub lightness: Option<f32>,
}

/// Colour wheel: hue ring, draggable puck (`value`, x/y in -1..1), lightness slider on the left
/// (`lightness` in -100..100). `lightness_id` keys the slider interaction and must be unique per wheel.
pub fn color_wheel(ui: &mut egui::Ui, value: [f32; 2], lightness: f32, size: f32, t: &Tokens, lightness_id: egui::Id) -> WheelChange {
    let mut out = WheelChange::default();
    let v = vec2(value[0], value[1]);
    let (area, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let c = area.center() + vec2(6.0, 0.0);
    let r_out = size / 2.0 - 4.0;
    let r_in = r_out - 10.0;
    let p = ui.painter();
    let n = 72;
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let rgb = filmcraft_color::hsl_to_rgb(1.0 - (i as f32 / n as f32), 0.75, 0.5);
        let col = Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8);
        let base = mesh.vertices.len() as u32;
        for (a, r) in [(a0, r_in), (a0, r_out), (a1, r_out), (a1, r_in)] {
            mesh.colored_vertex(c + vec2(a.cos(), -a.sin()) * r, col);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    p.add(mesh);
    p.circle_filled(c, r_in - 1.0, Color32::from_rgb(0x16, 0x16, 0x16));
    p.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], Stroke::new(1.0, t.text_faint));
    p.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(1.0, t.text_faint));
    let puck = c + vec2(v.x, -v.y) * (r_in - 4.0);
    p.circle_stroke(puck, 5.0, Stroke::new(1.5, Color32::WHITE));
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let d = (pos - c) / (r_in - 4.0);
        let len = d.length().min(1.0);
        let dir = if d.length() > 1e-6 { d / d.length() } else { d };
        out.puck = Some([dir.x * len, -dir.y * len]);
    }
    if resp.double_clicked() {
        out.puck = Some([0.0, 0.0]);
    }
    // lightness slider (vertical) at the wheel's left edge
    let track = Rect::from_min_max(pos2(area.min.x, area.min.y + 8.0), pos2(area.min.x + 4.0, area.max.y - 8.0));
    p.rect_filled(track, 2.0, Color32::from_rgb(0x4b, 0x4b, 0x4b));
    let ky = track.center().y - lightness / 100.0 * track.height() / 2.0;
    p.circle_stroke(pos2(track.center().x, ky), 4.0, Stroke::new(1.5, Color32::from_rgb(0xb0, 0xb0, 0xb0)));
    let lr = ui.interact(track.expand2(vec2(6.0, 0.0)), lightness_id, Sense::click_and_drag());
    if (lr.dragged() || lr.clicked())
        && let Some(pos) = lr.interact_pointer_pos()
    {
        let nv = ((track.center().y - pos.y) / (track.height() / 2.0) * 100.0).clamp(-100.0, 100.0);
        out.lightness = Some(nv);
    }
    out
}

/// The Color page's wheel (fork RESOLVE-UI-PLAN, Resolve-style primaries): a dark bezel, a thin
/// hue ring, a faint crosshair and a small puck. `value` is the puck (x, y in -1..1, y up); click
/// or drag inside the ring to place it, double-click to centre it. Returns the new puck position.
/// The master (brightness) control is a separate [`jog_bar`] under the wheel.
pub fn grade_wheel(ui: &mut egui::Ui, value: [f32; 2], size: f32) -> Option<[f32; 2]> {
    let (area, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let c = area.center();
    let r_bezel = size / 2.0 - 1.0;
    let r_ring = (r_bezel - (size * 0.07).clamp(4.0, 12.0)).max(8.0);
    let ring_w = (size * 0.022).clamp(2.0, 4.0);
    let r_in = r_ring - ring_w;
    let p = ui.painter();
    // bezel: a near-black disc with a soft lighter edge
    p.circle_filled(c, r_bezel, Color32::from_rgb(0x0c, 0x0c, 0x0d));
    p.circle_stroke(c, r_bezel, Stroke::new(1.0, Color32::from_rgb(0x2c, 0x2c, 0x30)));
    // hue ring (red at +x, counter-clockwise as `filmcraft_color::hsl_to_rgb` hue)
    let n = 120;
    let hue_col = |f: f32, a: u8| {
        let rgb = filmcraft_color::hsl_to_rgb(1.0 - f, 0.9, 0.55);
        Color32::from_rgba_unmultiplied((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8, a)
    };
    let mut mesh = egui::Mesh::default();
    // a faint glow of each hue just inside the ring, then the ring itself
    let glow = (r_in * 0.18).clamp(3.0, 14.0);
    for (r0, r1, a_in, a_out) in [(r_in - glow, r_in, 0u8, 46u8), (r_in, r_ring, 255, 255)] {
        for i in 0..n {
            let f0 = i as f32 / n as f32;
            let f1 = (i + 1) as f32 / n as f32;
            let (a0, a1) = (f0 * std::f32::consts::TAU, f1 * std::f32::consts::TAU);
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(c + vec2(a0.cos(), -a0.sin()) * r0, hue_col(f0, a_in));
            mesh.colored_vertex(c + vec2(a0.cos(), -a0.sin()) * r1, hue_col(f0, a_out));
            mesh.colored_vertex(c + vec2(a1.cos(), -a1.sin()) * r1, hue_col(f1, a_out));
            mesh.colored_vertex(c + vec2(a1.cos(), -a1.sin()) * r0, hue_col(f1, a_in));
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    // inner disc under the glow: slightly lighter in the middle
    let mut disc = egui::Mesh::default();
    disc.colored_vertex(c, Color32::from_rgb(0x2a, 0x2a, 0x2e));
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        disc.colored_vertex(c + vec2(a.cos(), -a.sin()) * r_in, Color32::from_rgb(0x17, 0x17, 0x1a));
    }
    for i in 0..n as u32 {
        disc.add_triangle(0, i + 1, i + 2);
    }
    p.add(disc);
    p.add(mesh);
    let faint = Stroke::new(1.0, Color32::from_white_alpha(22));
    p.line_segment([c - vec2(r_in, 0.0), c + vec2(r_in, 0.0)], faint);
    p.line_segment([c - vec2(0.0, r_in), c + vec2(0.0, r_in)], faint);
    let v = vec2(value[0], value[1]);
    let reach = (r_in - 4.0).max(1.0);
    let puck = c + vec2(v.x, -v.y) * reach;
    if v.length() > 0.01 {
        p.line_segment([c, puck], Stroke::new(1.0, Color32::from_white_alpha(110)));
        p.circle_filled(c, 1.5, Color32::from_gray(0x90));
    }
    p.circle_filled(puck, 3.5, Color32::WHITE);
    p.circle_stroke(puck, 3.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    let mut out = None;
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let d = (pos - c) / reach;
        let len = d.length().min(1.0);
        let dir = if d.length() > 1e-6 { d / d.length() } else { d };
        out = Some([dir.x * len, -dir.y * len]);
    }
    if resp.double_clicked() {
        out = Some([0.0, 0.0]);
    }
    out
}

/// A ridged horizontal jog bar (the master control under a Color page wheel): drag left / right
/// to turn it. Returns the bar's response and the horizontal drag this frame in points (0 when
/// idle); a double-click returns `f32::NAN` so the caller can zero the value.
pub fn jog_bar(ui: &mut egui::Ui, width: f32, id: egui::Id) -> (egui::Response, f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    // the ridges scroll with the accumulated drag so the bar seems to turn
    let phase_id = id.with("phase");
    let mut phase: f32 = ui.data(|d| d.get_temp(phase_id)).unwrap_or(0.0);
    let dx = if resp.dragged() { resp.drag_delta().x } else { 0.0 };
    if dx != 0.0 {
        phase = (phase + dx).rem_euclid(4.0);
        ui.data_mut(|d| d.insert_temp(phase_id, phase));
    }
    let p = ui.painter();
    p.rect_filled(rect, 3.0, Color32::from_rgb(0x12, 0x12, 0x14));
    let inner = rect.shrink2(vec2(6.0, 2.5));
    let mut x = inner.min.x + phase;
    while x < inner.max.x {
        // brighter towards the middle, like light falling on a cylinder
        let f = ((x - inner.center().x) / (inner.width() / 2.0)).abs().min(1.0);
        let g = (0x70 as f32 * (1.0 - f * f) + 0x22 as f32 * f * f) as u8;
        p.line_segment([pos2(x, inner.min.y), pos2(x, inner.max.y)], Stroke::new(1.0, Color32::from_gray(g)));
        x += 4.0;
    }
    if resp.hovered() || resp.dragged() {
        p.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(0x50)), egui::StrokeKind::Inside);
    }
    let out = if resp.double_clicked() { f32::NAN } else { dx };
    (resp, out)
}

/// The colour band under a Color page number box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Band {
    Solid(Color32),
    /// left → right gradient
    Gradient(Color32, Color32),
    /// the hue circle (Hue, Saturation)
    Hues,
}

/// A compact boxed number (Resolve-style), draggable sideways and editable by typing, with a thin
/// colour band under it (painted in the 4 points below the box: leave room). Returns the drag
/// value's response.
pub fn value_box(ui: &mut egui::Ui, value: &mut f32, width: f32, range: std::ops::RangeInclusive<f32>, decimals: usize, band: Band) -> egui::Response {
    let speed = ((range.end() - range.start()).abs() / 400.0).max(0.0005);
    let r = ui
        .scope(|ui| {
            let v = &mut ui.visuals_mut().widgets;
            for w in [&mut v.inactive, &mut v.hovered, &mut v.active] {
                w.weak_bg_fill = Color32::from_rgb(0x12, 0x12, 0x14);
                w.bg_fill = Color32::from_rgb(0x12, 0x12, 0x14);
                w.corner_radius = egui::CornerRadius::same(2);
            }
            v.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3a, 0x40));
            ui.add_sized(vec2(width, 18.0), egui::DragValue::new(value).range(range).speed(speed).fixed_decimals(decimals))
        })
        .inner;
    let band_rect = Rect::from_min_size(pos2(r.rect.min.x + 3.0, r.rect.max.y + 2.0), vec2(r.rect.width() - 6.0, 2.0));
    paint_band(ui.painter(), band_rect, band);
    r
}

fn paint_band(p: &egui::Painter, rect: Rect, band: Band) {
    match band {
        Band::Solid(c) => {
            p.rect_filled(rect, 0.0, c);
        }
        Band::Gradient(a, b) => {
            let mut m = egui::Mesh::default();
            m.colored_vertex(rect.left_top(), a);
            m.colored_vertex(rect.right_top(), b);
            m.colored_vertex(rect.right_bottom(), b);
            m.colored_vertex(rect.left_bottom(), a);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            p.add(m);
        }
        Band::Hues => {
            let n = 12;
            for i in 0..n {
                let rgb = filmcraft_color::hsl_to_rgb(i as f32 / n as f32, 0.85, 0.55);
                let col = Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8);
                let w = rect.width() / n as f32;
                p.rect_filled(Rect::from_min_size(pos2(rect.min.x + w * i as f32, rect.min.y), vec2(w, rect.height())), 0.0, col);
            }
        }
    }
}
