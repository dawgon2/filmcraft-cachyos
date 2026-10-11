//! Resolve workflow page bar (fork GRADE-PLAN G1.8): Media, Edit, Color, Fairlight and Deliver
//! along the bottom of the window, each mapped to a FilmCraft workspace or mode. Shown only when
//! Settings ▸ General ▸ Workflow is DaVinci Resolve. Automation ids `page.<id>`.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};

use crate::FilmcraftApp;
use crate::state::Mode;
use crate::theme::Tokens;

/// Height of the page bar.
pub const HEIGHT: f32 = 30.0;

/// (id, label, workspace; `None` = the Export mode).
pub const PAGES: [(&str, &str, Option<&str>); 5] = [
    ("media", "Media", Some("Assembly")),
    ("edit", "Edit", Some("Editing")),
    ("color", "Color", Some("Color Grading")),
    ("fairlight", "Fairlight", Some("Audio")),
    ("deliver", "Deliver", None),
];

/// The page the window shows now, if any.
pub fn current(app: &FilmcraftApp) -> Option<&'static str> {
    if app.ui.mode == Mode::Export {
        return Some("deliver");
    }
    if app.ui.mode != Mode::Edit {
        return None;
    }
    PAGES.iter().find(|p| p.2 == Some(app.ui.workspace.as_str())).map(|p| p.0)
}

/// Switch to page `id`; `false` for an unknown id.
pub fn open(app: &mut FilmcraftApp, id: &str) -> bool {
    let Some(&(_, _, ws)) = PAGES.iter().find(|p| p.0 == id) else { return false };
    match ws {
        Some(ws) => {
            app.ui.mode = Mode::Edit;
            if app.ui.workspace != ws {
                app.set_workspace(ws);
            }
        }
        None => app.ui.mode = Mode::Export,
    }
    true
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, t.header_bg);
    p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, t.separator));
    let cur = current(app);
    let w = 92.0;
    let total = w * PAGES.len() as f32;
    let mut x = rect.center().x - total / 2.0;
    let mut clicked = None;
    for (id, label, _) in PAGES {
        let r = Rect::from_min_size(pos2(x, rect.min.y + 3.0), vec2(w - 6.0, rect.height() - 6.0));
        let resp = ui.interact(r, egui::Id::new(("page", id)), Sense::click());
        let sel = cur == Some(id);
        if sel {
            p.rect_filled(r, 4.0, t.accent.gamma_multiply(0.35));
        } else if resp.hovered() {
            p.rect_filled(r, 4.0, t.hover);
        }
        p.text(r.center(), Align2::CENTER_CENTER, label, Tokens::ui(12.0), if sel { t.text } else { t.text_dim });
        app.auto.add(&format!("page.{id}"), r, label);
        if resp.clicked() {
            clicked = Some(id);
        }
        x += w;
    }
    if let Some(id) = clicked {
        open(app, id);
    }
}
