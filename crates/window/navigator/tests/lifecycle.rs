//! Which servers run, and when, shown with the stand-in language server: the question before
//! a server inside a repository runs (NAV-08), a server the user registered (NAV-14), a
//! candidate that fails its trial run or its start (LSP-02), what a row says of the
//! servers, and the stop of idle servers by a clock of the test's own.
//!
//! Every wait is for a state the Navigator tells, or for a line of the stand-in's log, with
//! a deadline; no test sleeps. A clause that says nothing happened is read after a
//! barrier: the Navigator handles one event at a time, in order, and tells one list of its
//! servers for each ask that changes nothing ([`barrier`]), so that a list told for a later
//! ask comes after everything an earlier one made it do, and what a recording script or a
//! log holds then is what the earlier ask caused.

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::*;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{
    Ask, Clock, Offer, Platform, RepositoryConsent, STOP_AFTER, ServerState, State, Told,
    WorkspaceRoot,
};

/// `text` as a TOML string.
fn q(text: impl AsRef<str>) -> String {
    serde_json::to_string(text.as_ref()).expect("a string")
}

/// `path` as a TOML string.
fn qp(path: &Path) -> String {
    q(path.display().to_string())
}

/// What a workspace of a repository is, for the Navigator.
fn root_of(workspace: &WorkspaceId, repository: &RepositoryId, folder: &Path) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository.clone(),
        folder: Some(folder.to_path_buf()),
    }
}

/// The answer of a user on the servers inside a repository.
fn consent(repository: &RepositoryId, allowed: bool) -> RepositoryConsent {
    RepositoryConsent {
        repository: repository.clone(),
        allowed,
    }
}

/// Sends `OnScreen(Some(workspace))` again, which changes nothing and is told as the list of
/// the Navigator's servers, and gives that list. The Navigator tells one list for each ask
/// of this kind, and the caller read every list of the asks before it, so that this list
/// comes after whatever those asks made the Navigator do.
fn barrier(running: &mut Running, workspace: &WorkspaceId) -> Vec<ServerState> {
    running.ask(Ask::OnScreen(Some(workspace.clone())));
    match running.wait_told("the list of servers told for the ask", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    }) {
        Told::Servers(states) => states,
        _ => unreachable!("only a list of servers satisfies the wait"),
    }
}

/// Asserts that the state shows no candidate tried or started: no server named, no program,
/// no process. The Navigator sets all three when it tries or starts a candidate, before the
/// trial run has left any trace.
fn assert_not_begun(state: &ServerState) {
    assert_eq!(
        (&state.server, &state.program, state.pid),
        (&None, &None, None),
        "a candidate was tried or started: {state:#?}"
    );
}

/// The state of the stand-in's server of `workspace` in a list.
fn standin_in(list: &[ServerState], workspace: &WorkspaceId) -> Option<ServerState> {
    list.iter()
        .find(|state| state.workspace == *workspace && state.language == "standin")
        .cloned()
}

/// Every line of a stand-in's log, as the JSON it is; none when the log is not there.
fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

/// The lines of a log whose `event` is `event`.
fn events(path: &Path, event: &str) -> Vec<Value> {
    log_of(path)
        .into_iter()
        .filter(|line| line["event"] == event)
        .collect()
}

/// The registry text of a row `standin` for `.sil` files whose one server is `server`, as
/// written after its name.
fn row_with(server: &str) -> String {
    format!(
        "[standin]\n\
         extensions = {{ sil = \"standin\" }}\n\
         markers = [\"main.sil\"]\n\
         \n\
         [[standin.server]]\n\
         name = \"standin\"\n\
         {server}\n"
    )
}

/// A row whose one server is the project's own `tools/standin-lsp`, with a trial run, and
/// logs to `log`.
fn project_row(log: &Path) -> String {
    row_with(&format!(
        "project = [\"tools/standin-lsp\"]\n\
         args = [\"--log\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}",
        qp(log)
    ))
}

/// A repository holding `main.sil` and the stand-in as its own `tools/standin-lsp`, a
/// script that records each run, committed: every worktree of it holds it. Gives the
/// repository and the program's path.
fn repository_with_its_own_server(scratch: &Scratch) -> (Repo, PathBuf) {
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let program = scratch.put_recording(&repo.root.join("tools"), "standin-lsp");
    repo.git(&["add", "tools"]);
    repo.git(&["commit", "--quiet", "-m", "the project's own server"]);
    (repo, program)
}

/// The arguments the row of [`project_row`] starts the server with, as a recording script
/// writes them.
fn started_with(log: &Path) -> String {
    format!("--log {}", log.display())
}

// ---------------------------------------------------------------------------------------
// NAV-08: the question before a server inside a repository runs.
// ---------------------------------------------------------------------------------------

#[test]
fn a_server_inside_a_repository_does_not_run_before_the_user_said_yes() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-a.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    running.ask(Ask::Consents(vec![]));
    running.show(&repo, &ws, &repository);

    let asking = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. })
    });
    assert_eq!(
        asking.state,
        State::Asking {
            repository: repository.clone(),
            program: program.clone()
        }
    );
    assert_not_begun(&asking);

    // Everything the ask for the workspace made the Navigator do is done: the list this
    // ask makes comes after it. The server was neither tried nor run. A trial run is on a
    // thread of its own and may leave its line after this list, so the clause is read from
    // what the Navigator says of the key in the list: a trial shows the server's name, a
    // start its program and its process.
    let list = barrier(&mut running, &ws);
    let still = standin_in(&list, &ws).unwrap();
    assert!(matches!(still.state, State::Asking { .. }), "{list:#?}");
    assert_not_begun(&still);
    assert_eq!(runs(&program), Vec::<String>::new());
    assert!(!log.exists(), "the server ran and wrote its log");

    // Control: the recorder is the one thing that shows a run, and a yes lets it run.
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program.clone()));
    assert_eq!(ready.server.as_deref(), Some("standin"));
    assert!(!ready.registered);
    assert_eq!(
        runs(&program),
        ["--version".to_owned(), started_with(&log)],
        "the trial run, then the server"
    );
    let start = &events(&log, "start")[0];
    let cwd = PathBuf::from(start["cwd"].as_str().unwrap());
    assert_eq!(cwd.canonicalize().unwrap(), repo.root);
    running.end();
}

