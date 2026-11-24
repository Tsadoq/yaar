//! The setter is this test binary started again: it sets the settings of the file named by
//! `YAAR_STORE_SETTER`, reports `set` on stdout and waits on stdin until killed with `SIGKILL`.

mod support;

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::Connection;
use support::Scratch;
use yaar_store::{
    FlowForm, OptionKey, RepositoryId, RepositoryRecord, Setting, Store, StoreError, TerminalPlace,
    Theme, TurnEndSelects, WorkspaceId,
};

const SETTER_VARIABLE: &str = "YAAR_STORE_SETTER";
/// Which round the setter does: `1` sets every setting, `2` sets the place of the terminal pane
/// again.
const ROUND_VARIABLE: &str = "YAAR_STORE_SETTER_ROUND";
const SETTER_TEST: &str =
    "the_setter_sets_the_settings_of_the_file_named_by_its_variable_and_does_nothing_without_it";
/// A process starting, a database opening and six calls that wait for the drive, on a loaded
/// machine.
const SET_WITHIN: Duration = Duration::from_secs(60);

fn w1() -> WorkspaceId {
    WorkspaceId::new("w-1")
}

fn w2() -> WorkspaceId {
    WorkspaceId::new("w-2")
}

fn keep_workspaces(store: &Store, workspaces: &[&str]) {
    store
        .put_repository(&RepositoryRecord {
            id: RepositoryId::new("r-1"),
            git_dir: PathBuf::from("/r/.git"),
        })
        .expect("put repository");
    for id in workspaces {
        store
            .put_workspace(
                &WorkspaceId::new(*id),
                &RepositoryId::new("r-1"),
                Path::new(&format!("/r/{id}")),
            )
            .expect("put workspace");
    }
}

fn refusal_of<T: std::fmt::Debug>(result: Result<T, StoreError>) -> String {
    match result {
        Err(StoreError::Database(sentence)) => sentence,
        other => panic!("not refused with a sentence: {other:?}"),
    }
}

#[test]
fn the_setter_sets_the_settings_of_the_file_named_by_its_variable_and_does_nothing_without_it() {
    let Some(file) = std::env::var_os(SETTER_VARIABLE) else {
        return;
    };
    let round = std::env::var(ROUND_VARIABLE).expect("the round");
    // The test harness writes `test <name> ... ` without ending the line.
    println!();
    let store = Store::open(Path::new(&file)).expect("the setter opens the store");
    match round.as_str() {
        "1" => {
            keep_workspaces(&store, &["w-1", "w-2"]);
            store.set_base(&w1(), "release").expect("set the base");
            for setting in [
                Setting::TurnEnd(TurnEndSelects::KeepSelection),
                Setting::StopAfter { seconds: 90 },
                Setting::Theme(Theme::Dark),
                Setting::TerminalPlace(TerminalPlace::Below),
                Setting::FlowForm {
                    workspace: w1(),
                    form: FlowForm::LiveDiff,
                },
            ] {
                store.set_setting(&setting).expect("set");
            }
        }
        "2" => store
            .set_setting(&Setting::TerminalPlace(TerminalPlace::Left))
            .expect("set"),
        other => panic!("no round {other}"),
    }
    println!("set");
    // A wait on the parent, which kills this process here.
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
}

/// Dropping it kills the process, so a failing test leaves none.
struct Setter {
    child: Child,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    /// Held open, so that the setter's wait on it lasts until it is killed.
    _stdin: Option<ChildStdin>,
}

impl Setter {
    fn start(file: &Path, round: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("the test binary"))
            .args([SETTER_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(SETTER_VARIABLE, file)
            .env(ROUND_VARIABLE, round)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::piped())
            .spawn()
            .expect("start the setter");
        let stdout = child.stdout.take().expect("piped output");
        let stdin = child.stdin.take();
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
            _stdin: stdin,
        }
    }

    fn wait_until_set(&self) {
        loop {
            match self.lines.recv_timeout(SET_WITHIN) {
                Ok(line) if line == "set" => return,
                Ok(_) => {}
                Err(why) => panic!("the setter never reported `set`: {why}"),
            }
        }
    }

    fn kill(&mut self) {
        // Killing a process that ended already is not a fault.
        let _ = self.child.kill();
        self.child.wait().expect("wait for the setter");
        if let Some(reader) = self.reader.take() {
            reader.join().expect("the reader ends with the pipe");
        }
    }
}

