//! The Grading Coach card (fork GRADE-PLAN G2.12): the current lesson step in a small card at the
//! bottom right, and a spotlight outline around the control the step is about.
//!
//! All lesson state lives in the engine (`coach.*` commands, `Session::coach`); this only draws
//! it, runs Back / Skip / Exit / Next through the engine, and reports `element` steps as done when
//! their control is on screen this frame. Automation ids: `coach.card`, `coach.back`, `coach.skip`,
//! `coach.exit`, `coach.next`, `coach.close` (the "lesson complete" card) and `coach.spotlight`.
//!
//! Above the card, "Ask the coach" (G2.12 part 2, [`ask_box`]): a question answered from the
//! fork's guide by the engine (`coach.ask`), with steps whose controls were checked against the
//! automation ids this UI knows ([`crate::coach_ids`]); a clicked step is outlined by the same
//! spotlight.
//!
//! Between the two, a context hint (G2.12 part 3, [`hint_card`]): the engine decides when one
//! shows (`Session::coach`'s `hint`); this draws it and reports the controls hints wait for
//! when they are on screen. Ids: `coach.hint.card`, `coach.hint.dismiss`, `coach.hint.lesson`,
//! `coach.hint.off`.
//!
//! The lesson list ([`lessons_list`]) opens from the Lessons button next to "Ask the coach" or
//! Help ▸ Grading Lessons… (`coach.showLessons`). Ids: `coach.lessons.open`, `coach.lessons`,
//! `coach.lessons.close`, `coach.lessons.item.{lesson id}` (its Start button).

use egui::{Align2, CornerRadius, Frame, Id, LayerId, Margin, Order, RichText, Stroke, StrokeKind, vec2};
use filmcraft_engine::coach::Expect;
use serde_json::{Value, json};

use crate::FilmcraftApp;

/// The card's width in points.
const WIDTH: f32 = 340.0;
/// At most this many `element` steps finish in one frame (consecutive steps already on screen).
const MAX_SEEN_PER_FRAME: usize = 8;

/// Element steps whose control is on screen this frame are done; outside a lesson, a control a
/// context hint waits for is reported once it is on screen.
fn report_elements(app: &mut FilmcraftApp) {
    let watched = app.session.coach_hint_elements();
    if let Some(id) = watched.into_iter().find(|id| app.auto.elements.iter().any(|e| e.id == *id)) {
        app.session.coach_element_seen(id);
    }
    for _ in 0..MAX_SEEN_PER_FRAME {
        let Some(Expect::Element(id)) = app.session.coach.step().map(|s| s.expect.clone()) else { return };
        if !app.auto.elements.iter().any(|e| e.id == id) {
            return;
        }
        app.session.coach_element_seen(&id);
    }
}

/// The current step's targets, once per step: a panel that scrolls calls this, scrolls the first
/// of its controls named here into view (`Ui::scroll_to_rect`) and then [`scrolled`], so the
/// highlighted control is never hidden below the fold. `None` once done (the user may scroll away).
/// The same for the control of an "Ask the coach" step that was clicked (while no lesson step
/// has a target).
pub fn scroll_request(app: &FilmcraftApp, ctx: &egui::Context) -> Option<Vec<String>> {
    if let Some(step) = app.session.coach.step().filter(|s| !s.targets.is_empty()) {
        let done: Option<u64> = ctx.data(|d| d.get_temp(Id::new("coach-scrolled")));
        return (done != Some(app.session.coach.revision)).then(|| step.targets.clone());
    }
    let id = app.ui.coach_ask.spotlight.clone()?;
    let done: Option<u64> = ctx.data(|d| d.get_temp(Id::new("coach-ask-scrolled")));
    (done != Some(app.ui.coach_ask.clicks)).then(|| vec![id])
}

/// The step's control was scrolled into view (see [`scroll_request`]).
pub fn scrolled(app: &FilmcraftApp, ctx: &egui::Context) {
    if app.session.coach.step().is_some_and(|s| !s.targets.is_empty()) {
        let rev = app.session.coach.revision;
        ctx.data_mut(|d| d.insert_temp(Id::new("coach-scrolled"), rev));
    } else {
        let clicks = app.ui.coach_ask.clicks;
        ctx.data_mut(|d| d.insert_temp(Id::new("coach-ask-scrolled"), clicks));
    }
}

