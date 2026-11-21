//! Component tests: the Store's calls for the user's settings, on a real database file.
//!
//! The setter is this test binary started again: [`the_setter_sets_the_settings_of_the_file_named_by_its_variable_and_does_nothing_without_it`]
//! does nothing unless the variable `YAAR_STORE_SETTER` names a database file. Once it
//! has set what its round says it reports `set` on its standard output and waits on its
//! standard input, until the test kills it with `SIGKILL`, as a Keeper is killed.

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
    FlowForm, RepositoryId, RepositoryRecord, Setting, Store, StoreError, TerminalPlace, Theme,
    TurnEndSelects, WorkspaceId,
};

/// The variable that names the database file the setter sets settings in.
const SETTER_VARIABLE: &str = "YAAR_STORE_SETTER";
/// The variable that says which round the setter does: `1` sets every setting, `2` sets
/// the place of the terminal pane again.
const ROUND_VARIABLE: &str = "YAAR_STORE_SETTER_ROUND";
/// The name of the test function the setter is: how the test binary is told to run it.
const SETTER_TEST: &str =
    "the_setter_sets_the_settings_of_the_file_named_by_its_variable_and_does_nothing_without_it";
/// How long the parent waits for the setter to report: a process starting, a database
/// opening and six calls that wait for the drive, on a loaded machine.
const SET_WITHIN: Duration = Duration::from_secs(60);

fn w1() -> WorkspaceId {
    WorkspaceId::new("w-1")
}

fn w2() -> WorkspaceId {
    WorkspaceId::new("w-2")
}

/// Keeps one repository and the workspaces named, in that order.
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

/// What the setter does: keeps two workspaces and sets what its round says, then tells
/// the test and waits to be killed.
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

/// The setter, started again from this binary. Dropping it kills the process and waits
/// for it, so that a failing test leaves none.
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

    /// Waits until the setter reports that every call it made has returned.
    fn wait_until_set(&self) {
        loop {
            match self.lines.recv_timeout(SET_WITHIN) {
                Ok(line) if line == "set" => return,
                Ok(_) => {}
                Err(why) => panic!("the setter never reported `set`: {why}"),
            }
        }
    }

    /// Kills the setter with `SIGKILL` and waits until it is gone.
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

/// Runs a setter on the file through one round, to the end of its calls, and kills it.
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
    // What a turn end selects, the stop time, the theme, the place of the terminal pane
    // and the form of Flow, in the order of the kinds; the second workspace has no form.
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
    // The base, kept with its workspace.
    let bases: Vec<_> = store
        .workspaces()
        .expect("workspaces")
        .into_iter()
        .map(|workspace| (workspace.id, workspace.base))
        .collect();
    assert_eq!(bases, [(w1(), Some("release".to_owned())), (w2(), None)]);
    drop(store);

    // A second setter changes one of them, and the new value is the one a store finds,
    // in the one place of the old.
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
        Setting::StopAfter { seconds: 0 },
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
    // Set in the opposite order to the one the workspaces were kept in.
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
    // Nothing was written for it, not even a row that `settings` hides while no such
    // workspace is kept: once a workspace of that id is kept, no form comes with it.
    store
        .put_workspace(
            &WorkspaceId::new("w-99"),
            &RepositoryId::new("r-1"),
            Path::new("/r/w-99"),
        )
        .expect("put workspace");
    assert_eq!(store.settings().expect("settings"), []);
    // The refusal leaves the store as it was: a setting is still kept after it.
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

    // What a later version, or a hand, left in the file: the Store has no call to write
    // it. Each row is written in the place of the one before of its name, so that the
    // list is read with one unknown row at a time among the known ones.
    let connection = Connection::open(scratch.file()).expect("open the file");
    for (name, value) in [
        ("turn_end", "sometimes"),
        ("stop_after", "soon"),
        ("stop_after", "-5"),
        // Parses as a number, but is not what the Store writes.
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
        // A workspace the store does not hold.
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
    // A value of the theme this build does not know takes the place of the known theme's
    // row, so the theme holds its default and only the form of Flow is listed.
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
    // Kept r-3 before r-2, so that the order they were kept in is not the order of their
    // ids: a list sorted by id would show it.
    keep_repository(&store, "r-3");
    keep_repository(&store, "r-2");
    // Set in the opposite order to the one the repositories were kept in, and the
    // settings of the other kinds after it, so that where the answers come in the list
    // is the Store's doing and not the order of the calls.
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
    // The first answer is the one that was changed, and the other is as it was.
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
    // Nothing was written for it, not even a row that `settings` hides while no such
    // repository is kept: once a repository of that id is kept, no answer comes with it.
    keep_repository(&store, "r-99");
    assert_eq!(store.settings().expect("settings"), []);
    // The refusal leaves the store as it was: an answer is still kept after it.
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

    // What a later version, or a hand, left in the file: the Store has no call to write
    // it. Each row is written beside the known one, so that the list is read with one
    // unknown row at a time among it.
    let connection = Connection::open(scratch.file()).expect("open the file");
    for (repository, name, value) in [
        ("r-1", "servers_inside", "maybe"),
        ("r-1", "servers_inside_always", "yes"),
        ("r-1", "zoom", "2"),
        // A repository the store does not hold.
        ("r-gone", "servers_inside", "yes"),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO repository_setting (repository, name, value)
                 VALUES (?1, ?2, ?3)",
                [repository, name, value],
            )
            .expect("write the row");
        // The unknown value takes the place of the known one: it is the one that is left
        // out, and the rest of the list is as it was.
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
