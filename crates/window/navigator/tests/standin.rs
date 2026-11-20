//! The stand-in language server, as the tests of the Navigator, the review pane and the
//! Window start it: what it answers, what it logs, and how it ends. And the support every
//! component test of the Navigator stands on (`tests/support/mod.rs`): each helper shown to
//! do what its doc says. The stand-in is driven here by `lsp::write_frame` and
//! `lsp::read_frame` over its pipes, as the Navigator's driver speaks to a server.

mod support;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::*;
use yaar_navigator::lsp::{file_uri, read_frame, write_frame};
use yaar_navigator::{Answered, Ask, AskId, Location, ServerState, State, Told, Tree};

// ---------------------------------------------------------------------------------------
// The stand-in, started and driven by the test.
// ---------------------------------------------------------------------------------------

/// A running stand-in and the test's end of its pipes. A reading thread hands each frame the
/// stand-in writes to a channel, so that every wait is for a frame, with a deadline, and
/// never a sleep between looks.
struct Standin {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<Value>,
    next_id: u64,
    /// Everything read and passed over while a wait looked for another message.
    passed_over: Vec<Value>,
}

impl Standin {
    /// Starts the stand-in with `args`, in the folder `cwd`, with `environment` added.
    fn start_in(cwd: &Path, args: &[&str], environment: &[(&str, &str)]) -> Standin {
        let mut child = Command::new(standin())
            .args(args)
            .envs(environment.iter().copied())
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the stand-in");
        let stdin = child.stdin.take();
        let mut stdout = std::io::BufReader::new(child.stdout.take().expect("its output"));
        let (to_test, frames) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(frame)) = read_frame(&mut stdout) {
                if to_test.send(frame).is_err() {
                    return;
                }
            }
        });
        Standin {
            child,
            stdin,
            frames,
            next_id: 1,
            passed_over: Vec::new(),
        }
    }

    fn start(scratch: &Scratch, args: &[&str]) -> Standin {
        Standin::start_in(&scratch.root, args, &[])
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("its input is open");
        write_frame(stdin, &message).expect("write to the stand-in");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Sends a request and gives the whole answer, `result` or `error`.
    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.message_where(&format!("the answer to {method}"), |message| {
            message.get("id") == Some(&json!(id)) && message.get("method").is_none()
        })
    }

    /// The result of a request that succeeds.
    fn result_of(&mut self, method: &str, params: Value) -> Value {
        let answer = self.ask(method, params);
        answer
            .get("result")
            .unwrap_or_else(|| panic!("{method} was not answered with a result: {answer}"))
            .clone()
    }

    /// The next frame the stand-in wrote.
    fn next(&mut self, what: &str) -> Value {
        match self.frames.recv_timeout(DEADLINE) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => panic!(
                "waited {DEADLINE:?} for {what}; passed over: {:#?}",
                self.passed_over
            ),
            Err(RecvTimeoutError::Disconnected) => panic!(
                "the stand-in ended while the test waited for {what}; passed over: {:#?}",
                self.passed_over
            ),
        }
    }

    /// The next frame that satisfies `wanted`; the others are kept in `passed_over`.
    fn message_where(&mut self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        loop {
            let frame = self.next(what);
            if wanted(&frame) {
                return frame;
            }
            self.passed_over.push(frame);
        }
    }

    /// `initialize` with `root` as the root, then `initialized`.
    fn start_session(&mut self, root: &Path, options: Value) -> Value {
        let result = self.result_of(
            "initialize",
            json!({
                "processId": null,
                "rootUri": file_uri(root),
                "initializationOptions": options,
                "capabilities": {},
            }),
        );
        self.notify("initialized", json!({}));
        result
    }

    fn hover(&mut self, file: &Path, line: u32, character: u32) -> Value {
        self.result_of("textDocument/hover", at(file, line, character))
    }

    fn locations(&mut self, method: &str, file: &Path, line: u32, character: u32) -> Value {
        self.result_of(method, at(file, line, character))
    }

    /// Closes its input, as a client that is gone does.
    fn close_input(&mut self) {
        self.stdin = None;
    }

    /// The code it ended with, waiting up to the deadline: the end of a process is a fact
    /// that stays true, looked at again with a short pause.
    fn exit_code(&mut self) -> i32 {
        let until = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().expect("ask the process") {
                return status.code().expect("it ended by a code, not a signal");
            }
            assert!(Instant::now() < until, "the stand-in did not end");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The params of a text document request at a position.
fn at(file: &Path, line: u32, character: u32) -> Value {
    json!({
        "textDocument": {"uri": file_uri(file)},
        "position": {"line": line, "character": character},
    })
}

/// A `Location`, as the stand-in answers a definition or a reference.
fn location(file: &Path, line: u32, from: u32, to: u32) -> Value {
    json!({
        "uri": file_uri(file),
        "range": {
            "start": {"line": line, "character": from},
            "end": {"line": line, "character": to},
        },
    })
}

/// What a hover of the stand-in answers for `name`.
fn hovering(name: &str) -> Value {
    json!({"contents": {"kind": "plaintext", "value": format!("standin: {name}")}})
}

/// The lines of a log, each a JSON object.
fn log_of(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_else(|error| panic!("read the log {}: {error}", file.display()))
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

fn events(log: &[Value]) -> Vec<&str> {
    log.iter()
        .map(|line| line["event"].as_str().expect("an event"))
        .collect()
}

/// The stand-in's `.sil` files for the tests of definition and references.
///
/// `main.sil` line 0 has a surrogate pair and an accented letter before `total`: its column
/// is 9 in UTF-16 units and 12 in bytes.
fn write_project(scratch: &Scratch) -> PathBuf {
    let root = scratch.root.join("repos").join("project");
    for (path, text) in [
        ("lib.sil", "def total\ndef other\n"),
        (
            "main.sil",
            "use 𝄞 é total\ntotal + totals + total\nuse other\n",
        ),
        (".hidden/lib.sil", "def total\n"),
        ("notes.txt", "def total\n"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    root
}

#[test]
fn version_prints_its_name_and_ends_with_success() {
    let scratch = Scratch::new();
    let log = scratch.root.join("version.log");
    let output = Command::new(standin())
        .args(["--version", "--log"])
        .arg(&log)
        .output()
        .expect("run it");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "standin-lsp 1\n");
    assert!(output.stderr.is_empty());
    assert!(!log.exists(), "a trial run writes no log");
}

#[test]
fn version_with_fail_trial_says_it_is_broken_and_ends_with_3() {
    let _scratch = Scratch::new();
    let output = Command::new(standin())
        .args(["--fail-trial", "--version"])
        .output()
        .expect("run it");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "standin-lsp: broken\n"
    );
}

#[test]
fn fail_trial_alone_changes_nothing() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &["--fail-trial"]);
    standin.start_session(&scratch.root, json!(null));
    assert_eq!(standin.result_of("shutdown", json!(null)), json!(null));
    standin.notify("exit", json!(null));
    assert_eq!(standin.exit_code(), 0);
}

