//! Driver tests: one language server, started from a `Launch`, against the stand-in (`--log`
//! records what it received). Waits are for callback events or log lines, with a deadline; no
//! sleeps. A negative is read after a barrier: an answer to a later request, handed over last.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::*;
use yaar_navigator::driver::{Launch, Server, ServerEvent};
use yaar_navigator::lsp::{file_uri, read_frame};
use yaar_navigator::registry::Ready;

struct Run {
    server: Server,
    events: Receiver<ServerEvent>,
    told: Vec<ServerEvent>,
}

impl Run {
    fn start(launch: Launch) -> Run {
        let (to_test, events) = channel();
        let server = Server::start(
            launch,
            Box::new(move |event| {
                let _ = to_test.send(event);
            }),
        )
        .expect("start the server");
        Run {
            server,
            events,
            told: Vec::new(),
        }
    }

    fn next(&mut self) -> ServerEvent {
        let event = match self.events.recv_timeout(DEADLINE) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => {
                panic!("no event within {DEADLINE:?}; told so far: {:?}", self.told)
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the callback was dropped; told so far: {:?}", self.told)
            }
        };
        self.told.push(event.clone());
        event
    }

    fn ready(&mut self) {
        while self.next() != ServerEvent::Ready {}
    }

    fn answer(&mut self, request: u64) -> Result<Value, String> {
        let read = self.told.iter().find_map(|event| match event {
            ServerEvent::Answered {
                request: id,
                result,
            } if *id == request => Some(result.clone()),
            _ => None,
        });
        if let Some(result) = read {
            return result;
        }
        loop {
            if let ServerEvent::Answered {
                request: id,
                result,
            } = self.next()
                && id == request
            {
                return result;
            }
        }
    }

    fn exited(&mut self) -> Option<i32> {
        loop {
            if let ServerEvent::Exited { code } = self.next() {
                return code;
            }
        }
    }

    fn hover(&mut self, file: &Path, line: u32, character: u32) -> Result<Value, String> {
        let request = self.ask_hover(file, line, character);
        self.answer(request)
    }

    fn ask_hover(&self, file: &Path, line: u32, character: u32) -> u64 {
        self.server
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": {"uri": file_uri(file)},
                    "position": {"line": line, "character": character},
                }),
            )
            .expect("send the request")
    }

    fn text(answer: &Result<Value, String>) -> Option<String> {
        yaar_navigator::lsp::hover_text(answer.as_ref().expect("an answer, not an error"))
    }
}

