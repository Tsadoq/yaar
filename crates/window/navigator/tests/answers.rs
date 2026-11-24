//! Answers to a name in a diff, with the stand-in language server. Waits are for what the
//! Navigator tells or a stand-in log line, with a deadline. The stand-in handles requests in
//! order, so that nothing was asked of it is read from its log after a later request.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::*;
use yaar_navigator::lsp::file_uri;
use yaar_navigator::{
    Answered, Ask, AskId, FileKind, Location, RepositoryConsent, State, Told, Tree,
};

fn q(text: impl AsRef<str>) -> String {
    serde_json::to_string(text.as_ref()).expect("a string")
}

fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

fn requests(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| line["event"] == "request" && line["method"] == method)
        .count()
}

fn events(log: &Path, event: &str) -> Vec<Value> {
    log_of(log)
        .into_iter()
        .filter(|line| line["event"] == event)
        .collect()
}

fn notifications(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| line["event"] == "notification" && line["method"] == method)
        .count()
}

fn row_ready_when(go: &Path, log: &Path) -> String {
    standin_row(
        "standin",
        &[
            "--ready",
            "notification",
            "--ready-when",
            &go.display().to_string(),
            "--log",
            &log.display().to_string(),
        ],
        "notification",
        "",
    )
}

fn row_logging(log: &Path) -> String {
    standin_row(
        "standin",
        &["--log", &log.display().to_string()],
        "answer",
        "",
    )
}

fn wait_for<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let until = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = look() {
            return found;
        }
        assert!(Instant::now() < until, "waited {DEADLINE:?} for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn at(path: &Path, line: u32, column: u32, text: &str) -> Location {
    Location {
        path: path.to_path_buf(),
        line,
        column,
        text: text.to_owned(),
    }
}

#[test]
fn a_hover_a_definition_and_references_asked_before_the_server_is_ready_are_answered_by_text_match_and_marked()
 {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("lib.sil", "def total\n"),
            ("main.sil", "def main\nuse total\n"),
        ],
    );
    let (go, log) = (scratch.root.join("go"), scratch.root.join("standin.log"));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_ready_when(&go, &log)));
    running.show(&repo, &ws, &repository);
    let held = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Starting && state.pid.is_some()
    });
    assert!(held.pid.is_some_and(is_alive), "{held:#?}");

    let main = repo.root.join("main.sil");
    let lib = repo.root.join("lib.sil");
    let at_total = || place(&repo, &ws, Tree::Worktree, "main.sil", 2, "total", 0);

    let hover = running.hover(at_total(), DEADLINE);
    assert!(hover.guessed, "{hover:#?}");
    assert_eq!(hover.value.as_deref(), Some("lib.sil:1: def total"));

    let definition = running.definition(at_total(), DEADLINE);
    assert!(definition.guessed, "{definition:#?}");
    assert_eq!(
        definition.value,
        [at(&main, 2, 4, "use total"), at(&lib, 1, 4, "def total")]
    );
    let references = running.references(at_total(), DEADLINE);
    assert!(references.guessed, "{references:#?}");
    assert_eq!(references.value, definition.value);

    std::fs::write(&go, "").expect("let the stand-in be ready");
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let hover = running.hover(at_total(), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: total".to_owned()),
            guessed: false
        }
    );
    let definition = running.definition(at_total(), DEADLINE);
    assert_eq!(
        definition,
        Answered {
            value: vec![at(&lib, 1, 4, "def total")],
            guessed: false
        }
    );
    let references = running.references(at_total(), DEADLINE);
    assert_eq!(
        references,
        Answered {
            value: vec![at(&lib, 1, 4, "def total"), at(&main, 2, 4, "use total")],
            guessed: false
        }
    );
    for method in [
        "textDocument/hover",
        "textDocument/definition",
        "textDocument/references",
    ] {
        assert_eq!(
            requests(&log, method),
            1,
            "{method} was asked of the server before it was ready"
        );
    }
    running.end();
}