#[test]
fn the_answer_is_remembered_for_the_repository_in_its_other_worktrees() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-a.log");
    let (ws, ws2, repository) = (workspace_id('1'), workspace_id('2'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. })
    });
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    // A second worktree of the repository holds the committed program too.
    let second = scratch.root.join("repos").join("second");
    repo.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "second",
        second.to_str().unwrap(),
    ]);
    assert!(second.join("tools").join("standin-lsp").exists());
    let told_before = running.told.len();
    running.ask(Ask::Workspaces(vec![
        root_of(&ws, &repository, &repo.root),
        root_of(&ws2, &repository, &second),
    ]));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let ready = running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(
        ready.program,
        Some(second.join("tools").join("standin-lsp"))
    );
    for told in &running.told[told_before..] {
        if let Told::Servers(list) = told {
            assert!(
                !list
                    .iter()
                    .any(|state| state.workspace == ws2
                        && matches!(state.state, State::Asking { .. })),
                "the second worktree asked again: {list:#?}"
            );
        }
    }
    // Both ran, each after its trial run: the committed script records into one file.
    assert_eq!(
        runs(&program),
        [
            "--version",
            &started_with(&log),
            "--version",
            &started_with(&log)
        ]
    );
    let starts = events(&log, "start");
    assert_eq!(starts.len(), 2);
    let cwd = PathBuf::from(starts[1]["cwd"].as_str().unwrap());
    assert_eq!(cwd.canonicalize().unwrap(), second);
    running.end();
}

#[test]
fn a_no_passes_the_server_over_for_good() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-b.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    running.ask(Ask::Workspaces(vec![root_of(&ws, &repository, &repo.root)]));
    running.ask(Ask::OnScreen(Some(ws.clone())));
    running.ask(Ask::Consents(vec![consent(&repository, false)]));

    // The question goes, and no other candidate is left.
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == (State::Missing { offer: None })
    });
    assert!(!missing.registered);
    assert_eq!(missing.program, None);
    let list = barrier(&mut running, &ws);
    assert_eq!(
        standin_in(&list, &ws).unwrap().state,
        State::Missing { offer: None }
    );
    assert_eq!(runs(&program), Vec::<String>::new());
    assert!(!log.exists());

    // The no is kept for the repository: another worktree of it, shown later, passes the
    // server over without a question. Its copy of the program is the same script, which
    // records into the one file.
    let second = scratch.root.join("repos").join("second");
    repo.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "second",
        second.to_str().unwrap(),
    ]);
    let (ws2, told_before) = (workspace_id('2'), running.told.len());
    running.ask(Ask::Workspaces(vec![
        root_of(&ws, &repository, &repo.root),
        root_of(&ws2, &repository, &second),
    ]));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let missing = running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        state.state == (State::Missing { offer: None })
    });
    assert_not_begun(&missing);
    let list = barrier(&mut running, &ws2);
    assert_eq!(
        standin_in(&list, &ws2).unwrap().state,
        State::Missing { offer: None }
    );
    for told in &running.told[told_before..] {
        if let Told::Servers(list) = told {
            assert!(
                !list
                    .iter()
                    .any(|state| state.workspace == ws2
                        && !matches!(state.state, State::Missing { .. })),
                "the second worktree was asked about, or began a candidate: {list:#?}"
            );
        }
    }
    assert_eq!(runs(&program), Vec::<String>::new());
    assert!(!log.exists());
    running.end();
}

#[test]
fn a_no_told_before_the_workspace_is_shown_passes_the_server_over_with_no_question() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-g.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    running.ask(Ask::Consents(vec![consent(&repository, false)]));
    running.show(&repo, &ws, &repository);
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == (State::Missing { offer: None })
    });
    assert_not_begun(&missing);
    let list = barrier(&mut running, &ws);
    assert_eq!(
        standin_in(&list, &ws).unwrap().state,
        State::Missing { offer: None }
    );
    for told in &running.told {
        if let Told::Servers(list) = told {
            assert!(
                !list
                    .iter()
                    .any(|state| !matches!(state.state, State::Missing { .. })),
                "a question was shown, or a candidate begun: {list:#?}"
            );
        }
    }
    assert_eq!(runs(&program), Vec::<String>::new());
    assert!(!log.exists());
    running.end();
}

#[test]
fn a_yes_told_before_the_workspace_is_shown_lets_the_server_run_with_no_question() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-c.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    running.show(&repo, &ws, &repository);
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program));
    for told in &running.told {
        if let Told::Servers(list) = told {
            assert!(
                !list
                    .iter()
                    .any(|state| matches!(state.state, State::Asking { .. })),
                "a question was shown: {list:#?}"
            );
        }
    }
    running.end();
}

