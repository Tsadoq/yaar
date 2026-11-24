//! Holds docs/guide.md to the app; each scan has a control half that runs it on a doctored copy.

const GUIDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/guide.md"
));

#[test]
fn the_guide_names_every_kept_key_of_this_system_in_backquotes() {
    let kept = yaar_window::kept_keys();
    assert!(!kept.is_empty());
    let missing: Vec<String> = kept
        .iter()
        .filter(|control| !GUIDE.contains(&format!("`{}`", control.key())))
        .map(|control| format!("`{}` of {}", control.key(), control.title))
        .collect();
    assert!(
        missing.is_empty(),
        "the guide does not name these kept keys: {}",
        missing.join(", ")
    );
}

/// A key is a whole item: `Cmd-Home` names no `cmd-h`.
fn keys_of_the_tables(guide: &str, macos: bool) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_table = false;
    for line in guide.lines() {
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

fn not_in_the_tables<'a>(guide: &str, macos: bool, wanted: &[&'a str]) -> Vec<&'a str> {
    let named = keys_of_the_tables(guide, macos);
    wanted
        .iter()
        .copied()
        .filter(|key| !named.contains(&key.to_string()))
        .collect()
}

#[test]
fn the_guide_names_the_key_of_every_control_of_the_app_in_the_tables_words() {
    // The tables write `Cmd-Alt-Down` and `Ctrl-Alt-Down`, and say the comma in words.
    let controls = yaar_window::controls();
    let wanted: Vec<String> = controls
        .iter()
        .filter(|control| control.keystroke.starts_with("secondary-"))
        .map(|control| control.key().replace(',', "comma"))
        .collect();
    assert!(!wanted.is_empty());
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let missing = not_in_the_tables(GUIDE, cfg!(target_os = "macos"), &wanted);
    assert!(
        missing.is_empty(),
        "the key tables of the guide do not name these keys: {}",
        missing.join(", ")
    );
}

/// In the app table the cell is a `, ` list; in the kept-keys table the keys are the backquoted
/// items.
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

