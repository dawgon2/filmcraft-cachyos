//! Fork RS1: the Resolve workflow shows the Resolve theme, the Premiere workflow its own themes,
//! and either workflow can pick any theme in Settings ▸ Appearance.

use egui::vec2;
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::theme::ThemeKind;
use serde_json::json;

fn frames(h: &mut Harness<'_, FilmcraftApp>, n: usize) {
    for _ in 0..n {
        h.step();
        assert!(h.state().ui_error.is_none(), "the app caught a UI panic: {:?}", h.state().ui_error);
    }
}

fn kind(h: &Harness<'_, FilmcraftApp>) -> ThemeKind {
    h.state().tokens.kind
}

fn run(h: &mut Harness<'_, FilmcraftApp>, id: &str, params: serde_json::Value) {
    h.state_mut().session.execute(id, params).unwrap();
    frames(h, 3);
}

#[test]
fn workflow_switch_swaps_the_theme_and_back() {
    let mut session = Session::default();
    // the same start in both builds: the Premiere workflow with its default theme
    session.prefs.workflow = Workflow::Premiere;
    let app = FilmcraftApp::new(session);
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(10_000).build_eframe(move |_| app);
    frames(&mut h, 3);
    assert_eq!(kind(&h), ThemeKind::Dark);

    run(&mut h, "settings.setWorkflow", json!({"workflow": "resolve"}));
    assert_eq!(kind(&h), ThemeKind::Resolve, "Resolve workflow: Resolve theme");

    run(&mut h, "settings.setWorkflow", json!({"workflow": "premiere"}));
    assert_eq!(kind(&h), ThemeKind::Dark, "back to Premiere: the theme it had");

    // a Premiere theme chosen before the switch comes back too
    run(&mut h, "prefs.set", json!({"key": "appearance.darkTheme", "value": "dark"}));
    assert_eq!(kind(&h), ThemeKind::Medium);
    run(&mut h, "settings.setWorkflow", json!({"workflow": "resolve"}));
    assert_eq!(kind(&h), ThemeKind::Resolve);
    run(&mut h, "settings.setWorkflow", json!({"workflow": "premiere"}));
    assert_eq!(kind(&h), ThemeKind::Medium);

    // either workflow can pick any theme
    run(&mut h, "prefs.set", json!({"key": "appearance.darkTheme", "value": "resolve"}));
    assert_eq!(kind(&h), ThemeKind::Resolve, "Resolve theme in the Premiere workflow");
    run(&mut h, "settings.setWorkflow", json!({"workflow": "resolve"}));
    run(&mut h, "prefs.set", json!({"key": "appearance.darkTheme", "value": "darkest"}));
    assert_eq!(kind(&h), ThemeKind::Dark, "Premiere theme in the Resolve workflow");
}