impl Drop for Setter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn set_and_kill(file: &Path, round: &str) {
    let mut setter = Setter::start(file, round);
    setter.wait_until_set();
    setter.kill();
}

#[test]
fn every_setting_is_still_set_after_the_process_that_set_it_is_killed() {
    let scratch = Scratch::new();
    set_and_kill(&scratch.file(), "1");

    let store = Store::open(&scratch.file()).expect("open after the kill");
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::TurnEnd(TurnEndSelects::KeepSelection),
            Setting::StopAfter { seconds: 90 },
            Setting::Theme(Theme::Dark),
            Setting::TerminalPlace(TerminalPlace::Below),
            Setting::FlowForm {
                workspace: w1(),
                form: FlowForm::LiveDiff,
            },
        ]
    );
    let bases: Vec<_> = store
        .workspaces()
        .expect("workspaces")
        .into_iter()
        .map(|workspace| (workspace.id, workspace.base))
        .collect();
    assert_eq!(bases, [(w1(), Some("release".to_owned())), (w2(), None)]);
    drop(store);

    set_and_kill(&scratch.file(), "2");
    let store = Store::open(&scratch.file()).expect("open after the second kill");
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::TurnEnd(TurnEndSelects::KeepSelection),
            Setting::StopAfter { seconds: 90 },
            Setting::Theme(Theme::Dark),
            Setting::TerminalPlace(TerminalPlace::Left),
            Setting::FlowForm {
                workspace: w1(),
                form: FlowForm::LiveDiff,
            },
        ]
    );
}

#[test]
fn a_store_that_nobody_set_anything_in_lists_no_setting() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    assert_eq!(store.settings().expect("settings"), []);
}

#[test]
fn every_value_of_every_setting_is_read_back_as_it_was_set() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    let every = [
        Setting::TurnEnd(TurnEndSelects::WhatChanged),
        Setting::TurnEnd(TurnEndSelects::KeepSelection),
        Setting::StopAfter { seconds: 1 },
        Setting::StopAfter { seconds: 600 },
        Setting::StopAfter { seconds: u32::MAX },
        Setting::Theme(Theme::System),
        Setting::Theme(Theme::Light),
        Setting::Theme(Theme::Dark),
        Setting::TerminalPlace(TerminalPlace::Right),
        Setting::TerminalPlace(TerminalPlace::Below),
        Setting::TerminalPlace(TerminalPlace::Left),
        Setting::FlowForm {
            workspace: w1(),
            form: FlowForm::Timeline,
        },
        Setting::FlowForm {
            workspace: w1(),
            form: FlowForm::LiveDiff,
        },
    ];
    for setting in every {
        store.set_setting(&setting).expect("set");
        let read = store.settings().expect("settings");
        assert!(
            read.contains(&setting),
            "{setting:?} was set and {read:?} is read"
        );
    }
}

#[test]
fn setting_a_setting_again_replaces_the_value_and_lists_one_entry() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    for theme in [Theme::Light, Theme::Dark, Theme::Dark] {
        store.set_setting(&Setting::Theme(theme)).expect("set");
    }
    for form in [FlowForm::LiveDiff, FlowForm::Timeline] {
        store
            .set_setting(&Setting::FlowForm {
                workspace: w1(),
                form,
            })
            .expect("set");
    }
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::Theme(Theme::Dark),
            Setting::FlowForm {
                workspace: w1(),
                form: FlowForm::Timeline,
            },
        ]
    );
}

#[test]
fn the_form_of_flow_is_kept_for_each_workspace_and_listed_in_the_order_the_workspaces_were_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-2", "w-1"]);
    for (workspace, form) in [(w1(), FlowForm::LiveDiff), (w2(), FlowForm::Timeline)] {
        store
            .set_setting(&Setting::FlowForm { workspace, form })
            .expect("set");
    }
    store
        .set_setting(&Setting::Theme(Theme::Light))
        .expect("set");
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::Theme(Theme::Light),
            Setting::FlowForm {
                workspace: w2(),
                form: FlowForm::Timeline,
            },
            Setting::FlowForm {
                workspace: w1(),
                form: FlowForm::LiveDiff,
            },
        ]
    );
}