#[test]
fn on_a_past_change_an_unchanged_line_is_answered_by_the_server_and_a_changed_line_by_text_match() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let past = "use alpha\nuse beta\n";
    let repo = scratch.repository(
        "project",
        &[("main.sil", past), ("other.sil", "def beta\n")],
    );
    std::fs::write(
        repo.root.join("main.sil"),
        "use gamma\nuse alpha\nuse delta\n",
    )
    .expect("change the file");
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let tree = || Tree::Past {
        version: repo.base.clone(),
        text: Arc::from(past),
    };
    // Line 1 of the past text is line 2 of the file now.
    let unchanged = running.hover(
        place(&repo, &ws, tree(), "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(
        unchanged,
        Answered {
            value: Some("standin: alpha".to_owned()),
            guessed: false
        }
    );
    let changed = running.hover(
        place(&repo, &ws, tree(), "main.sil", 2, "beta", 0),
        DEADLINE,
    );
    assert_eq!(
        changed,
        Answered {
            value: Some("other.sil:1: def beta".to_owned()),
            guessed: true
        }
    );
    let definition = running.definition(
        place(&repo, &ws, tree(), "main.sil", 2, "beta", 0),
        DEADLINE,
    );
    assert!(definition.guessed, "{definition:#?}");
    let again = running.hover(
        place(&repo, &ws, tree(), "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert!(!again.guessed, "{again:#?}");
    assert_eq!(requests(&log, "textDocument/hover"), 2);
    assert_eq!(requests(&log, "textDocument/definition"), 0);
    running.end();
}

#[test]
fn on_a_past_change_a_file_gone_from_disk_is_answered_by_text_match() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let past = "use alpha\n";
    let repo = scratch.repository(
        "project",
        &[("main.sil", "def main\n"), ("other.sil", "def alpha\n")],
    );
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let tree = Tree::Past {
        version: repo.base.clone(),
        text: Arc::from(past),
    };
    let hover = running.hover(place(&repo, &ws, tree, "gone.sil", 1, "alpha", 0), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("other.sil:1: def alpha".to_owned()),
            guessed: true
        }
    );
    running.hover(
        place(&repo, &ws, Tree::Worktree, "other.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(requests(&log, "textDocument/hover"), 1);
    running.end();
}

#[test]
fn a_ready_later_candidate_answers_while_the_question_waits() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\nuse alpha\n")]);
    let log = scratch.root.join("standin.log");
    scratch.put_recording(&repo.root.join("tools"), "standin-lsp");
    repo.git(&["add", "tools"]);
    repo.git(&["commit", "--quiet", "-m", "the project's own server"]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = standin_row(
        "standin",
        &["--log", &log.display().to_string()],
        "answer",
        &format!("project = [{}]", q("tools/standin-lsp")),
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.ask(Ask::Consents(vec![]));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. })
    });

    let answer = wait_for("the machine's copy to answer", || {
        let answer = running.hover(
            place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
            DEADLINE,
        );
        (!answer.guessed).then_some(answer)
    });
    assert_eq!(answer.value.as_deref(), Some("standin: alpha"));

    let last = running
        .told
        .iter()
        .rev()
        .find_map(|told| match told {
            Told::Servers(states) => Some(states),
            _ => None,
        })
        .expect("a list of servers was told");
    assert!(
        last.iter()
            .any(|state| matches!(state.state, State::Asking { .. })),
        "{last:#?}"
    );
    running.end();
}

fn signal(pid: u32, signal: libc::c_int) {
    // SAFETY: kill sends a signal and touches nothing of ours; the process is a child of the
    // Navigator of this test, which holds it until the test ends.
    let sent = unsafe { libc::kill(pid as libc::pid_t, signal) };
    assert_eq!(sent, 0, "signal {signal} to {pid}");
}

fn answered_id(told: &Told) -> Option<AskId> {
    match told {
        Told::Hover { id, .. } | Told::Definition { id, .. } | Told::References { id, .. } => {
            Some(*id)
        }
        _ => None,
    }
}