#[test]
fn an_argument_it_does_not_have_ends_with_64_and_a_sentence() {
    let _scratch = Scratch::new();
    for args in [
        &["--bogus"][..],
        &["--ready", "sideways"],
        &["--ready"],
        &["--ready-when"],
        &["--log"],
        &["--ask-configuration"],
    ] {
        let output = Command::new(standin())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run it");
        assert_eq!(output.status.code(), Some(64), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("standin-lsp: "),
            "{args:?}"
        );
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn initialize_is_answered_with_its_capabilities_and_logged_with_what_it_received() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let log_arg = log.display().to_string();
    let mut standin = Standin::start_in(
        &scratch.root,
        &["--log", &log_arg, "--fail-trial"],
        &[("STANDIN_MARK", "yes")],
    );
    let result = standin.result_of(
        "initialize",
        json!({
            "rootUri": file_uri(&scratch.root),
            "initializationOptions": {"a": [1, 2]},
            "capabilities": {"textDocument": {"hover": {"contentFormat": ["plaintext"]}}},
        }),
    );
    assert_eq!(
        result,
        json!({
            "capabilities": {
                "hoverProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "textDocumentSync": 1,
            },
            "serverInfo": {"name": "standin-lsp"},
        })
    );

    // A line is written before what it tells is answered: read right after the answer.
    let log = log_of(&log);
    assert_eq!(events(&log), ["start", "request", "initialize"]);
    let start = &log[0];
    assert_eq!(start["pid"], json!(standin.child.id()));
    assert_eq!(start["cwd"], json!(scratch.root.display().to_string()));
    assert_eq!(start["args"], json!(["--log", log_arg, "--fail-trial"]));
    assert_eq!(start["env"]["STANDIN_MARK"], json!("yes"));
    assert_eq!(
        start["env"]["HOME"],
        json!(scratch.root.join("home").display().to_string())
    );
    let names: Vec<&String> = start["env"].as_object().unwrap().keys().collect();
    assert!(
        names.iter().all(
            |name| ["HOME", "TMPDIR", "XDG_CACHE_HOME"].contains(&name.as_str())
                || name.starts_with("STANDIN_")
        ),
        "{names:?}"
    );
    assert_eq!(
        log[1],
        json!({"event": "request", "method": "initialize", "id": 1})
    );
    assert_eq!(
        log[2],
        json!({
            "event": "initialize",
            "rootUri": file_uri(&scratch.root),
            "initializationOptions": {"a": [1, 2]},
            "capabilities": {"textDocument": {"hover": {"contentFormat": ["plaintext"]}}},
        })
    );
}

#[test]
fn a_request_and_a_notification_are_each_logged_with_their_method() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(&scratch, &["--log", &log.display().to_string()]);
    standin.start_session(&scratch.root, json!(null));
    standin.notify("workspace/didChangeWatchedFiles", json!({"changes": []}));
    standin.result_of("shutdown", json!(null));

    let log = log_of(&log);
    let seen: Vec<(&str, &str)> = log[1..]
        .iter()
        .map(|line| {
            (
                line["event"].as_str().unwrap(),
                line["method"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("request", "initialize"),
            ("initialize", ""),
            ("notification", "initialized"),
            ("ready", ""),
            ("notification", "workspace/didChangeWatchedFiles"),
            ("request", "shutdown"),
        ]
    );
    assert_eq!(log[6]["id"], json!(2), "the id of the request, as sent");
}

#[test]
fn it_is_ready_at_initialized_when_it_waits_for_nothing() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let main = project.join("main.sil");
    let mut standin = Standin::start(&scratch, &[]);
    standin.result_of("initialize", json!({"rootUri": file_uri(&project)}));
    assert_eq!(
        standin.hover(&main, 0, 9),
        json!(null),
        "not ready before it was told it may start"
    );
    standin.notify("initialized", json!({}));
    assert_eq!(standin.hover(&main, 0, 9), hovering("total"));
}

#[test]
fn hover_is_null_before_it_is_ready_and_the_name_after() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let main = project.join("main.sil");
    let go = scratch.root.join("go");
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(
        &scratch,
        &[
            "--ready",
            "notification",
            "--ready-when",
            &go.display().to_string(),
            "--log",
            &log.display().to_string(),
        ],
    );
    standin.start_session(&project, json!(null));
    let status = standin.message_where("its status", |m| m["method"] == "standin/status");
    assert_eq!(status["params"], json!({"ready": false}));

    // Its first answer comes after `initialized` was handled: a stand-in that ignored
    // `--ready-when` would be ready by then.
    assert_eq!(standin.hover(&main, 0, 9), json!(null));
    assert!(!events(&log_of(&log)).contains(&"ready"));

    std::fs::write(&go, "").unwrap();
    let status = standin.message_where("its status", |m| m["method"] == "standin/status");
    assert_eq!(status["params"], json!({"ready": true}));
    assert_eq!(standin.hover(&main, 0, 9), hovering("total"));
    let log = log_of(&log);
    assert_eq!(events(&log).iter().filter(|e| **e == "ready").count(), 1);
}

#[test]
fn hover_is_null_where_no_name_is() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let main = project.join("main.sil");
    let mut standin = Standin::start(&scratch, &[]);
    standin.start_session(&project, json!(null));
    // The surrogate pair at units 4 and 5, a space at 6, the accented letter at 7.
    assert_eq!(standin.hover(&main, 0, 4), json!(null));
    assert_eq!(standin.hover(&main, 0, 5), json!(null));
    assert_eq!(standin.hover(&main, 0, 6), json!(null));
    assert_eq!(standin.hover(&main, 0, 7), hovering("é"));
    assert_eq!(
        standin.hover(&main, 9, 0),
        json!(null),
        "past the last line"
    );
    assert_eq!(
        standin.hover(&project.join("missing.sil"), 0, 0),
        json!(null),
        "a file that is not there"
    );
}

#[test]
fn definition_and_references_count_positions_in_utf16_units() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let (lib, main) = (project.join("lib.sil"), project.join("main.sil"));
    let mut standin = Standin::start(&scratch, &[]);
    standin.start_session(&project, json!(null));

    // `total` after a surrogate pair and an accented letter: unit 9, byte 12.
    assert_eq!(
        standin.locations("textDocument/definition", &main, 0, 9),
        json!([location(&lib, 0, 4, 9)])
    );
    assert_eq!(
        standin.locations("textDocument/references", &main, 0, 9),
        json!([
            location(&lib, 0, 4, 9),
            location(&main, 0, 9, 14),
            location(&main, 1, 0, 5),
            location(&main, 1, 17, 22),
        ]),
        "by path, then line, then column; `totals` is another name, and neither the folder \
         that starts with a dot nor the file that is not .sil is read"
    );
    // The same from inside the name, and from a reference on another line.
    assert_eq!(
        standin.locations("textDocument/definition", &main, 1, 19),
        json!([location(&lib, 0, 4, 9)])
    );
    assert_eq!(
        standin.locations("textDocument/definition", &main, 2, 4),
        json!([location(&lib, 1, 4, 9)]),
        "`use other` leads to `def other`"
    );
}

#[test]
fn a_name_with_no_definition_or_no_name_at_the_place_has_no_location() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let main = project.join("main.sil");
    let mut standin = Standin::start(&scratch, &[]);
    standin.start_session(&project, json!(null));
    // `use` defines nothing; the space after it is no name.
    assert_eq!(
        standin.locations("textDocument/definition", &main, 0, 1),
        json!([])
    );
    assert_eq!(
        standin.locations("textDocument/definition", &main, 0, 3),
        json!([])
    );
    assert_eq!(
        standin.locations("textDocument/references", &main, 0, 3),
        json!([])
    );
}

#[test]
fn definition_and_references_are_empty_before_it_is_ready() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let main = project.join("main.sil");
    let go = scratch.root.join("go");
    let mut standin = Standin::start(&scratch, &["--ready-when", &go.display().to_string()]);
    standin.start_session(&project, json!(null));
    assert_eq!(
        standin.locations("textDocument/definition", &main, 0, 9),
        json!([])
    );
    assert_eq!(
        standin.locations("textDocument/references", &main, 0, 9),
        json!([])
    );
}

#[test]
fn what_the_client_opened_or_changed_is_read_before_the_file_on_disk() {
    let scratch = Scratch::new();
    let project = write_project(&scratch);
    let lib = project.join("lib.sil");
    let uri = file_uri(&lib);
    let mut standin = Standin::start(&scratch, &[]);
    standin.start_session(&project, json!(null));

    assert_eq!(standin.hover(&lib, 0, 4), hovering("total"));
    standin.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": uri, "languageId": "standin", "version": 1, "text": "def gamma\n"}}),
    );
    assert_eq!(standin.hover(&lib, 0, 4), hovering("gamma"));
    standin.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": "def beta\n"}],
        }),
    );
    assert_eq!(standin.hover(&lib, 0, 4), hovering("beta"));
    // A definition is where the files on disk define the name: `beta` has none there.
    assert_eq!(
        standin.locations("textDocument/definition", &lib, 0, 4),
        json!([])
    );
    standin.notify(
        "textDocument/didClose",
        json!({"textDocument": {"uri": uri}}),
    );
    assert_eq!(standin.hover(&lib, 0, 4), hovering("total"));
}

#[test]
fn ready_progress_creates_the_token_begins_and_ends_once_ready() {
    let scratch = Scratch::new();
    let go = scratch.root.join("go");
    let mut standin = Standin::start(
        &scratch,
        &[
            "--ready",
            "progress",
            "--ready-when",
            &go.display().to_string(),
        ],
    );
    standin.start_session(&scratch.root, json!(null));

    let create = standin.next("the request for the token");
    assert_eq!(create["method"], "window/workDoneProgress/create");
    assert_eq!(create["params"], json!({"token": "standin"}));
    assert!(
        create.get("id").is_some(),
        "a request, so that the client answers"
    );
    let begin = standin.next("the beginning");
    assert_eq!(begin["method"], "$/progress");
    assert_eq!(begin["params"]["token"], "standin");
    assert_eq!(begin["params"]["value"]["kind"], "begin");

    // Nothing ended before the file is there: the next thing is the answer to this.
    assert_eq!(
        standin.hover(&scratch.root.join("none.sil"), 0, 0),
        json!(null)
    );
    assert!(standin.passed_over.is_empty(), "{:?}", standin.passed_over);

    std::fs::write(&go, "").unwrap();
    let end = standin.next("the end");
    assert_eq!(end["method"], "$/progress");
    assert_eq!(
        end["params"],
        json!({"token": "standin", "value": {"kind": "end"}})
    );
}

#[test]
fn ready_progress_with_nothing_to_wait_for_ends_at_once() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &["--ready", "progress"]);
    standin.start_session(&scratch.root, json!(null));
    let kinds: Vec<Value> = (0..3)
        .map(|_| {
            let message = standin.next("a message of the progress");
            message
                .pointer("/params/value/kind")
                .cloned()
                .unwrap_or_else(|| message["method"].clone())
        })
        .collect();
    assert_eq!(
        kinds,
        [
            json!("window/workDoneProgress/create"),
            json!("begin"),
            json!("end")
        ]
    );
}

#[test]
fn ready_notification_says_not_ready_then_ready() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &["--ready", "notification"]);
    standin.start_session(&scratch.root, json!(null));
    let first = standin.next("the first status");
    let second = standin.next("the second status");
    assert_eq!(first["method"], "standin/status");
    assert_eq!(first["params"], json!({"ready": false}));
    assert_eq!(second["method"], "standin/status");
    assert_eq!(second["params"], json!({"ready": true}));
}

#[test]
fn ready_answer_sends_nothing_unasked() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &["--ready", "answer"]);
    standin.start_session(&scratch.root, json!(null));
    standin.hover(&scratch.root.join("none.sil"), 0, 0);
    assert!(standin.passed_over.is_empty(), "{:?}", standin.passed_over);
}