#[test]
fn the_form_of_flow_of_a_workspace_the_store_does_not_hold_is_refused_with_its_id_and_keeps_nothing()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);

    let sentence = refusal_of(store.set_setting(&Setting::FlowForm {
        workspace: WorkspaceId::new("w-99"),
        form: FlowForm::LiveDiff,
    }));
    assert!(sentence.contains("w-99"), "{sentence}");
    assert_eq!(store.settings().expect("settings"), []);
    // Nothing is written, not even a row that `settings` hides until such a workspace is kept.
    store
        .put_workspace(
            &WorkspaceId::new("w-99"),
            &RepositoryId::new("r-1"),
            Path::new("/r/w-99"),
        )
        .expect("put workspace");
    assert_eq!(store.settings().expect("settings"), []);
    store
        .set_setting(&Setting::Theme(Theme::Dark))
        .expect("set");
    assert_eq!(
        store.settings().expect("settings"),
        [Setting::Theme(Theme::Dark)]
    );
}

#[test]
fn a_row_with_a_name_or_a_value_this_build_does_not_know_is_left_out_of_the_list() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1", "w-2"]);
    store
        .set_setting(&Setting::Theme(Theme::Dark))
        .expect("set");
    store
        .set_setting(&Setting::FlowForm {
            workspace: w2(),
            form: FlowForm::Timeline,
        })
        .expect("set");
    let known = [
        Setting::Theme(Theme::Dark),
        Setting::FlowForm {
            workspace: w2(),
            form: FlowForm::Timeline,
        },
    ];

    // Rows the Store has no call to write, as a later version or a hand leaves them. Each takes
    // the place of the row before of its name, so the list holds one unknown row at a time.
    let connection = Connection::open(scratch.file()).expect("open the file");
    for (name, value) in [
        ("turn_end", "sometimes"),
        ("stop_after", "soon"),
        ("stop_after", "-5"),
        ("stop_after", "+90"),
        ("stop_after", "4294967296"),
        ("terminal_place", "above"),
        ("colour", "red"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO setting (name, value) VALUES (?1, ?2)",
                [name, value],
            )
            .expect("write the row");
        assert_eq!(
            store.settings().expect("settings"),
            known,
            "{name} = {value} is left out"
        );
    }
    for (workspace, name, value) in [
        ("w-1", "flow_form", "sideways"),
        ("w-1", "zoom", "2"),
        ("w-gone", "flow_form", "live_diff"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO workspace_setting (workspace, name, value)
                 VALUES (?1, ?2, ?3)",
                [workspace, name, value],
            )
            .expect("write the row");
        assert_eq!(
            store.settings().expect("settings"),
            known,
            "{workspace} {name} = {value} is left out"
        );
    }
    connection
        .execute(
            "UPDATE setting SET value = 'sepia' WHERE name = 'theme'",
            [],
        )
        .expect("write the row");
    assert_eq!(
        store.settings().expect("settings"),
        [Setting::FlowForm {
            workspace: w2(),
            form: FlowForm::Timeline,
        }],
        "theme = sepia is left out"
    );
}

fn keep_repository(store: &Store, id: &str) {
    store
        .put_repository(&RepositoryRecord {
            id: RepositoryId::new(id),
            git_dir: PathBuf::from(format!("/{id}/.git")),
        })
        .expect("put repository");
}

fn answer(repository: &str, allowed: bool) -> Setting {
    Setting::ServersInRepository {
        repository: RepositoryId::new(repository),
        allowed,
    }
}

