//! Colour-grading widgets that work on plain data (no project or effect types), shared by the
//! Lumetri panel and any other grader: a curve editor and a colour wheel.

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
