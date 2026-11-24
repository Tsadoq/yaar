//! Absence is asserted after a `barrier`: the Navigator handles one event at a time and tells
//! one list of servers for an ask that changes nothing, which follows all an earlier ask caused.

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

fn q(text: impl AsRef<str>) -> String {
    serde_json::to_string(text.as_ref()).expect("a string")
}

fn qp(path: &Path) -> String {
    q(path.display().to_string())
}

fn root_of(workspace: &WorkspaceId, repository: &RepositoryId, folder: &Path) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository.clone(),
        folder: Some(folder.to_path_buf()),
    }
}

fn consent(repository: &RepositoryId, allowed: bool) -> RepositoryConsent {
    RepositoryConsent {
        repository: repository.clone(),
        allowed,
    }
}

fn barrier(running: &mut Running, workspace: &WorkspaceId) -> Vec<ServerState> {
    running.ask(Ask::OnScreen(Some(workspace.clone())));
    match running.wait_told("the list of servers told for the ask", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    }) {
        Told::Servers(states) => states,
        _ => unreachable!("only a list of servers satisfies the wait"),
    }
}

fn assert_not_begun(state: &ServerState) {
    assert_eq!(
        (&state.server, &state.program, state.pid),
        (&None, &None, None),
        "a candidate was tried or started: {state:#?}"
    );
}

fn standin_in(list: &[ServerState], workspace: &WorkspaceId) -> Option<ServerState> {
    list.iter()
        .find(|state| state.workspace == *workspace && state.language == "standin")
        .cloned()
}

fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

fn events(path: &Path, event: &str) -> Vec<Value> {
    log_of(path)
        .into_iter()
        .filter(|line| line["event"] == event)
        .collect()
}

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

fn project_row(log: &Path) -> String {
    row_with(&format!(
        "project = [\"tools/standin-lsp\"]\n\
         args = [\"--log\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}",
        qp(log)
    ))
}

fn repository_with_its_own_server(scratch: &Scratch) -> (Repo, PathBuf) {
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let program = scratch.put_recording(&repo.root.join("tools"), "standin-lsp");
    repo.git(&["add", "tools"]);
    repo.git(&["commit", "--quiet", "-m", "the project's own server"]);
    (repo, program)
}

fn started_with(log: &Path) -> String {
    format!("--log {}", log.display())
}

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

    let list = barrier(&mut running, &ws);
    let still = standin_in(&list, &ws).unwrap();
    assert!(matches!(still.state, State::Asking { .. }), "{list:#?}");
    assert_not_begun(&still);
    assert_eq!(runs(&program), Vec::<String>::new());
    assert!(!log.exists(), "the server ran and wrote its log");

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

    running.ask(Ask::Consents(vec![
        consent(&other, true),
        consent(&mine, true),
    ]));
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    running.end();
}

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
    assert_eq!(ready.pid, meanwhile.pid);
    assert_eq!(ready.program, Some(on_the_machine));
    assert_eq!(runs(&program), Vec::<String>::new());
    running.end();
}

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

    // The yes stops the machine's server and searches again. Were that server's exit taken for
    // the exit of the one now starting, the key would pass the project's copy over.
    running.ask(Ask::Consents(vec![consent(&repository, true)]));
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program));
    assert_ne!(ready.pid, Some(old));
    assert!(!is_alive(old), "the server that came after still runs");

    let list = barrier(&mut running, &ws);
    let still = standin_in(&list, &ws).unwrap();
    assert_eq!((still.state, still.pid), (State::Ready, ready.pid));
    running.end();
}

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