#[test]
fn the_answer_on_a_repository_is_listed_after_the_settings_of_the_workspaces_in_the_order_the_repositories_were_kept()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    // Kept r-3 before r-2, so the order they were kept in differs from the order of their ids.
    keep_repository(&store, "r-3");
    keep_repository(&store, "r-2");
    // Set in the opposite order to the repositories, other kinds after, so the list order is the
    // Store's doing.
    for setting in [
        answer("r-2", true),
        answer("r-3", false),
        answer("r-1", true),
        Setting::FlowForm {
            workspace: w1(),
            form: FlowForm::LiveDiff,
        },
        Setting::Theme(Theme::Dark),
    ] {
        store.set_setting(&setting).expect("set");
    }
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::Theme(Theme::Dark),
            Setting::FlowForm {
                workspace: w1(),
                form: FlowForm::LiveDiff,
            },
            answer("r-1", true),
            answer("r-3", false),
            answer("r-2", true),
        ]
    );
}

#[test]
fn a_second_answer_on_a_repository_takes_the_place_of_the_first_and_lists_one_entry() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    keep_repository(&store, "r-2");
    for setting in [
        answer("r-1", true),
        answer("r-2", true),
        answer("r-1", false),
        answer("r-1", false),
    ] {
        store.set_setting(&setting).expect("set");
    }
    assert_eq!(
        store.settings().expect("settings"),
        [answer("r-1", false), answer("r-2", true)]
    );
    store.set_setting(&answer("r-1", true)).expect("set");
    assert_eq!(
        store.settings().expect("settings"),
        [answer("r-1", true), answer("r-2", true)]
    );
}

#[test]
fn the_answer_on_a_repository_the_store_does_not_hold_is_refused_with_its_id_and_keeps_nothing() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);

    let sentence = refusal_of(store.set_setting(&answer("r-99", true)));
    assert!(sentence.contains("r-99"), "{sentence}");
    assert_eq!(store.settings().expect("settings"), []);
    // Nothing is written, not even a row that `settings` hides until such a repository is kept.
    keep_repository(&store, "r-99");
    assert_eq!(store.settings().expect("settings"), []);
    store.set_setting(&answer("r-1", false)).expect("set");
    assert_eq!(store.settings().expect("settings"), [answer("r-1", false)]);
}

#[test]
fn the_answer_on_a_repository_is_held_by_a_store_opened_anew() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    keep_repository(&store, "r-2");
    store.set_setting(&answer("r-1", true)).expect("set");
    store.set_setting(&answer("r-2", false)).expect("set");
    drop(store);

    let store = Store::open(&scratch.file()).expect("open again");
    assert_eq!(
        store.settings().expect("settings"),
        [answer("r-1", true), answer("r-2", false)]
    );
}

#[test]
fn a_row_of_a_repository_that_is_unknown_or_not_held_is_left_out_of_the_list() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    store.set_setting(&answer("r-1", true)).expect("set");
    let known = [answer("r-1", true)];

    // Rows the Store has no call to write, as a later version or a hand leaves them. The first
    // replaces the known row, so the list is empty; each pass sets the known row again.
    let connection = Connection::open(scratch.file()).expect("open the file");
    for (repository, name, value) in [
        ("r-1", "servers_inside", "maybe"),
        ("r-1", "servers_inside_always", "yes"),
        ("r-1", "zoom", "2"),
        ("r-gone", "servers_inside", "yes"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO repository_setting (repository, name, value)
                 VALUES (?1, ?2, ?3)",
                [repository, name, value],
            )
            .expect("write the row");
        let expected: &[Setting] = if value == "maybe" { &[] } else { &known };
        assert_eq!(
            store.settings().expect("settings"),
            expected,
            "{repository} {name} = {value} is left out"
        );
        store.set_setting(&answer("r-1", true)).expect("set again");
    }
}

#[test]
fn the_answer_on_a_repository_is_kept_as_its_name_and_a_word_in_a_table_of_its_own() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    store.set_setting(&answer("r-1", true)).expect("set");
    store.set_setting(&answer("r-1", false)).expect("set");

    let connection = Connection::open(scratch.file()).expect("open the file");
    let kept: Vec<(String, String, String)> = connection
        .prepare("SELECT repository, name, value FROM repository_setting")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("read");
    assert_eq!(
        kept,
        [(
            "r-1".to_owned(),
            "servers_inside".to_owned(),
            "no".to_owned()
        )]
    );
    let app_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM setting", [], |row| row.get(0))
        .expect("count");
    assert_eq!(app_rows, 0, "no row of the whole app's table");
}