#[test]
fn a_yes_for_another_repository_does_not_let_the_server_run() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let log = scratch.root.join("run-d.log");
    let (ws, mine, other) = (workspace_id('1'), repository_id('1'), repository_id('2'));
    let mut running = Running::start(scratch.config(&project_row(&log)));
    // Told before the workspace is shown, and told after.
    running.ask(Ask::Consents(vec![consent(&other, true)]));
    running.show(&repo, &ws, &mine);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. })
    });
    let list = barrier(&mut running, &ws);
    assert!(
        matches!(standin_in(&list, &ws).unwrap().state, State::Asking { .. }),
        "{list:#?}"
    );

    running.ask(Ask::Consents(vec![
        consent(&other, true),
        consent(&repository_id('3'), true),
    ]));
    let list = barrier(&mut running, &ws);
    assert!(
        matches!(standin_in(&list, &ws).unwrap().state, State::Asking { .. }),
        "{list:#?}"
    );
    assert_eq!(runs(&program), Vec::<String>::new());

    // Control: its own repository's yes lets it run.
    running.ask(Ask::Consents(vec![
        consent(&other, true),
        consent(&mine, true),
    ]));
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    running.end();
}

/// A row whose one server has the project's own copy first and the machine's after it, both
/// the stand-in, so that the second can run while the first waits for its answer.
fn project_and_machine_row(log: &Path) -> String {
    row_with(&format!(
        "project = [\"tools/standin-lsp\"]\n\
         machine = [\"standin-lsp\"]\n\
         args = [\"--log\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}",
        qp(log)
    ))
}

#[test]
fn a_later_candidate_runs_while_the_question_waits_and_a_yes_replaces_it() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let on_the_machine = scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("run-e.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_and_machine_row(&log)));
    running.show(&repo, &ws, &repository);

    // The question takes the line, and the machine's copy runs meanwhile.
    let meanwhile = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. }) && state.pid.is_some()
    });
    assert_eq!(
        meanwhile.state,
        State::Asking {
            repository: repository.clone(),
            program: program.clone()
        }
    );
    assert_eq!(meanwhile.program, Some(on_the_machine));
    let old = meanwhile.pid.unwrap();
    assert_eq!(runs(&program), Vec::<String>::new());

    // The yes searches the key again: the copy that was passed over is the first now.
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program.as_deref() == Some(program.as_path())
    });
    assert_ne!(ready.pid, Some(old));
    assert!(!is_alive(old), "the server that came after still runs");
    running.end();
}

#[test]
fn a_no_ends_the_question_and_the_key_shows_the_server_it_runs() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let on_the_machine = scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("run-f.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&project_and_machine_row(&log)));
    running.show(&repo, &ws, &repository);
    let meanwhile = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. }) && state.pid.is_some()
    });

    running.ask(Ask::Consents(vec![consent(&repository, false)]));
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    // The server that ran goes on, and the one inside the repository never ran.
    assert_eq!(ready.pid, meanwhile.pid);
    assert_eq!(ready.program, Some(on_the_machine));
    assert_eq!(runs(&program), Vec::<String>::new());
    running.end();
}

/// [`project_and_machine_row`] with no trial run: each candidate is started at once, so that
/// the key a yes searches again is starting, not trying, when the old server's exit is
/// handled.
fn project_and_machine_row_without_a_trial(log: &Path) -> String {
    row_with(&format!(
        "project = [\"tools/standin-lsp\"]\n\
         machine = [\"standin-lsp\"]\n\
         args = [\"--log\", {}]\n\
         ready = {{ kind = \"answer\" }}",
        qp(log)
    ))
}

#[test]
fn the_exit_of_the_server_a_yes_replaced_is_not_taken_for_the_exit_of_the_one_that_follows() {
    let scratch = Scratch::new();
    let (repo, program) = repository_with_its_own_server(&scratch);
    let on_the_machine = scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("run-h.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&project_and_machine_row_without_a_trial(&log)));
    running.show(&repo, &ws, &repository);
    let meanwhile = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. }) && state.pid.is_some()
    });
    assert_eq!(meanwhile.program, Some(on_the_machine));
    let old = meanwhile.pid.unwrap();

    // The yes stops the machine's copy and starts the project's at once. The machine's copy
    // ends while the key is starting the other: that exit is of an earlier search, and
    // taken for the exit of the one starting, the key would pass the project's copy over.
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program));
    assert_ne!(ready.pid, Some(old));
    assert!(!is_alive(old), "the server that came after still runs");

    // Nothing the replaced server left behind moves the key later.
    let list = barrier(&mut running, &ws);
    let still = standin_in(&list, &ws).unwrap();
    assert_eq!((still.state, still.pid), (State::Ready, ready.pid));
    running.end();
}

// ---------------------------------------------------------------------------------------
// NAV-14: a server the user registered.
// ---------------------------------------------------------------------------------------

/// The user's file: a row `standin` for `.sil` files, a new language, whose one server
/// `mine` is `command`, ready by the stand-in's notification.
fn write_user_row(scratch: &Scratch, command: &[String]) {
    let command: Vec<String> = command.iter().map(q).collect();
    std::fs::write(
        scratch.config("").user_file,
        format!(
            "[standin]\n\
             extensions = {{ sil = \"standin\" }}\n\
             markers = [\"main.sil\"]\n\
             \n\
             [[standin.server]]\n\
             name = \"mine\"\n\
             command = [{}]\n\
             ready = {{ kind = \"notification\", method = \"standin/status\", match = {{ ready = true }} }}\n",
            command.join(", ")
        ),
    )
    .expect("write the user's file");
}