fn answers_to(running: &mut Running, ids: &[AskId]) -> Vec<Told> {
    for id in ids {
        if !running
            .told
            .iter()
            .any(|told| answered_id(told) == Some(*id))
        {
            running.wait_told("the answer to an ask", DEADLINE, |told| {
                answered_id(told) == Some(*id)
            });
        }
    }
    ids.iter()
        .map(|id| {
            running
                .told
                .iter()
                .find(|told| answered_id(told) == Some(*id))
                .expect("the answer was waited for")
                .clone()
        })
        .collect()
}

#[test]
fn requests_in_flight_at_once_to_one_server_are_each_answered_with_their_own_name() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];
    let main: String = names.iter().map(|name| format!("use {name}\n")).collect();
    let repo = scratch.repository(
        "project",
        &[("main.sil", main.as_str()), ("notes.txt", "milk\nmilk\n")],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let pid = ready.pid.expect("a ready server runs");

    // Stopped, so that every request is on its way when the first answer comes.
    signal(pid, libc::SIGSTOP);
    let mut ids = Vec::new();
    for (at, name) in names.iter().enumerate() {
        let id = AskId(100 + at as u64);
        let place = place(
            &repo,
            &ws,
            Tree::Worktree,
            "main.sil",
            at as u32 + 1,
            name,
            0,
        );
        running.ask(Ask::Hover { id, place });
        ids.push(id);
    }
    let references = AskId(200);
    running.ask(Ask::References {
        id: references,
        place: place(&repo, &ws, Tree::Worktree, "main.sil", 6, "zeta", 0),
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    signal(pid, libc::SIGCONT);

    for (told, name) in answers_to(&mut running, &ids).into_iter().zip(names) {
        let Told::Hover { answer, .. } = told else {
            panic!("a hover was answered by {told:?}");
        };
        assert_eq!(
            answer,
            Answered {
                value: Some(format!("standin: {name}")),
                guessed: false
            }
        );
    }
    let told = answers_to(&mut running, &[references]).remove(0);
    let Told::References { answer, .. } = told else {
        panic!("the references were answered by {told:?}");
    };
    assert_eq!(
        answer,
        Answered {
            value: vec![at(&repo.root.join("main.sil"), 6, 4, "use zeta")],
            guessed: false
        }
    );
    running.end();
}

#[test]
fn an_ask_left_waiting_when_its_server_was_stopped_is_answered_by_text_match_and_never_takes_the_answer_of_a_later_one()
 {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository("project", &[("main.sil", "def alpha\nuse alpha\n")]);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    let first = running
        .wait_state(&ws, "standin", None, DEADLINE, |state| {
            state.state == State::Ready
        })
        .pid
        .expect("a ready server runs");
    let at_alpha = || place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0);

    signal(first, libc::SIGSTOP);
    running.ask(Ask::Hover {
        id: AskId(1),
        place: at_alpha(),
    });
    running.ask(Ask::Workspaces(vec![]));
    running.wait_told("the servers of the workspace gone", DEADLINE, |told| {
        matches!(told, Told::Servers(states) if states.iter().all(|state| state.workspace != ws))
    });
    assert!(!is_alive(first), "the stopped server was not ended");

    running.show(&repo, &ws, &repository);
    let second = running
        .wait_state(&ws, "standin", None, DEADLINE, |state| {
            state.state == State::Ready && state.pid != Some(first)
        })
        .pid
        .expect("a ready server runs");
    assert_ne!(first, second);
    running.ask(Ask::Hover {
        id: AskId(2),
        place: at_alpha(),
    });

    let told = answers_to(&mut running, &[AskId(1), AskId(2)]);
    let Told::Hover { answer: first, .. } = &told[0] else {
        panic!("a hover was answered by {:?}", told[0]);
    };
    assert!(
        first.guessed,
        "the ask of the server that ended was answered by it: {first:#?}"
    );
    assert_eq!(first.value.as_deref(), Some("main.sil:1: def alpha"));
    let Told::Hover { answer: second, .. } = &told[1] else {
        panic!("a hover was answered by {:?}", told[1]);
    };
    assert_eq!(
        *second,
        Answered {
            value: Some("standin: alpha".to_owned()),
            guessed: false
        }
    );
    running.end();
}

#[test]
fn an_ask_waiting_at_a_server_the_navigator_stops_is_answered_at_once_by_text_match_and_marked() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse alpha\n"),
            ("notes.txt", "milk\n"),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    let pid = running
        .wait_state(&ws, "standin", None, DEADLINE, |state| {
            state.state == State::Ready
        })
        .pid
        .expect("a ready server runs");

    signal(pid, libc::SIGSTOP);
    running.ask(Ask::Hover {
        id: AskId(100),
        place: place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert!(
        !running
            .told
            .iter()
            .any(|told| answered_id(told) == Some(AskId(100))),
        "the stopped server answered: {:#?}",
        running.told
    );

    running.ask(Ask::Workspaces(vec![]));
    let told = answers_to(&mut running, &[AskId(100)]).remove(0);
    let Told::Hover { answer, .. } = told else {
        panic!("a hover was answered by {told:?}");
    };
    assert_eq!(
        answer,
        Answered {
            value: Some("main.sil:1: def alpha".to_owned()),
            guessed: true
        }
    );
    assert!(!is_alive(pid), "the stopped server was not ended");
    running.end();
}

#[test]
fn an_ask_waiting_at_a_stopped_server_when_the_port_is_dropped_leaves_the_navigator_ended_and_the_server_gone()
 {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse alpha\n"),
            ("notes.txt", "milk\n"),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    let pid = running
        .wait_state(&ws, "standin", None, DEADLINE, |state| {
            state.state == State::Ready
        })
        .pid
        .expect("a ready server runs");

    signal(pid, libc::SIGSTOP);
    running.ask(Ask::Hover {
        id: AskId(100),
        place: place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert!(
        !running
            .told
            .iter()
            .any(|told| answered_id(told) == Some(AskId(100))),
        "the stopped server answered: {:#?}",
        running.told
    );

    let told = running.end_and_read();
    assert!(
        !told
            .iter()
            .any(|told| answered_id(told) == Some(AskId(100))),
        "the ask was answered at the end: {told:#?}"
    );
    assert!(!is_alive(pid), "the stopped server was not ended");
}

#[test]
fn an_ask_waiting_at_a_server_that_a_consent_makes_the_navigator_search_for_again_is_answered_at_once()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse alpha\n"),
            ("notes.txt", "milk\n"),
        ],
    );
    scratch.put_recording(&repo.root.join("tools"), "standin-lsp");
    repo.git(&["add", "tools"]);
    repo.git(&["commit", "--quiet", "-m", "the project's own server"]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let row = standin_row(
        "standin",
        &[
            "--log",
            &scratch.root.join("standin.log").display().to_string(),
        ],
        "answer",
        &format!("project = [{}]", q("tools/standin-lsp")),
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.ask(Ask::Consents(vec![]));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Asking { .. })
    });
    let hover = |running: &mut Running| {
        running.hover(
            place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
            DEADLINE,
        )
    };
    let ready = wait_for("the machine's copy to answer", || {
        let answer = hover(&mut running);
        (!answer.guessed).then_some(answer)
    });
    assert_eq!(ready.value.as_deref(), Some("standin: alpha"));
    let pid = running
        .told
        .iter()
        .rev()
        .find_map(|told| match told {
            Told::Servers(states) => states.iter().find_map(|state| state.pid),
            _ => None,
        })
        .expect("the machine's copy runs");

    signal(pid, libc::SIGSTOP);
    running.ask(Ask::Hover {
        id: AskId(100),
        place: place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert!(
        !running
            .told
            .iter()
            .any(|told| answered_id(told) == Some(AskId(100))),
        "the stopped server answered: {:#?}",
        running.told
    );

    running.ask(Ask::Consents(vec![RepositoryConsent {
        repository,
        allowed: true,
    }]));
    let told = answers_to(&mut running, &[AskId(100)]).remove(0);
    let Told::Hover { answer, .. } = told else {
        panic!("a hover was answered by {told:?}");
    };
    assert_eq!(
        answer,
        Answered {
            value: Some("main.sil:1: def alpha".to_owned()),
            guessed: true
        }
    );
    assert!(!is_alive(pid), "the stopped server was not ended");
    running.end();
}

#[test]
fn a_file_outside_the_worktree_is_asked_about_by_its_absolute_path() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository("project", &[("main.sil", "def main\n")]);
    let outside = scratch.root.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("make the folder");
    let file = outside.join("dependency.sil");
    std::fs::write(&file, "def helper\nuse helper\n").expect("write the file");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let path = file.to_str().expect("a UTF-8 path");
    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, path, 2, "helper", 0),
        DEADLINE,
    );
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: helper".to_owned()),
            guessed: false
        }
    );
    running.end();
}

