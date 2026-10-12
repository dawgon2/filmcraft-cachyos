//! Color page **Clips** panel (fork, after Resolve's Color page): the sequence's video clips as a
//! numbered thumbnail strip above a thin mini timeline, and the Program monitor's **split
//! screen** (adjacent or selected clips side by side, each with its grade).
//!
//! A click on a thumbnail selects that clip for grading (and moves the playhead into it); a click
//! or drag in the mini timeline moves the playhead. Thumbnails show the clip's source at its
//! middle frame (the cheap background thumbnails the Timeline uses); the split screen renders the
//! graded sequence frames.
//!
//! Automation ids: `clips.strip` (the thumbnail strip), `clips.card.<clip>`, `clips.mini`, `clips.split`, `clips.split.<off|adjacent|
//! selected>`, `program.split.<clip>` (a split-screen tile). State: `ui.panels.clips`.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_project::{ClipId, Sequence, TrackItem};
use filmcraft_time::{Tick, TimeDisplay, format_time};
use serde_json::json;

use crate::FilmcraftApp;
use crate::frames::{FrameKey, Target};
use crate::panels::panel_state::SplitScreen;
use crate::theme::Tokens;

/// Most clips the strip lists and the split screen tiles (a long sequence scrolls the strip).
pub const MAX_CLIPS: usize = 2_000;
pub const MAX_TILES: usize = 16;
const HEADER_H: f32 = 24.0;
const MINI_H: f32 = 26.0;
const CARD_GAP: f32 = 8.0;
/// Height of the card's text under its thumbnail (number · track, timecode).
const CARD_TEXT_H: f32 = 30.0;

/// One video clip of the sequence, in timeline order.
#[derive(Clone)]
pub struct ClipRef {
    pub clip: ClipId,
    /// Video track index (0 = V1).
    pub track: usize,
    pub item: TrackItem,
}

/// The sequence's video clips in timeline order (start, then the higher track first), as
/// Resolve's Color page numbers them.
pub fn clips(q: &Sequence) -> Vec<ClipRef> {
    let mut v: Vec<ClipRef> = Vec::new();
    for (ti, tr) in q.video_tracks.iter().enumerate() {
        for it in &tr.items {
            if v.len() >= MAX_CLIPS {
                break;
            }
            v.push(ClipRef { clip: it.id, track: ti, item: it.clone() });
        }
    }
    v.sort_by(|a, b| a.item.start.cmp(&b.item.start).then(b.track.cmp(&a.track)));
    v
}

/// The clip being graded: the first selected video clip, else the top enabled clip under the
/// playhead.
pub fn current(app: &FilmcraftApp, list: &[ClipRef]) -> Option<usize> {
    let sel = &app.session.state.selection;
    if let Some(i) = list.iter().position(|c| sel.contains(&c.clip)) {
        return Some(i);
    }
    let ph = app.session.playhead();
    list.iter().enumerate().filter(|(_, c)| c.item.enabled && c.item.start <= ph && ph < c.item.end()).max_by_key(|(_, c)| c.track).map(|(i, _)| i)
}

/// Select `clip` for grading; the playhead moves to its first frame unless it is already in it.
fn pick(app: &mut FilmcraftApp, c: &ClipRef) {
    let ph = app.session.playhead();
    let mut res = app.session.execute("timeline.select", json!({"clips": [c.clip.0]}));
    if res.is_ok() && !(c.item.start <= ph && ph < c.item.end()) {
        res = app.session.execute("playhead.set", json!({"time": c.item.start.0}));
    }
    if let Err(e) = res {
        app.ui.status = e.to_string();
    }
}

fn label(mode: SplitScreen) -> &'static str {
    match mode {
        SplitScreen::Off => tl!("Split Screen Off"),
        SplitScreen::Adjacent => tl!("Adjacent Clips"),
        SplitScreen::Selected => tl!("Selected Clips"),
    }
}