fn run(app: &mut FilmcraftApp, cmd: &str) {
    if let Err(e) = app.session.execute(cmd, json!({})) {
        app.ui.status = e.to_string();
    }
}

pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    report_elements(app);
    let (card, spotlit) = lesson_card(app, ctx);
    let mut above = card.map(|r| r.height() + 8.0).unwrap_or(0.0);
    if let Some(r) = hint_card(app, ctx, above) {
        above += r.height() + 8.0;
    }
    ask_box(app, ctx, above);
    // a clicked answer step's control, while no lesson step outlines one
    if !spotlit && let Some(id) = app.ui.coach_ask.spotlight.clone() {
        spotlight(app, ctx, std::slice::from_ref(&id));
    }
}

/// Outline the first of `targets` that is on screen (this frame's rect, else last frame's) and
/// register it as `coach.spotlight`; returns the control's rect. Nothing is drawn over a modal
/// window (Settings, recovery…): the control under it can't be used anyway.
fn spotlight(app: &mut FilmcraftApp, ctx: &egui::Context, targets: &[String]) -> Option<egui::Rect> {
    let t = app.tokens;
    let screen = ctx.content_rect();
    let rect_of = |e: &crate::automation::Element| egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]));
    let on_screen = |r: &egui::Rect| r.is_positive() && r.is_finite();
    // (the panel holding a control clips it: one scrolled out of its panel isn't on screen)
    let panel_of = |r: egui::Rect| {
        app.auto
            .elements
            .iter()
            .filter(|e| e.id.starts_with("panel.") && rect_of(e).intersects(r))
            .map(rect_of)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or(screen)
    };
    let target = targets.iter().find_map(|id| {
        let found = app.auto.elements.iter().find(|e| &e.id == id).or_else(|| app.auto.previous.iter().find(|e| &e.id == id));
        let r = found.map(rect_of).filter(on_screen)?;
        let clip = panel_of(r).expand(4.0).intersect(screen);
        on_screen(&r.intersect(clip)).then(|| (id.clone(), r, clip))
    });
    let target_rect = target.as_ref().map(|(_, r, _)| *r);
    let modal = ctx.memory(|m| m.top_modal_layer()).is_some();
    if let Some((id, r, clip)) = target.filter(|_| !modal) {
        let r = r.expand(3.0);
        let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("coach-spotlight"))).with_clip_rect(clip);
        p.rect_filled(r, 4.0, t.accent.gamma_multiply(0.12));
        p.rect_stroke(r, 4.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
        app.auto.add("coach.spotlight", r.intersect(clip), &id);
    }
    target_rect
}

/// How far above the window's bottom edge the coach's cards sit: clear of the status bar and the
/// 30 pt page bar G1.8 had; the Resolve workflow's taller page bar (fork RS3) lifts them by the
/// difference.
fn bottom_offset(app: &FilmcraftApp) -> f32 {
    let taller = if app.resolve_workflow() { (crate::panels::pages::HEIGHT - 30.0).max(0.0) } else { 0.0 };
    -48.0 - taller
}