fn app_rows(file: &Path) -> Vec<(String, String)> {
    let connection = Connection::open(file).expect("open the file");
    connection
        .prepare("SELECT name, value FROM setting ORDER BY name")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("read")
}

fn folder(repository: &str, path: Option<&str>) -> Setting {
    Setting::WorktreesFolder {
        repository: RepositoryId::new(repository),
        folder: path.map(PathBuf::from),
    }
}

#[test]
fn the_keep_states_option_key_and_theme_are_read_back_as_set_replaced_in_place_and_held_by_a_store_opened_anew()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    let kinds = [
        [
            Setting::KeepStates { days: 1 },
            Setting::KeepStates { days: 30 },
            Setting::KeepStates { days: u32::MAX },
        ]
        .as_slice(),
        &[
            Setting::OptionKey(OptionKey::Meta),
            Setting::OptionKey(OptionKey::Characters),
            Setting::OptionKey(OptionKey::ByLayout),
        ],
        &[
            Setting::Theme(Theme::Named("Solarized".to_owned())),
            Setting::Theme(Theme::Named("Nord".to_owned())),
            Setting::Theme(Theme::System),
        ],
    ];
    for values in kinds {
        for setting in values {
            store.set_setting(setting).expect("set");
            let read = store.settings().expect("settings");
            let of_kind: Vec<_> = read
                .iter()
                .filter(|read| std::mem::discriminant(*read) == std::mem::discriminant(setting))
                .collect();
            assert_eq!(of_kind, [setting], "{setting:?} was set");
        }
    }
    let last = [
        Setting::Theme(Theme::Named("Nord".to_owned())),
        Setting::KeepStates { days: 7 },
        Setting::OptionKey(OptionKey::Meta),
    ];
    for setting in &last {
        store.set_setting(setting).expect("set");
    }
    drop(store);

    let store = Store::open(&scratch.file()).expect("open again");
    assert_eq!(
        store.settings().expect("settings"),
        [
            Setting::Theme(Theme::Named("Nord".to_owned())),
            Setting::KeepStates { days: 7 },
            Setting::OptionKey(OptionKey::Meta),
        ]
    );
}

#[test]
fn a_theme_named_with_a_space_and_a_letter_outside_ascii_is_read_back_as_named() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    for name in ["Crème brûlée", "x", &"é".repeat(64), "a:b|c"] {
        let setting = Setting::Theme(Theme::Named(name.to_owned()));
        store.set_setting(&setting).expect("set");
        assert_eq!(store.settings().expect("settings"), [setting], "{name}");
    }
    assert_eq!(
        app_rows(&scratch.file()),
        [("theme".to_owned(), "named:a:b|c".to_owned())]
    );
}

#[test]
fn light_and_dark_are_kept_as_an_older_store_wrote_them_and_the_rows_it_wrote_are_read() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    // A built-in theme named explicitly is the same row as the theme itself, so an older Store
    // reads it.
    for (name, written, read) in [
        ("Light", "light", Theme::Light),
        ("Dark", "dark", Theme::Dark),
    ] {
        store
            .set_setting(&Setting::Theme(Theme::Named(name.to_owned())))
            .expect("set");
        assert_eq!(
            app_rows(&scratch.file()),
            [("theme".to_owned(), written.to_owned())]
        );
        assert_eq!(store.settings().expect("settings"), [Setting::Theme(read)]);
    }
    store
        .set_setting(&Setting::Theme(Theme::Named("light".to_owned())))
        .expect("set");
    assert_eq!(
        app_rows(&scratch.file()),
        [("theme".to_owned(), "named:light".to_owned())]
    );

    let connection = Connection::open(scratch.file()).expect("open the file");
    for (value, theme) in [("light", Theme::Light), ("dark", Theme::Dark)] {
        connection
            .execute(
                "UPDATE setting SET value = ?1 WHERE name = 'theme'",
                [value],
            )
            .expect("write the row");
        assert_eq!(store.settings().expect("settings"), [Setting::Theme(theme)]);
    }
}