fn launch(scratch: &Scratch, root: &Path, args: &[&str]) -> Launch {
    Launch {
        program: standin(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        root: root.to_owned(),
        environment: scratch.environment(),
        capabilities: json!({}),
        initialization_options: None,
        settings: json!({}),
        ready: Ready::Answer,
        language_ids: vec![("sil".to_owned(), "standin".to_owned())],
        log: Some(scratch.root.join("stderr.log")),
    }
}

fn project(scratch: &Scratch, text: &str) -> (PathBuf, PathBuf) {
    let root = scratch.root.join("repos").join("project");
    std::fs::create_dir_all(&root).expect("make the project");
    let file = root.join("main.sil");
    std::fs::write(&file, text).expect("write the file");
    (root, file)
}

fn log(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

fn logged(path: &Path, event: &str) -> Option<Value> {
    log(path).into_iter().find(|line| line["event"] == event)
}

fn wait_logged(path: &Path, event: &str) -> Value {
    let end = Instant::now() + DEADLINE;
    loop {
        if let Some(line) = logged(path, event) {
            return line;
        }
        assert!(
            Instant::now() < end,
            "no `{event}` in {} within {DEADLINE:?}; it holds {:?}",
            path.display(),
            log(path)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn notifications(path: &Path) -> Vec<String> {
    log(path)
        .iter()
        .filter(|line| line["event"] == "notification")
        .map(|line| line["method"].as_str().expect("a method").to_owned())
        .collect()
}

fn wait_notified(path: &Path, method: &str) {
    let end = Instant::now() + DEADLINE;
    while !notifications(path).iter().any(|seen| seen == method) {
        assert!(
            Instant::now() < end,
            "no `{method}` in {} within {DEADLINE:?}; it holds {:?}",
            path.display(),
            notifications(path)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `launch` with the stand-in behind `tee`, which copies what the driver sends into `frames`.
fn recorded(scratch: &Scratch, root: &Path, args: &[&str], frames: &Path) -> Launch {
    let mut given = launch(scratch, root, &[]);
    given.program = PathBuf::from("/bin/sh");
    given.args = [
        "-c",
        "/usr/bin/tee \"$0\" | exec \"$@\"",
        frames.to_str().unwrap(),
    ]
    .into_iter()
    .chain(std::iter::once(standin().to_str().unwrap()))
    .chain(args.iter().copied())
    .map(str::to_owned)
    .collect();
    given
}

fn frames_so_far(path: &Path) -> Vec<Value> {
    let mut input = std::io::Cursor::new(std::fs::read(path).unwrap_or_default());
    let mut found = Vec::new();
    while let Ok(Some(frame)) = read_frame(&mut input) {
        found.push(frame);
    }
    found
}

fn wait_frames(path: &Path, enough: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let end = Instant::now() + DEADLINE;
    loop {
        let frames = frames_so_far(path);
        if enough(&frames) {
            return frames;
        }
        assert!(
            Instant::now() < end,
            "the frames in {} never held what was waited for within {DEADLINE:?}: {frames:?}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn methods(frames: &[Value]) -> Vec<&str> {
    frames
        .iter()
        .filter_map(|frame| frame["method"].as_str())
        .collect()
}

#[cfg(target_os = "linux")]
fn is_stopped(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(')')?.1.trim_start().chars().next())
        .is_some_and(|state| state == 'T')
}

#[cfg(not(target_os = "linux"))]
fn is_stopped(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('T')
        })
}

fn freeze(pid: u32) {
    // SAFETY: a signal to a process of this test's own.
    let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
    assert_eq!(sent, 0, "signal the server");
    let end = Instant::now() + DEADLINE;
    while !is_stopped(pid) {
        assert!(Instant::now() < end, "the server did not stop");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn kill(pid: u32) {
    // SAFETY: a signal to a process of this test's own.
    let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    assert_eq!(sent, 0, "signal the server");
}

#[test]
fn a_server_gets_exactly_the_environment_it_is_given() {
    // The variables a server must not see are set before the scratch's own listener thread
    // exists. This leans on nextest, which runs every test as a process of its own: under a
    // runner that shares one process, other tests would spawn processes while this sets them.
    // SAFETY: see above.
    unsafe {
        std::env::set_var("STANDIN_LEAK", "leaked");
        std::env::set_var("XDG_CACHE_HOME", "/nowhere/leaked");
    }
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let dump = scratch.root.join("environment.txt");
    let stand_in_log = scratch.root.join("standin.log");
    let mut given = launch(&scratch, &root, &[]);
    given.program = PathBuf::from("/bin/sh");
    given.args = [
        "-c",
        "/usr/bin/env > \"$1\"; shift; exec \"$@\"",
        "sh",
        dump.to_str().expect("a path"),
        standin().to_str().expect("a path"),
        "--log",
        stand_in_log.to_str().expect("a path"),
    ]
    .map(str::to_owned)
    .to_vec();
    given.environment = vec![
        ("HOME".into(), scratch.root.join("home").into()),
        ("STANDIN_X".into(), "x".into()),
    ];
    let mut run = Run::start(given);
    run.ready();

    let names: Vec<String> = std::fs::read_to_string(&dump)
        .expect("the environment the program saw")
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_owned()))
        .collect();
    assert!(names.contains(&"HOME".to_owned()), "{names:?}");
    assert!(names.contains(&"STANDIN_X".to_owned()), "{names:?}");
    for name in &names {
        assert!(
            ["HOME", "STANDIN_X", "PWD", "OLDPWD", "SHLVL", "_"].contains(&name.as_str()),
            "the server saw {name}, which it was not given: {names:?}"
        );
    }
    let start = wait_logged(&stand_in_log, "start");
    assert_eq!(
        start["env"]["HOME"],
        scratch.root.join("home").to_str().unwrap()
    );
    assert_eq!(start["env"]["STANDIN_X"], "x");
    assert_eq!(start["env"].get("STANDIN_LEAK"), None);
    assert_eq!(start["env"].get("XDG_CACHE_HOME"), None);
    run.server.stop(STOP);
}

#[test]
fn a_server_works_in_the_folder_it_serves_and_is_told_it_in_initialize() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let stand_in_log = scratch.root.join("standin.log");
    let mut run = Run::start(launch(
        &scratch,
        &root,
        &["--log", stand_in_log.to_str().unwrap()],
    ));
    run.ready();

    let start = logged(&stand_in_log, "start").expect("start");
    assert_eq!(start["cwd"], root.to_str().unwrap());
    let initialize = logged(&stand_in_log, "initialize").expect("initialize");
    assert_eq!(initialize["rootUri"], file_uri(&root));
    run.server.stop(STOP);
}

#[test]
fn initialize_carries_the_options_and_the_rows_capabilities_merged_over_the_drivers() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let stand_in_log = scratch.root.join("standin.log");
    let mut given = launch(&scratch, &root, &["--log", stand_in_log.to_str().unwrap()]);
    given.capabilities = json!({
        "experimental": {"flag": true},
        "textDocument": {"hover": {"contentFormat": ["plaintext"]}},
    });
    given.initialization_options = Some(json!({"mode": "strict", "depth": 3}));
    let mut run = Run::start(given);
    run.ready();

    let initialize = logged(&stand_in_log, "initialize").expect("initialize");
    assert_eq!(
        initialize["initializationOptions"],
        json!({"mode": "strict", "depth": 3})
    );
    let capabilities = &initialize["capabilities"];
    assert_eq!(capabilities["experimental"]["flag"], true);
    assert_eq!(
        capabilities["general"]["positionEncodings"],
        json!(["utf-16"])
    );
    assert_eq!(
        capabilities["textDocument"]["hover"]["contentFormat"],
        json!(["plaintext"])
    );
    assert_eq!(
        capabilities["textDocument"]["definition"]["linkSupport"],
        true
    );
    assert_eq!(capabilities["workspace"]["configuration"], true);
    assert_eq!(capabilities["window"]["workDoneProgress"], true);
    run.server.stop(STOP);
}

#[test]
fn a_server_that_asks_for_a_setting_is_answered_with_the_one_of_its_section() {
    let settings = json!({"a": {"b": {"c": 1}}, "other": 2});
    let cases = [
        ("a.b", json!([{"c": 1}])),
        ("a.nothere", json!([null])),
        ("other.deeper", json!([null])),
        ("", json!([settings.clone()])),
    ];
    for (n, (section, expected)) in cases.into_iter().enumerate() {
        let scratch = Scratch::new();
        let (root, _) = project(&scratch, "def a\n");
        let stand_in_log = scratch.root.join(format!("standin-{n}.log"));
        let mut given = launch(
            &scratch,
            &root,
            &[
                "--ask-configuration",
                section,
                "--log",
                stand_in_log.to_str().unwrap(),
            ],
        );
        given.settings = settings.clone();
        let run = Run::start(given);
        let line = wait_logged(&stand_in_log, "configuration");
        assert_eq!(line["result"], expected, "section {section:?}");
        run.server.stop(STOP);
    }
}

fn ready_only_when_its_rule_is_met(kind: &str, rule: Ready) {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def total\n");
    let go = scratch.root.join("go");
    let stand_in_log = scratch.root.join("standin.log");
    let mut given = launch(
        &scratch,
        &root,
        &[
            "--ready",
            kind,
            "--ready-when",
            go.to_str().unwrap(),
            "--ask-configuration",
            "",
            "--log",
            stand_in_log.to_str().unwrap(),
        ],
    );
    given.ready = rule;
    let mut run = Run::start(given);
    wait_logged(&stand_in_log, "configuration");
    let before = run.hover(&file, 0, 5);
    assert_eq!(before, Ok(Value::Null));
    assert!(
        !run.told.contains(&ServerEvent::Ready),
        "told ready before its rule was met: {:?}",
        run.told
    );
    assert!(!run.server.is_ready());

    std::fs::write(&go, "").expect("create the file");
    run.ready();
    assert!(run.server.is_ready());
    let after = run.hover(&file, 0, 5);
    assert_eq!(Run::text(&after), Some("standin: total".to_owned()));
    let readies = run
        .told
        .iter()
        .filter(|event| **event == ServerEvent::Ready)
        .count();
    assert_eq!(readies, 1, "told once: {:?}", run.told);
    run.server.stop(STOP);
}

#[test]
fn a_server_ready_by_a_notification_is_not_ready_before_its_file_exists_and_is_told_once_after() {
    ready_only_when_its_rule_is_met(
        "notification",
        Ready::Notification {
            method: "standin/status".to_owned(),
            matches: serde_json::from_value(json!({"ready": true})).unwrap(),
        },
    );
}

#[test]
fn a_server_ready_by_its_progress_is_not_ready_before_its_file_exists_and_is_told_once_after() {
    ready_only_when_its_rule_is_met("progress", Ready::Progress);
}

#[test]
fn a_server_ready_by_its_answer_is_told_ready_once_after_initialize() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def total\n");
    let never = scratch.root.join("never");
    // The row's rule is `Answer`, which the stand-in does not follow: it says it is not ready and
    // then asks for its configuration. Each follows the `initialize` answer; none may tell `Ready`
    // again.
    let stand_in_log = scratch.root.join("standin.log");
    let mut run = Run::start(launch(
        &scratch,
        &root,
        &[
            "--ready",
            "notification",
            "--ready-when",
            never.to_str().unwrap(),
            "--ask-configuration",
            "",
            "--log",
            stand_in_log.to_str().unwrap(),
        ],
    ));
    run.ready();
    assert!(run.server.is_ready());
    wait_logged(&stand_in_log, "configuration");
    let answer = run.hover(&file, 0, 5);
    assert_eq!(answer, Ok(Value::Null), "the stand-in itself is not ready");
    assert_eq!(
        run.told
            .iter()
            .filter(|e| **e == ServerEvent::Ready)
            .count(),
        1,
        "told once: {:?}",
        run.told
    );
    assert_eq!(run.told[0], ServerEvent::Ready);
    run.server.stop(STOP);
}

#[test]
fn a_hover_is_answered_from_the_text_last_opened_and_by_the_number_of_its_request() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def disk\n");
    let stand_in_log = scratch.root.join("standin.log");
    let mut run = Run::start(launch(
        &scratch,
        &root,
        &["--log", stand_in_log.to_str().unwrap()],
    ));
    run.ready();

    run.server.open(&file, "def opened\n").expect("open");
    let first = run.ask_hover(&file, 0, 5);
    let second = run.ask_hover(&file, 0, 0);
    assert_ne!(first, second, "each request has a number of its own");
    let second_answer = run.answer(second);
    let first_answer = run.answer(first);
    assert_eq!(Run::text(&first_answer), Some("standin: opened".to_owned()));
    assert_eq!(Run::text(&second_answer), Some("standin: def".to_owned()));

    run.server.open(&file, "def changed\n").expect("open again");
    let third = run.hover(&file, 0, 5);
    assert_eq!(Run::text(&third), Some("standin: changed".to_owned()));
    assert_eq!(
        notifications(&stand_in_log),
        [
            "initialized",
            "workspace/didChangeConfiguration",
            "textDocument/didOpen",
            "textDocument/didChange"
        ]
    );
    run.server.stop(STOP);
}

#[test]
fn a_file_of_no_extension_the_row_names_is_refused_before_anything_is_sent() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let stand_in_log = scratch.root.join("standin.log");
    let mut run = Run::start(launch(
        &scratch,
        &root,
        &["--log", stand_in_log.to_str().unwrap()],
    ));
    run.ready();

    let refused = run.server.open(&root.join("notes.txt"), "text");
    assert!(refused.is_err(), "{refused:?}");
    let barrier = run.hover(&root.join("main.sil"), 0, 5);
    assert!(barrier.is_ok());
    assert!(
        !notifications(&stand_in_log).contains(&"textDocument/didOpen".to_owned()),
        "{:?}",
        notifications(&stand_in_log)
    );
    run.server.stop(STOP);
}

#[test]
fn a_request_the_server_refuses_is_answered_with_its_error() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();

    let request = run
        .server
        .request("standin/unknown", json!({}))
        .expect("send");
    let answer = run.answer(request);
    let error = answer.expect_err("an error");
    assert!(error.contains("does not answer"), "{error}");
    run.server.stop(STOP);
}

#[test]
fn a_cancelled_request_has_no_answer_told_and_the_server_is_told_to_cancel() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def total\n");
    let stand_in_log = scratch.root.join("standin.log");
    // The callback holds the reading thread on the first answer until the test lets it go, so the
    // test cancels a request whose answer certainly has not been handed over.
    let (to_test, events) = channel();
    let (hold, held) = channel::<()>();
    let held = Mutex::new(held);
    let armed = std::sync::Arc::new(AtomicBool::new(false));
    let seen_armed = std::sync::Arc::clone(&armed);
    let server = Server::start(
        launch(&scratch, &root, &["--log", stand_in_log.to_str().unwrap()]),
        Box::new(move |event| {
            if matches!(event, ServerEvent::Answered { .. })
                && seen_armed.swap(false, Ordering::SeqCst)
            {
                let _ = held.lock().unwrap().recv();
            }
            let _ = to_test.send(event);
        }),
    )
    .expect("start");
    let mut run = Run {
        server,
        events,
        told: Vec::new(),
    };
    run.ready();

    armed.store(true, Ordering::SeqCst);
    let a = run.server.request("standin/unknown", json!({})).unwrap();
    let b = run.ask_hover(&file, 0, 0);
    let c = run.ask_hover(&file, 0, 5);
    run.server.cancel(b);
    hold.send(()).expect("release the callback");

    assert!(run.answer(a).is_err());
    let answer = run.answer(c);
    assert_eq!(Run::text(&answer), Some("standin: total".to_owned()));
    assert!(
        !run.told.iter().any(|event| matches!(
            event,
            ServerEvent::Answered { request, .. } if *request == b
        )),
        "the answer of a cancelled request was told: {:?}",
        run.told
    );
    wait_notified(&stand_in_log, "$/cancelRequest");
    run.server.stop(STOP);
}