fn wait_for_configuration(log: &Path) {
    let until = Instant::now() + DEADLINE;
    while events(log, "configuration").is_empty() {
        assert!(
            Instant::now() < until,
            "the server was not answered its configuration in {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
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
            "--ask-configuration".to_owned(),
            "standin".to_owned(),
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
    // The server waits for a file not made yet. It asks for its configuration after `initialized`,
    // which the reading thread answers only after handling the `initialize` answer: any Ready
    // that answer gave would be in the channel before the barrier's list.
    wait_for_configuration(&log);
    assert!(events(&log, "ready").is_empty());
    let list = barrier(&mut running, &ws);
    let still = standin_in(&list, &ws).unwrap();
    assert_eq!((still.state, still.pid), (State::Starting, starting.pid));

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
    assert_eq!(runs(&broken), ["--version"]);
    assert_eq!(events(&log, "start").len(), 1);
    running.end();
}

#[test]
fn a_candidate_that_passes_its_trial_and_fails_to_initialize_is_passed_over_at_run_time() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
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

#[test]
fn a_server_that_dies_after_it_was_ready_is_passed_over_for_the_next_candidate() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
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
    // The Navigator shows the second ready on the answer to `initialize`; the stand-in logs `ready`
    // on the `initialized` that follows.
    let until = Instant::now() + DEADLINE;
    while events(&log, "ready").len() < 2 {
        assert!(
            Instant::now() < until,
            "the second server did not log that it was ready in {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(events(&log, "ready").len(), 2);
    assert_eq!(events(&log, "exit")[0]["code"], 2);
    running.end();
}

#[test]
fn a_candidate_whose_trial_run_does_not_end_in_time_is_killed_and_passed_over() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    let noted = put_hung(&bin, "standin-hung");
    let good = scratch.put_standin(&bin, "standin-lsp");
    let log = scratch.root.join("lsp02-hung.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&machine_row(&["standin-hung", "standin-lsp"], &log)));
    running.show(&repo, &ws, &repository);

    let ready = running.wait_state(
        &ws,
        "standin",
        None,
        yaar_navigator::TRIAL_DEADLINE + DEADLINE,
        |state| state.state == State::Ready,
    );
    assert_eq!(ready.program, Some(good));
    let pid = noted_process(&noted);
    assert!(
        !is_alive(pid),
        "the trial run that took too long still runs"
    );
    running.end();
}

#[test]
fn a_trial_run_on_its_way_is_killed_and_reaped_when_the_running_navigator_is_dropped_by_a_panic() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let bin = scratch.root.join("bin");
    let noted = put_hung(&bin, "standin-hung");
    let log = scratch.root.join("dropped.log");
    let row = machine_row(&["standin-hung"], &log);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));

    let pid = std::cell::Cell::new(None);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let running = Running::start(scratch.config(&row));
        running.show(&repo, &ws, &repository);
        pid.set(Some(noted_process(&noted)));
        panic!("a test failed with the trial run on its way");
    }));
    assert!(outcome.is_err());

    let pid = pid.get().expect("the trial run was on its way");
    // SAFETY: signal 0 only asks whether the process exists; it sends nothing.
    let signalled = unsafe { libc::kill(pid as libc::pid_t, 0) };
    assert_ne!(signalled, 0, "the trial run {pid} was not reaped");
}

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

fn row_with_a_hung_trial(scratch: &Scratch) -> (String, PathBuf) {
    let noted = put_hung(&scratch.root.join("bin"), "standin-hung");
    let row = machine_row(&["standin-hung"], &scratch.root.join("hung.log"));
    (row, noted)
}