#[test]
fn a_row_of_the_new_kinds_that_this_build_does_not_accept_is_left_out_of_the_list() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    store
        .set_setting(&Setting::TerminalPlace(TerminalPlace::Left))
        .expect("set");
    let known = [Setting::TerminalPlace(TerminalPlace::Left)];
    let connection = Connection::open(scratch.file()).expect("open the file");
    let too_long = format!("named:{}", "x".repeat(65));
    for (name, value) in [
        ("stop_after", "0"),
        ("stop_after", "000"),
        ("keep_states", "0"),
        ("keep_states", " 30"),
        ("keep_states", "30 "),
        ("keep_states", "+30"),
        ("keep_states", "-1"),
        ("keep_states", ""),
        ("keep_states", "4294967296"),
        ("keep_states", "soon"),
        ("option_key", "alt"),
        ("option_key", ""),
        ("theme", "named:"),
        ("theme", "named:two\nlines"),
        ("theme", too_long.as_str()),
        ("theme", "Named:Nord"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO setting (name, value) VALUES (?1, ?2)",
                [name, value],
            )
            .expect("write the row");
        assert_eq!(
            store.settings().expect("settings"),
            known,
            "{name} = {value:?} is left out"
        );
        connection
            .execute("DELETE FROM setting WHERE name = ?1", [name])
            .expect("remove the row");
    }
    for (name, value, setting) in [
        ("keep_states", "1", Setting::KeepStates { days: 1 }),
        (
            "option_key",
            "characters",
            Setting::OptionKey(OptionKey::Characters),
        ),
    ] {
        connection
            .execute(
                "INSERT INTO setting (name, value) VALUES (?1, ?2)",
                [name, value],
            )
            .expect("write the row");
        assert_eq!(
            store.settings().expect("settings"),
            [known[0].clone(), setting]
        );
        connection
            .execute("DELETE FROM setting WHERE name = ?1", [name])
            .expect("remove the row");
    }
}

#[test]
fn a_value_the_table_refuses_is_refused_with_the_setting_named_and_keeps_nothing() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    let before = [
        Setting::StopAfter { seconds: 90 },
        Setting::Theme(Theme::Named("Nord".to_owned())),
        Setting::KeepStates { days: 5 },
    ];
    for setting in &before {
        store.set_setting(setting).expect("set");
    }
    let rows = app_rows(&scratch.file());
    for (refused, named) in [
        (Setting::StopAfter { seconds: 0 }, "stop_after"),
        (Setting::KeepStates { days: 0 }, "keep_states"),
        (Setting::Theme(Theme::Named(String::new())), "theme"),
        (Setting::Theme(Theme::Named("x".repeat(65))), "theme"),
        (Setting::Theme(Theme::Named("é".repeat(65))), "theme"),
        (Setting::Theme(Theme::Named("a\tb".to_owned())), "theme"),
        (Setting::Theme(Theme::Named("a\u{85}b".to_owned())), "theme"),
    ] {
        let sentence = refusal_of(store.set_setting(&refused));
        assert!(sentence.contains(named), "{refused:?}: {sentence}");
        assert_eq!(
            app_rows(&scratch.file()),
            rows,
            "{refused:?} kept something"
        );
        assert_eq!(store.settings().expect("settings"), before);
    }
}

#[test]
fn a_folder_for_worktrees_is_read_back_replaced_by_none_and_listed_after_the_answers_of_its_repository()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &["w-1"]);
    keep_repository(&store, "r-2");
    store
        .set_setting(&Setting::TurnEnd(TurnEndSelects::KeepSelection))
        .expect("set");

    store
        .set_setting(&folder("r-1", Some("/a b/é")))
        .expect("set");
    assert_eq!(
        store.settings().expect("settings")[1..],
        [folder("r-1", Some("/a b/é"))]
    );
    store
        .set_setting(&folder("r-2", Some("/work/trees")))
        .expect("set");
    store.set_setting(&answer("r-1", true)).expect("set");
    store
        .set_setting(&Setting::FlowForm {
            workspace: w1(),
            form: FlowForm::LiveDiff,
        })
        .expect("set");
    let expected = [
        Setting::TurnEnd(TurnEndSelects::KeepSelection),
        Setting::FlowForm {
            workspace: w1(),
            form: FlowForm::LiveDiff,
        },
        answer("r-1", true),
        folder("r-1", Some("/a b/é")),
        folder("r-2", Some("/work/trees")),
    ];
    assert_eq!(store.settings().expect("settings"), expected);

    store.set_setting(&folder("r-1", None)).expect("set");
    drop(store);
    let store = Store::open(&scratch.file()).expect("open again");
    let mut expected = expected;
    expected[3] = folder("r-1", None);
    assert_eq!(store.settings().expect("settings"), expected);
    store
        .set_setting(&folder("r-1", Some("/else")))
        .expect("set");
    expected[3] = folder("r-1", Some("/else"));
    assert_eq!(store.settings().expect("settings"), expected);
}