#[test]
fn a_file_of_no_rows_extension_is_answered_by_text_match() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("standin.log");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def main\n"),
            ("notes.txt", "remember the milk\n"),
            ("more.txt", "the milk is in the fridge\n"),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert_eq!(
        hover,
        Answered {
            value: Some("more.txt:1: the milk is in the fridge".to_owned()),
            guessed: true
        }
    );
    running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "main", 0),
        DEADLINE,
    );
    assert_eq!(requests(&log, "textDocument/hover"), 1);
    running.end();
}

#[test]
fn a_hover_by_text_match_shows_a_line_once_when_the_line_holds_the_name_twice() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def main\n"),
            ("notes.txt", "remember the milk\n"),
            ("more.txt", "milk first, milk second\nno more milk\n"),
            ("other.txt", "milk\n"),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running =
        Running::start(scratch.config(&row_logging(&scratch.root.join("standin.log"))));
    running.show(&repo, &ws, &repository);

    // A line counts once, not a match.
    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert_eq!(
        hover,
        Answered {
            value: Some(
                "more.txt:1: milk first, milk second\nmore.txt:2: no more milk\nother.txt:1: milk"
                    .to_owned()
            ),
            guessed: true
        }
    );
    running.end();
}

#[test]
fn a_hover_on_a_file_of_no_row_in_a_workspace_with_no_file_of_a_row_starts_no_server() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("notes.txt", "remember the milk\n"),
            ("more.txt", "the milk is in the fridge\n"),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);

    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert_eq!(
        hover,
        Answered {
            value: Some("more.txt:1: the milk is in the fridge".to_owned()),
            guessed: true
        }
    );
    running.ask(Ask::OnScreen(Some(ws.clone())));
    running.wait_told("the list of servers told for the ask", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    });
    let keys_of_the_row: Vec<_> = running
        .told
        .iter()
        .filter_map(|told| match told {
            Told::Servers(states) => Some(states),
            _ => None,
        })
        .flatten()
        .filter(|state| state.language == "standin")
        .collect();
    assert!(keys_of_the_row.is_empty(), "{keys_of_the_row:#?}");
    running.end();
}