#[test]
fn ask_configuration_asks_for_its_section_and_logs_the_answer() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(
        &scratch,
        &[
            "--ask-configuration",
            "standin.section",
            "--log",
            &log.display().to_string(),
        ],
    );
    standin.start_session(&scratch.root, json!(null));
    let asked = standin.message_where("its request for configuration", |m| {
        m["method"] == "workspace/configuration"
    });
    assert_eq!(
        asked["params"],
        json!({"items": [{"section": "standin.section"}]})
    );
    standin.send(json!({"jsonrpc": "2.0", "id": asked["id"], "result": [{"k": "v"}]}));

    // The stand-in reads in order: when it answers this, it has read the answer above.
    standin.result_of("shutdown", json!(null));
    let configuration: Vec<Value> = log_of(&log)
        .into_iter()
        .filter(|line| line["event"] == "configuration")
        .collect();
    assert_eq!(
        configuration,
        [json!({"event": "configuration", "result": [{"k": "v"}]})]
    );
}

#[test]
fn without_ask_configuration_it_asks_nothing_and_logs_no_configuration() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(&scratch, &["--log", &log.display().to_string()]);
    standin.start_session(&scratch.root, json!(null));
    standin.send(json!({"jsonrpc": "2.0", "id": 2, "result": [null]}));
    standin.result_of("shutdown", json!(null));
    assert!(standin.passed_over.is_empty());
    assert!(!events(&log_of(&log)).contains(&"configuration"));
}

#[test]
fn fail_initialize_answers_with_an_error_and_ends_with_1() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(
        &scratch,
        &["--fail-initialize", "--log", &log.display().to_string()],
    );
    let answer = standin.ask("initialize", json!({"rootUri": file_uri(&scratch.root)}));
    assert_eq!(answer["error"]["code"], json!(-32603));
    assert!(answer["error"]["message"].is_string());
    assert!(answer.get("result").is_none());
    assert_eq!(standin.exit_code(), 1);
    let log = log_of(&log);
    assert_eq!(events(&log), ["start", "request", "initialize", "exit"]);
    assert_eq!(log[3]["code"], json!(1));
}

#[test]
fn exit_after_ready_ends_with_2_once_it_is_ready() {
    let scratch = Scratch::new();
    let go = scratch.root.join("go");
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(
        &scratch,
        &[
            "--exit-after-ready",
            "--ready-when",
            &go.display().to_string(),
            "--log",
            &log.display().to_string(),
        ],
    );
    standin.start_session(&scratch.root, json!(null));
    // Alive and answering until the file is there.
    assert_eq!(
        standin.hover(&scratch.root.join("none.sil"), 0, 0),
        json!(null)
    );
    std::fs::write(&go, "").unwrap();
    assert_eq!(standin.exit_code(), 2);
    let log = log_of(&log);
    let events = events(&log);
    assert_eq!(&events[events.len() - 2..], ["ready", "exit"]);
    assert_eq!(log.last().unwrap()["code"], json!(2));
}

#[test]
fn exit_after_ready_with_nothing_to_wait_for_ends_with_2_at_initialized() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &["--exit-after-ready"]);
    standin.start_session(&scratch.root, json!(null));
    assert_eq!(standin.exit_code(), 2);
}

#[test]
fn shutdown_answers_null_and_exit_after_it_ends_with_0() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(&scratch, &["--log", &log.display().to_string()]);
    standin.start_session(&scratch.root, json!(null));
    assert_eq!(
        standin.ask("shutdown", json!(null)).get("result"),
        Some(&json!(null))
    );
    standin.notify("exit", json!(null));
    assert_eq!(standin.exit_code(), 0);
    assert_eq!(
        log_of(&log).last().unwrap(),
        &json!({"event": "exit", "code": 0})
    );
}

#[test]
fn exit_with_no_shutdown_ends_with_1() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(&scratch, &["--log", &log.display().to_string()]);
    standin.start_session(&scratch.root, json!(null));
    standin.notify("exit", json!(null));
    assert_eq!(standin.exit_code(), 1);
    assert_eq!(
        log_of(&log).last().unwrap(),
        &json!({"event": "exit", "code": 1})
    );
}

#[test]
fn the_end_of_its_input_ends_it_with_0() {
    let scratch = Scratch::new();
    let log = scratch.root.join("run.log");
    let mut standin = Standin::start(&scratch, &["--log", &log.display().to_string()]);
    standin.start_session(&scratch.root, json!(null));
    standin.close_input();
    assert_eq!(standin.exit_code(), 0);
    assert_eq!(
        log_of(&log).last().unwrap(),
        &json!({"event": "exit", "code": 0})
    );
}

#[test]
fn a_stream_that_is_no_frame_ends_it_with_1() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &[]);
    standin
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Content-Length: nine\r\n\r\n")
        .unwrap();
    assert_eq!(standin.exit_code(), 1);
}

#[test]
fn any_other_request_is_a_method_not_found_and_an_unknown_notification_is_ignored() {
    let scratch = Scratch::new();
    let mut standin = Standin::start(&scratch, &[]);
    standin.start_session(&scratch.root, json!(null));
    standin.notify("standin/unheard-of", json!({}));
    let answer = standin.ask("textDocument/rename", json!({}));
    assert_eq!(answer["error"]["code"], json!(-32601));
    assert!(answer.get("result").is_none());
    assert!(standin.passed_over.is_empty());
    // Still alive and answering.
    assert_eq!(standin.result_of("shutdown", json!(null)), json!(null));
}

// ---------------------------------------------------------------------------------------
// The support.
// ---------------------------------------------------------------------------------------

#[test]
fn a_scratch_is_a_folder_of_its_own_with_its_links_resolved_and_goes_when_dropped() {
    let scratch = Scratch::new();
    let root = scratch.root.clone();
    assert_eq!(root, root.canonicalize().unwrap());
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    for folder in [
        "home",
        "cache",
        "config",
        "data",
        "tmp",
        "cargo",
        "bin",
        "repos",
        "servers",
        "navigator",
    ] {
        assert!(root.join(folder).is_dir(), "{folder}");
    }
    assert_eq!(std::env::var_os("HOME"), Some(root.join("home").into()));
    assert_eq!(
        std::env::var("GIT_CONFIG_GLOBAL").as_deref(),
        Ok("/dev/null")
    );
    assert_eq!(std::env::var("GIT_CONFIG_NOSYSTEM").as_deref(), Ok("1"));
    assert!(std::env::var_os("GIT_DIR").is_none());
    drop(scratch);
    assert!(!root.exists());
}

#[test]
fn two_scratches_of_one_process_are_two_folders() {
    let first = Scratch::new();
    let second = Scratch::new();
    assert_ne!(first.root, second.root);
    assert!(first.root.is_dir() && second.root.is_dir());
}

#[test]
fn the_recorder_npm_appends_its_arguments_to_its_log_and_fails() {
    let scratch = Scratch::new();
    assert!(scratch.npm_ran().is_empty(), "no run, no line");
    for args in [&["install", "left pad"][..], &["view", "x"]] {
        let status = Command::new(scratch.root.join("bin").join("npm"))
            .args(args)
            .status()
            .expect("run the recorder");
        assert_eq!(status.code(), Some(1));
    }
    assert_eq!(scratch.npm_ran(), ["install left pad", "view x"]);
}

/// What `git status --porcelain --ignored --untracked-files=all` says, as lines.
fn status_lines(repo: &Repo) -> Vec<String> {
    repo.git(&[
        "status",
        "--porcelain",
        "--ignored",
        "--untracked-files=all",
    ])
    .lines()
    .map(str::to_owned)
    .collect()
}