/// Writes a candidate whose trial run never ends and ignores `SIGTERM`: it ignores the signal,
/// notes its pid, then becomes `sleep` (an ignored signal stays so across `exec`). Gives the pid
/// file.
#[cfg(target_os = "linux")]
fn put_hung_ignoring_term(bin: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(bin).expect("make the folder");
    let hung = bin.join(name);
    let noted = bin.join(format!("{name}.pid"));
    std::fs::write(
        &hung,
        format!(
            "#!/bin/sh\ntrap '' TERM\necho $$ > {}\nexec sleep 600\n",
            noted.display()
        ),
    )
    .expect("write the script");
    std::fs::set_permissions(&hung, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");
    noted
}

#[cfg(target_os = "linux")]
fn row_with_a_hung_trial_ignoring_term(scratch: &Scratch) -> (String, PathBuf) {
    let noted = put_hung_ignoring_term(&scratch.root.join("bin"), "standin-hung");
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

#[cfg(target_os = "linux")]
#[test]
fn a_trial_run_ends_when_the_window_that_started_it_is_killed() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let this_test = "a_trial_run_ends_when_the_window_that_started_it_is_killed";
    if std::env::var_os("YAAR_WINDOW_CHILD").is_some() {
        let scratch = Scratch::new();
        let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
        let (row, noted) = row_with_a_hung_trial_ignoring_term(&scratch);
        let (ws, repository) = (workspace_id('1'), repository_id('1'));
        let running = Running::start(scratch.config(&row));
        running.show(&repo, &ws, &repository);
        println!("TRIAL {} {}", noted_process(&noted), scratch.root.display());
        // Waits to be killed; the end of its input, which a failed test leaves, ends it.
        let _ = std::io::stdin().read_line(&mut String::new());
        return;
    }

    let mut window = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", this_test, "--nocapture", "--test-threads=1"])
        .env("YAAR_WINDOW_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the test again as a Window of its own");
    let output = window.stdout.take().expect("the output was piped");
    let said = BufReader::new(output)
        .lines()
        .map_while(Result::ok)
        .find_map(|line| line.split_once("TRIAL ").map(|(_, said)| said.to_owned()))
        .expect("the Window said which trial run it started");
    let (pid, scratch) = said.split_once(' ').expect("a process and a folder");
    let pid: u32 = pid.parse().expect("a process number");
    assert!(is_alive(pid), "the trial run runs while its Window does");

    window.kill().expect("kill the Window");
    window.wait().expect("reap the Window");
    let end = Instant::now() + DEADLINE;
    while is_alive(pid) {
        if Instant::now() >= end {
            // SAFETY: a signal to a process of this test's own.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("the trial run outlived the Window that started it");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // The Window was killed, so its scratch was not removed.
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn a_trial_run_on_its_way_is_killed_when_its_workspace_leaves_the_list() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (row, noted) = row_with_a_hung_trial(&scratch);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |_| true);
    let pid = noted_process(&noted);
    assert!(is_alive(pid), "the trial run is on its way");

    running.ask(Ask::Workspaces(vec![]));
    running.wait_told(
        "the list with no server",
        DEADLINE,
        |told| matches!(told, Told::Servers(list) if list.is_empty()),
    );
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
    assert_eq!(env["STANDIN_TEXT"], "{root}");
    // The server's own variable wins over the configuration's.
    assert_eq!(env["HOME"], format!("{root}/home").as_str());
    assert_eq!(
        env["XDG_CACHE_HOME"],
        scratch.root.join("cache").display().to_string().as_str()
    );

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
    assert_eq!(run("{root}"), State::Ready);
}

#[test]
fn the_servers_are_told_in_the_order_of_the_workspaces_then_the_rows() {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def beta\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // Two rows in an order that is not the alphabet's, over the same files and marker.
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
    for _ in 0..2 {
        let list = running.wait_told("a list of servers", DEADLINE, |told| {
            matches!(told, Told::Servers(_))
        });
        assert_eq!(list, Told::Servers(vec![]));
    }
    let list = barrier(&mut running, &ws_deep);
    assert_eq!(list, vec![], "a server was looked for with no marker");

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
    // The readiness rule waits for a file nobody makes, so the key is starting each time the
    // workspace is shown.
    let row = row_with(&format!(
        "machine = [\"standin-lsp\"]\n\
         args = [\"--ready\", \"notification\", \"--ready-when\", {}, \
         \"--ask-configuration\", \"standin\", \"--log\", {}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"notification\", method = \"standin/status\", match = {{ ready = true }} }}",
        qp(&never),
        qp(&log)
    ));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let first = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Starting && state.pid.is_some()
    });
    wait_for_configuration(&log);
    assert!(events(&log, "ready").is_empty());
    for _ in 0..3 {
        let list = barrier(&mut running, &ws);
        let shown = standin_in(&list, &ws).unwrap();
        assert_eq!((shown.state, shown.pid), (State::Starting, first.pid));
    }
    assert_eq!(
        runs(&program),
        [
            "--version".to_owned(),
            format!(
                "--ready notification --ready-when {} --ask-configuration standin --log {}",
                never.display(),
                log.display()
            )
        ]
    );
    assert_eq!(events(&log, "start").len(), 1);
    std::fs::write(&never, "").expect("make the file the server waits for");
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.pid, first.pid);
    running.end();
}

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
            // The user's row names no title or extension: the built-in row's stand.
            // `broken` has an unknown key and is passed over.
            ("alpha", "Alpha", vec!["a", "al"]),
            ("beta", "beta", vec!["b"]),
            ("gamma", "Gamma", vec!["g"]),
        ]
    );
    running.end();
}

#[test]
fn the_cache_folders_of_windows_that_no_longer_run_are_removed_at_the_start() {
    let scratch = Scratch::new();
    let cache = scratch.root.join("navigator");
    // A process that ended and was reaped: no process has its number now.
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
    // A Window of ours that runs, names not of the form, and numbers that are no process id.
    let stays = [
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
    let first = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let first_pid = first.pid.unwrap();

    let list = barrier(&mut running, &ws2);
    assert_eq!(standin_in(&list, &ws1).unwrap().pid, Some(first_pid));
    assert_eq!(standin_in(&list, &ws2), None);

    clock.advance(STOP_AFTER - Duration::from_secs(1));
    let list = barrier(&mut running, &ws2);
    assert_eq!(standin_in(&list, &ws1).unwrap().pid, Some(first_pid));
    assert!(is_alive(first_pid));

    clock.advance(Duration::from_secs(1));
    let list = barrier(&mut running, &ws2);
    assert_eq!(list, vec![], "the idle server is still listed");
    assert!(!is_alive(first_pid), "the idle server still runs");

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

    running.ask(Ask::Workspaces(vec![root_of(&ws, &repository, &repo.root)]));
    let second = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let second_pid = second.pid.unwrap();
    assert_ne!(second_pid, first_pid);

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
