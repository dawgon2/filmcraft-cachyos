//! The launch's `--data-dir` reaches the folders found without the launch arguments: the AI and
//! speech models were looked for in the default data folder, so a model installed under
//! `--data-dir` showed as "not installed" (owner, 2026-10-15). Its own test binary: the setting is
//! process-wide.

use filmcraft_engine::autosave::{data_dir, set_launch_data_dir};

#[test]
fn models_follow_the_launch_data_dir() {
    let dir = std::env::temp_dir().join("filmcraft-launch-data-dir-test");
    set_launch_data_dir(&dir);
    assert_eq!(data_dir(), Some(dir.clone()));
    assert_eq!(filmcraft_engine::ai_explain::models_dir(), Some(dir.join("models")));
    assert_eq!(filmcraft_engine::transcript::models_dir(), Some(dir.join("models")));
    // the first launch value wins
    set_launch_data_dir(std::path::Path::new("/elsewhere"));
    assert_eq!(data_dir(), Some(dir));
}
