//! The documented list of keys and the code cannot part: the README names every kept key of
//! the system the test runs on, as the app binds it, in backquotes, and the key of every
//! other control of the app's own, as the tables write it; and the other way, every key
//! the tables name for the system is the key of a control, so that a key the app lost does
//! not stay in the README. The tables are the two of the key tables headed `On macOS | On
//! Linux` (the keys of the app, and the keys kept in a terminal) and the table headed
//! `Key | Does` under "Keys on the settings screen", which is held to the keys of
//! `yaar_frame::settings_screen::controls()` in both directions. Every scan has a control
//! half: a copy of the README with a key added or a row taken away, which the scan must
//! find. The other tables of the README, the keys of the review pane among them, are read
//! by no test here.

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

/// The keys of one cell of a key table. In the table of what the app does the cell is a list
/// (`Cmd-K, Cmd-A`): its items split at `, `, the comma said in words. In the table of the
/// kept keys the cell is a sentence and the keys are in backquotes: `^A` and the Cmd keys
/// the sentence says of other terminals are none. A key starts with a modifier.
fn keys_of_a_cell(cell: &str, kept_table: bool) -> Vec<String> {
    let is_key = |item: &str| {
        ["cmd-", "ctrl-", "shift-", "alt-"]
            .iter()
            .any(|modifier| item.starts_with(modifier))
    };
    let mut keys: Vec<String> = if kept_table {
        cell.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_lowercase)
            .filter(|item| is_key(item))
            .collect()
    } else {
        cell.split(", ")
            .map(|item| item.trim().to_lowercase())
            .filter(|item| is_key(item))
            .map(|item| match item.strip_suffix("comma") {
                Some(modifiers) => format!("{modifiers},"),
                None => item,
            })
            .collect()
    };
    keys.sort();
    keys.dedup();
    keys
}

/// The keys a README names for one system in the two key tables, `On macOS | On Linux | Does`
/// and `On macOS | On Linux` of the terminal, with the cell `the same` read as the macOS one.
/// The second is the kept keys: the flag says which table a key came from.
fn keys_named_for_the_system(readme: &str, macos: bool) -> Vec<(String, bool)> {
    let mut named = Vec::new();
    let mut table: Option<bool> = None;
    for line in readme.lines() {
        if !line.starts_with('|') {
            table = None;
            continue;
        }
        let cells: Vec<&str> = line.split('|').skip(1).map(str::trim).collect();
        match (cells.first(), cells.get(1)) {
            (Some(&"On macOS"), Some(&"On Linux")) => {
                table = Some(cells.get(2) == Some(&""));
            }
            _ if table.is_some() && cells.first().is_some_and(|cell| cell.starts_with("---")) => {}
            _ => {
                let Some(kept) = table else { continue };
                let on_macos = keys_of_a_cell(cells[0], kept);
                let cell = if macos { cells[0] } else { cells[1] };
                let keys = if cell == "the same" {
                    on_macos
                } else {
                    keys_of_a_cell(cell, kept)
                };
                named.extend(keys.into_iter().map(|key| (key, kept)));
            }
        }
    }
    named
}

/// What the tables of `readme` name for the system that no row of `controls` or, for the
/// table of the kept keys, of `kept` has as its key: the README's words about a key the
/// app lost.
fn named_without_a_control(
    readme: &str,
    macos: bool,
    controls: &[String],
    kept: &[String],
) -> Vec<String> {
    keys_named_for_the_system(readme, macos)
        .into_iter()
        .filter(|(key, in_the_kept_table)| {
            !(if *in_the_kept_table { kept } else { controls }).contains(key)
        })
        .map(|(key, _)| key)
        .collect()
}

#[test]
fn every_key_the_readme_tables_name_for_this_system_is_the_key_of_a_control() {
    let controls: Vec<String> = yaar_window::controls()
        .iter()
        .map(yaar_frame::Control::key)
        .collect();
    let kept: Vec<String> = yaar_window::kept_keys()
        .iter()
        .map(yaar_frame::Control::key)
        .collect();
    let macos = cfg!(target_os = "macos");
    assert!(!keys_named_for_the_system(README, macos).is_empty());
    let lost = named_without_a_control(README, macos, &controls, &kept);
    assert!(
        lost.is_empty(),
        "the README names keys that no control of the app has on this system: {}",
        lost.join(", ")
    );
}

