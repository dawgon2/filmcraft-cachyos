//! FilmCraft's own window title bar on Linux and Windows (fork; Settings ▸ General ▸ Window title
//! bar), as DaVinci Resolve does it: no desktop title bar, the minimise / maximise / close buttons
//! at the right end of the top row, and resize edges along the window border. The top rows already
//! move the window (`ViewportCommand::StartDrag`) and maximise it on a double click; every top row
//! leaves [`reserved_width`] free at its right end for the buttons drawn here.
//!
//! Automation ids: `window.minimize`, `window.maximize`, `window.close`.

use egui::{Color32, CursorIcon, Rect, ResizeDirection, Sense, Stroke, ViewportCommand, pos2, vec2};

use crate::FilmcraftApp;

/// Width of one window button.
const BUTTON_W: f32 = 40.0;
/// Width of the resize band along each edge, and of the corner squares.
const EDGE: f32 = 4.0;
const CORNER: f32 = 12.0;
/// Hover colour of the close button (the usual "danger" red, not tied to the theme).
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc4, 0x2b, 0x1c);

/// Whether this platform can draw its own title bar (macOS keeps the system's traffic lights).
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "windows"));

/// What Settings ▸ General ▸ Window title bar asks for on this platform: `true` = FilmCraft draws
/// the title bar (the default), `false` = the desktop's. Unknown values count as the default.
pub fn wants_own(title_bar: &str) -> bool {
    SUPPORTED && title_bar != "system"
}

/// Width the top rows leave free at their right end for the window buttons.
pub fn reserved_width(app: &FilmcraftApp) -> f32 {
    if app.window_controls { 3.0 * BUTTON_W } else { 0.0 }
}

/// Height of the topmost row of the current screen (where the buttons sit).
fn top_row_height(app: &FilmcraftApp) -> f32 {
    if app.ui.home.open {
        crate::panels::home::TITLE_H
    } else if !app.ui.show_header {
        30.0
    } else if app.resolve_workflow() {
        if app.ui.show_menu_bar { crate::panels::page_toolbar::MENU_H } else { crate::panels::page_toolbar::height(app) }
    } else {
        38.0
    }
}

/// Draw the window buttons and handle the resize edges; called last in the UI pass so they sit on
/// top of the panels. Does nothing while the desktop draws the title bar.
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    if !app.window_controls {
        return;
    }
    let ctx = ui.ctx().clone();
    let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    let full = ui.max_rect();
    let t = app.tokens;
    let h = top_row_height(app);
    let p = ui.painter().clone();
    let mut x = full.max.x - 3.0 * BUTTON_W;
    let buttons: [(&str, &str); 3] =
        [("window.minimize", tl!("Minimize")), ("window.maximize", if maximized { tl!("Restore") } else { tl!("Maximize") }), ("window.close", tl!("Close"))];
    for (i, (id, label)) in buttons.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, full.min.y), vec2(BUTTON_W, h));
        x += BUTTON_W;
        let resp = ui.interact(r, egui::Id::new(id), Sense::click()).on_hover_text(label);
        app.auto.add(id, r, label);
        let close = i == 2;
        if resp.hovered() {
            p.rect_filled(r, 0.0, if close { CLOSE_HOVER } else { t.hover });
        }
        let color = if resp.hovered() { if close { Color32::WHITE } else { t.text } } else { t.text_dim };
        let stroke = Stroke::new(1.0, color);
        let c = r.center();
        match i {
            0 => {
                p.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], stroke);
            }
            1 if maximized || fullscreen => {
                // restore: two overlapping squares
                p.rect_stroke(Rect::from_center_size(pos2(c.x - 1.5, c.y + 1.5), vec2(8.0, 8.0)), 0.0, stroke, egui::StrokeKind::Middle);
                p.line_segment([pos2(c.x - 1.5, c.y - 4.5), pos2(c.x + 4.5, c.y - 4.5)], stroke);
                p.line_segment([pos2(c.x + 4.5, c.y - 4.5), pos2(c.x + 4.5, c.y + 1.5)], stroke);
            }
            1 => {
                p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 0.0, stroke, egui::StrokeKind::Middle);
            }
            _ => {
                p.line_segment([pos2(c.x - 5.0, c.y - 5.0), pos2(c.x + 5.0, c.y + 5.0)], stroke);
                p.line_segment([pos2(c.x + 5.0, c.y - 5.0), pos2(c.x - 5.0, c.y + 5.0)], stroke);
            }
        }
        if resp.clicked() {
            match i {
                0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                1 if fullscreen => ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false)),
                1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                // a close request: unsaved changes still ask first (`intercept_quit`)
                _ => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
    // after the buttons, so the outer few points of the top-right corner still resize
    if !maximized && !fullscreen {
        resize_edges(ui, full);
    }
}