fn mode_id(mode: SplitScreen) -> &'static str {
    match mode {
        SplitScreen::Off => "off",
        SplitScreen::Adjacent => "adjacent",
        SplitScreen::Selected => "selected",
    }
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let Some(q) = app.session.active_sequence().cloned() else {
        crate::dock::placeholder(ui, rect, &t, tl!("(no sequences)"));
        return;
    };
    let rate = q.settings.frame_rate;
    let list = clips(&q);
    let cur = current(app, &list);
    let mut autos: Vec<(String, Rect, String)> = Vec::new();
    let mut picked: Option<usize> = None;
    let mut seek: Option<Tick> = None;
    let p = ui.painter_at(rect);

    // ---- header: clip count · split screen
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let count = match list.len() {
        1 => tl!("1 clip").to_string(),
        n => crate::i18n::fmt(tl!("{n} clips"), &[("n", &n.to_string())]),
    };
    p.text(pos2(header.min.x + 8.0, header.center().y), Align2::LEFT_CENTER, count, Tokens::ui(11.5), t.text_dim);
    let mode = app.ui.panels.clips.split;
    let dr = Rect::from_min_size(pos2(header.max.x - 168.0, header.min.y + 2.0), vec2(160.0, 20.0));
    let mut chosen = mode;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(dr));
    let cb = egui::ComboBox::from_id_salt("clips-split").selected_text(label(mode)).width(152.0).show_ui(&mut child, |ui| {
        for m in [SplitScreen::Off, SplitScreen::Adjacent, SplitScreen::Selected] {
            let r = ui.selectable_value(&mut chosen, m, label(m));
            autos.push((format!("clips.split.{}", mode_id(m)), r.rect, label(m).into()));
        }
    });
    autos.push(("clips.split".into(), cb.response.rect, tl!("Split Screen").into()));
    cb.response.on_hover_text(tl!("Show several clips side by side in the Program monitor (Ctrl+Alt+W)"));
    if chosen != mode {
        app.ui.panels.clips.split = chosen;
    }

    // ---- thumbnail strip, scrolling sideways
    let strip = Rect::from_min_max(pos2(rect.min.x, header.max.y), pos2(rect.max.x, rect.max.y - MINI_H - 2.0));
    if list.is_empty() {
        p.text(strip.center(), Align2::CENTER_CENTER, tl!("No video clips in this sequence"), Tokens::ui(12.0), t.text_faint);
    } else if strip.height() > CARD_TEXT_H + 16.0 {
        autos.push(("clips.strip".into(), strip, tl!("Clips").into()));
        let thumb_h = (strip.height() - CARD_TEXT_H - 8.0).clamp(16.0, 160.0);
        let (sw, sh) = (q.settings.width.max(1) as f32, q.settings.height.max(1) as f32);
        let thumb_w = (thumb_h * sw / sh).clamp(32.0, 300.0);
        // wide enough for the timecode under the thumbnail
        let card_w = thumb_w.max(96.0);
        let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(strip).id_salt("clips-strip"));
        egui::ScrollArea::horizontal().id_salt("clips-strip-scroll").auto_shrink([false, false]).show(&mut sui, |ui| {
            let total = list.len() as f32 * (card_w + CARD_GAP) + CARD_GAP;
            let (area, _) = ui.allocate_exact_size(vec2(total, strip.height() - 4.0), Sense::hover());
            let clip_rect = ui.clip_rect();
            let p = ui.painter_at(clip_rect);
            for (i, c) in list.iter().enumerate() {
                let x = area.min.x + CARD_GAP + i as f32 * (card_w + CARD_GAP);
                let card = Rect::from_min_size(pos2(x, area.min.y + 4.0), vec2(card_w, thumb_h + CARD_TEXT_H));
                if !clip_rect.intersects(card) {
                    continue;
                }
                let resp = ui.interact(card, egui::Id::new(("clips-card", c.clip.0)), Sense::click());
                let sel = cur == Some(i);
                if resp.hovered() {
                    p.rect_filled(card.expand(3.0), 4.0, t.hover);
                }
                let thumb = Rect::from_min_size(card.min + vec2((card_w - thumb_w) / 2.0, 0.0), vec2(thumb_w, thumb_h));
                p.rect_filled(thumb, 2.0, t.monitor_bg);
                if c.item.enabled {
                    let mid = c.item.start + Tick(c.item.duration.0 / 2);
                    let mt = rate.snap(c.item.source_time_at(mid));
                    if let Some((tex, size)) = app.thumbnail(&ctx, c.item.item, mt, 160) {
                        let r = crate::panels::monitor::fit(thumb, size.x.max(1.0), size.y.max(1.0));
                        p.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                } else {
                    p.text(thumb.center(), Align2::CENTER_CENTER, tl!("Disabled"), Tokens::ui(10.5), t.text_faint);
                }
                let graded = c.item.grade.as_ref().is_some_and(|g| !g.nodes.is_empty() && !g.bypass);
                let stroke = if sel { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.text_faint.gamma_multiply(0.5)) };
                p.rect_stroke(thumb, 2.0, stroke, StrokeKind::Outside);
                // number · track, with a dot when the clip has a grade; then the start timecode
                let num = format!("{:02}  V{}", i + 1, c.track + 1);
                let col = if sel { t.text } else { t.text_dim };
                let p = p.with_clip_rect(card.intersect(clip_rect));
                let nr = p.text(pos2(card.min.x + 2.0, thumb.max.y + 3.0), Align2::LEFT_TOP, num, Tokens::ui(11.0), col);
                if graded {
                    p.circle_filled(pos2(nr.max.x + 8.0, nr.center().y), 3.0, t.accent);
                }
                let tc = format_time(c.item.start, rate, q.settings.drop_frame, TimeDisplay::Timecode, 48000);
                p.text(pos2(card.min.x + 2.0, thumb.max.y + 16.0), Align2::LEFT_TOP, tc, Tokens::ui(10.0), t.text_faint);
                let name = c.item.name.clone();
                let resp = resp.on_hover_text(name.clone());
                autos.push((format!("clips.card.{}", c.clip.0), card.intersect(clip_rect), name));
                if resp.clicked() {
                    picked = Some(i);
                }
            }
            // keep the current clip in view when it changes (arrow keys, the timeline)
            let key = egui::Id::new("clips-strip-last");
            let last: Option<usize> = ui.data(|d| d.get_temp(key));
            if last != cur {
                ui.data_mut(|d| d.insert_temp(key, cur));
                if let Some(i) = cur {
                    let x = area.min.x + CARD_GAP + i as f32 * (card_w + CARD_GAP);
                    ui.scroll_to_rect(Rect::from_min_size(pos2(x, area.min.y), vec2(card_w, 1.0)), None);
                }
            }
        });
    }

    // ---- mini timeline: every video track as a thin lane, the playhead, click / drag to seek
    let mini = Rect::from_min_max(pos2(rect.min.x + 8.0, rect.max.y - MINI_H), pos2(rect.max.x - 8.0, rect.max.y - 4.0));
    let dur = q.duration().max(rate.frame_duration());
    let x_of = |tk: Tick| mini.min.x + (tk.0 as f64 / dur.0.max(1) as f64).clamp(0.0, 1.0) as f32 * mini.width();
    p.rect_filled(mini, 2.0, t.monitor_bg);
    let lanes = q.video_tracks.len().clamp(1, 6);
    let lane_h = (mini.height() - 2.0) / lanes as f32;
    for (i, c) in list.iter().enumerate() {
        if c.track >= lanes {
            continue;
        }
        let y = mini.max.y - 1.0 - (c.track as f32 + 1.0) * lane_h;
        let r = Rect::from_min_max(pos2(x_of(c.item.start), y + 0.5), pos2(x_of(c.item.end()).max(x_of(c.item.start) + 1.0), y + lane_h - 0.5));
        let fill = if cur == Some(i) {
            t.accent
        } else if c.item.enabled {
            Color32::from_rgb(0x3d, 0x6e, 0x8c)
        } else {
            Color32::from_gray(0x44)
        };
        p.rect_filled(r, 1.0, fill);
    }
    let ph = app.session.playhead();
    let px = x_of(ph);
    p.line_segment([pos2(px, mini.min.y - 2.0), pos2(px, mini.max.y + 2.0)], Stroke::new(1.5, t.playhead));
    let mresp = ui.interact(mini, egui::Id::new("clips-mini"), Sense::click_and_drag());
    autos.push(("clips.mini".into(), mini, tl!("Mini timeline").into()));
    if (mresp.clicked() || mresp.dragged())
        && let Some(pp) = mresp.interact_pointer_pos()
    {
        let f = ((pp.x - mini.min.x) / mini.width().max(1.0)).clamp(0.0, 1.0) as f64;
        seek = Some(rate.snap(Tick((dur.0 as f64 * f) as i64)));
    }

    for (id, r, l) in autos {
        if r.is_positive() {
            app.auto.add(&id, r, &l);
        }
    }
    if let Some(c) = picked.and_then(|i| list.get(i)) {
        pick(app, c);
    }
    if let Some(to) = seek
        && let Err(e) = app.session.execute("playhead.set", json!({"time": to.0}))
    {
        app.ui.status = e.to_string();
    }
}