#[test]
fn a_registered_server_shows_as_starting_and_then_as_ready() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (go, log) = (scratch.root.join("go"), scratch.root.join("mine-a.log"));
    write_user_row(
        &scratch,
        &[
            standin().display().to_string(),
            "--ready".to_owned(),
            "notification".to_owned(),
            "--ready-when".to_owned(),
            go.display().to_string(),
            "--log".to_owned(),
            log.display().to_string(),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(""));
    running.show(&repo, &ws, &repository);

    let starting = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Starting && state.pid.is_some()
    });
    assert!(starting.registered);
    assert_eq!(starting.server.as_deref(), Some("mine"));
    // The server is not ready: it waits for a file the test has not made.
    assert!(events(&log, "ready").is_empty());

    std::fs::write(&go, "").expect("make the file the server waits for");
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert!(ready.registered);
    assert_eq!(ready.pid, starting.pid);
    assert_eq!(ready.program, Some(standin()));
    running.end();
}

#[test]
fn a_registered_server_whose_command_fails_shows_as_missing_with_no_offer() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let failing = scratch.put_failing(&scratch.root.join("tools"), "failing", 1);
    write_user_row(&scratch, &[failing.display().to_string()]);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(""));
    running.show(&repo, &ws, &repository);

    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == (State::Missing { offer: None })
    });
    assert!(missing.registered);
    assert_eq!(missing.pid, None);
    // It was run, and passed over.
    assert!(!runs(&failing).is_empty());
    running.end();
}

#[test]
fn a_registered_server_whose_command_names_no_file_shows_as_missing_with_no_offer() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    write_user_row(
        &scratch,
        &[scratch
            .root
            .join("tools")
            .join("nothere")
            .display()
            .to_string()],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(""));
    running.show(&repo, &ws, &repository);
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == (State::Missing { offer: None })
    });
    assert!(missing.registered);
    running.end();
}

// ---------------------------------------------------------------------------------------
// LSP-02: a candidate that fails is passed over.
// ---------------------------------------------------------------------------------------

/// A row of one server found on the machine by these names, in this order, with a trial
/// run, logging to `log`.
fn machine_row(names: &[&str], log: &Path) -> String {
    let names: Vec<String> = names.iter().map(q).collect();
    row_with(&format!(
        "machine = [{}]\n\
         args = [\"--log\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}",
        names.join(", "),
        qp(log)
    ))
}

#[test]
fn a_candidate_that_fails_its_trial_run_is_passed_over_and_the_next_is_used() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    let broken = scratch.put_failing(&bin, "standin-broken", 3);
    let good = scratch.put_standin(&bin, "standin-lsp");
    let log = scratch.root.join("lsp02.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&machine_row(&["standin-broken", "standin-lsp"], &log)));
    running.show(&repo, &ws, &repository);

    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(good));
    // Tried once, and never run as a server: the next candidate is Ready, which the
    // Navigator moved on to only after the trial of this one ended.
    assert_eq!(runs(&broken), ["--version"]);
    assert_eq!(events(&log, "start").len(), 1);
    running.end();
}

#[test]
fn a_candidate_that_passes_its_trial_and_fails_to_initialize_is_passed_over_at_run_time() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    // Passes `--version`, then answers `initialize` with an error and ends.
    let early = bin.join("standin-early");
    std::fs::write(
        &early,
        format!(
            "#!/bin/sh\nexec {} --fail-initialize \"$@\"\n",
            standin().display()
        ),
    )
    .expect("write the script");
    std::fs::set_permissions(&early, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");
    let good = scratch.put_standin(&bin, "standin-lsp");
    let log = scratch.root.join("lsp02-early.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&machine_row(&["standin-early", "standin-lsp"], &log)));
    running.show(&repo, &ws, &repository);

    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(good));
    // The first was started, and failed; the second started after it.
    let starts = events(&log, "start");
    assert_eq!(starts.len(), 2, "{starts:#?}");
    assert!(
        starts[0]["args"]
            .as_array()
            .unwrap()
            .contains(&Value::from("--fail-initialize"))
    );
    assert!(
        !starts[1]["args"]
            .as_array()
            .unwrap()
            .contains(&Value::from("--fail-initialize"))
    );
    running.end();
}

/// Waits for `answers.rs` (P10): a server that was ready and ended has its asks answered by
/// `answers::server_gone`, which is a `todo!()` until then.
#[test]
fn a_server_that_dies_after_it_was_ready_is_passed_over_for_the_next_candidate() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    // Passes `--version`, becomes ready, and ends.
    let dies = bin.join("standin-dies");
    std::fs::write(
        &dies,
        format!(
            "#!/bin/sh\nexec {} --exit-after-ready \"$@\"\n",
            standin().display()
        ),
    )
    .expect("write the script");
    std::fs::set_permissions(&dies, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");
    let good = scratch.put_standin(&bin, "standin-lsp");
    let log = scratch.root.join("lsp02-dies.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&machine_row(&["standin-dies", "standin-lsp"], &log)));
    running.show(&repo, &ws, &repository);

    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program.as_deref() == Some(good.as_path())
    });
    assert!(is_alive(ready.pid.unwrap()));
    // The first became ready, and ended; the second started after it.
    assert_eq!(events(&log, "ready").len(), 2);
    assert_eq!(events(&log, "exit")[0]["code"], 2);
    running.end();
}

#[test]
fn a_candidate_whose_trial_run_does_not_end_in_time_is_killed_and_passed_over() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    // Notes its process, then becomes `sleep`, which outlasts any deadline.
    let noted = put_hung(&bin, "standin-hung");
    let good = scratch.put_standin(&bin, "standin-lsp");
    let log = scratch.root.join("lsp02-hung.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&machine_row(&["standin-hung", "standin-lsp"], &log)));
    running.show(&repo, &ws, &repository);

    // The trial gets `yaar_navigator::TRIAL_DEADLINE` of real time.
    let ready = running.wait_state(&ws, "standin", None, SERVER_DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(good));
    let pid = noted_process(&noted);
    assert!(
        !is_alive(pid),
        "the trial run that took too long still runs"
    );
    running.end();
}