#[test]
fn a_server_that_fails_initialize_is_told_exited_and_never_ready() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &["--fail-initialize"]));
    run.exited();
    assert!(
        !run.told.contains(&ServerEvent::Ready),
        "ready: {:?}",
        run.told
    );
    assert!(!run.server.is_ready());
    assert!(run.server.request("textDocument/hover", json!({})).is_err());
}

#[test]
fn a_server_that_says_it_is_ready_before_its_initialize_fails_is_told_exited_and_never_ready() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    // The shell writes the status frame and only then becomes the stand-in: the frame is in the
    // pipe before the stand-in answers `initialize`, so the driver reads it first.
    let status = json!({"jsonrpc": "2.0", "method": "standin/status", "params": {"ready": true}});
    let with_status_first = |standin_args: &[&str]| {
        let mut given = launch(&scratch, &root, &[]);
        given.ready = Ready::Notification {
            method: "standin/status".to_owned(),
            matches: serde_json::from_value(json!({"ready": true})).unwrap(),
        };
        given.args = [
            "-c",
            "printf 'Content-Length: %d\\r\\n\\r\\n%s' ${#1} \"$1\"; shift; exec \"$0\" \"$@\"",
            standin().to_str().unwrap(),
            &status.to_string(),
        ]
        .into_iter()
        .chain(["--ready", "notification"])
        .chain(standin_args.iter().copied())
        .map(str::to_owned)
        .collect();
        given.program = PathBuf::from("/bin/sh");
        given
    };

    let mut run = Run::start(with_status_first(&["--fail-initialize"]));
    run.exited();
    assert!(
        !run.told.contains(&ServerEvent::Ready),
        "told ready before the server answered initialize: {:?}",
        run.told
    );
    assert!(!run.server.is_ready());

    let mut control = Run::start(with_status_first(&[]));
    control.ready();
    assert!(control.server.is_ready());
    let Run {
        server,
        events,
        mut told,
    } = control;
    server.stop(STOP);
    loop {
        let event = events
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("no `Exited` within {DEADLINE:?}; told: {told:?}"));
        let exited = matches!(event, ServerEvent::Exited { .. });
        told.push(event);
        if exited {
            break;
        }
    }
    assert_eq!(
        told,
        [ServerEvent::Ready, ServerEvent::Exited { code: Some(0) }]
    );
}