/// The clips the split screen shows (with the index of the current one among them): none when
/// it is off or has fewer than two clips to show.
pub fn split_tiles(app: &FilmcraftApp) -> (Vec<ClipRef>, Option<usize>) {
    let mode = app.ui.panels.clips.split;
    if mode == SplitScreen::Off {
        return (Vec::new(), None);
    }
    let Some(q) = app.session.active_sequence() else { return (Vec::new(), None) };
    let list = clips(q);
    let cur = current(app, &list);
    let (tiles, at): (Vec<ClipRef>, Option<usize>) = match mode {
        SplitScreen::Off => (Vec::new(), None),
        SplitScreen::Adjacent => {
            let Some(i) = cur else { return (Vec::new(), None) };
            let lo = i.saturating_sub(1);
            let hi = i.saturating_add(2).min(list.len());
            (list.get(lo..hi).map(<[ClipRef]>::to_vec).unwrap_or_default(), Some(i - lo))
        }
        SplitScreen::Selected => {
            let sel = &app.session.state.selection;
            let v: Vec<ClipRef> = list.iter().filter(|c| sel.contains(&c.clip)).take(MAX_TILES).cloned().collect();
            let at = cur.and_then(|i| list.get(i)).and_then(|c| v.iter().position(|x| x.clip == c.clip));
            (v, at)
        }
    };
    if tiles.len() < 2 { (Vec::new(), None) } else { (tiles, at) }
}