/// The lesson card (or the "lesson complete" card): its rect, and whether a lesson step's
/// control is outlined.
fn lesson_card(app: &mut FilmcraftApp, ctx: &egui::Context) -> (Option<egui::Rect>, bool) {
    let t = app.tokens;
    let screen = ctx.content_rect();
    // above the status bar and the page bar
    let offset = vec2(-16.0, bottom_offset(app));
    let frame = Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.accent)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(12));

    let Some(run_state) = app.session.coach.active.clone() else {
        // the "lesson complete" card, until closed
        let Some(done) = app.session.coach.finished.clone() else { return (None, false) };
        let mut close = false;
        let area = egui::Area::new(Id::new("coach-card")).order(Order::Foreground).anchor(Align2::RIGHT_BOTTOM, offset).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(WIDTH.min(screen.width() - 32.0).max(120.0));
                ui.label(RichText::new(tl!("Lesson complete")).strong().color(t.text));
                ui.label(RichText::new(&done.title).color(t.text_dim));
                ui.add_space(6.0);
                let r = ui.button(tl!("Close"));
                app.auto.add_response("coach.close", &r, "Close");
                close = r.clicked();
            });
        });
        app.auto.add("coach.card", area.response.rect, &done.title);
        let rect = area.response.rect;
        if close {
            run(app, "coach.exit");
        }
        return (Some(rect), false);
    };
    let Some(step) = run_state.current().cloned() else { return (None, false) };

    // the spotlight: an outline around the first target control on screen
    let target_rect = spotlight(app, ctx, &step.targets);

    let n = run_state.step + 1;
    let total = run_state.lesson.steps.len();
    let manual = step.expect == Expect::Manual;
    let mut clicked: Option<&'static str> = None;
    let area = egui::Area::new(Id::new("coach-card")).order(Order::Foreground).anchor(Align2::RIGHT_BOTTOM, offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(WIDTH.min(screen.width() - 32.0).max(120.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(&run_state.lesson.title).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(tlf!("Step {n} of {total}", n = n, total = total)).small().color(t.text_faint));
                });
            });
            let tags: Vec<&str> = run_state.lesson.provenance.iter().map(|p| p.tag()).collect();
            ui.label(RichText::new(tags.join(" ")).small().color(t.text_faint));
            ui.add_space(4.0);
            ui.label(RichText::new(&step.text).color(t.text));
            if !step.why.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{}: {}", tl!("Why"), step.why)).color(t.text_dim));
            }
            if !step.targets.is_empty() && target_rect.is_none() {
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("The highlighted control isn't on screen yet.")).small().italics().color(t.text_faint));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let r = ui.add_enabled(run_state.step > 0, egui::Button::new(tl!("Back")));
                app.auto.add_response("coach.back", &r, "Back");
                if r.clicked() {
                    clicked = Some("coach.back");
                }
                let r = ui.button(tl!("Skip"));
                app.auto.add_response("coach.skip", &r, "Skip");
                if r.clicked() {
                    clicked = Some("coach.skip");
                }
                let r = ui.button(tl!("Exit"));
                app.auto.add_response("coach.exit", &r, "Exit");
                if r.clicked() {
                    clicked = Some("coach.exit");
                }
                if manual {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.button(RichText::new(tl!("Next")).strong());
                        app.auto.add_response("coach.next", &r, "Next");
                        if r.clicked() {
                            clicked = Some("coach.next");
                        }
                    });
                }
            });
        });
    });
    app.auto.add("coach.card", area.response.rect, &run_state.lesson.title);
    if let Some(cmd) = clicked {
        run(app, cmd);
    }
    (Some(area.response.rect), target_rect.is_some())
}

// ---------- Ask the coach ----------

/// Ask the engine (`coach.ask`); the answer replaces the last one.
fn ask(app: &mut FilmcraftApp) {
    let q = app.ui.coach_ask.input.trim().to_string();
    if q.is_empty() {
        return;
    }
    app.ui.coach_ask.spotlight = None;
    match app.session.execute("coach.ask", json!({ "question": q })) {
        Ok(v) => app.ui.coach_ask.answer = Some(v),
        Err(e) => app.ui.status = e.to_string(),
    }
}

/// What a click in the box asks for (done after drawing).
enum AskAction {
    Open,
    Close,
    Ask,
    Step(usize),
    Lesson(String),
    /// Open (true) or close the lesson list.
    Lessons(bool),
}

