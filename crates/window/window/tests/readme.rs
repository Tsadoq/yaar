//! The documented list of kept keys and the code cannot part: the README names every kept
//! key of the system the test runs on, as the app binds it, in backquotes.

const README: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../README.md"));

#[test]
fn the_readme_names_every_kept_key_of_this_system_in_backquotes() {
    let kept = yaar_window::kept_keys();
    assert!(!kept.is_empty());
    for control in &kept {
        let key = control.key();
        assert!(
            README.contains(&format!("`{key}`")),
            "the README does not name the kept key `{key}` of {}",
            control.title
        );
    }
}