fn committed_files(repo: &Repo) -> Vec<String> {
    repo.git(&["ls-tree", "-r", "--name-only", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_rust_fixture_is_its_base_committed_and_its_change_not() {
    let scratch = Scratch::new();
    let repo = scratch.fixture(Fixture::Rust);
    assert_eq!(repo.base, repo.git(&["rev-parse", "HEAD"]));
    assert_eq!(repo.base.len(), 40);
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "1");
    assert_eq!(repo.root, repo.root.canonicalize().unwrap());
    assert!(repo.root.starts_with(scratch.root.join("repos")));
    assert_eq!(
        committed_files(&repo),
        [
            ".gitignore",
            "Cargo.lock",
            "Cargo.toml",
            "src/geometry.rs",
            "src/lib.rs",
            "src/report.rs",
            "src/strings.rs"
        ]
    );
    assert_eq!(
        repo.git(&["show", "HEAD:.gitignore"]),
        "target/",
        "dot-gitignore is the project's .gitignore"
    );
    // The change: three files rewritten, `strings.rs` gone, `text.rs` new.
    assert_eq!(
        status_lines(&repo),
        [
            " M src/geometry.rs",
            " M src/lib.rs",
            " M src/report.rs",
            " D src/strings.rs",
            "?? src/text.rs"
        ]
    );
    assert_eq!(repo.git(&["config", "--local", "user.name"]), "A Test");
    assert_eq!(
        repo.git(&["config", "--local", "user.email"]),
        "test@example.invalid"
    );
}

#[test]
fn the_typescript_fixture_has_its_dependencies_ignored_and_its_change_not_committed() {
    let scratch = Scratch::new();
    let repo = scratch.fixture(Fixture::TypeScript);
    assert_eq!(repo.base, repo.git(&["rev-parse", "HEAD"]));
    let committed = committed_files(&repo);
    assert!(committed.contains(&".gitignore".to_owned()));
    assert!(committed.contains(&"tools/build.js".to_owned()));
    assert!(committed.contains(&"src/strings.ts".to_owned()));
    assert!(
        !committed
            .iter()
            .any(|path| path.starts_with("node_modules")),
        "the dependencies are not the base's"
    );
    assert!(
        repo.root
            .join("node_modules/tinyshape/index.d.ts")
            .is_file()
    );
    let status = status_lines(&repo);
    assert!(
        status.contains(&" D src/strings.ts".to_owned()),
        "{status:?}"
    );
    assert!(
        status.contains(&"?? src/text/case.ts".to_owned()),
        "{status:?}"
    );
    assert!(
        status
            .iter()
            .any(|line| line.starts_with("!! node_modules/")),
        "ignored, as the README says: {status:?}"
    );
    assert!(!repo.root.join("src/strings.ts").exists());
}

#[test]
fn the_python_fixture_keeps_its_virtual_environment_out_of_git() {
    let scratch = Scratch::new();
    let repo = scratch.fixture(Fixture::Python);
    assert_eq!(repo.base, repo.git(&["rev-parse", "HEAD"]));
    assert!(
        repo.root
            .join(".venv/lib/python3.12/site-packages/tinyshape/__init__.py")
            .is_file()
    );
    let status = status_lines(&repo);
    assert!(
        status.contains(&" D app/strings.py".to_owned()),
        "{status:?}"
    );
    assert!(
        status.contains(&"?? app/text/case.py".to_owned()),
        "{status:?}"
    );
    assert!(
        status.iter().any(|line| line.starts_with("!! .venv/")),
        "{status:?}"
    );
    assert!(
        !committed_files(&repo)
            .iter()
            .any(|path| path.starts_with(".venv"))
    );
}

#[test]
fn the_status_of_a_repository_is_git_status_in_porcelain_v2_with_ignored_and_untracked_files() {
    let scratch = Scratch::new();
    let repo = scratch.fixture(Fixture::Rust);
    std::fs::create_dir_all(repo.root.join("target/debug")).unwrap();
    std::fs::write(repo.root.join("target/debug/out"), "x").unwrap();
    let status = repo.status();
    let lines: Vec<&str> = status.lines().collect();
    assert_eq!(lines.len(), 6, "{status}");
    assert!(lines[0].starts_with("1 .M "), "{status}");
    assert!(lines[3].starts_with("1 .D "), "{status}");
    assert_eq!(lines[4], "? src/text.rs");
    assert!(lines[5].starts_with("! target"), "{status}");
}

#[test]
fn a_repository_holds_its_files_committed_once_with_its_own_identity() {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "plain",
        &[("main.sil", "def a\n"), ("deep/er/lib.sil", "def b\n")],
    );
    assert_eq!(repo.root, scratch.root.join("repos").join("plain"));
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "1");
    assert_eq!(repo.base, repo.git(&["rev-parse", "HEAD"]));
    assert_eq!(committed_files(&repo), ["deep/er/lib.sil", "main.sil"]);
    assert!(status_lines(&repo).is_empty(), "nothing uncommitted");
    assert_eq!(repo.git(&["config", "--local", "user.name"]), "A Test");
    assert_eq!(repo.git(&["branch", "--show-current"]), "main");
}

#[test]
fn git_runs_with_the_fixed_identity_gives_no_line_end_and_panics_with_gits_words() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "def a\n")]);
    assert_eq!(
        repo.git(&["var", "GIT_AUTHOR_IDENT"]),
        "A Test <test@example.invalid> 1767268800 +0000"
    );
    assert_eq!(repo.git(&["show", "HEAD:main.sil"]), "def a");
    std::fs::write(repo.root.join("new.sil"), "").unwrap();
    assert_eq!(repo.git(&["status", "--porcelain"]), "?? new.sil");
    std::fs::write(repo.root.join("main.sil"), "def b\n").unwrap();
    assert_eq!(
        repo.git(&["status", "--porcelain"]),
        " M main.sil\n?? new.sil",
        "the first character of the first line is kept, the last line's end is not"
    );

    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repo.git(&["rev-parse", "--verify", "no-such-revision"])
    }))
    .expect_err("git fails on an unknown revision");
    let message = panic_text(failure);
    assert!(message.contains("no-such-revision"), "{message}");
    assert!(message.contains("failed"), "{message}");
}

#[test]
fn files_under_lists_every_file_with_its_size_by_path() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "def a\n")]);
    assert!(
        repo.files_under("target").is_empty(),
        "a folder that is not there"
    );
    for (path, text) in [
        ("target/b.txt", "bb"),
        ("target/a/z.txt", "zzz"),
        ("target/a/y.txt", ""),
    ] {
        let file = repo.root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    std::os::unix::fs::symlink("b.txt", repo.root.join("target/link")).unwrap();
    assert_eq!(
        repo.files_under("target"),
        [
            (repo.root.join("target/a/y.txt"), 0),
            (repo.root.join("target/a/z.txt"), 3),
            (repo.root.join("target/b.txt"), 2),
            (repo.root.join("target/link"), 5),
        ]
    );
    assert_eq!(repo.files_under("target/a").len(), 2);
    assert!(
        repo.files_under("")
            .contains(&(repo.root.join("main.sil"), 6)),
        "the whole worktree, git's own folder included"
    );
}

#[test]
fn line_is_the_text_of_a_line_counted_from_1_with_no_line_end() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("a/b.sil", "one\r\ntwo\nthree")]);
    assert_eq!(repo.line("a/b.sil", 1), "one");
    assert_eq!(repo.line("a/b.sil", 2), "two");
    assert_eq!(repo.line("a/b.sil", 3), "three");
    let past_the_end =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| repo.line("a/b.sil", 4)));
    assert!(past_the_end.is_err());
}

#[test]
fn the_environment_of_a_server_is_the_scratchs_and_nothing_of_this_process() {
    let scratch = Scratch::new();
    let environment: std::collections::BTreeMap<String, String> = scratch
        .environment()
        .into_iter()
        .map(|(name, value)| {
            (
                name.into_string().expect("a name"),
                value.into_string().expect("a value"),
            )
        })
        .collect();
    let root = scratch.root.display().to_string();
    for (name, folder) in [
        ("HOME", "home"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("TMPDIR", "tmp"),
        ("CARGO_HOME", "cargo"),
    ] {
        assert_eq!(environment[name], format!("{root}/{folder}"), "{name}");
    }
    assert_eq!(environment["CARGO_NET_OFFLINE"], "true");
    assert_eq!(environment["RUSTUP_AUTO_INSTALL"], "0");
    assert_eq!(environment["LANG"], "C.UTF-8");
    assert!(!environment.contains_key("NO_PROXY") && !environment.contains_key("no_proxy"));

    // The search path: the recorder first, then the folder of this test's cargo, then the
    // system's two.
    let search: Vec<PathBuf> = std::env::split_paths(&environment["PATH"]).collect();
    let cargo = std::env::var_os("CARGO")
        .or_else(|| option_env!("CARGO").map(Into::into))
        .expect("Cargo names its own program");
    let cargo_folder = Path::new(&cargo).parent().unwrap().to_path_buf();
    assert_eq!(
        search,
        [
            scratch.root.join("bin"),
            cargo_folder.clone(),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin")
        ]
    );
    assert!(cargo_folder.join("cargo").is_file() && cargo_folder.join("rustc").is_file());

    // The six proxy variables name one address.
    let proxy = &environment["HTTP_PROXY"];
    assert!(proxy.starts_with("http://127.0.0.1:"), "{proxy}");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        assert_eq!(&environment[name], proxy, "{name}");
    }

    // Nothing else: the names are exactly these (the rustup variables when this process has
    // them).
    let mut expected: Vec<&str> = vec![
        "ALL_PROXY",
        "CARGO_HOME",
        "CARGO_NET_OFFLINE",
        "HOME",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "LANG",
        "PATH",
        "RUSTUP_AUTO_INSTALL",
        "TMPDIR",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "all_proxy",
        "http_proxy",
        "https_proxy",
    ];
    for name in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        match std::env::var(name) {
            Ok(value) => {
                assert_eq!(environment[name], value, "{name} as this process has it");
                expected.push(name);
            }
            Err(_) => assert!(!environment.contains_key(name), "{name} is not invented"),
        }
    }
    expected.sort_unstable();
    let names: Vec<&str> = environment.keys().map(String::as_str).collect();
    assert_eq!(names, expected);
}

#[test]
fn the_proxy_variables_name_the_listener_the_scratch_keeps() {
    let scratch = Scratch::new();
    assert!(scratch.proxy_heard().is_empty());
    let environment = scratch.environment();
    let proxy = environment
        .iter()
        .find(|(name, _)| name == "HTTPS_PROXY")
        .map(|(_, value)| value.to_str().unwrap().to_owned())
        .unwrap();
    let address = proxy.strip_prefix("http://").unwrap();

    let mut connection = std::net::TcpStream::connect(address).expect("connect to the proxy");
    connection
        .write_all(b"CONNECT registry.invalid:443 HTTP/1.1\r\nHost: registry.invalid:443\r\n\r\n")
        .unwrap();
    let mut answer = String::new();
    connection.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 404"), "{answer}");
    let heard = scratch.proxy_heard();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].line, "CONNECT registry.invalid:443");
}