#[test]
fn a_server_that_fails_initialize_and_runs_on_is_killed_and_told_exited() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut given = launch(&scratch, &root, &[]);
    given.program = PathBuf::from("/bin/sh");
    given.args = [
        "-c",
        "\"$0\" --fail-initialize; exec sleep 3600",
        standin().to_str().unwrap(),
    ]
    .map(str::to_owned)
    .to_vec();
    let mut run = Run::start(given);
    assert_eq!(run.exited(), None, "told: {:?}", run.told);
    assert!(
        !run.told.contains(&ServerEvent::Ready),
        "ready: {:?}",
        run.told
    );
    assert!(!is_alive(run.server.pid()));
}

#[test]
fn a_server_that_ends_after_ready_is_told_ready_then_exited_with_its_code() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &["--exit-after-ready"]));
    run.ready();
    assert_eq!(run.exited(), Some(2));
    assert_eq!(
        run.told,
        [ServerEvent::Ready, ServerEvent::Exited { code: Some(2) }]
    );
    assert!(!run.server.is_ready());
    assert!(run.server.request("textDocument/hover", json!({})).is_err());
    assert!(run.server.open(&file, "def b\n").is_err());
}

#[test]
fn a_request_still_waiting_when_the_server_dies_is_answered_with_an_error_before_exited() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();
    let pid = run.server.pid();

    freeze(pid);
    let waiting = run.ask_hover(&file, 0, 5);
    kill(pid);

    let ServerEvent::Answered { request, result } = run.next() else {
        panic!("the waiting request was not answered first: {:?}", run.told);
    };
    assert_eq!(request, waiting);
    assert!(result.is_err());
    assert_eq!(run.next(), ServerEvent::Exited { code: None });
}