/// The "Ask the coach" box, above the lesson card: a question box that answers from the fork's
/// guide with steps naming real controls (`coach.ask`). Shown in the Color Grading workspace,
/// with a lesson, or once opened. Automation ids: `coach.ask.open`, `coach.ask.close`,
/// `coach.ask.input`, `coach.ask.go`, `coach.ask.step.{n}` (1-based; a click outlines the
/// step's control) and `coach.ask.lesson` (starts the suggested lesson).
fn ask_box(app: &mut FilmcraftApp, ctx: &egui::Context, above: f32) {
    let lesson_up = app.session.coach.active.is_some() || app.session.coach.finished.is_some();
    if !(app.ui.coach_ask.open || app.ui.coach_ask.lessons || lesson_up || app.ui.workspace == "Color Grading") {
        return;
    }
    let t = app.tokens;
    let screen = ctx.content_rect();
    let offset = vec2(-16.0, bottom_offset(app) - above);
    let frame = Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.control_border)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10));
    let mut action: Option<AskAction> = None;
    let area = egui::Area::new(Id::new("coach-ask")).order(Order::Foreground).anchor(Align2::RIGHT_BOTTOM, offset).show(ctx, |ui| {
        if app.ui.coach_ask.lessons {
            action = lessons_list(app, ui, frame, above);
            return;
        }
        if !app.ui.coach_ask.open {
            let pill = |text: &str| {
                egui::Button::new(RichText::new(text).small().color(t.text))
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0, t.control_border))
                    .corner_radius(CornerRadius::same(10))
            };
            ui.horizontal(|ui| {
                let r = ui.add(pill(tl!("Lessons")));
                app.auto.add_response("coach.lessons.open", &r, "Lessons");
                if r.clicked() {
                    action = Some(AskAction::Lessons(true));
                }
                let r = ui.add(pill(tl!("Ask the coach")));
                app.auto.add_response("coach.ask.open", &r, "Ask the coach");
                if r.clicked() {
                    action = Some(AskAction::Open);
                }
            });
            return;
        }
        frame.show(ui, |ui| {
            let width = WIDTH.min(screen.width() - 32.0).max(120.0);
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Ask the coach")).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.small_button("×").on_hover_text(tl!("Close"));
                    app.auto.add_response("coach.ask.close", &r, "Close");
                    if r.clicked() {
                        action = Some(AskAction::Close);
                    }
                });
            });
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut app.ui.coach_ask.input)
                    .hint_text(tl!("e.g. how do I make the sky bluer?"))
                    .char_limit(300)
                    .desired_width((width - 64.0).max(60.0));
                let r = ui.add(edit);
                app.auto.add_response("coach.ask.input", &r, "Ask the coach");
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let go = ui.add_enabled(!app.ui.coach_ask.input.trim().is_empty(), egui::Button::new(tl!("Ask")));
                app.auto.add_response("coach.ask.go", &go, "Ask");
                if go.clicked() || enter {
                    action = Some(AskAction::Ask);
                }
            });
            let Some(answer) = app.ui.coach_ask.answer.clone() else { return };
            ui.add_space(6.0);
            let steps = answer["steps"].as_array().cloned().unwrap_or_default();
            if steps.is_empty() {
                let covered = answer["covered"].as_bool().unwrap_or(false);
                let asked_nothing = answer["message"].as_str().is_some_and(|m| m.starts_with("Ask about"));
                let none = if asked_nothing {
                    Some(tl!("Ask about a grading task."))
                } else if !covered {
                    Some(tl!("The guide doesn't cover this yet."))
                } else if !answer["lesson"].is_object() {
                    Some(tl!("The guide has no step-by-step answer for this yet."))
                } else {
                    None
                };
                if let Some(none) = none {
                    ui.label(RichText::new(none).color(t.text_dim));
                }
                if let Some(summary) = answer["summary"].as_str().filter(|s| !s.is_empty()) {
                    ui.label(RichText::new(summary).small().color(t.text_dim));
                }
            } else {
                egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, st) in steps.iter().enumerate() {
                        let n = i + 1;
                        let text = st["text"].as_str().unwrap_or("");
                        let ui_id = st["ui"].as_str();
                        let lit = ui_id.is_some() && app.ui.coach_ask.spotlight.as_deref() == ui_id;
                        let color = if lit { t.accent } else { t.text };
                        let r = ui.add(egui::Label::new(RichText::new(format!("{n}. {text}")).color(color)).wrap().sense(egui::Sense::click()));
                        let r = if ui_id.is_some() {
                            r.on_hover_text(tl!("Click to highlight this control.")).on_hover_cursor(egui::CursorIcon::PointingHand)
                        } else {
                            r
                        };
                        app.auto.add_response(&format!("coach.ask.step.{n}"), &r, text);
                        if r.clicked() {
                            action = Some(AskAction::Step(i));
                        }
                        ui.add_space(2.0);
                    }
                });
            }
            if let (Some(id), Some(title)) = (answer["lesson"]["id"].as_str(), answer["lesson"]["title"].as_str()) {
                ui.add_space(4.0);
                let label = if steps.is_empty() { tlf!("Try the lesson “{title}”", title = title) } else { tlf!("Lesson: {title}", title = title) };
                let r = ui.link(label);
                app.auto.add_response("coach.ask.lesson", &r, title);
                if r.clicked() {
                    action = Some(AskAction::Lesson(id.to_string()));
                }
            }
            // the section the steps come from (a "Why" section's steps come from its "Steps")
            let k = if steps.is_empty() { 0 } else { answer["stepsSource"].as_u64().and_then(|k| usize::try_from(k).ok()).unwrap_or(0) };
            if let Some(src) = answer["sources"].get(k).or_else(|| answer["sources"].get(0)) {
                let tags: Vec<&str> = src["provenance"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let from = tlf!("From the guide: {heading}", heading = src["heading"].as_str().unwrap_or(""));
                ui.label(RichText::new(format!("{from} ({}) {}", src["page"].as_str().unwrap_or(""), tags.join(" "))).small().color(t.text_faint));
            }
            let removed = answer["removed"].as_u64().unwrap_or(0);
            if removed > 0 {
                ui.label(
                    RichText::new(match (answer["removedControls"].as_u64().unwrap_or(0) > 0, answer["removedCommands"].as_u64().unwrap_or(0) > 0) {
                        (true, false) => tlf!("{n} steps left out: they name controls that aren't available here.", n = removed),
                        (false, true) => tlf!("{n} steps left out: they name commands that aren't available here.", n = removed),
                        _ => tlf!("{n} steps left out: they name controls or commands that aren't available here.", n = removed),
                    })
                    .small()
                    .italics()
                    .color(t.text_faint),
                );
            }
        });
    });
    if app.ui.coach_ask.lessons {
        app.auto.add("coach.lessons", area.response.rect, "Lessons");
    } else if app.ui.coach_ask.open {
        app.auto.add("coach.ask", area.response.rect, "Ask the coach");
    }
    match action {
        Some(AskAction::Open) => app.ui.coach_ask.open = true,
        Some(AskAction::Lessons(open)) => app.ui.coach_ask.lessons = open,
        Some(AskAction::Close) => {
            app.ui.coach_ask.open = false;
            app.ui.coach_ask.spotlight = None;
        }
        Some(AskAction::Ask) => ask(app),
        Some(AskAction::Step(i)) => {
            let st = app.ui.coach_ask.answer.as_ref().and_then(|a| a["steps"].get(i)).cloned().unwrap_or_default();
            if let Some(id) = st["ui"].as_str() {
                let same = app.ui.coach_ask.spotlight.as_deref() == Some(id);
                app.ui.coach_ask.spotlight = (!same).then(|| id.to_string());
                app.ui.coach_ask.clicks = app.ui.coach_ask.clicks.wrapping_add(1);
            } else if let Some(cmd) = st["command"].as_str() {
                app.ui.status = tlf!("This step runs the command {cmd}.", cmd = cmd);
            }
        }
        Some(AskAction::Lesson(id)) => {
            app.ui.coach_ask.spotlight = None;
            app.ui.coach_ask.lessons = false;
            if let Err(e) = app.session.execute("coach.start", json!({ "lesson": id })) {
                app.ui.status = e.to_string();
            }
        }
        None => {}
    }
}