#[test]
fn the_files_that_changed_are_told_to_a_ready_server_once_and_a_gone_one_does_no_harm() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let log = scratch.root.join("standin.log");
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    running.ask(Ask::FilesChanged {
        workspace: ws.clone(),
        paths: vec![
            PathBuf::from("main.sil"),
            PathBuf::from("main.sil"),
            PathBuf::from("gone.sil"),
        ],
    });
    wait_for("the notice to reach the server", || {
        (notifications(&log, "workspace/didChangeWatchedFiles") > 0).then_some(())
    });
    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(hover.value.as_deref(), Some("standin: alpha"));
    running.ask(Ask::FilesChanged {
        workspace: ws.clone(),
        paths: Vec::new(),
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(notifications(&log, "workspace/didChangeWatchedFiles"), 1);
    running.end();
}

#[test]
fn the_files_that_changed_are_told_to_a_server_that_is_starting_and_reach_it_once_it_is_released() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let (go, log) = (scratch.root.join("go"), scratch.root.join("standin.log"));
    let repo = scratch.repository(
        "project",
        &[("main.sil", "def alpha\n"), ("notes.txt", "milk\n")],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_ready_when(&go, &log)));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Starting && state.pid.is_some()
    });

    running.ask(Ask::FilesChanged {
        workspace: ws.clone(),
        paths: vec![PathBuf::from("main.sil")],
    });
    running.hover(
        place(&repo, &ws, Tree::Worktree, "notes.txt", 1, "milk", 0),
        DEADLINE,
    );
    assert!(
        events(&log, "ready").is_empty(),
        "the server was ready before it was released"
    );

    std::fs::write(&go, "").expect("make the file the server waits for");
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let changes = wait_for("the notice to reach the server", || {
        let noticed: Vec<Value> = log_of(&log)
            .into_iter()
            .filter(|line| {
                line["event"] == "notification"
                    && line["method"] == "workspace/didChangeWatchedFiles"
            })
            .map(|line| line["changes"].clone())
            .collect();
        (!noticed.is_empty()).then_some(noticed)
    });
    assert_eq!(
        changes,
        [serde_json::json!([
            {"uri": file_uri(&repo.root.join("main.sil")), "type": 2}
        ])]
    );
    running.end();
}