#[test]
fn stop_ends_the_server_by_shutdown_and_exit_and_returns_with_the_process_reaped() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();
    let pid = run.server.pid();
    assert!(is_alive(pid));

    run.server.stop(STOP);
    // The stand-in ends with 0 only for `exit` after `shutdown`. `Exited` is told before stop
    // returns, so this reads it without waiting.
    let told: Vec<ServerEvent> = run.events.try_iter().collect();
    assert_eq!(told, [ServerEvent::Exited { code: Some(0) }]);
    assert!(!is_alive(pid), "the process still exists after stop");
}

#[test]
fn stop_kills_a_server_that_does_not_end_within_the_time_and_returns_with_it_reaped() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();
    let pid = run.server.pid();

    freeze(pid);
    run.server.stop(Duration::from_millis(50));
    let told: Vec<ServerEvent> = run.events.try_iter().collect();
    assert_eq!(told, [ServerEvent::Exited { code: None }]);
    assert!(!is_alive(pid), "the process still exists after stop");
}

#[test]
fn a_server_that_reads_nothing_stalls_neither_the_caller_nor_the_stop() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();
    let pid = run.server.pid();
    freeze(pid);

    let text = "x".repeat(1024 * 1024);
    let (done, finished) = channel();
    let server = &run.server;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ = done.send((
                server.open(&file, &text),
                server.request("textDocument/hover", json!({})).map(|_| ()),
            ));
        });
        let (opened, asked) = finished
            .recv_timeout(DEADLINE)
            .expect("the caller waited for a server that reads nothing");
        assert_eq!(opened, Ok(()));
        assert_eq!(asked, Ok(()));
    });
    run.server.stop(Duration::from_millis(50));
    assert!(!is_alive(pid));
}