/// The resize bands along the edges and the corner squares: hover shows the resize cursor, a drag
/// hands the resize to the window manager.
fn resize_edges(ui: &mut egui::Ui, full: Rect) {
    use ResizeDirection as D;
    let (l, r, top, b) = (full.min.x, full.max.x, full.min.y, full.max.y);
    let zones = [
        (Rect::from_min_max(pos2(l, top), pos2(l + CORNER, top + CORNER)), D::NorthWest, CursorIcon::ResizeNorthWest),
        (Rect::from_min_max(pos2(r - CORNER, top), pos2(r, top + CORNER)), D::NorthEast, CursorIcon::ResizeNorthEast),
        (Rect::from_min_max(pos2(l, b - CORNER), pos2(l + CORNER, b)), D::SouthWest, CursorIcon::ResizeSouthWest),
        (Rect::from_min_max(pos2(r - CORNER, b - CORNER), pos2(r, b)), D::SouthEast, CursorIcon::ResizeSouthEast),
        (Rect::from_min_max(pos2(l + CORNER, top), pos2(r - CORNER, top + EDGE)), D::North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(l + CORNER, b - EDGE), pos2(r - CORNER, b)), D::South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(pos2(l, top + CORNER), pos2(l + EDGE, b - CORNER)), D::West, CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(r - EDGE, top + CORNER), pos2(r, b - CORNER)), D::East, CursorIcon::ResizeEast),
    ];
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        // corners are squares at the very corner only (the band, not the whole square), so the
        // window buttons and the panels' edges stay clickable
        let bands = if i < 4 { corner_band(rect, full) } else { vec![rect] };
        for (j, band) in bands.into_iter().enumerate() {
            let resp = ui.interact(band, egui::Id::new(("window-resize", i, j)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(cursor);
            }
            if resp.drag_started() {
                ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        }
    }
}

/// The L-shaped band of a corner square that touches the window border (two rects).
fn corner_band(square: Rect, full: Rect) -> Vec<Rect> {
    let horizontal = if square.min.y <= full.min.y {
        Rect::from_min_max(square.min, pos2(square.max.x, square.min.y + EDGE))
    } else {
        Rect::from_min_max(pos2(square.min.x, square.max.y - EDGE), square.max)
    };
    let vertical = if square.min.x <= full.min.x {
        Rect::from_min_max(square.min, pos2(square.min.x + EDGE, square.max.y))
    } else {
        Rect::from_min_max(pos2(square.max.x - EDGE, square.min.y), square.max)
    };
    vec![horizontal, vertical]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_title_bar_follows_the_setting() {
        assert_eq!(wants_own("filmcraft"), SUPPORTED);
        assert_eq!(wants_own("garbage"), SUPPORTED);
        assert!(!wants_own("system"));
    }

    #[test]
    fn corner_bands_touch_the_border() {
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0));
        let se = Rect::from_min_max(pos2(88.0, 68.0), pos2(100.0, 80.0));
        let b = corner_band(se, full);
        assert_eq!(b[0], Rect::from_min_max(pos2(88.0, 76.0), pos2(100.0, 80.0)));
        assert_eq!(b[1], Rect::from_min_max(pos2(96.0, 68.0), pos2(100.0, 80.0)));
    }
}