#[test]
fn a_configuration_holds_the_scratchs_folders_the_table_given_and_the_system_clock() {
    let scratch = Scratch::new();
    let before = Instant::now();
    let config = scratch.config("[a]\n");
    let now = config.clock.now();
    let after = Instant::now();
    assert_eq!(config.builtin, "[a]\n");
    assert_eq!(
        config.user_file,
        scratch.root.join("data").join("language-servers.toml")
    );
    assert!(
        !config.user_file.exists(),
        "there only when the test writes it"
    );
    assert_eq!(config.servers, scratch.root.join("servers"));
    assert_eq!(config.cache, scratch.root.join("navigator"));
    assert_eq!(config.search_path, [scratch.root.join("bin")]);
    assert_eq!(config.environment, scratch.environment());
    assert_eq!(config.platform, yaar_navigator::Platform::current());
    assert_eq!(config.stop_after, yaar_navigator::STOP_AFTER);
    assert_eq!(config.stop_after, Duration::from_secs(600));
    assert!(before <= now && now <= after, "the system's clock");
}

#[test]
fn left_outside_names_what_a_server_left_beside_the_apps_folders_but_not_cargos_own_files() {
    let scratch = Scratch::new();
    assert!(scratch.left_outside().is_empty());
    let write = |path: &str| {
        let file = scratch.root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "x").unwrap();
    };
    for path in [
        "home/.npm/_cacache/index",
        "cache/typescript/x",
        "config/y",
        "data/z",
        "tmp/ty-1",
        "cargo/registry/index/foo",
    ] {
        write(path);
    }
    // Cargo's own, and the app's.
    for path in [
        "cargo/.package-cache",
        "cargo/.package-cache-mutate",
        "cargo/.global-cache",
        "cargo/registry/CACHEDIR.TAG",
        "servers/standin/1/standin-lsp",
        "navigator/1-1/logs/1.log",
        "repos/r/main.sil",
        "npm.log",
    ] {
        write(path);
    }
    let left: Vec<String> = scratch
        .left_outside()
        .iter()
        .map(|path| {
            path.strip_prefix(&scratch.root)
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    assert_eq!(
        left,
        [
            "cache/typescript/x",
            "cargo/registry/index/foo",
            "config/y",
            "data/z",
            "home/.npm/_cacache/index",
            "tmp/ty-1"
        ]
    );
}

#[test]
fn real_servers_with_its_variable_unset_fails_with_the_sentence() {
    let _scratch = Scratch::new();
    let this_test = "real_servers_with_its_variable_unset_fails_with_the_sentence";
    if std::env::var_os("YAAR_SUPPORT_CHILD").is_some() {
        // The child process: the one call that must fail.
        let _ = real_servers();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", this_test, "--nocapture", "--test-threads=1"])
        .env("YAAR_SUPPORT_CHILD", "1")
        .env_remove(REAL_SERVERS)
        .output()
        .expect("run the test in a process of its own");
    assert!(!output.status.success(), "the test must fail, never skip");
    let words = String::from_utf8_lossy(&output.stderr);
    assert!(
        words.contains(
            "This test needs the real language servers: run `cargo xtask language-servers \
             <folder>` once, and set YAAR_LANGUAGE_SERVERS to that folder. Found: \
             YAAR_LANGUAGE_SERVERS is not set."
        ),
        "{words}"
    );
}

#[test]
fn real_servers_names_what_it_found_when_the_variable_is_empty_or_no_folder() {
    let scratch = Scratch::new();
    assert_eq!(
        real_servers_in(None),
        Err("YAAR_LANGUAGE_SERVERS is not set".to_owned())
    );
    assert_eq!(
        real_servers_in(Some("".into())),
        Err("YAAR_LANGUAGE_SERVERS is not set".to_owned())
    );
    let missing = scratch.root.join("no-such-folder");
    assert_eq!(
        real_servers_in(Some(missing.clone().into())),
        Err(format!("{} is not a folder", missing.display()))
    );
}

/// Waits for the registry (P6) and the downloader (P8): an empty folder holds no pinned
/// version, and the sentence names every server of the table.
#[test]
fn real_servers_in_a_folder_that_lacks_a_pinned_version_names_the_servers() {
    let scratch = Scratch::new();
    let empty = scratch.root.join("servers");
    let found = real_servers_in(Some(empty.clone().into())).expect_err("nothing is pinned there");
    assert!(found.starts_with(&format!("{} lacks the pinned version of ", empty.display())));
    for server in ["tsgo", "ty", "rust-analyzer"] {
        assert!(found.contains(server), "{found}");
    }
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|item| item.as_str().expect("a string"))
        .collect()
}

#[test]
fn a_row_of_the_stand_in_is_a_table_of_one_server_with_what_it_was_asked() {
    let text = standin_row(
        "standin",
        &["--log", "a \"b\" \\ c"],
        "notification",
        "environment = { A = \"1\" }",
    );
    let parsed: toml::Table = text.parse().expect("the row is TOML");
    assert_eq!(parsed.keys().collect::<Vec<_>>(), ["standin"]);
    let row = parsed["standin"].as_table().unwrap();
    assert_eq!(
        row.keys().collect::<Vec<_>>(),
        ["extensions", "markers", "server"]
    );
    assert_eq!(row["extensions"]["sil"].as_str(), Some("standin"));
    assert_eq!(strings(&row["markers"]), ["main.sil"]);
    let servers = row["server"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    let server = servers[0].as_table().unwrap();
    assert_eq!(server["name"].as_str(), Some("standin"));
    assert_eq!(strings(&server["machine"]), ["standin-lsp"]);
    assert_eq!(strings(&server["trial"]), ["--version"]);
    assert_eq!(strings(&server["args"]), ["--log", "a \"b\" \\ c"]);
    assert_eq!(server["environment"]["A"].as_str(), Some("1"));
    assert_eq!(server["ready"]["kind"].as_str(), Some("notification"));
    assert_eq!(server["ready"]["method"].as_str(), Some("standin/status"));
    assert_eq!(server["ready"]["match"]["ready"].as_bool(), Some(true));
    assert!(!server.contains_key("download") && !server.contains_key("command"));
}

#[test]
fn a_row_of_the_stand_in_has_the_other_two_readiness_rules_and_takes_a_download_as_written() {
    for (ready, kind) in [("answer", "answer"), ("progress", "progress")] {
        let text = standin_row("row", &[], ready, "");
        let parsed: toml::Table = text.parse().unwrap();
        let server = &parsed["row"]["server"][0];
        assert_eq!(server["ready"]["kind"].as_str(), Some(kind));
        assert!(strings(&server["args"]).is_empty());
    }
    let download = "\n[[row.server.download]]\nplatform = \"linux-x64\"\nversion = \"1\"\n";
    let parsed: toml::Table = standin_row("row", &[], "answer", download).parse().unwrap();
    let server = &parsed["row"]["server"][0];
    assert_eq!(
        server["download"][0]["platform"].as_str(),
        Some("linux-x64")
    );
    assert_eq!(server["trial"][0].as_str(), Some("--version"));
}

#[test]
fn a_readiness_that_is_none_of_the_three_is_refused() {
    let refused = std::panic::catch_unwind(|| standin_row("row", &[], "soon", ""));
    assert!(refused.is_err());
}

/// Waits for the registry (P6): the row passes through its reading with no error.
#[test]
fn the_registry_reads_a_row_of_the_stand_in_without_complaint() {
    let text = standin_row("standin", &["--ready", "notification"], "notification", "");
    let (registry, errors) = yaar_navigator::Registry::read(&text, None);
    assert!(errors.is_empty(), "{errors:?}");
    let row = registry.row("standin").expect("the row");
    assert_eq!(row.extensions, [("sil".to_owned(), "standin".to_owned())]);
    assert_eq!(row.markers, ["main.sil"]);
}

fn run(program: &Path, args: &[&str]) -> std::process::Output {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run the program")
}

#[test]
fn put_standin_makes_an_executable_copy_that_runs_as_the_stand_in() {
    let scratch = Scratch::new();
    let folder = scratch.root.join("repos").join("some").join("tools");
    let program = scratch.put_standin(&folder, "standin-lsp");
    assert_eq!(program, folder.join("standin-lsp"));
    assert_ne!(program, standin());
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(&program).unwrap().permissions(),
    );
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(
        std::fs::read(&program).unwrap(),
        std::fs::read(standin()).unwrap()
    );
    let output = run(&program, &["--version"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "standin-lsp 1\n");
    // A second copy under another name, in a folder that is there.
    let again = scratch.put_standin(&folder, "mine");
    assert!(again.is_file());
}

#[test]
fn a_failing_script_records_every_run_and_ends_with_its_code() {
    let scratch = Scratch::new();
    // A folder with a space and an apostrophe: the script quotes its paths.
    let folder = scratch.root.join("repos").join("it's here");
    let program = scratch.put_failing(&folder, "standin-broken", 3);
    assert!(runs(&program).is_empty(), "never run, no line");
    assert!(!folder.join("standin-broken.runs").exists());

    assert_eq!(run(&program, &["--version"]).status.code(), Some(3));
    assert_eq!(runs(&program), ["--version"]);
    assert_eq!(
        run(&program, &["--log", "a b", "--ready"]).status.code(),
        Some(3)
    );
    assert_eq!(runs(&program), ["--version", "--log a b --ready"]);
    assert_eq!(run(&program, &[]).status.code(), Some(3));
    assert_eq!(runs(&program), ["--version", "--log a b --ready", ""]);

    let other = scratch.put_failing(&folder, "standin-other", 0);
    assert_eq!(run(&other, &["x"]).status.code(), Some(0));
    assert_eq!(runs(&other), ["x"]);
    assert_eq!(
        runs(&program).len(),
        3,
        "each script records in its own file"
    );
}

#[test]
fn a_recording_script_leaves_a_line_and_runs_the_stand_in_with_the_same_arguments() {
    let scratch = Scratch::new();
    let folder = scratch.root.join("repos").join("it's here").join("tools");
    let program = scratch.put_recording(&folder, "standin-lsp");
    assert!(runs(&program).is_empty());

    let output = run(&program, &["--version"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "standin-lsp 1\n");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(runs(&program), ["--version"]);

    // Its exit is the stand-in's: `--fail-trial --version` ends with 3.
    let broken = run(&program, &["--fail-trial", "--version"]);
    assert_eq!(broken.status.code(), Some(3));
    assert_eq!(runs(&program), ["--version", "--fail-trial --version"]);

    // And it serves, through `exec`: what answers on its pipes is the stand-in.
    let mut through_it = Command::new(&program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = through_it.stdin.take().unwrap();
    write_frame(
        &mut input,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )
    .unwrap();
    let mut output = std::io::BufReader::new(through_it.stdout.take().unwrap());
    let answer = read_frame(&mut output).unwrap().unwrap();
    assert_eq!(answer["result"]["serverInfo"]["name"], "standin-lsp");
    assert_eq!(runs(&program), ["--version", "--fail-trial --version", ""]);
    drop(input);
    assert_eq!(through_it.wait().unwrap().code(), Some(0));
}

#[test]
fn the_ids_of_workspaces_and_repositories_are_a_digit_32_times() {
    assert_eq!(workspace_id('7').as_str(), "7".repeat(32));
    assert_eq!(repository_id('a').as_str(), "a".repeat(32));
    assert_ne!(workspace_id('1'), workspace_id('2'));
}

#[test]
fn a_place_is_the_column_in_bytes_of_the_nth_whole_word_in_the_line() {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "plain",
        &[(
            "main.sil",
            "use é total\ntotal + totals + total\nsubtotal total_2 total\n",
        )],
    );
    let ws = workspace_id('1');
    let in_worktree = |line, nth| place(&repo, &ws, Tree::Worktree, "main.sil", line, "total", nth);

    // The accented letter is two bytes, so the column is 7 and not 6.
    assert_eq!(in_worktree(1, 0).column, 7);
    assert_eq!(in_worktree(2, 0).column, 0);
    assert_eq!(in_worktree(2, 1).column, 17, "`totals` is another name");
    assert_eq!(
        in_worktree(3, 0).column,
        17,
        "neither `subtotal` nor `total_2`"
    );
    let found = in_worktree(2, 1);
    assert_eq!(found.workspace, ws);
    assert_eq!(found.tree, Tree::Worktree);
    assert_eq!(found.path, PathBuf::from("main.sil"));
    assert_eq!(found.line, 2);
    assert_eq!(found.name, "total");
    assert!(found.shown.is_empty());

    for (line, nth) in [(1, 1), (2, 2), (4, 0)] {
        let refused =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| in_worktree(line, nth)));
        assert!(refused.is_err(), "line {line}, occurrence {nth}");
    }
}

#[test]
fn a_place_in_a_version_is_read_from_git_and_in_a_past_version_from_its_text() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "use alpha\nuse beta\n")]);
    std::fs::write(
        repo.root.join("main.sil"),
        "use gamma\nuse alpha\nuse beta\n",
    )
    .unwrap();
    let ws = workspace_id('1');

    let old = place(
        &repo,
        &ws,
        Tree::Version(repo.base.clone()),
        "main.sil",
        1,
        "alpha",
        0,
    );
    assert_eq!(old.column, 4);
    assert_eq!(old.tree, Tree::Version(repo.base.clone()));
    // On disk line 1 is `use gamma`: it holds no `alpha`.
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| place(
            &repo,
            &ws,
            Tree::Worktree,
            "main.sil",
            1,
            "alpha",
            0
        )))
        .is_err()
    );

    let past = Tree::Past {
        version: repo.base.clone(),
        text: "use alpha\nuse beta\n".into(),
    };
    let there = place(&repo, &ws, past.clone(), "main.sil", 2, "beta", 0);
    assert_eq!((there.line, there.column, there.tree), (2, 4, past));

    // A file outside the worktree is named by its absolute path.
    let outside = scratch.root.join("outside.sil");
    std::fs::write(&outside, "x = outside\n").unwrap();
    let far = place(
        &repo,
        &ws,
        Tree::Worktree,
        outside.to_str().unwrap(),
        1,
        "outside",
        0,
    );
    assert_eq!((far.column, far.path), (4, outside));
}