// ---------- the lesson list ----------

/// The lesson list (in place of the Ask box): each lesson's title, a tick when finished, its
/// one-line summary and a Start button (`coach.lessons.item.{id}`).
fn lessons_list(app: &mut FilmcraftApp, ui: &mut egui::Ui, frame: Frame, above: f32) -> Option<AskAction> {
    let t = app.tokens;
    let width = WIDTH.min(ui.ctx().content_rect().width() - 32.0).max(120.0);
    let mut action = None;
    frame.show(ui, |ui| {
        ui.set_width(width);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Lessons")).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.small_button("×").on_hover_text(tl!("Close"));
                app.auto.add_response("coach.lessons.close", &r, "Close");
                if r.clicked() {
                    action = Some(AskAction::Lessons(false));
                }
            });
        });
        ui.add_space(4.0);
        // (an area anchored at the bottom only gets the room it had last frame: ask for the
        // list's height outright, within the window)
        // (minus the cards stacked below it)
        let height = (ui.ctx().content_rect().height() - 200.0 - above).clamp(120.0, 360.0);
        egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, true]).show(ui, |ui| {
            for l in filmcraft_engine::coach::bundled() {
                let done = app.session.prefs.color.coach.completed.contains(&l.id);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&l.title).strong().color(t.text));
                    if done {
                        ui.label(RichText::new("✔").color(t.accent)).on_hover_text(tl!("Completed"));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if done { tl!("Again") } else { tl!("Start") };
                        let r = ui.button(label);
                        app.auto.add_response(&format!("coach.lessons.item.{}", l.id), &r, &l.title);
                        if r.clicked() {
                            action = Some(AskAction::Lesson(l.id.clone()));
                        }
                    });
                });
                if !l.summary.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&l.summary).small().color(t.text_dim)).wrap());
                }
                ui.add_space(6.0);
            }
        });
    });
    action
}

