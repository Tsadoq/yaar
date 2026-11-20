//! What a name in a diff is, answered with the stand-in language server: text match until a
//! server is ready (LSP-03), the position of an unchanged line carried through a past change
//! (OLD-07), a ready later candidate that answers while the question of an earlier one waits,
//! a file outside the worktree, a file of a language no row knows, the files a reviewer
//! opens through an answer, and what a server is told of changed files.
//!
//! Every wait is for something the Navigator tells or for a line of the stand-in's log, with
//! a deadline. A clause that says nothing was asked of the server is read from the stand-in's
//! log after a later request, through the same path, was logged: the stand-in handles its
//! requests in order, so a request that was sent before it is in the log by then.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::*;
use yaar_navigator::{Answered, Ask, AskId, FileKind, Location, State, Told, Tree};

/// `text` as a TOML string.
fn q(text: impl AsRef<str>) -> String {
    serde_json::to_string(text.as_ref()).expect("a string")
}

/// Every line of a stand-in's log, as the JSON it is; none when the log is not there.
fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

/// How many requests of `method` the stand-in logged.
fn requests(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| line["event"] == "request" && line["method"] == method)
        .count()
}

/// How many notifications of `method` the stand-in logged.
fn notifications(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| line["event"] == "notification" && line["method"] == method)
        .count()
}

/// The stand-in's row `standin`, ready by `notification`, that is ready once `go` exists,
/// and logs to `log`.
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

/// The stand-in's row `standin`, ready at once, logging to `log`.
fn row_logging(log: &Path) -> String {
    standin_row(
        "standin",
        &["--log", &log.display().to_string()],
        "answer",
        "",
    )
}

/// Waits until `look` gives something, looking again after a short pause, up to `DEADLINE`:
/// a repeated look at one fact that stays true once it is.
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

/// A location, as an answer holds it.
fn at(path: &Path, line: u32, column: u32, text: &str) -> Location {
    Location {
        path: path.to_path_buf(),
        line,
        column,
        text: text.to_owned(),
    }
}

// ---------------------------------------------------------------------------------------
// LSP-03: before the server is ready, text match, marked.
// ---------------------------------------------------------------------------------------

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
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Starting
    });

    let main = repo.root.join("main.sil");
    let lib = repo.root.join("lib.sil");
    let at_total = || place(&repo, &ws, Tree::Worktree, "main.sil", 2, "total", 0);

    // A hover asked before the server is ready is answered by text match, and marked: the
    // other lines that hold the name.
    let hover = running.hover(at_total(), DEADLINE);
    assert!(hover.guessed, "{hover:#?}");
    assert_eq!(hover.value.as_deref(), Some("lib.sil:1: def total"));

    // So is a definition, and so is a list of references: every match, the file under the
    // pointer first.
    let definition = running.definition(at_total(), DEADLINE);
    assert!(definition.guessed, "{definition:#?}");
    assert_eq!(
        definition.value,
        [at(&main, 2, 4, "use total"), at(&lib, 1, 4, "def total")]
    );
    let references = running.references(at_total(), DEADLINE);
    assert!(references.guessed, "{references:#?}");
    assert_eq!(references.value, definition.value);

    // Once the server is ready the same asks are answered by it, unmarked. Each of them is
    // logged, and the stand-in handles its requests in order: that none of the three asks
    // before it reached the server is read from the log now, with one request of each kind.
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

// ---------------------------------------------------------------------------------------
// OLD-07: the new side of a past change.
// ---------------------------------------------------------------------------------------

#[test]
fn on_a_past_change_an_unchanged_line_is_answered_by_the_server_and_a_changed_line_by_text_match() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let past = "use alpha\nuse beta\n";
    let repo = scratch.repository(
        "project",
        &[("main.sil", past), ("other.sil", "def beta\n")],
    );
    // The files on disk since: a line in front, `alpha` one line down, `beta` replaced.
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
    // Unchanged since: line 1 of the past text is line 2 of the file now. Asked at line 1
    // of the file on disk the server would say `gamma`.
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
    // Changed since: no server is asked; the name is looked for by text match.
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
    // A definition on a changed line is a text match too, and a line that moved is asked
    // again after it: the request before the changed one and the one after it are the only
    // two of the server's log.
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
    // `gone.sil` is in the past version only.
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
    // The barrier: a later hover through the same path is the first request of the log.
    running.hover(
        place(&repo, &ws, Tree::Worktree, "other.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(requests(&log, "textDocument/hover"), 1);
    running.end();
}

// ---------------------------------------------------------------------------------------
// Of its own.
// ---------------------------------------------------------------------------------------

#[test]
fn a_ready_later_candidate_answers_while_the_question_waits() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\nuse alpha\n")]);
    let log = scratch.root.join("standin.log");
    // The project's own copy comes first and lies inside the repository: it is the
    // question. The machine's copy comes after it and is ready meanwhile.
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

    // Asked again, with a short pause, until the machine's copy answers.
    let answer = wait_for("the machine's copy to answer", || {
        let answer = running.hover(
            place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
            DEADLINE,
        );
        (!answer.guessed).then_some(answer)
    });
    assert_eq!(answer.value.as_deref(), Some("standin: alpha"));

    // The line still shows the question: the last list of servers told says so.
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
    // No server is asked about a file of a language no row knows: the barrier is a later
    // hover on a file of the row, the first request of the log.
    running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "main", 0),
        DEADLINE,
    );
    assert_eq!(requests(&log, "textDocument/hover"), 1);
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

    // A path told twice, one that no longer exists, and one outside the worktree.
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
    // The Navigator went on: it answers.
    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(hover.value.as_deref(), Some("standin: alpha"));
    // One notice for the one ask; nothing for a list with no path.
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
    // Ignored by git, outside the worktree: neither is the worktree's file.
    let file = open(repo.root.join("vendor/ignored.sil")).expect("an ignored file");
    assert_eq!(file.kind, FileKind::Outside);
    let file = open(outside.join("far.sil")).expect("a file outside");
    assert_eq!(file.kind, FileKind::Outside);
    assert_eq!(file.text, "def far\n");
    // A file that cannot be read is told as the reason.
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
    // Four characters of four bytes and two UTF-16 units each in front of the name: its
    // byte column is 21 and its UTF-16 column 13, past the end of a line of 18 units.
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