#[test]
fn what_a_server_writes_on_its_standard_error_is_appended_to_the_log_whose_folder_is_made() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let log_file = scratch.root.join("cache").join("logs").join("1.log");
    std::fs::create_dir_all(log_file.parent().unwrap()).unwrap();
    std::fs::write(&log_file, "an earlier line\n").unwrap();
    let deeper = scratch.root.join("cache").join("not-yet").join("2.log");

    for (path, earlier) in [(&log_file, true), (&deeper, false)] {
        let mut given = launch(&scratch, &root, &["--version", "--fail-trial"]);
        given.log = Some(path.clone());
        let mut run = Run::start(given);
        assert_eq!(run.exited(), Some(3));
        let text = std::fs::read_to_string(path).expect("the log");
        assert!(text.contains("standin-lsp: broken"), "{text:?}");
        assert_eq!(text.starts_with("an earlier line\n"), earlier, "{text:?}");
    }
}

#[test]
fn a_program_that_cannot_be_started_is_an_error_that_names_it() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut given = launch(&scratch, &root, &[]);
    given.program = scratch.root.join("bin").join("not-there");
    let error = Server::start(given.clone(), Box::new(|_| {}))
        .err()
        .expect("an error");
    assert!(error.contains("not-there"), "{error}");
}