#[test]
fn the_scan_for_keys_the_app_lost_finds_a_key_of_a_table_of_either_kind_and_of_either_column() {
    let readme = "\
Kept: `cmd-z` in a line of its own.

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-comma | Ctrl-comma | Settings | anywhere |
| Cmd-K | none; the key reaches the program | Clears | in a terminal |
| Ctrl-Tab | the same | Moves | anywhere |
| Cmd-Home, Cmd-Lost | Ctrl-Gone | Scrolls | anywhere |

| On macOS | On Linux |
| --- | --- |
| `cmd-c`, `cmd-v`: copy and paste | `ctrl-shift-c`, `ctrl-shift-v` |
| `shift-pageup`: scroll | the same |
| every Cmd key: `cmd-o` and `cmd-nowhere` run their action, send `^A` | none: Ctrl keys |
";
    let keys = |keys: &[&str]| keys.iter().map(|key| key.to_string()).collect::<Vec<_>>();
    let controls = keys(&[
        "cmd-,",
        "ctrl-,",
        "cmd-k",
        "ctrl-tab",
        "cmd-home",
        "ctrl-home",
        "cmd-o",
    ]);
    let kept = keys(&[
        "cmd-c",
        "cmd-v",
        "ctrl-shift-c",
        "ctrl-shift-v",
        "shift-pageup",
        "ctrl-tab",
        "cmd-o",
    ]);
    // On macOS: a key of the first column that no control has, in either kind of table, and
    // a backquoted key, but not `cmd-z` of a line outside the tables or `^A`.
    assert_eq!(
        named_without_a_control(readme, true, &controls, &kept),
        ["cmd-lost", "cmd-nowhere"]
    );
    // On Linux: the second column, with `the same` read as the first and `none` no key.
    assert_eq!(
        named_without_a_control(readme, false, &controls, &kept),
        ["ctrl-gone"]
    );
    // A key of the kept table is held to the kept keys, not to every control.
    let only_controls = keys(&[
        "cmd-,", "cmd-k", "ctrl-tab", "cmd-home", "cmd-o", "cmd-lost",
    ]);
    assert_eq!(
        named_without_a_control(readme, true, &only_controls, &[]),
        ["cmd-c", "cmd-v", "shift-pageup", "cmd-nowhere", "cmd-o"]
    );
    // Every key named is a control: nothing is lost.
    let all = keys(&["cmd-,", "cmd-k", "ctrl-tab", "cmd-home", "cmd-lost"]);
    let kept = keys(&["cmd-c", "cmd-v", "shift-pageup", "cmd-o", "cmd-nowhere"]);
    assert!(named_without_a_control(readme, true, &all, &kept).is_empty());
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

/// The heading of the section whose table `keys_of_the_screens_table` reads.
const SCREEN_HEADING: &str = "### Keys on the settings screen";

/// The keys the table of the settings screen names: the backquoted items of the first cell
/// of each row of the table under [`SCREEN_HEADING`], up to the next heading, as written.
fn keys_of_the_screens_table(readme: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_section = false;
    for line in readme.lines() {
        if line.starts_with('#') {
            in_section = line == SCREEN_HEADING;
        } else if in_section && line.starts_with('|') {
            let cell = line.split('|').nth(1).unwrap_or("");
            // The header and the rule of the table hold no backquote.
            keys.extend(cell.split('`').skip(1).step_by(2).map(str::to_owned));
        }
    }
    keys
}

/// The keys of the screen that the table of `readme` does not name, and the keys it names
/// that no control of the screen has, each in the order they come.
fn the_screens_table_and_its_controls_part(
    readme: &str,
    controls: &[String],
) -> (Vec<String>, Vec<String>) {
    let named = keys_of_the_screens_table(readme);
    let unnamed = controls
        .iter()
        .filter(|key| !named.contains(key))
        .cloned()
        .collect();
    let lost = named
        .iter()
        .filter(|key| !controls.contains(key))
        .cloned()
        .collect();
    (unnamed, lost)
}

fn the_screens_keys() -> Vec<String> {
    yaar_frame::settings_screen::controls()
        .iter()
        .map(yaar_frame::Control::key)
        .collect()
}

#[test]
fn the_readme_table_of_the_settings_screen_names_the_key_of_every_control_of_the_screen_and_no_other()
 {
    let controls = the_screens_keys();
    assert!(!controls.is_empty());
    assert!(!keys_of_the_screens_table(README).is_empty());
    let (unnamed, lost) = the_screens_table_and_its_controls_part(README, &controls);
    assert!(
        unnamed.is_empty(),
        "the table of the settings screen in the README does not name these keys: {}",
        unnamed.join(", ")
    );
    assert!(
        lost.is_empty(),
        "the table of the settings screen in the README names keys no control of the screen has: {}",
        lost.join(", ")
    );
}

#[test]
fn the_scan_of_the_screens_table_finds_a_key_added_to_the_table_and_a_row_taken_from_it() {
    let controls = the_screens_keys();
    let row = "| `shift-c` | Cleans up the recorded states at once |\n";
    assert!(README.contains(row), "the row this test works on is there");

    // A key the app does not have, in a row of its own and in a cell of two keys.
    let added = README.replace(row, &format!("{row}| `x` | Does what no control does |\n"));
    assert_eq!(
        the_screens_table_and_its_controls_part(&added, &controls),
        (Vec::new(), vec!["x".to_owned()])
    );
    let shared = README.replace("| `j`, `k` |", "| `j`, `k`, `y` |");
    assert_ne!(shared, README, "the cell this test works on is there");
    assert_eq!(
        the_screens_table_and_its_controls_part(&shared, &controls),
        (Vec::new(), vec!["y".to_owned()])
    );

    // A key the app has and the table lost: the row, and one key of a shared cell.
    let removed = README.replace(row, "");
    assert_eq!(
        the_screens_table_and_its_controls_part(&removed, &controls),
        (vec!["shift-c".to_owned()], Vec::new())
    );
    let one_of_two = README.replace("| `j`, `k` |", "| `j` |");
    assert_eq!(
        the_screens_table_and_its_controls_part(&one_of_two, &controls),
        (vec!["k".to_owned()], Vec::new())
    );

    // A key named in another table or in a sentence is no key of this one.
    let elsewhere = README.replace(
        "### Keys in a terminal",
        "### Keys in a terminal\n\n| Key | Does |\n| --- | --- |\n| `zz` | Elsewhere |\n",
    );
    assert_eq!(
        the_screens_table_and_its_controls_part(&elsewhere, &controls),
        (Vec::new(), Vec::new())
    );
}