// ---------------------------------------------------------------------------------------
// Running: a test's end of the port.
// ---------------------------------------------------------------------------------------

/// The Frame's end `Running` holds, and the Navigator's end of the same port, played by the
/// test.
fn played() -> (Running, yaar_navigator::PortEnd) {
    let (port, end) = yaar_navigator::port();
    (Running::over(port), end)
}

fn languages() -> Told {
    Told::Languages(vec![yaar_navigator::Language {
        name: "standin".into(),
        title: "Stand-in".into(),
        extensions: vec!["sil".into()],
    }])
}

fn state(workspace: char, language: &str, old: Option<&str>, pid: Option<u32>) -> ServerState {
    ServerState {
        workspace: workspace_id(workspace),
        language: language.to_owned(),
        title: language.to_owned(),
        old_side: old.map(str::to_owned),
        registered: false,
        state: State::Starting,
        server: None,
        program: None,
        pid,
        update: None,
    }
}

fn ready(mut state: ServerState) -> ServerState {
    state.state = State::Ready;
    state
}

fn is_ready(state: &ServerState) -> bool {
    state.state == State::Ready
}

fn panic_text(failure: Box<dyn std::any::Any + Send>) -> String {
    failure
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            failure
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_default()
}

#[test]
fn wait_told_gives_the_first_thing_that_satisfies_and_keeps_everything_told_in_order() {
    let (mut running, end) = played();
    end.told.send_blocking(languages()).unwrap();
    end.told.send_blocking(Told::Servers(vec![])).unwrap();
    end.told
        .send_blocking(Told::Servers(vec![state('1', "a", None, None)]))
        .unwrap();
    end.told.send_blocking(languages()).unwrap();

    let found = running.wait_told(
        "a list with a server",
        DEADLINE,
        |told| matches!(told, Told::Servers(states) if !states.is_empty()),
    );
    assert!(matches!(&found, Told::Servers(states) if states.len() == 1));
    assert_eq!(running.told.len(), 3, "read up to the one wanted, in order");
    assert!(matches!(running.told[0], Told::Languages(_)));
    assert_eq!(running.told[1], Told::Servers(vec![]));
    assert_eq!(running.told[2], found);

    // What was read does not count again; what was not read yet does.
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        running.wait_told(
            "a list with a server again",
            Duration::from_millis(50),
            |told| matches!(told, Told::Servers(states) if !states.is_empty()),
        )
    }))
    .expect_err("the one told before was read already");
    let text = panic_text(failure);
    assert!(text.contains("a list with a server again"), "{text}");
    assert!(
        text.contains("Languages"),
        "everything told meanwhile: {text}"
    );
    assert_eq!(running.told.len(), 4);
}

#[test]
fn wait_told_that_reaches_its_deadline_says_what_it_waited_for_and_what_came() {
    let (mut running, end) = played();
    end.told.send_blocking(Told::Servers(vec![])).unwrap();
    let began = Instant::now();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        running.wait_told(
            "a thing that never comes",
            Duration::from_millis(100),
            |_| false,
        )
    }))
    .expect_err("nothing satisfies it");
    assert!(
        began.elapsed() >= Duration::from_millis(100),
        "it waited the whole time"
    );
    let text = panic_text(failure);
    assert!(text.contains("a thing that never comes"), "{text}");
    assert!(text.contains("Servers"), "{text}");
}

#[test]
fn wait_told_on_a_navigator_that_ended_fails_at_once_and_says_so() {
    let (mut running, end) = played();
    drop(end);
    let began = Instant::now();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        running.wait_told("an answer", DEADLINE, |_| true)
    }))
    .expect_err("the channel is closed");
    assert!(
        began.elapsed() < DEADLINE,
        "no waiting for a Navigator that is gone"
    );
    assert!(panic_text(failure).contains("ended"));
}

#[test]
fn wait_told_wakes_for_a_thing_told_from_another_thread() {
    let (mut running, end) = played();
    let teller = std::thread::spawn(move || {
        end.told.send_blocking(languages()).unwrap();
        end
    });
    let found = running.wait_told("the languages", DEADLINE, |told| {
        matches!(told, Told::Languages(_))
    });
    assert_eq!(found, languages());
    drop(teller.join().unwrap());
}