#[test]
fn a_folder_for_worktrees_is_kept_as_its_path_in_the_table_of_the_repository() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    store
        .set_setting(&folder("r-1", Some("/a b/é")))
        .expect("set");
    store.set_setting(&folder("r-1", None)).expect("set");
    let read = |file: &Path| -> Vec<(String, String, String)> {
        Connection::open(file)
            .expect("open the file")
            .prepare("SELECT repository, name, value FROM repository_setting")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("read")
    };
    assert_eq!(
        read(&scratch.file()),
        [(
            "r-1".to_owned(),
            "worktrees_folder".to_owned(),
            String::new()
        )]
    );
    store
        .set_setting(&folder("r-1", Some("/a b/é")))
        .expect("set");
    assert_eq!(
        read(&scratch.file()),
        [(
            "r-1".to_owned(),
            "worktrees_folder".to_owned(),
            "/a b/é".to_owned()
        )]
    );
}

#[test]
fn a_folder_that_is_not_absolute_or_not_text_is_refused_and_so_is_one_for_a_repository_not_held() {
    use std::os::unix::ffi::OsStringExt;
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    store
        .set_setting(&folder("r-1", Some("/kept")))
        .expect("set");
    let kept = [folder("r-1", Some("/kept"))];

    for relative in ["rel/dir", "", ".", "~/trees", "./x"] {
        let sentence = refusal_of(store.set_setting(&folder("r-1", Some(relative))));
        assert!(
            sentence.contains("worktrees_folder"),
            "{relative:?}: {sentence}"
        );
        assert!(sentence.contains("r-1"), "{relative:?}: {sentence}");
        assert_eq!(store.settings().expect("settings"), kept, "{relative:?}");
    }
    // A path with bytes that are not text has no lossy form.
    let not_text = PathBuf::from(std::ffi::OsString::from_vec(b"/caf\xe9".to_vec()));
    let sentence = refusal_of(store.set_setting(&Setting::WorktreesFolder {
        repository: RepositoryId::new("r-1"),
        folder: Some(not_text),
    }));
    assert!(sentence.contains("worktrees_folder"), "{sentence}");
    assert_eq!(store.settings().expect("settings"), kept);

    let sentence = refusal_of(store.set_setting(&folder("r-99", Some("/x"))));
    assert!(sentence.contains("r-99"), "{sentence}");
    let sentence = refusal_of(store.set_setting(&folder("r-99", None)));
    assert!(sentence.contains("r-99"), "{sentence}");
    keep_repository(&store, "r-99");
    assert_eq!(store.settings().expect("settings"), kept);
}

#[test]
fn a_row_of_a_folder_that_is_not_absolute_or_whose_repository_is_not_held_is_left_out() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    keep_workspaces(&store, &[]);
    let connection = Connection::open(scratch.file()).expect("open the file");
    for (repository, value) in [
        ("r-1", "rel"),
        ("r-1", " /padded"),
        ("r-1", "~/x"),
        ("r-gone", "/x"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO repository_setting (repository, name, value)
                 VALUES (?1, 'worktrees_folder', ?2)",
                [repository, value],
            )
            .expect("write the row");
        assert_eq!(
            store.settings().expect("settings"),
            [],
            "{repository} {value:?} is left out"
        );
    }
    connection
        .execute(
            "UPDATE repository_setting SET value = '' WHERE repository = 'r-1'",
            [],
        )
        .expect("write the row");
    assert_eq!(store.settings().expect("settings"), [folder("r-1", None)]);
}