/// Writes `<bin>/<name>`, a candidate whose trial run never ends: it notes its process id in
/// a file, then becomes `sleep`, which outlasts any deadline. Gives that file.
fn put_hung(bin: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(bin).expect("make the folder");
    let hung = bin.join(name);
    let noted = bin.join(format!("{name}.pid"));
    std::fs::write(
        &hung,
        format!("#!/bin/sh\necho $$ > {}\nexec sleep 600\n", noted.display()),
    )
    .expect("write the script");
    std::fs::set_permissions(&hung, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");
    noted
}

/// The process id a script of [`put_hung`] noted, waiting for it to be there: a repeated
/// look at one fact that stays true once it is.
fn noted_process(noted: &Path) -> u32 {
    let until = Instant::now() + DEADLINE;
    loop {
        if let Some(pid) = std::fs::read_to_string(noted)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < until,
            "the script did not note its process in {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The row and the file of a scratch whose one candidate is a trial run that never ends.
fn row_with_a_hung_trial(scratch: &Scratch) -> (String, PathBuf) {
    let noted = put_hung(&scratch.root.join("bin"), "standin-hung");
    let row = machine_row(&["standin-hung"], &scratch.root.join("hung.log"));
    (row, noted)
}

#[test]
fn a_trial_run_on_its_way_is_killed_when_the_navigator_ends() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (row, noted) = row_with_a_hung_trial(&scratch);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let pid = noted_process(&noted);
    assert!(is_alive(pid), "the trial run is on its way");
    running.end();
    assert!(!is_alive(pid), "the trial run outlived the Navigator");
}

#[test]
fn a_trial_run_on_its_way_is_killed_when_its_workspace_leaves_the_list() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (row, noted) = row_with_a_hung_trial(&scratch);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    // Every list told so far is read, up to the one that holds the key: the lists the empty
    // workspaces told before it are not the one the wait below is for.
    running.wait_state(&ws, "standin", None, DEADLINE, |_| true);
    let pid = noted_process(&noted);
    assert!(is_alive(pid), "the trial run is on its way");

    running.ask(Ask::Workspaces(vec![]));
    running.wait_told(
        "the list with no server",
        DEADLINE,
        |told| matches!(told, Told::Servers(list) if list.is_empty()),
    );
    // The stop reaped it before the list was told.
    assert!(!is_alive(pid), "the trial run outlived its key");
    running.end();
}

#[test]
fn a_row_with_a_download_for_this_machine_and_no_candidate_is_missing_with_the_offer() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let platform = Platform::current().expect("a machine the built-in rows download for");
    let row = row_with(&format!(
        "machine = [\"standin-lsp\"]\n\
         trial = [\"--version\"]\n\
         {}",
        a_download(platform, "2", 4096)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);

    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert_eq!(
        missing.state,
        State::Missing {
            offer: Some(Offer {
                server: "standin".to_owned(),
                version: "2".to_owned(),
                size: 4096,
            })
        }
    );
    assert!(!missing.registered);
    assert_eq!(missing.server, None);
    running.end();
}

#[test]
fn a_row_whose_candidates_all_fail_is_missing_with_the_offer_too() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    scratch.put_failing(&scratch.root.join("bin"), "standin-lsp", 3);
    let platform = Platform::current().expect("a machine the built-in rows download for");
    let row = row_with(&format!(
        "machine = [\"standin-lsp\"]\n\
         trial = [\"--version\"]\n\
         {}",
        a_download(platform, "2", 4096)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert!(matches!(missing.state, State::Missing { offer: Some(_) }));
    running.end();
}

// ---------------------------------------------------------------------------------------
// What a row pins, and what the app downloaded.
// ---------------------------------------------------------------------------------------

/// A `[[standin.server.download]]` for `platform`, to be put at the end of a server's table.
fn a_download(platform: Platform, version: &str, size: u64) -> String {
    format!(
        "[[standin.server.download]]\n\
         platform = {}\n\
         version = {}\n\
         url = \"http://127.0.0.1:1/standin.tar.gz\"\n\
         sha256 = \"{}\"\n\
         archive = \"tar.gz\"\n\
         strip = 1\n\
         program = \"standin-lsp\"\n\
         size = {size}\n",
        q(platform.name()),
        q(version),
        "a".repeat(64),
    )
}

/// Writes what the downloader leaves for version `version` of the server `standin`: the
/// program in its folder, and `installed.toml` naming it.
fn install(scratch: &Scratch, version: &str) -> PathBuf {
    let folder = scratch.root.join("servers").join("standin");
    let program = scratch.put_standin(&folder.join(version), "standin-lsp");
    std::fs::write(
        folder.join("installed.toml"),
        format!(
            "version = {}\nsha256 = \"{}\"\nprogram = \"standin-lsp\"\n",
            q(version),
            "b".repeat(64)
        ),
    )
    .expect("write installed.toml");
    program
}

/// The state of the stand-in's server in a scratch where version 1 is installed and the row
/// pins `pinned`.
fn with_version_1_installed_and_the_row_pinning(pinned: &str) -> (ServerState, PathBuf) {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let program = install(&scratch, "1");
    let platform = Platform::current().expect("a machine the built-in rows download for");
    let row = row_with(&format!(
        "args = [\"--log\", {}]\n\
         trial = [\"--version\"]\n\
         {}",
        qp(&scratch.root.join("downloaded.log")),
        a_download(platform, pinned, 4096)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    running.end();
    (ready, program)
}

#[test]
fn a_downloaded_server_that_is_not_the_version_the_row_pins_runs_and_shows_the_update() {
    let (ready, program) = with_version_1_installed_and_the_row_pinning("2");
    assert_eq!(ready.program, Some(program));
    assert_eq!(
        ready.update,
        Some(Offer {
            server: "standin".to_owned(),
            version: "2".to_owned(),
            size: 4096,
        })
    );
}

#[test]
fn a_downloaded_server_of_the_version_the_row_pins_shows_no_update() {
    let (ready, program) = with_version_1_installed_and_the_row_pinning("1");
    assert_eq!(ready.program, Some(program));
    assert_eq!(ready.update, None);
}

// ---------------------------------------------------------------------------------------
// What a row says of the server's start.
// ---------------------------------------------------------------------------------------

#[test]
fn the_variables_of_a_row_are_put_in_where_the_server_starts_and_its_log_is_the_navigators() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("repos").join("vars.log");
    let platform = Platform::current().expect("a machine the built-in rows download for");
    let row = row_with(&format!(
        "machine = [\"standin-lsp\"]\n\
         args = [\"--log\", \"{{root}}/../vars.log\"]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}\n\
         \n\
         [standin.server.environment]\n\
         STANDIN_ROOT = \"{{root}}\"\n\
         STANDIN_WORKTREE = \"{{worktree}}\"\n\
         STANDIN_TARGET = \"{{target}}\"\n\
         STANDIN_PLATFORM = \"{{platform}}\"\n\
         STANDIN_TEXT = \"{{{{root}}}}\"\n\
         HOME = \"{{root}}/home\"\n\
         {}",
        a_download(platform, "2", 4096)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let start = events(&log, "start").remove(0);
    let root = repo.root.display().to_string();
    assert_eq!(
        start["args"],
        serde_json::json!(["--log", format!("{root}/../vars.log")])
    );
    let env = &start["env"];
    assert_eq!(env["STANDIN_ROOT"], root.as_str());
    assert_eq!(env["STANDIN_WORKTREE"], root.as_str());
    assert_eq!(
        env["STANDIN_TARGET"],
        scratch
            .root
            .join("navigator")
            .join("targets")
            .join(ws.as_str())
            .join("new")
            .display()
            .to_string()
            .as_str()
    );
    assert_eq!(env["STANDIN_PLATFORM"], platform.name());
    // `{{` and `}}` are braces.
    assert_eq!(env["STANDIN_TEXT"], "{root}");
    // The server's own variable wins over the configuration's, which names every other.
    assert_eq!(env["HOME"], format!("{root}/home").as_str());
    assert_eq!(
        env["XDG_CACHE_HOME"],
        scratch.root.join("cache").display().to_string().as_str()
    );

    // Its standard error goes to the first log of this Navigator's own cache.
    let own: Vec<PathBuf> = std::fs::read_dir(scratch.root.join("navigator"))
        .expect("the cache folder")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("logs").join("1.log").exists())
        .collect();
    assert_eq!(own.len(), 1, "{own:?}");
    running.end();
}

#[test]
fn a_candidate_of_a_row_that_names_a_folder_it_has_no_value_for_is_passed_over() {
    // `{servers}` is the folder of a downloaded copy: a copy found on the machine has none.
    let run = |variable: &str| {
        let scratch = Scratch::new();
        let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
        scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
        let row = row_with(&format!(
            "machine = [\"standin-lsp\"]\n\
             trial = [\"--version\"]\n\
             ready = {{ kind = \"answer\" }}\n\
             \n\
             [standin.server.environment]\n\
             STANDIN_X = \"{variable}/x\"\n"
        ));
        let (ws, repository) = (workspace_id('1'), repository_id('1'));
        let mut running = Running::start(scratch.config(&row));
        running.show(&repo, &ws, &repository);
        let state = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
            matches!(state.state, State::Ready | State::Missing { .. })
        });
        running.end();
        state.state
    };
    assert_eq!(run("{servers}"), State::Missing { offer: None });
    // Control: a variable that has a value.
    assert_eq!(run("{root}"), State::Ready);
}

// ---------------------------------------------------------------------------------------
// Which keys there are, and the order they are told in.
// ---------------------------------------------------------------------------------------

#[test]
fn the_servers_are_told_in_the_order_of_the_workspaces_then_the_rows() {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def beta\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // Two rows, in an order that is not the alphabet's, over the same files and marker.
    let server =
        "machine = [\"standin-lsp\"]\ntrial = [\"--version\"]\nready = { kind = \"answer\" }\n";
    let rows = format!(
        "[zeta]\nextensions = {{ zeta = \"zeta\" }}\nmarkers = [\"main.sil\"]\n\n\
         [[zeta.server]]\nname = \"standin\"\n{server}\n\
         [alpha]\nextensions = {{ alpha = \"alpha\" }}\nmarkers = [\"main.sil\"]\n\n\
         [[alpha.server]]\nname = \"standin\"\n{server}"
    );
    let (ws_a, ws_b, repository) = (workspace_id('a'), workspace_id('b'), repository_id('1'));
    let mut running = Running::start(scratch.config(&rows));
    // The list puts `b` first; `a` is shown first.
    running.ask(Ask::Workspaces(vec![
        root_of(&ws_b, &repository, &second.root),
        root_of(&ws_a, &repository, &first.root),
    ]));
    running.ask(Ask::OnScreen(Some(ws_a.clone())));
    running.ask(Ask::OnScreen(Some(ws_b.clone())));

    let told = running.wait_told("the four servers, each ready", DEADLINE, |told| {
        matches!(told, Told::Servers(list)
            if list.len() == 4 && list.iter().all(|state| state.state == State::Ready))
    });
    let Told::Servers(list) = told else {
        unreachable!()
    };
    let names: Vec<(WorkspaceId, &str)> = list
        .iter()
        .map(|state| (state.workspace.clone(), state.language.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (ws_b.clone(), "zeta"),
            (ws_b.clone(), "alpha"),
            (ws_a.clone(), "zeta"),
            (ws_a.clone(), "alpha"),
        ]
    );
    assert!(list.iter().all(|state| state.old_side.is_none()));
    assert_eq!(list[0].title, "zeta");
    running.end();
}

#[test]
fn a_row_whose_marker_is_not_at_the_worktrees_root_starts_nothing() {
    let scratch = Scratch::new();
    let deep = scratch.repository("deep", &[("lib/main.sil", "def alpha\n")]);
    let marked = scratch.repository("marked", &[("main.sil", "def alpha\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = machine_row(&["standin-lsp"], &scratch.root.join("markers.log"));
    let (ws_deep, ws_marked, repository) =
        (workspace_id('a'), workspace_id('b'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.ask(Ask::Workspaces(vec![
        root_of(&ws_deep, &repository, &deep.root),
        root_of(&ws_marked, &repository, &marked.root),
    ]));
    running.ask(Ask::OnScreen(Some(ws_deep.clone())));
    // One list for the workspaces, one for the workspace shown: both are read, so that the
    // list the barrier makes is the next.
    for _ in 0..2 {
        let list = running.wait_told("a list of servers", DEADLINE, |told| {
            matches!(told, Told::Servers(_))
        });
        assert_eq!(list, Told::Servers(vec![]));
    }
    let list = barrier(&mut running, &ws_deep);
    assert_eq!(list, vec![], "a server was looked for with no marker");

    // Control: the same row with its marker at the root.
    running.ask(Ask::OnScreen(Some(ws_marked.clone())));
    running.wait_state(&ws_marked, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    running.end();
}

#[test]
fn showing_a_workspace_again_does_not_search_again_for_a_server_that_is_starting() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let program = scratch.put_recording(&scratch.root.join("bin"), "standin-lsp");
    let (never, log) = (scratch.root.join("never"), scratch.root.join("again.log"));
    let row = row_with(&format!(
        "machine = [\"standin-lsp\"]\n\
         args = [\"--log\", {}, \"--ready-when\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}",
        qp(&log),
        qp(&never)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    // The rule is `answer`, so the server is ready at once; what is shown again is a
    // workspace whose server has been found.
    let first = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    for _ in 0..3 {
        let list = barrier(&mut running, &ws);
        assert_eq!(standin_in(&list, &ws).unwrap().pid, first.pid);
    }
    assert_eq!(
        runs(&program),
        [
            "--version".to_owned(),
            format!("--log {} --ready-when {}", log.display(), never.display())
        ]
    );
    assert_eq!(events(&log, "start").len(), 1);
    running.end();
}

// ---------------------------------------------------------------------------------------
// The registry as the Navigator tells it.
// ---------------------------------------------------------------------------------------

#[test]
fn the_languages_told_are_the_rows_read_a_row_of_the_users_in_the_place_of_the_built_in_one() {
    let scratch = Scratch::new();
    let builtin = "[alpha]\n\
         title = \"Alpha\"\n\
         extensions = { a = \"alpha\", al = \"alpha\" }\n\
         [[alpha.server]]\n\
         name = \"alpha\"\n\
         machine = [\"alpha\"]\n\
         \n\
         [beta]\n\
         extensions = { b = \"beta\" }\n\
         [[beta.server]]\n\
         name = \"beta\"\n\
         machine = [\"beta\"]\n";
    let user = "[alpha]\n\
         [[alpha.server]]\n\
         command = [\"/bin/true\"]\n\
         \n\
         [broken]\n\
         colour = \"red\"\n\
         \n\
         [gamma]\n\
         title = \"Gamma\"\n\
         extensions = { g = \"gamma\" }\n\
         [[gamma.server]]\n\
         command = [\"/bin/true\"]\n";
    std::fs::write(scratch.config("").user_file, user).expect("write the user's file");
    let running = Running::start(scratch.config(builtin));
    let Told::Languages(languages) = &running.told[0] else {
        panic!(
            "the Navigator tells its languages first: {:?}",
            running.told
        )
    };
    let seen: Vec<(&str, &str, Vec<&str>)> = languages
        .iter()
        .map(|language| {
            (
                language.name.as_str(),
                language.title.as_str(),
                language.extensions.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            // The user's row names no title and no extension: the built-in row's stand.
            ("alpha", "Alpha", vec!["a", "al"]),
            ("beta", "beta", vec!["b"]),
            // The row with a key the format does not have is passed over.
            ("gamma", "Gamma", vec!["g"]),
        ]
    );
    running.end();
}

#[test]
fn the_cache_folders_of_windows_that_no_longer_run_are_removed_at_the_start() {
    let scratch = Scratch::new();
    let cache = scratch.root.join("navigator");
    // A process that has ended and was reaped: no process has its number now.
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("start sh");
    let ended = child.id();
    child.wait().expect("wait for sh");
    let alive = std::process::id();

    let made = |name: String| {
        let folder = cache.join(name);
        std::fs::create_dir_all(folder.join("old")).expect("make the folder");
        std::fs::write(folder.join("old").join("file"), "x").expect("write a file");
        folder
    };
    let gone = [made(format!("{ended}-1")), made(format!("{ended}-12"))];
    let stays = [
        // A Window of ours that runs, a folder that is not of the form, and one whose
        // number is no process id's.
        made(format!("{alive}-99")),
        made("targets".to_owned()),
        made("notes".to_owned()),
        made(format!("{ended}-x")),
        made(format!("x-{ended}")),
        made(format!("{ended}-")),
        made("0-1".to_owned()),
        made("99999999999999999999-1".to_owned()),
    ];
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_with("machine = [\"nothere\"]")));
    // Told once the registry was read; the removal follows it, and this list after both.
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |_| true);
    for folder in &gone {
        assert!(!folder.exists(), "{} was left", folder.display());
    }
    for folder in &stays {
        assert!(folder.exists(), "{} was removed", folder.display());
    }
    running.end();
}

// ---------------------------------------------------------------------------------------
// The workspaces, and the clock.
// ---------------------------------------------------------------------------------------

/// A time that moves only when the test moves it.
struct TestClock {
    base: Instant,
    offset: Mutex<Duration>,
}

impl TestClock {
    fn new() -> Arc<TestClock> {
        Arc::new(TestClock {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
        })
    }

    fn advance(&self, by: Duration) {
        *self.offset.lock().unwrap() += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }
}

#[test]
fn a_workspace_off_screen_for_the_set_time_has_its_server_stopped_and_it_starts_again_on_screen() {
    assert_eq!(STOP_AFTER, Duration::from_secs(600));
    let scratch = Scratch::new();
    let shown = scratch.repository("shown", &[("main.sil", "def alpha\n")]);
    let other = scratch.repository("other", &[("readme.txt", "nothing\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = machine_row(&["standin-lsp"], &scratch.root.join("clock.log"));
    let (ws1, ws2, repository) = (workspace_id('1'), workspace_id('2'), repository_id('1'));
    let clock = TestClock::new();
    let mut config = scratch.config(&row);
    config.clock = clock.clone();
    assert_eq!(config.stop_after, STOP_AFTER);
    let mut running = Running::start(config);
    running.ask(Ask::Workspaces(vec![
        root_of(&ws1, &repository, &shown.root),
        root_of(&ws2, &repository, &other.root),
    ]));
    running.ask(Ask::OnScreen(Some(ws1.clone())));
    // The workspace on screen has its server, with no ask about a file.
    let first = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let first_pid = first.pid.unwrap();

    // Off screen; the workspace with no marker has nothing to run.
    let list = barrier(&mut running, &ws2);
    assert_eq!(standin_in(&list, &ws1).unwrap().pid, Some(first_pid));
    assert_eq!(standin_in(&list, &ws2), None);

    // One second short of the set time: still running.
    clock.advance(STOP_AFTER - Duration::from_secs(1));
    let list = barrier(&mut running, &ws2);
    assert_eq!(standin_in(&list, &ws1).unwrap().pid, Some(first_pid));
    assert!(is_alive(first_pid));

    // The set time: stopped, and the process is gone before the list that says so.
    clock.advance(Duration::from_secs(1));
    let list = barrier(&mut running, &ws2);
    assert_eq!(list, vec![], "the idle server is still listed");
    assert!(!is_alive(first_pid), "the idle server still runs");

    // Back on screen: it starts again.
    running.ask(Ask::OnScreen(Some(ws1.clone())));
    let again = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.pid != Some(first_pid)
    });
    assert!(is_alive(again.pid.unwrap()));
    running.end();
}

#[test]
fn a_workspace_on_screen_is_never_stopped_by_the_clock() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = machine_row(&["standin-lsp"], &scratch.root.join("on-screen.log"));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let clock = TestClock::new();
    let mut config = scratch.config(&row);
    config.clock = clock.clone();
    let mut running = Running::start(config);
    running.show(&repo, &ws, &repository);
    let first = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    clock.advance(STOP_AFTER * 3);
    let list = barrier(&mut running, &ws);
    assert_eq!(standin_in(&list, &ws).unwrap().pid, first.pid);
    assert!(is_alive(first.pid.unwrap()));
    running.end();
}

#[test]
fn a_workspace_that_leaves_the_list_has_its_server_stopped_and_its_build_folders_removed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = machine_row(&["standin-lsp"], &scratch.root.join("list.log"));
    let (ws, never_listed, repository) = (workspace_id('1'), workspace_id('2'), repository_id('1'));
    let targets = scratch.root.join("navigator").join("targets");
    let own_targets = targets.join(ws.as_str());
    let foreign_targets = targets.join(never_listed.as_str());
    for folder in [&own_targets, &foreign_targets] {
        std::fs::create_dir_all(folder.join("new")).expect("make the build folder");
        std::fs::write(folder.join("new").join("output"), "x").expect("write a build output");
    }
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let first = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let first_pid = first.pid.unwrap();

    // A worktree that is gone: its server stops, its build folder stays for the worktree
    // that comes back.
    running.ask(Ask::Workspaces(vec![WorkspaceRoot {
        id: ws.clone(),
        repository: repository.clone(),
        folder: None,
    }]));
    let told = running.wait_told(
        "the list with no server",
        DEADLINE,
        |told| matches!(told, Told::Servers(list) if list.is_empty()),
    );
    assert_eq!(told, Told::Servers(vec![]));
    assert!(!is_alive(first_pid));
    assert!(own_targets.exists());

    // The worktree is back, and the workspace is on screen: a server again.
    running.ask(Ask::Workspaces(vec![root_of(&ws, &repository, &repo.root)]));
    let second = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let second_pid = second.pid.unwrap();
    assert_ne!(second_pid, first_pid);

    // The workspace is no longer listed.
    running.ask(Ask::Workspaces(vec![]));
    running.wait_told(
        "the list with no server",
        DEADLINE,
        |told| matches!(told, Told::Servers(list) if list.is_empty()),
    );
    assert!(!is_alive(second_pid));
    assert!(
        !own_targets.exists(),
        "the build folder of a closed workspace stays"
    );
    assert!(
        foreign_targets.exists(),
        "the build folder of a workspace never listed was removed"
    );
    running.end();
}