#[test]
fn wait_state_gives_the_state_of_the_server_asked_for_and_no_other() {
    let (mut running, end) = played();
    let new_side = ready(state('1', "standin", None, Some(7)));
    end.told
        .send_blocking(Told::Servers(vec![state('1', "standin", None, Some(7))]))
        .unwrap();
    end.told
        .send_blocking(Told::Servers(vec![
            ready(state('2', "standin", None, Some(9))),
            ready(state('1', "other", None, Some(10))),
            ready(state('1', "standin", Some("abc"), Some(8))),
            new_side.clone(),
        ]))
        .unwrap();

    let got = running.wait_state(&workspace_id('1'), "standin", None, DEADLINE, is_ready);
    assert_eq!(
        got, new_side,
        "the files on disk of workspace 1, in its row"
    );
    assert_eq!(
        running.told.len(),
        2,
        "the first list had it starting, and did not satisfy"
    );
}

#[test]
fn wait_state_of_an_old_side_waits_for_that_one_and_not_for_the_files_on_disk() {
    let (mut running, end) = played();
    end.told
        .send_blocking(Told::Servers(vec![
            ready(state('1', "standin", None, Some(7))),
            state('1', "standin", Some("abc"), Some(8)),
        ]))
        .unwrap();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        running.wait_state(
            &workspace_id('1'),
            "standin",
            Some("abc"),
            Duration::from_millis(50),
            is_ready,
        )
    }))
    .expect_err("the old side is only starting");
    assert!(panic_text(failure).contains("standin"));

    let old = ready(state('1', "standin", Some("abc"), Some(8)));
    end.told
        .send_blocking(Told::Servers(vec![old.clone()]))
        .unwrap();
    let got = running.wait_state(
        &workspace_id('1'),
        "standin",
        Some("abc"),
        DEADLINE,
        is_ready,
    );
    assert_eq!(got, old);
}

#[test]
fn hover_definition_and_references_ask_with_ids_of_their_own_and_return_the_answer_to_that_id() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "use alpha\n")]);
    let ws = workspace_id('1');
    let (mut running, end) = played();
    let here = || place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0);
    let target = Location {
        path: repo.root.join("main.sil"),
        line: 1,
        column: 4,
        text: "use alpha".into(),
    };

    let teller = {
        let target = target.clone();
        std::thread::spawn(move || {
            let mut ids = Vec::new();
            for _ in 0..3 {
                let (id, told) = match end.asks.recv_blocking().unwrap() {
                    Ask::Hover { id, place } => {
                        assert_eq!(place.name, "alpha");
                        (
                            id,
                            Told::Hover {
                                id,
                                answer: Answered {
                                    value: Some("hover".into()),
                                    guessed: true,
                                },
                            },
                        )
                    }
                    Ask::Definition { id, .. } => (
                        id,
                        Told::Definition {
                            id,
                            answer: Answered {
                                value: vec![target.clone()],
                                guessed: false,
                            },
                        },
                    ),
                    Ask::References { id, .. } => (
                        id,
                        Told::References {
                            id,
                            answer: Answered {
                                value: vec![target.clone(), target.clone()],
                                guessed: true,
                            },
                        },
                    ),
                    other => panic!("not an ask of this test: {other:?}"),
                };
                // An answer to another ask first: it must not be taken for this one's.
                end.told
                    .send_blocking(Told::Hover {
                        id: AskId(id.0 + 1000),
                        answer: Answered {
                            value: None,
                            guessed: false,
                        },
                    })
                    .unwrap();
                end.told.send_blocking(told).unwrap();
                ids.push(id);
            }
            ids
        })
    };
    let hover = running.hover(here(), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("hover".into()),
            guessed: true
        }
    );
    let definition = running.definition(here(), DEADLINE);
    assert_eq!(
        definition,
        Answered {
            value: vec![target.clone()],
            guessed: false
        }
    );
    let references = running.references(here(), DEADLINE);
    assert_eq!(references.value.len(), 2);
    assert!(references.guessed);
    let ids = teller.join().unwrap();
    assert_eq!(ids.len(), 3);
    assert!(
        ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2],
        "{ids:?}"
    );
}

#[test]
fn ask_hands_the_navigator_the_ask_and_show_hands_it_the_workspace_then_the_screen() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "def a\n")]);
    let (running, end) = played();
    running.ask(Ask::OnScreen(None));
    running.show(&repo, &workspace_id('1'), &repository_id('2'));
    assert_eq!(end.asks.recv_blocking().unwrap(), Ask::OnScreen(None));
    assert_eq!(
        end.asks.recv_blocking().unwrap(),
        Ask::Workspaces(vec![yaar_navigator::WorkspaceRoot {
            id: workspace_id('1'),
            repository: repository_id('2'),
            folder: Some(repo.root.clone()),
        }])
    );
    assert_eq!(
        end.asks.recv_blocking().unwrap(),
        Ask::OnScreen(Some(workspace_id('1')))
    );
}

#[test]
fn the_keepers_export_is_the_version_as_plain_files_in_the_folder_asked() {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "plain",
        &[
            ("main.sil", "use alpha\n"),
            ("deep/er/lib.sil", "def alpha\n"),
        ],
    );
    let first = repo.base.clone();
    std::fs::write(repo.root.join("main.sil"), "use beta\n").unwrap();
    std::fs::remove_file(repo.root.join("deep/er/lib.sil")).unwrap();
    repo.git(&["add", "--all"]);
    repo.git(&["commit", "--quiet", "-m", "second"]);

    let (mut running, end) = played();
    let into = scratch.root.join("navigator").join("old").join("1");
    let asked = yaar_navigator::ExportAsk {
        workspace: workspace_id('1'),
        version: first,
        into: into.clone(),
    };
    end.told
        .send_blocking(Told::ExportWanted(asked.clone()))
        .unwrap();
    let given = running.export_when_wanted(&repo, DEADLINE);

    assert_eq!(given, asked);
    assert_eq!(
        end.asks.recv_blocking().unwrap(),
        Ask::Exported {
            asked,
            failed: None
        }
    );
    assert_eq!(
        std::fs::read_to_string(into.join("main.sil")).unwrap(),
        "use alpha\n"
    );
    assert_eq!(
        std::fs::read_to_string(into.join("deep/er/lib.sil")).unwrap(),
        "def alpha\n"
    );
    let mut names: Vec<String> = std::fs::read_dir(&into)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["deep", "main.sil"],
        "no header of the archive is written as a file"
    );
}

#[test]
fn an_export_of_a_version_git_does_not_know_is_answered_as_a_failure_with_its_reason() {
    let scratch = Scratch::new();
    let repo = scratch.repository("plain", &[("main.sil", "def a\n")]);
    let (mut running, end) = played();
    let asked = yaar_navigator::ExportAsk {
        workspace: workspace_id('1'),
        version: "0".repeat(40),
        into: scratch.root.join("navigator").join("old").join("1"),
    };
    end.told
        .send_blocking(Told::ExportWanted(asked.clone()))
        .unwrap();
    running.export_when_wanted(&repo, DEADLINE);
    match end.asks.recv_blocking().unwrap() {
        Ask::Exported {
            asked: got,
            failed: Some(reason),
        } => {
            assert_eq!(got, asked);
            assert!(!reason.is_empty());
        }
        other => panic!("not a failure: {other:?}"),
    }
    assert!(!asked.into.exists(), "nothing was written");
}

#[test]
fn a_process_is_alive_while_it_runs_and_not_once_it_ended_whether_or_not_it_was_waited_for() {
    let _scratch = Scratch::new();
    assert!(is_alive(std::process::id()));

    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start cat");
    let pid = child.id();
    assert!(is_alive(pid));
    drop(child.stdin.take());
    // Its output closes as it ends. What is not yet the end of the process is looked at
    // again, with a short pause: a zombie stays one until it is waited for.
    let mut rest = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut rest).unwrap();
    let until = Instant::now() + DEADLINE;
    while is_alive(pid) {
        assert!(Instant::now() < until, "cat ended and stays alive");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "it had ended, and was waited for only now"
    );
    assert!(!is_alive(pid), "gone");
}

#[test]
fn end_passes_when_every_process_told_is_gone_and_fails_naming_one_that_is_not() {
    let _scratch = Scratch::new();
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("start cat");
    let pid = child.id();

    let (mut running, end) = played();
    end.told
        .send_blocking(Told::Servers(vec![state('1', "standin", None, Some(pid))]))
        .unwrap();
    running.wait_told("the list", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    });
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| running.end()))
        .expect_err("a process told is alive");
    let text = panic_text(failure);
    assert!(text.contains(&pid.to_string()), "{text}");

    drop(child.stdin.take());
    child.wait().unwrap();
    let (mut running, end) = played();
    end.told
        .send_blocking(Told::Servers(vec![state('1', "standin", None, Some(pid))]))
        .unwrap();
    running.wait_told("the list", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    });
    running.end();
}

#[test]
fn end_reads_the_pids_of_what_was_told_and_not_yet_read() {
    let _scratch = Scratch::new();
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("start cat");
    let pid = child.id();
    let (running, end) = played();
    // Told, never read by a wait: `end` still proves this process gone.
    end.told
        .send_blocking(Told::Servers(vec![state(
            '1',
            "standin",
            Some("v"),
            Some(pid),
        )]))
        .unwrap();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| running.end()));
    assert!(failure.is_err());
    drop(child.stdin.take());
    child.wait().unwrap();
}

/// Waits for the lifecycle (P9) and the registry (P6): a Navigator started on the stand-in's
/// row tells its languages, and its end proves every server gone.
#[test]
fn a_started_navigator_tells_its_languages_and_its_end_is_clean() {
    let scratch = Scratch::new();
    let row = standin_row("standin", &[], "answer", "");
    let running = Running::start(scratch.config(&row));
    assert!(matches!(
        &running.told[0],
        Told::Languages(languages) if languages.iter().any(|language| language.name == "standin")
    ));
    running.end();
}