fn keys_named_for_the_system(guide: &str, macos: bool) -> Vec<(String, bool)> {
    let mut named = Vec::new();
    let mut table: Option<bool> = None;
    for line in guide.lines() {
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

fn named_without_a_control(
    guide: &str,
    macos: bool,
    controls: &[String],
    kept: &[String],
) -> Vec<String> {
    keys_named_for_the_system(guide, macos)
        .into_iter()
        .filter(|(key, in_the_kept_table)| {
            !(if *in_the_kept_table { kept } else { controls }).contains(key)
        })
        .map(|(key, _)| key)
        .collect()
}

#[test]
fn every_key_the_guide_tables_name_for_this_system_is_the_key_of_a_control() {
    let controls: Vec<String> = yaar_window::controls()
        .iter()
        .map(yaar_frame::Control::key)
        .collect();
    let kept: Vec<String> = yaar_window::kept_keys()
        .iter()
        .map(yaar_frame::Control::key)
        .collect();
    let macos = cfg!(target_os = "macos");
    assert!(!keys_named_for_the_system(GUIDE, macos).is_empty());
    let lost = named_without_a_control(GUIDE, macos, &controls, &kept);
    assert!(
        lost.is_empty(),
        "the guide names keys that no control of the app has on this system: {}",
        lost.join(", ")
    );
}

#[test]
fn the_scan_for_keys_the_app_lost_finds_a_key_of_a_table_of_either_kind_and_of_either_column() {
    let guide = "\
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
    assert_eq!(
        named_without_a_control(guide, true, &controls, &kept),
        ["cmd-lost", "cmd-nowhere"]
    );
    assert_eq!(
        named_without_a_control(guide, false, &controls, &kept),
        ["ctrl-gone"]
    );
    let only_controls = keys(&[
        "cmd-,", "cmd-k", "ctrl-tab", "cmd-home", "cmd-o", "cmd-lost",
    ]);
    assert_eq!(
        named_without_a_control(guide, true, &only_controls, &[]),
        ["cmd-c", "cmd-v", "shift-pageup", "cmd-nowhere", "cmd-o"]
    );
    let all = keys(&["cmd-,", "cmd-k", "ctrl-tab", "cmd-home", "cmd-lost"]);
    let kept = keys(&["cmd-c", "cmd-v", "shift-pageup", "cmd-o", "cmd-nowhere"]);
    assert!(named_without_a_control(guide, true, &all, &kept).is_empty());
}

#[test]
fn the_scan_of_the_key_tables_finds_a_missing_row_a_prefix_and_a_key_named_elsewhere() {
    let guide = "\
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
    assert!(not_in_the_tables(guide, true, &["cmd-home", "cmd-end", "cmd-comma"]).is_empty());
    assert!(not_in_the_tables(guide, true, &["cmd-alt-down", "cmd-alt-up"]).is_empty());
    assert_eq!(not_in_the_tables(guide, true, &["cmd-h"]), ["cmd-h"]);
    assert_eq!(
        not_in_the_tables(guide, true, &["cmd-alt-d"]),
        ["cmd-alt-d"]
    );
    assert_eq!(
        not_in_the_tables(
            guide,
            false,
            &["ctrl-home", "ctrl-comma", "ctrl-alt-down", "ctrl-h"]
        ),
        ["ctrl-alt-down", "ctrl-h"]
    );
    assert_eq!(
        not_in_the_tables(guide, true, &["ctrl-home"]),
        ["ctrl-home"]
    );
    let without = guide.replace("| Cmd-comma | Ctrl-comma | Settings | anywhere |\n", "");
    assert_eq!(
        not_in_the_tables(&without, true, &["cmd-comma"]),
        ["cmd-comma"]
    );
}

#[test]
fn the_guide_says_that_on_linux_only_ctrl_shift_c_and_ctrl_shift_v_are_kept_in_a_terminal_besides_the_old_ones()
 {
    // Hard-wrapped lines are one line to a reader.
    let guide = GUIDE.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        guide.contains(
            "`shift-pagedown`, the only keys kept in a terminal on Linux are `ctrl-shift-c` and `ctrl-shift-v`"
        ),
        "the guide does not say which keys are kept in a terminal on Linux"
    );
}

const SCREEN_HEADING: &str = "### Keys on the settings screen";

fn keys_of_the_screens_table(guide: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_section = false;
    for line in guide.lines() {
        if line.starts_with('#') {
            in_section = line == SCREEN_HEADING;
        } else if in_section && line.starts_with('|') {
            let cell = line.split('|').nth(1).unwrap_or("");
            keys.extend(cell.split('`').skip(1).step_by(2).map(str::to_owned));
        }
    }
    keys
}

fn the_screens_table_and_its_controls_part(
    guide: &str,
    controls: &[String],
) -> (Vec<String>, Vec<String>) {
    let named = keys_of_the_screens_table(guide);
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
fn the_guide_table_of_the_settings_screen_names_the_key_of_every_control_of_the_screen_and_no_other()
 {
    let controls = the_screens_keys();
    assert!(!controls.is_empty());
    assert!(!keys_of_the_screens_table(GUIDE).is_empty());
    let (unnamed, lost) = the_screens_table_and_its_controls_part(GUIDE, &controls);
    assert!(
        unnamed.is_empty(),
        "the table of the settings screen in the guide does not name these keys: {}",
        unnamed.join(", ")
    );
    assert!(
        lost.is_empty(),
        "the table of the settings screen in the guide names keys no control of the screen has: {}",
        lost.join(", ")
    );
}

#[test]
fn the_scan_of_the_screens_table_finds_a_key_added_to_the_table_and_a_row_taken_from_it() {
    let controls = the_screens_keys();
    let row = "| `shift-c` | Cleans up the recorded states at once |\n";
    assert!(GUIDE.contains(row), "the row this test works on is there");

    let added = GUIDE.replace(row, &format!("{row}| `x` | Does what no control does |\n"));
    assert_eq!(
        the_screens_table_and_its_controls_part(&added, &controls),
        (Vec::new(), vec!["x".to_owned()])
    );
    let shared = GUIDE.replace("| `j`, `k` |", "| `j`, `k`, `y` |");
    assert_ne!(shared, GUIDE, "the cell this test works on is there");
    assert_eq!(
        the_screens_table_and_its_controls_part(&shared, &controls),
        (Vec::new(), vec!["y".to_owned()])
    );

    let removed = GUIDE.replace(row, "");
    assert_eq!(
        the_screens_table_and_its_controls_part(&removed, &controls),
        (vec!["shift-c".to_owned()], Vec::new())
    );
    let one_of_two = GUIDE.replace("| `j`, `k` |", "| `j` |");
    assert_eq!(
        the_screens_table_and_its_controls_part(&one_of_two, &controls),
        (vec!["k".to_owned()], Vec::new())
    );

    let elsewhere = GUIDE.replace(
        "### Keys in a terminal",
        "### Keys in a terminal\n\n| Key | Does |\n| --- | --- |\n| `zz` | Elsewhere |\n",
    );
    assert_eq!(
        the_screens_table_and_its_controls_part(&elsewhere, &controls),
        (Vec::new(), Vec::new())
    );
}