/// Columns and rows of a grid for `n` tiles in `area` whose tiles are `aspect` wide: the layout
/// with the largest tiles.
pub fn grid(n: usize, area: Rect, aspect: f32) -> (usize, usize) {
    let n = n.max(1);
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    let mut best = (n, 1, 0.0f32);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cw = area.width() / cols as f32;
        let ch = area.height() / rows as f32;
        let w = cw.min(ch * aspect);
        if w >= best.2 {
            best = (cols, rows, w);
        }
    }
    (best.0, best.1)
}

/// Draw the split screen in the Program monitor's `area`: each clip's graded frame (the playhead
/// for the current clip, the middle frame for the others), numbered, the current one outlined.
/// A click on a tile selects that clip.
pub fn split_screen(app: &mut FilmcraftApp, ui: &mut egui::Ui, area: Rect, tiles: &[ClipRef], at: Option<usize>) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let Some(seq_id) = app.session.state.active_sequence else { return };
    let Some(q) = app.session.active_sequence() else { return };
    let rate = q.settings.frame_rate;
    let (sw, sh) = (q.settings.width.max(1) as f32, q.settings.height.max(1) as f32);
    let numbers: Vec<usize> = {
        let all = clips(q);
        tiles.iter().map(|c| all.iter().position(|x| x.clip == c.clip).unwrap_or(0) + 1).collect()
    };
    let (cols, rows) = grid(tiles.len(), area, sw / sh);
    let cell = vec2(area.width() / cols as f32, area.height() / rows as f32);
    let ppp = ctx.pixels_per_point();
    let project = app.session.project.clone();
    let ph = app.session.playhead();
    let mut picked: Option<ClipRef> = None;
    for (i, c) in tiles.iter().enumerate() {
        let (col, row) = (i % cols, i / cols);
        let cr = Rect::from_min_size(area.min + vec2(col as f32 * cell.x, row as f32 * cell.y), cell).shrink(3.0);
        let pic = crate::panels::monitor::fit(cr, sw, sh);
        ui.painter().rect_filled(pic, 0.0, t.monitor_bg);
        let is_cur = at == Some(i);
        let tk = if is_cur && c.item.start <= ph && ph < c.item.end() { ph } else { c.item.start + Tick(c.item.duration.0 / 2) };
        let frame = rate.frame_at(tk);
        let scale = crate::panels::monitor::quantize_scale((pic.width() * ppp / sw).clamp(1.0 / 32.0, 1.0));
        let key =
            FrameKey { target: Target::Sequence(seq_id), frame, size: (scale * 1000.0) as u32, revision: app.session.revision, draft: false, temporal: false };
        app.frames.request(key, rate.tick_of(frame), scale, &project, if is_cur { 0 } else { 2 });
        let name = format!("program-split-{i}");
        let tex = match app.frames.get(&key) {
            Some(img) => Some(app.texture_for(&ctx, &name, key, &img)),
            // the tile's last frame until the new one is ready (a grade edit re-renders it)
            None => app.texture_key(&name).filter(|k| k.frame == frame && k.target == key.target).and_then(|_| app.texture_existing(&name).map(|x| x.0)),
        };
        if let Some(tex) = tex {
            ui.painter().image(tex, pic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if is_cur {
            ui.painter().rect_stroke(pic, 0.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
        }
        let tag = format!("{:02}  {}", numbers.get(i).copied().unwrap_or(0), c.item.name);
        let galley = ui.painter().layout_no_wrap(tag, Tokens::ui(11.0), Color32::WHITE);
        let tr = Rect::from_min_size(pic.left_bottom() + vec2(4.0, -galley.size().y - 6.0), galley.size() + vec2(8.0, 4.0));
        let tr = tr.intersect(pic);
        ui.painter().rect_filled(tr, 3.0, Color32::from_black_alpha(160));
        ui.painter().with_clip_rect(tr).galley(tr.min + vec2(4.0, 2.0), galley, Color32::WHITE);
        let resp = ui.interact(pic, egui::Id::new(("program-split", c.clip.0)), Sense::click());
        app.auto.add(&format!("program.split.{}", c.clip.0), pic, &c.item.name);
        if resp.clicked() && !is_cur {
            picked = Some(c.clone());
        }
    }
    if let Some(c) = picked {
        pick(app, &c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_picks_the_largest_tiles() {
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        assert_eq!(grid(2, wide, 16.0 / 9.0), (2, 1));
        assert_eq!(grid(3, wide, 16.0 / 9.0), (2, 2));
        assert_eq!(grid(4, wide, 16.0 / 9.0), (2, 2));
        let strip = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 300.0));
        assert_eq!(grid(3, strip, 16.0 / 9.0), (3, 1));
        // hostile inputs never panic or divide by zero
        assert_eq!(grid(0, wide, f32::NAN), (1, 1));
        let _ = grid(MAX_TILES, Rect::NOTHING, 0.0);
    }
}