// ---------------------------------------------------------------------------------------
// Loopback and the packing of downloads.
// ---------------------------------------------------------------------------------------

/// The `host:port` a loopback listens on.
fn address_of(loopback: &Loopback) -> String {
    loopback
        .url("")
        .strip_prefix("http://")
        .unwrap()
        .trim_end_matches('/')
        .to_owned()
}

/// One request on a fresh connection, written as given, and everything the listener
/// answered until it closed.
fn exchange(loopback: &Loopback, request: &str) -> Vec<u8> {
    let mut connection = std::net::TcpStream::connect(address_of(loopback))
        .expect("connect to the loopback listener");
    connection.write_all(request.as_bytes()).unwrap();
    let mut answer = Vec::new();
    connection.read_to_end(&mut answer).unwrap();
    answer
}

/// The status line, headers (names lowercased) and body of an answer.
fn split_answer(answer: &[u8]) -> (String, Vec<(String, String)>, Vec<u8>) {
    let at = answer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("an answer has a header");
    let head = String::from_utf8(answer[..at].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap().to_owned();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect();
    (status, headers, answer[at + 4..].to_vec())
}

#[test]
fn a_loopback_serves_a_file_and_keeps_the_request_line_and_the_headers_in_order() {
    let loopback = Loopback::serve(vec![
        ("a.bin".to_owned(), vec![0, 1, 2, 255]),
        ("b.txt".to_owned(), b"second".to_vec()),
    ]);
    assert!(loopback.url("a.bin").starts_with("http://127.0.0.1:"));
    assert!(loopback.url("a.bin").ends_with("/a.bin"));
    assert!(loopback.heard().is_empty());

    let answer = exchange(
        &loopback,
        "GET /a.bin HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: A Test\r\nX-Two:  2 \r\nAccept: */*\r\n\r\n",
    );
    let (status, headers, body) = split_answer(&answer);
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains(&("content-length".to_owned(), "4".to_owned())));
    assert_eq!(body, [0, 1, 2, 255]);

    // Kept before the answer was written: the client has the answer and `heard` holds it.
    assert_eq!(
        loopback.heard(),
        [Heard {
            line: "GET /a.bin".to_owned(),
            headers: vec![
                ("host".to_owned(), "127.0.0.1".to_owned()),
                ("user-agent".to_owned(), "A Test".to_owned()),
                ("x-two".to_owned(), "2".to_owned()),
                ("accept".to_owned(), "*/*".to_owned()),
            ],
        }]
    );
    let second = exchange(&loopback, "GET /b.txt HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(split_answer(&second).2, b"second");
    let lines: Vec<String> = loopback
        .heard()
        .into_iter()
        .map(|heard| heard.line)
        .collect();
    assert_eq!(lines, ["GET /a.bin", "GET /b.txt"]);
}

#[test]
fn a_loopback_answers_a_redirect_with_its_location_as_written_and_any_other_request_404() {
    let loopback = Loopback::serve(vec![("a".to_owned(), b"served".to_vec())]);
    loopback.redirect("a", "/b?x=1");
    loopback.redirect("elsewhere", "http://127.0.0.1:1/c");

    let (status, headers, body) =
        split_answer(&exchange(&loopback, "GET /a HTTP/1.1\r\nHost: x\r\n\r\n"));
    assert_eq!(status, "HTTP/1.1 302 Found");
    assert!(
        headers.contains(&("location".to_owned(), "/b?x=1".to_owned())),
        "{headers:?}"
    );
    assert!(body.is_empty());
    let (_, headers, _) = split_answer(&exchange(&loopback, "GET /elsewhere HTTP/1.1\r\n\r\n"));
    assert!(headers.contains(&("location".to_owned(), "http://127.0.0.1:1/c".to_owned())));

    for request in [
        "GET /nothing HTTP/1.1\r\n\r\n",
        "GET / HTTP/1.1\r\n\r\n",
        "POST /a HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
        "HEAD /a HTTP/1.1\r\n\r\n",
    ] {
        let (status, _, body) = split_answer(&exchange(&loopback, request));
        assert_eq!(status, "HTTP/1.1 404 Not Found", "{request:?}");
        assert!(body.is_empty());
    }
    let lines: Vec<String> = loopback
        .heard()
        .into_iter()
        .map(|heard| heard.line)
        .collect();
    assert_eq!(
        lines,
        [
            "GET /a",
            "GET /elsewhere",
            "GET /nothing",
            "GET /",
            "POST /a",
            "HEAD /a"
        ]
    );
}

#[test]
fn a_loopback_keeps_a_connect_line_and_answers_it_404() {
    let loopback = Loopback::serve(Vec::new());
    let (status, _, _) = split_answer(&exchange(
        &loopback,
        "CONNECT registry.npmjs.org:443 HTTP/1.1\r\nHost: registry.npmjs.org:443\r\n\r\n",
    ));
    assert_eq!(status, "HTTP/1.1 404 Not Found");
    assert_eq!(loopback.heard()[0].line, "CONNECT registry.npmjs.org:443");
}

#[test]
fn a_loopback_that_a_client_holds_open_without_a_request_blocks_no_other_client() {
    let loopback = Loopback::serve(vec![("a".to_owned(), b"x".to_vec())]);
    let _stalled = std::net::TcpStream::connect(address_of(&loopback)).unwrap();
    let (status, _, body) = split_answer(&exchange(&loopback, "GET /a HTTP/1.1\r\n\r\n"));
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, b"x");
    assert_eq!(
        loopback.heard().len(),
        1,
        "a connection with no request is no request"
    );
}

#[test]
fn a_loopback_that_is_dropped_listens_no_more() {
    let loopback = Loopback::serve(Vec::new());
    let address = address_of(&loopback);
    assert!(std::net::TcpStream::connect(&address).is_ok());
    drop(loopback);
    assert!(std::net::TcpStream::connect(&address).is_err());
}

#[test]
fn two_loopbacks_are_two_ports_and_each_hears_its_own() {
    let one = Loopback::serve(vec![("a".to_owned(), b"one".to_vec())]);
    let two = Loopback::serve(vec![("a".to_owned(), b"two".to_vec())]);
    assert_ne!(one.url("a"), two.url("a"));
    assert_eq!(
        split_answer(&exchange(&one, "GET /a HTTP/1.1\r\n\r\n")).2,
        b"one"
    );
    assert_eq!(one.heard().len(), 1);
    assert!(two.heard().is_empty());
}

/// The entries of a `.tar.gz`: raw path, bytes, mode.
fn unpacked(bytes: &[u8]) -> Vec<(String, Vec<u8>, u32)> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("an entry");
            let path = String::from_utf8(entry.path_bytes().into_owned()).unwrap();
            let mode = entry.header().mode().unwrap();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).unwrap();
            (path, content, mode)
        })
        .collect()
}

#[test]
fn tar_gz_packs_each_file_with_its_bytes_and_its_mode_in_order() {
    let archive = tar_gz(&[
        ("standin/standin-lsp", b"#!/bin/sh\n", 0o755),
        ("standin/README", b"", 0o644),
        ("standin/data.bin", &[0, 255, 7], 0o600),
    ]);
    assert_eq!(&archive[..2], [0x1f, 0x8b], "gzip");
    assert_eq!(
        unpacked(&archive),
        [
            (
                "standin/standin-lsp".to_owned(),
                b"#!/bin/sh\n".to_vec(),
                0o755
            ),
            ("standin/README".to_owned(), Vec::new(), 0o644),
            ("standin/data.bin".to_owned(), vec![0, 255, 7], 0o600),
        ]
    );
    assert!(unpacked(&tar_gz(&[])).is_empty());
}

#[test]
fn tar_gz_writes_a_path_as_given_so_that_an_archive_a_downloader_must_refuse_can_be_packed() {
    let long = format!("standin/{}/file", "d".repeat(120));
    let archive = tar_gz(&[
        ("../escape", b"a", 0o644),
        ("/absolute", b"b", 0o644),
        ("standin/../up", b"c", 0o644),
        (&long, b"d", 0o644),
    ]);
    let paths: Vec<String> = unpacked(&archive)
        .into_iter()
        .map(|entry| entry.0)
        .collect();
    assert_eq!(
        paths,
        ["../escape", "/absolute", "standin/../up", long.as_str()]
    );
}

#[test]
fn gz_is_one_gzip_file_of_the_bytes() {
    let bytes: Vec<u8> = (0..=255).cycle().take(5000).collect();
    let packed = gz(&bytes);
    assert_eq!(&packed[..2], [0x1f, 0x8b]);
    assert!(packed.len() < bytes.len());
    let mut back = Vec::new();
    flate2::read::GzDecoder::new(&packed[..])
        .read_to_end(&mut back)
        .unwrap();
    assert_eq!(back, bytes);
    let mut nothing = Vec::new();
    flate2::read::GzDecoder::new(&gz(b"")[..])
        .read_to_end(&mut nothing)
        .unwrap();
    assert!(nothing.is_empty());
}

#[test]
fn sha256_is_the_digest_in_lowercase_hexadecimal() {
    assert_eq!(
        sha256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(sha256(b"other").len(), 64);
    assert_ne!(sha256(b"other"), sha256(b"another"));
}
