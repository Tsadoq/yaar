//! The documented list of keys and the code cannot part: the README names every kept key of
//! the system the test runs on, as the app binds it, in backquotes, and the key of every
//! other control of the app's own, as the tables write it.

const README: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../README.md"));

#[test]
fn the_readme_names_every_kept_key_of_this_system_in_backquotes() {
    let kept = yaar_window::kept_keys();
    assert!(!kept.is_empty());
    let missing: Vec<String> = kept
        .iter()
        .filter(|control| !README.contains(&format!("`{}`", control.key())))
        .map(|control| format!("`{}` of {}", control.key(), control.title))
        .collect();
    assert!(
        missing.is_empty(),
        "the README does not name these kept keys: {}",
        missing.join(", ")
    );
}

#[test]
fn the_readme_names_the_key_of_every_control_of_the_app_in_the_tables_words() {
    // The tables write `Cmd-Alt-Down` and `Ctrl-Alt-Down`, and say the comma in words.
    let readme = README.to_lowercase();
    let missing: Vec<String> = yaar_window::controls()
        .iter()
        .filter(|control| control.keystroke.starts_with("secondary-"))
        .filter(|control| !readme.contains(&control.key().replace(',', "comma")))
        .map(|control| format!("{} ({})", control.key(), control.title))
        .collect();
    assert!(
        missing.is_empty(),
        "the README does not name these keys: {}",
        missing.join(", ")
    );
}

#[test]
fn the_readme_says_that_on_linux_only_ctrl_shift_c_and_ctrl_shift_v_are_kept_in_a_terminal_besides_the_old_ones()
 {
    // Hard-wrapped lines are one line to a reader.
    let readme = README.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        readme.contains(
            "`shift-pagedown`, the only keys kept in a terminal on Linux are `ctrl-shift-c` and `ctrl-shift-v`"
        ),
        "the README does not say which keys are kept in a terminal on Linux"
    );
}
