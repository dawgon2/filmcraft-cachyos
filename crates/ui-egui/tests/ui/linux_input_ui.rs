//! LINUX-PLAN L1.6: input as Wayland delivers it, through the whole app in both workflows. An
//! input method (fcitx5 / IBus over `zwp_text_input_v3`) composes text with preedit events and
//! commits it; a touchpad scrolls in pixels (`PixelDelta`) where a wheel scrolls in lines, and
//! winit 0.30 has no Wayland pinch, so Alt + scroll is the zoom that works there.

use egui::{Event, ImeEvent, Modifiers, MouseWheelUnit, PointerButton, Rect, TouchPhase, pos2, vec2};
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::json;

fn harness(workflow: Workflow) -> Harness<'static, FilmcraftApp> {
    let mut session = Session::default();
    session.prefs.workflow = workflow;
    session.execute("file.openDemoProject", json!({})).expect("demo project");
    let app = FilmcraftApp::new(session);
    Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(10_000).build_eframe(move |_| app)
}

fn frames(h: &mut Harness<'_, FilmcraftApp>, n: usize) {
    for _ in 0..n {
        h.step();
        assert!(h.state().ui_error.is_none(), "the app caught a UI panic: {:?}", h.state().ui_error);
    }
}

fn find(h: &Harness<'_, FilmcraftApp>, matches: impl Fn(&str) -> bool) -> Option<Rect> {
    let e = h.state().auto.elements.iter().find(|e| matches(&e.id))?;
    let [x, y, w, hh] = e.rect;
    Some(Rect::from_min_size(pos2(x, y), vec2(w, hh)))
}

fn wheel(h: &mut Harness<'_, FilmcraftApp>, at: egui::Pos2, unit: MouseWheelUnit, delta: egui::Vec2, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::MouseWheel { unit, delta, modifiers, phase: TouchPhase::Move });
    frames(h, 2);
}

#[test]
fn an_input_method_composes_into_a_text_field_in_both_workflows() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let mut h = harness(workflow);
        frames(&mut h, 4);
        let Some(search) = find(&h, |id| id.starts_with("project") && id.ends_with(".search")) else {
            panic!("{workflow:?}: no Project panel search field");
        };
        h.event(Event::PointerMoved(search.center()));
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: search.center(), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            frames(&mut h, 1);
        }
        // fcitx5 / IBus: the candidate shows while composing, then the chosen text is committed
        h.event(Event::Ime(ImeEvent::Preedit { text: "にほん".into(), active_range_chars: Some(0..3) }));
        frames(&mut h, 1);
        h.event(Event::Ime(ImeEvent::Preedit { text: "日本".into(), active_range_chars: Some(0..2) }));
        frames(&mut h, 1);
        h.event(Event::Ime(ImeEvent::Commit("日本".into())));
        frames(&mut h, 2);
        assert_eq!(h.state().ui.project_search, "日本", "{workflow:?}: committed text lands once, preedit doesn't stay");
        // a cancelled composition (empty preedit) leaves the text alone
        h.event(Event::Ime(ImeEvent::Preedit { text: "ご".into(), active_range_chars: None }));
        frames(&mut h, 1);
        h.event(Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None }));
        frames(&mut h, 2);
        assert_eq!(h.state().ui.project_search, "日本", "{workflow:?}");
    }
}

#[test]
fn touchpad_pixels_and_wheel_lines_scroll_the_timeline_and_alt_zooms_in_both_workflows() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let mut h = harness(workflow);
        frames(&mut h, 4);
        let Some(clip) = find(&h, |id| id.strip_prefix("timeline.clip.").is_some_and(|rest| !rest.contains('.'))) else {
            panic!("{workflow:?}: no Timeline clip on screen");
        };
        let at = clip.center();
        // zoom in first so there is room to scroll sideways
        let pps = h.state().ui.timeline.target_pps;
        wheel(&mut h, at, MouseWheelUnit::Point, vec2(0.0, 60.0), Modifiers::ALT);
        frames(&mut h, 4);
        let zoomed = h.state().ui.timeline.target_pps;
        assert!(zoomed > pps, "{workflow:?}: Alt + touchpad scroll zooms ({pps} → {zoomed})");
        // two-finger swipe sideways: many small pixel steps
        let before = h.state().ui.timeline.target_scroll;
        for _ in 0..6 {
            wheel(&mut h, at, MouseWheelUnit::Point, vec2(-25.0, 0.0), Modifiers::NONE);
        }
        let swiped = h.state().ui.timeline.target_scroll;
        assert!(swiped != before, "{workflow:?}: a touchpad swipe scrolls the Timeline ({before} → {swiped})");
        // a wheel's line steps with Shift scroll sideways too
        wheel(&mut h, at, MouseWheelUnit::Line, vec2(0.0, -3.0), Modifiers::SHIFT);
        let lined = h.state().ui.timeline.target_scroll;
        assert!(lined != swiped, "{workflow:?}: Shift + wheel lines scroll sideways ({swiped} → {lined})");
        // huge deltas are clamped and never reach the view as infinities (NaN, infinite and
        // near-f32::MAX events aren't sent: egui's own wheel smoothing debug-asserts on them before
        // the Timeline sees them)
        for d in [vec2(1e9, 1e9), vec2(-1e9, 0.0), vec2(0.0, -1e9)] {
            wheel(&mut h, at, MouseWheelUnit::Point, d, Modifiers::NONE);
            wheel(&mut h, at, MouseWheelUnit::Page, d, Modifiers::ALT);
        }
        let v = &h.state().ui.timeline;
        assert!(v.target_scroll.is_finite() && v.target_pps.is_finite() && v.target_pps > 0.0, "{workflow:?}: {} {}", v.target_scroll, v.target_pps);
    }
}