// ---------- context hints ----------

/// The context hint on screen (the engine chose it), `above` points over the bottom-right
/// anchor: why, what, a link to its lesson, Got it and Turn hints off. Returns its rect.
fn hint_card(app: &mut FilmcraftApp, ctx: &egui::Context, above: f32) -> Option<egui::Rect> {
    let hint = app.session.coach.hint.clone()?;
    let t = app.tokens;
    let screen = ctx.content_rect();
    let offset = vec2(-16.0, bottom_offset(app) - above);
    let frame = Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.accent)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10));
    let lesson = hint.lesson.as_deref().and_then(filmcraft_engine::coach::lesson);
    let mut clicked: Option<(&'static str, Value)> = None;
    let area = egui::Area::new(Id::new("coach-hint")).order(Order::Foreground).anchor(Align2::RIGHT_BOTTOM, offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(WIDTH.min(screen.width() - 32.0).max(120.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Tip")).small().color(t.accent));
                ui.label(RichText::new(&hint.title).strong().color(t.text));
            });
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(&hint.why).color(t.text)).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&hint.text).color(t.text_dim)).wrap());
            ui.add_space(6.0);
            if let Some(l) = &lesson {
                let r = ui.link(tlf!("Lesson: {title}", title = l.title.as_str()));
                app.auto.add_response("coach.hint.lesson", &r, &l.title);
                if r.clicked() {
                    clicked = Some(("coach.start", json!({ "lesson": l.id })));
                }
            }
            ui.horizontal(|ui| {
                let r = ui.button(tl!("Got it"));
                app.auto.add_response("coach.hint.dismiss", &r, "Got it");
                if r.clicked() {
                    clicked = Some(("coach.dismissHint", json!({})));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.add(egui::Button::new(RichText::new(tl!("Turn hints off")).small().color(t.text_faint)).frame(false));
                    app.auto.add_response("coach.hint.off", &r, "Turn hints off");
                    if r.clicked() {
                        clicked = Some(("coach.dismissHint", json!({"off": true})));
                    }
                });
            });
        });
    });
    let rect = area.response.rect;
    app.auto.add("coach.hint.card", rect, &hint.title);
    if let Some((cmd, params)) = clicked
        && let Err(e) = app.session.execute(cmd, params)
    {
        app.ui.status = e.to_string();
    }
    Some(rect)
}
