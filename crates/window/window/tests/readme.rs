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

/// The keys the key tables of `readme` name for one system, each as written in a cell,
/// lowercased: the cells of the first column (`macos`) or the second of every table headed
/// `On macOS | On Linux | Does`, split at `, `. A key is a whole item, so `Cmd-Home` names
/// no `cmd-h`, and a key named elsewhere in the README (the backquoted lists) names nothing.
fn keys_of_the_tables(readme: &str, macos: bool) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_table = false;
    for line in readme.lines() {
        if !line.starts_with('|') {
            in_table = false;
        } else if line.starts_with("| On macOS | On Linux | Does") {
            in_table = true;
        } else if in_table {
            let cell = line.split('|').nth(if macos { 1 } else { 2 }).unwrap_or("");
            keys.extend(cell.split(", ").map(|key| key.trim().to_lowercase()));
        }
    }
    keys
}

/// What of `wanted`, as `key().replace(',', "comma")` words them, the tables of `readme` do
/// not name for the system.
fn not_in_the_tables<'a>(readme: &str, macos: bool, wanted: &[&'a str]) -> Vec<&'a str> {
    let named = keys_of_the_tables(readme, macos);
    wanted
        .iter()
        .copied()
        .filter(|key| !named.contains(&key.to_string()))
        .collect()
}

#[test]
fn the_readme_names_the_key_of_every_control_of_the_app_in_the_tables_words() {
    // The tables write `Cmd-Alt-Down` and `Ctrl-Alt-Down`, and say the comma in words.
    let controls = yaar_window::controls();
    let wanted: Vec<String> = controls
        .iter()
        .filter(|control| control.keystroke.starts_with("secondary-"))
        .map(|control| control.key().replace(',', "comma"))
        .collect();
    assert!(!wanted.is_empty());
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let missing = not_in_the_tables(README, cfg!(target_os = "macos"), &wanted);
    assert!(
        missing.is_empty(),
        "the key tables of the README do not name these keys: {}",
        missing.join(", ")
    );
}

#[test]
fn the_scan_of_the_key_tables_finds_a_missing_row_a_prefix_and_a_key_named_elsewhere() {
    let readme = "\
Kept: `cmd-h` and `ctrl-h`, and Cmd-H in a sentence.

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-Home, Cmd-End | Ctrl-Home | Scrolls | anywhere |
| Cmd-comma | Ctrl-comma | Settings | anywhere |
| Cmd-Alt-Down, Cmd-Alt-Up | none | Moves | anywhere |

| Key | Does |
| --- | --- |
| Cmd-H | not a key table of the app |
";
    // Named: as whole items of the first column, in any case.
    assert!(not_in_the_tables(readme, true, &["cmd-home", "cmd-end", "cmd-comma"]).is_empty());
    assert!(not_in_the_tables(readme, true, &["cmd-alt-down", "cmd-alt-up"]).is_empty());
    // A prefix of a key named, the same key in the backquoted line, in a sentence and in
    // another kind of table: none names it.
    assert_eq!(not_in_the_tables(readme, true, &["cmd-h"]), ["cmd-h"]);
    assert_eq!(
        not_in_the_tables(readme, true, &["cmd-alt-d"]),
        ["cmd-alt-d"]
    );
    // The column of the system: the Linux column holds the Ctrl keys.
    assert_eq!(
        not_in_the_tables(
            readme,
            false,
            &["ctrl-home", "ctrl-comma", "ctrl-alt-down", "ctrl-h"]
        ),
        ["ctrl-alt-down", "ctrl-h"]
    );
    assert_eq!(
        not_in_the_tables(readme, true, &["ctrl-home"]),
        ["ctrl-home"]
    );
    // A table that is missing its row reports it.
    let without = readme.replace("| Cmd-comma | Ctrl-comma | Settings | anywhere |\n", "");
    assert_eq!(
        not_in_the_tables(&without, true, &["cmd-comma"]),
        ["cmd-comma"]
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