#[test]
fn the_files_that_changed_are_told_and_an_empty_list_tells_nothing() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def a\n");
    let gone = root.join("gone.sil");
    let frames = scratch.root.join("frames");
    let mut run = Run::start(recorded(&scratch, &root, &[], &frames));
    run.ready();

    run.server.files_changed(&[]);
    run.hover(&file, 0, 5).expect("answer");
    let seen = wait_frames(&frames, |frames| {
        methods(frames).contains(&"textDocument/hover")
    });
    assert!(
        !methods(&seen).contains(&"workspace/didChangeWatchedFiles"),
        "{:?}",
        methods(&seen)
    );

    run.server.files_changed(&[file.clone(), gone.clone()]);
    let seen = wait_frames(&frames, |frames| {
        methods(frames).contains(&"workspace/didChangeWatchedFiles")
    });
    let notices: Vec<&Value> = seen
        .iter()
        .filter(|frame| frame["method"] == "workspace/didChangeWatchedFiles")
        .collect();
    assert_eq!(notices.len(), 1, "one notice for the whole list");
    assert_eq!(
        notices[0]["params"]["changes"],
        json!([
            {"uri": file_uri(&file), "type": 2},
            {"uri": file_uri(&gone), "type": 3},
        ])
    );
    run.server.stop(STOP);
}

#[test]
fn a_file_opened_again_and_again_is_told_at_the_next_version_each_time() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def a\n");
    let other = root.join("other.sil");
    let frames = scratch.root.join("frames");
    let mut run = Run::start(recorded(&scratch, &root, &[], &frames));
    run.ready();

    for text in ["def one\n", "def two\n", "def three\n"] {
        run.server.open(&file, text).expect("open");
    }
    run.server.open(&other, "def x\n").expect("open");
    let seen = wait_frames(&frames, |frames| {
        methods(frames)
            .iter()
            .filter(|method| method.starts_with("textDocument/did"))
            .count()
            == 4
    });
    let told: Vec<&Value> = seen
        .iter()
        .filter(|frame| {
            frame["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("textDocument/did"))
        })
        .collect();
    assert_eq!(
        told[0]["params"],
        json!({"textDocument": {
            "uri": file_uri(&file), "languageId": "standin", "version": 1, "text": "def one\n",
        }})
    );
    for (n, (frame, text)) in told[1..3]
        .iter()
        .zip(["def two\n", "def three\n"])
        .enumerate()
    {
        assert_eq!(frame["method"], "textDocument/didChange");
        assert_eq!(
            frame["params"],
            json!({
                "textDocument": {"uri": file_uri(&file), "version": n + 2},
                "contentChanges": [{"text": text}],
            })
        );
    }
    assert_eq!(told[3]["method"], "textDocument/didOpen");
    assert_eq!(told[3]["params"]["textDocument"]["uri"], file_uri(&other));
    assert_eq!(told[3]["params"]["textDocument"]["version"], 1);
    run.server.stop(STOP);
}