#[test]
fn a_file_opened_through_an_answer_is_told_with_what_it_is() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def main\n"),
            (".gitignore", "vendor/\n"),
            ("src/inner.sil", "def inner\n"),
        ],
    );
    std::fs::create_dir_all(repo.root.join("vendor")).expect("make the folder");
    std::fs::write(repo.root.join("vendor/ignored.sil"), "def ignored\n").expect("write");
    let outside = scratch.root.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("make the folder");
    std::fs::write(outside.join("far.sil"), "def far\n").expect("write");
    std::fs::write(outside.join("binary.bin"), [0xff_u8, 0xfe, 0x00]).expect("write");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);

    let mut open = |path: PathBuf| {
        let id = AskId(900 + running.told.len() as u64);
        running.ask(Ask::Open {
            id,
            location: at(&path, 1, 0, ""),
        });
        match running.wait_told(
            "the answer to an open",
            DEADLINE,
            |told| matches!(told, Told::Opened { id: got, .. } if *got == id),
        ) {
            Told::Opened { file, .. } => file,
            _ => unreachable!("only the answer to the ask satisfies the wait"),
        }
    };

    let file = open(repo.root.join("src/inner.sil")).expect("a file of the worktree");
    assert_eq!(file.text, "def inner\n");
    assert_eq!(file.path, repo.root.join("src/inner.sil"));
    assert_eq!(
        file.kind,
        FileKind::Worktree {
            workspace: ws.clone(),
            relative: PathBuf::from("src/inner.sil")
        }
    );
    let file = open(repo.root.join("vendor/ignored.sil")).expect("an ignored file");
    assert_eq!(file.kind, FileKind::Outside);
    let file = open(outside.join("far.sil")).expect("a file outside");
    assert_eq!(file.kind, FileKind::Outside);
    assert_eq!(file.text, "def far\n");
    let reason = open(outside.join("binary.bin")).expect_err("not UTF-8");
    assert!(reason.contains("UTF-8"), "{reason}");
    let reason = open(outside.join("not-there.sil")).expect_err("not there");
    assert!(reason.contains("not-there.sil"), "{reason}");
    running.end();
}

#[test]
fn a_name_after_wide_characters_is_asked_at_its_utf16_column_and_answered_in_bytes() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // Four 4-byte characters of two UTF-16 units each before the name: byte column 21, UTF-16
    // column 13, past the end of a line of 18 units.
    let repo = scratch.repository(
        "project",
        &[
            ("lib.sil", "def alpha\n"),
            (
                "main.sil",
                "use \u{1D11E}\u{1D11E}\u{1D11E}\u{1D11E} alpha\n",
            ),
        ],
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_logging(&scratch.root.join("log"))));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let at_alpha = || place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0);
    assert_eq!(at_alpha().column, 21);
    let hover = running.hover(at_alpha(), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: alpha".to_owned()),
            guessed: false
        }
    );
    let references = running.references(at_alpha(), DEADLINE);
    assert_eq!(
        references,
        Answered {
            value: vec![
                at(&repo.root.join("lib.sil"), 1, 4, "def alpha"),
                at(
                    &repo.root.join("main.sil"),
                    1,
                    21,
                    "use \u{1D11E}\u{1D11E}\u{1D11E}\u{1D11E} alpha"
                ),
            ],
            guessed: false
        }
    );
    running.end();
}