#[test]
fn what_a_caller_sends_before_the_server_is_initialized_follows_initialized() {
    let scratch = Scratch::new();
    let (root, file) = project(&scratch, "def total\n");
    let frames = scratch.root.join("frames");
    let mut run = Run::start(recorded(&scratch, &root, &[], &frames));
    run.server.files_changed(std::slice::from_ref(&file));
    run.server.open(&file, "def early\n").expect("open");
    let early = run.ask_hover(&file, 0, 5);

    let answer = run.answer(early);
    assert_eq!(Run::text(&answer), Some("standin: early".to_owned()));
    let seen = wait_frames(&frames, |frames| {
        methods(frames).contains(&"textDocument/hover")
    });
    assert_eq!(
        methods(&seen),
        [
            "initialize",
            "initialized",
            "workspace/didChangeConfiguration",
            "workspace/didChangeWatchedFiles",
            "textDocument/didOpen",
            "textDocument/hover",
        ]
    );
    run.server.stop(STOP);
}

#[test]
fn a_server_dropped_is_killed_and_reaped() {
    let scratch = Scratch::new();
    let (root, _) = project(&scratch, "def a\n");
    let mut run = Run::start(launch(&scratch, &root, &[]));
    run.ready();
    let pid = run.server.pid();

    let Run { server, events, .. } = run;
    drop(server);
    match events.recv_timeout(DEADLINE) {
        Ok(ServerEvent::Exited { code: None }) => {}
        other => panic!("expected the exit of a process that was killed, got {other:?}"),
    }
    assert!(!is_alive(pid));
}

/// The server ignores `SIGTERM` and, unlike the stand-in, does not end when its input closes
/// (nor does rust-analyzer): that it ends when its Window is killed shows the signal is `SIGKILL`.
#[cfg(target_os = "linux")]
#[test]
fn a_server_ends_when_the_window_that_started_it_is_killed() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let this_test = "a_server_ends_when_the_window_that_started_it_is_killed";
    if std::env::var_os("YAAR_WINDOW_CHILD").is_some() {
        let scratch = Scratch::new();
        let (root, _) = project(&scratch, "def a\n");
        let mut launch = launch(&scratch, &root, &[]);
        let ignoring = scratch.root.join("ignoring");
        let name = std::ffi::CString::new(ignoring.to_str().expect("a path of text"))
            .expect("a path with no NUL");
        // SAFETY: `name` is a NUL-terminated path that lives through the call.
        assert_eq!(
            unsafe { libc::mkfifo(name.as_ptr(), 0o600) },
            0,
            "make the pipe"
        );
        launch.program = PathBuf::from("/bin/sh");
        launch.args = vec![
            "-c".to_owned(),
            format!(
                "trap '' TERM; echo ignoring > '{}'; exec sleep 600",
                ignoring.display()
            ),
        ];
        let run = Run::start(launch);
        let (sent, received) = std::sync::mpsc::channel();
        let pipe = ignoring.clone();
        std::thread::spawn(move || {
            let _ = sent.send(std::fs::read_to_string(pipe));
        });
        let told = received
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| {
                panic!("waited {DEADLINE:?} for the server to say it ignores the signal")
            })
            .expect("read the server's word on the pipe");
        assert_eq!(told.trim(), "ignoring");
        println!("SERVER {} {}", run.server.pid(), scratch.root.display());
        // The child Window waits here to be killed; the end of its input, which a failed test
        // leaves, ends it.
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
        .find_map(|line| line.split_once("SERVER ").map(|(_, said)| said.to_owned()))
        .expect("the Window said which server it started");
    let (pid, scratch) = said.split_once(' ').expect("a process and a folder");
    let pid: u32 = pid.parse().expect("a process number");
    assert!(is_alive(pid), "the server runs while its Window does");

    window.kill().expect("kill the Window");
    window.wait().expect("reap the Window");
    let end = Instant::now() + DEADLINE;
    while is_alive(pid) {
        if Instant::now() >= end {
            kill(pid);
            panic!("the server outlived the Window that started it");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = std::fs::remove_dir_all(scratch);
}

const STOP: Duration = Duration::from_secs(10);
