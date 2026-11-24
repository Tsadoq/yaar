//! Runs the built `yaar-pi-report` with payloads on stdin, playing the Keeper's end.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use yaar_adapter_pi::{DEADLINE, NAME, TOOL_DEADLINE};
use yaar_link::{
    Fact, Hello, LinkError, Message, PROTOCOL_VERSION, Refusal, Reply, Request, RequestId,
    SOCKET_PATH_MAX, SOCKET_VARIABLE, TERMINAL_VARIABLE, TerminalId, WorkspaceId, read_message,
    socket_address, write_message,
};

/// Slack over a deadline for the program to start and leave on a busy machine.
const MARGIN: Duration = Duration::from_secs(3);

const PATIENCE: Duration = Duration::from_secs(20);

const MANY_FACTS: usize = 200;

const MODES: [(&str, &[&str]); 2] = [("a report", &[]), ("a tool call", &["--tool"])];

struct Scratch {
    folder: PathBuf,
    also: Vec<PathBuf>,
}

impl Scratch {
    /// Under a short path: a socket path has a length limit.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = PathBuf::from(format!("/tmp/yaar-pi-report-{}-{n}", std::process::id()));
        make_private(&folder);
        Self {
            folder,
            also: Vec::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
        for path in &self.also {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn make_private(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
        .expect("make a private folder");
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).expect("set its mode");
}

#[derive(Default)]
struct Heard {
    hello: Option<Hello>,
    requests: Vec<(RequestId, Request)>,
    left_with: Option<LinkError>,
}

#[derive(Clone)]
enum Keeper {
    Answers,
    AnswersWith(Vec<u8>),
    AnswersAndLeaves(Vec<u8>),
    AnswersAfterOtherThings(Vec<u8>),
    AnswersTooLate,
    Refuses,
    Silent,
    Chatty,
    OfAnotherProtocol,
    HangsUp,
}

fn a_fact() -> Message {
    Message::Fact(Fact::ChangesChanged {
        workspace: WorkspaceId::new("w1"),
    })
}

struct Holding {
    answered: Receiver<Instant>,
    release: Sender<()>,
}

struct World {
    scratch: Scratch,
    wanted: PathBuf,
    keeper: Option<JoinHandle<Heard>>,
}

impl World {
    fn new() -> Self {
        let scratch = Scratch::new();
        let wanted = scratch.folder.join("keeper.sock");
        Self {
            scratch,
            wanted,
            keeper: None,
        }
    }

    /// Too long to bind: the program must reach the socket at the short place the Link computes.
    fn with_a_long_path() -> Self {
        let scratch = Scratch::new();
        let base = scratch
            .folder
            .join("a-folder-name-that-makes-the-path-too-long-");
        let missing =
            (SOCKET_PATH_MAX + 1).saturating_sub(base.as_os_str().len() + "/k.sock".len());
        let folder = PathBuf::from(format!("{}{}", base.display(), "x".repeat(missing)));
        make_private(&folder);
        let wanted = folder.join("k.sock");
        assert!(
            socket_address(&wanted) != wanted,
            "the path is too long to bind"
        );
        Self {
            scratch,
            wanted,
            keeper: None,
        }
    }

    fn bind(&mut self) -> UnixListener {
        let address = socket_address(&self.wanted);
        if address != self.wanted {
            let folder = address.parent().expect("the short place has a folder");
            make_private(folder);
            self.scratch.also.push(folder.to_owned());
        }
        UnixListener::bind(&address).expect("bind the Keeper's socket")
    }

    fn keeper(&mut self, behaviour: Keeper) {
        let listener = self.bind();
        self.keeper = Some(thread::spawn(move || serve(&listener, &behaviour)));
    }

    fn keeper_that_holds(&mut self, answer: Reply) -> Holding {
        let listener = self.bind();
        let (answered_to, answered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        self.keeper = Some(thread::spawn(move || {
            let mut heard = Heard::default();
            let Some(mut stream) = accept_within(&listener, PATIENCE) else {
                return heard;
            };
            let Some(id) = greet(&mut stream, &mut heard, false) else {
                return heard;
            };
            let reply = Message::Reply {
                to: id,
                reply: Ok(answer),
            };
            write_message(&mut stream, &reply).expect("answer");
            let _ = answered_to.send(Instant::now());
            // The end stays open, with no option set on it: the program may be gone.
            let _ = released.recv_timeout(PATIENCE);
            heard
        }));
        Holding { answered, release }
    }

    fn stale_socket(&self) {
        drop(UnixListener::bind(&self.wanted).expect("bind a socket"));
    }

    fn heard(&mut self) -> Heard {
        self.keeper
            .take()
            .expect("a Keeper was started")
            .join()
            .expect("the Keeper's thread ends")
    }

    fn run(&self, arguments: &[&str], terminal: Option<&str>, input: &[u8]) -> Ran {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yaar-pi-report"));
        command
            .args(arguments)
            .env_clear()
            .env("HOME", &self.scratch.folder)
            .env("TMPDIR", &self.scratch.folder)
            .env(SOCKET_VARIABLE, &self.wanted)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(terminal) = terminal {
            command.env(TERMINAL_VARIABLE, terminal);
        }
        // Cargo takes the program out of its place for a moment at every build, and tests that
        // build run beside this one: look again.
        let give_up = Instant::now() + Duration::from_secs(5);
        let (mut child, started) = loop {
            let started = Instant::now();
            match command.spawn() {
                Ok(child) => break (child, started),
                Err(why) if why.kind() == ErrorKind::NotFound && Instant::now() < give_up => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(why) => panic!("start the reporting program: {why}"),
            }
        };
        let mut stdin = child.stdin.take().expect("the program's standard input");
        let _ = stdin.write_all(input);
        drop(stdin);
        let output = child.wait_with_output().expect("wait for the program");
        let exited = Instant::now();
        Ran {
            status: output.status,
            printed: output.stdout,
            started,
            exited,
        }
    }
}

struct Ran {
    status: ExitStatus,
    printed: Vec<u8>,
    started: Instant,
    exited: Instant,
}

impl Ran {
    fn took(&self) -> Duration {
        self.exited.duration_since(self.started)
    }

    fn assert_printed(&self, expected: &[u8], what: &str) {
        assert_eq!(self.status.code(), Some(0), "{what}: the program exits 0");
        assert!(
            self.printed == expected,
            "{what}: the program printed {:?}, not {:?}",
            String::from_utf8_lossy(&self.printed),
            String::from_utf8_lossy(expected)
        );
    }

    fn assert_no_word(&self, what: &str) {
        self.assert_printed(b"", what);
    }

    fn assert_error(&self, why: &str, what: &str) {
        let expected = serde_json::to_vec(&serde_json::json!({ "error": why })).expect("json");
        self.assert_printed(&expected, what);
    }
}

fn greet(
    stream: &mut UnixStream,
    heard: &mut Heard,
    of_another_protocol: bool,
) -> Option<RequestId> {
    stream
        .set_read_timeout(Some(PATIENCE))
        .expect("set the Keeper's read timeout");
    let Ok(Message::Hello(hello)) = read_message(stream) else {
        return None;
    };
    let mut mine = Hello::for_this_process("test keeper");
    if of_another_protocol {
        mine.protocol = PROTOCOL_VERSION + 1;
    }
    write_message(stream, &Message::Hello(mine)).expect("answer the hello");
    heard.hello = Some(hello);
    if of_another_protocol {
        return None;
    }
    loop {
        match read_message(stream) {
            Ok(Message::Request { id, request }) => {
                heard.requests.push((id, request));
                return Some(id);
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

fn serve(listener: &UnixListener, behaviour: &Keeper) -> Heard {
    let mut heard = Heard::default();
    let Some(mut stream) = accept_within(listener, PATIENCE) else {
        return heard;
    };
    if matches!(behaviour, Keeper::HangsUp) {
        return heard;
    }
    let of_another_protocol = matches!(behaviour, Keeper::OfAnotherProtocol);
    let id = greet(&mut stream, &mut heard, of_another_protocol);
    if of_another_protocol {
        heard.left_with = until_the_program_leaves(&mut stream, &mut heard);
        return heard;
    }
    let Some(id) = id else {
        return heard;
    };
    let reply = |reply| Message::Reply { to: id, reply };
    match behaviour {
        Keeper::Answers => {
            write_message(&mut stream, &reply(Ok(Reply::Done))).expect("answer");
        }
        Keeper::AnswersWith(bytes) => {
            let answer = Reply::Answer {
                bytes: bytes.clone(),
            };
            write_message(&mut stream, &reply(Ok(answer))).expect("answer");
        }
        Keeper::AnswersAndLeaves(bytes) => {
            // The program reads a frame, then the clock, then the next: with this many it is
            // still reading when the Keeper has long closed.
            for _ in 0..MANY_FACTS {
                write_message(&mut stream, &a_fact()).expect("tell a fact");
            }
            let answer = Reply::Answer {
                bytes: bytes.clone(),
            };
            write_message(&mut stream, &reply(Ok(answer))).expect("answer");
            return heard;
        }
        Keeper::AnswersAfterOtherThings(bytes) => {
            write_message(&mut stream, &a_fact()).expect("tell a fact");
            let other = |n, reply| Message::Reply {
                to: RequestId(id.0 + n),
                reply,
            };
            let not_for_it = Reply::Answer {
                bytes: b"the answer to another request".to_vec(),
            };
            write_message(&mut stream, &other(7, Ok(not_for_it))).expect("answer another");
            let refusal = Refusal::Failed {
                reason: "the refusal of another request".to_owned(),
            };
            write_message(&mut stream, &other(8, Err(refusal))).expect("refuse another");
            let answer = Reply::Answer {
                bytes: bytes.clone(),
            };
            write_message(&mut stream, &reply(Ok(answer))).expect("answer");
        }
        Keeper::AnswersTooLate => {
            heard.left_with = until_the_program_leaves(&mut stream, &mut heard);
            let late = Reply::Answer {
                bytes: b"too late".to_vec(),
            };
            let _ = write_message(&mut stream, &reply(Ok(late)));
            return heard;
        }
        Keeper::Refuses => {
            let refusal = Refusal::Failed {
                reason: "no Adapter of this name".to_owned(),
            };
            write_message(&mut stream, &reply(Err(refusal))).expect("refuse");
        }
        Keeper::Silent => {}
        Keeper::Chatty => {
            let until = Instant::now() + PATIENCE;
            while Instant::now() < until && write_message(&mut stream, &a_fact()).is_ok() {
                thread::sleep(Duration::from_millis(200));
            }
            return heard;
        }
        Keeper::OfAnotherProtocol | Keeper::HangsUp => unreachable!("handled above"),
    }
    heard.left_with = until_the_program_leaves(&mut stream, &mut heard);
    heard
}

/// Holds the connection until the program closes its end, as the real Keeper does. Sets no
/// option: macOS refuses one on a socket whose other end has closed.
fn until_the_program_leaves(stream: &mut UnixStream, heard: &mut Heard) -> Option<LinkError> {
    loop {
        match read_message(stream) {
            Ok(Message::Request { id, request }) => heard.requests.push((id, request)),
            Ok(_) => {}
            Err(why) => return Some(why),
        }
    }
}

fn accept_within(listener: &UnixListener, patience: Duration) -> Option<UnixStream> {
    listener.set_nonblocking(true).expect("poll the listener");
    let until = Instant::now() + patience;
    while Instant::now() < until {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).expect("block on the stream");
                return Some(stream);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("accept failed: {e}"),
        }
    }
    None
}

fn the_one_report(heard: &Heard) -> &yaar_link::AgentReport {
    let [(id, Request::Report { report })] = heard.requests.as_slice() else {
        panic!("the program sends one report, got {:?}", heard.requests);
    };
    assert_eq!(*id, RequestId(0), "the report is request 0");
    report
}

fn a_payload() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event": "tool_call",
        "sessionId": "s1",
        "sessionFile": "/home/u/.pi/agent/sessions/s1.jsonl",
        "cwd": "/work/tree",
        "toolName": "bash",
        "toolCallId": "c1",
        "input": {"command": "echo \"hi\"\n"},
    }))
    .expect("a payload")
}

#[test]
fn a_report_reaches_the_keeper_as_one_request_of_pi_with_its_terminal_its_parent_and_the_payload_as_it_was()
 {
    let payload = a_payload();
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(arguments, Some("t7"), &payload);
        let heard = world.heard();

        let report = the_one_report(&heard);
        assert_eq!(report.agent, NAME, "{what}");
        assert_eq!(report.terminal, Some(TerminalId::new("t7")), "{what}");
        assert_eq!(
            report.pid,
            std::process::id(),
            "{what}: the parent of the program is this test, its caller"
        );
        assert_eq!(report.payload, payload, "{what}: byte for byte");
        let hello = heard.hello.expect("the program says hello first");
        assert_eq!(hello.protocol, PROTOCOL_VERSION, "{what}");
        assert!(
            hello.app_version.starts_with("yaar-pi-report "),
            "{what}: {}",
            hello.app_version
        );
        assert_eq!(ran.status.code(), Some(0), "{what}");
    }
}

#[test]
fn what_the_keeper_answers_is_printed_byte_for_byte_and_done_prints_nothing_without_the_tool_flag()
{
    let answers: [(&str, Vec<u8>); 3] = [
        (
            "a JSON object with quotes, a line break and an accent",
            serde_json::to_vec(&serde_json::json!({"review": "say \"hi\"\n\u{e9}"}))
                .expect("an answer"),
        ),
        ("a lone line end", b"\n".to_vec()),
        (
            "bytes that are no text",
            vec![0xff, 0x00, 0xfe, b'\r', b'\n'],
        ),
    ];
    for (what, arguments) in MODES {
        for (kind, bytes) in &answers {
            let mut world = World::new();
            world.keeper(Keeper::AnswersWith(bytes.clone()));

            let ran = world.run(arguments, Some("t1"), &a_payload());
            world.heard();

            ran.assert_printed(bytes, &format!("{what}: {kind}"));
        }
    }

    let mut world = World::new();
    world.keeper(Keeper::Answers);
    let ran = world.run(&[], Some("t1"), &a_payload());
    world.heard();
    ran.assert_no_word("a report answered with Done");
}

#[test]
fn a_call_of_the_reply_tool_that_the_keeper_answers_with_done_or_refuses_prints_why() {
    let mut world = World::new();
    world.keeper(Keeper::Answers);
    let ran = world.run(&["--tool"], Some("t1"), &a_payload());
    world.heard();
    ran.assert_error("the app did not keep the reply", "Done");

    let mut world = World::new();
    world.keeper(Keeper::Refuses);
    let ran = world.run(&["--tool"], Some("t1"), &a_payload());
    world.heard();
    ran.assert_error("no Adapter of this name", "a refusal");

    let mut world = World::new();
    world.keeper(Keeper::Refuses);
    let ran = world.run(&[], Some("t1"), &a_payload());
    world.heard();
    ran.assert_no_word("a refused report");
}

#[test]
fn with_no_keeper_listening_a_tool_call_says_the_app_is_not_running_and_a_report_says_nothing() {
    let world = World::new();

    let nothing_there = world.run(&[], Some("t1"), &a_payload());
    let tool_nothing_there = world.run(&["--tool"], Some("t1"), &a_payload());
    world.stale_socket();
    let nobody_behind_the_file = world.run(&[], Some("t1"), &a_payload());
    let tool_nobody_behind_the_file = world.run(&["--tool"], Some("t1"), &a_payload());

    nothing_there.assert_no_word("a report, no socket at all");
    nobody_behind_the_file.assert_no_word("a report, a socket file nobody listens at");
    tool_nothing_there.assert_error("the app is not running", "a tool call, no socket at all");
    tool_nobody_behind_the_file.assert_error(
        "the app is not running",
        "a tool call, a socket file nobody listens at",
    );
    for ran in [&nothing_there, &nobody_behind_the_file] {
        assert!(ran.took() < DEADLINE, "it took {:?}", ran.took());
    }
}

#[test]
fn a_keeper_that_hangs_up_leaves_a_report_without_a_word_and_does_not_hold_it() {
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::HangsUp);

        let ran = world.run(arguments, Some("t1"), &a_payload());
        world.heard();

        if arguments.is_empty() {
            ran.assert_no_word(what);
        } else {
            ran.assert_error("the app is not running", what);
        }
        assert!(ran.took() < DEADLINE, "{what}: it took {:?}", ran.took());
    }
}

#[test]
fn a_keeper_of_another_protocol_is_sent_no_request_and_is_not_running_to_the_program() {
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::OfAnotherProtocol);

        let ran = world.run(arguments, Some("t1"), &a_payload());
        let heard = world.heard();

        assert!(heard.hello.is_some(), "{what}: the program did say hello");
        assert_eq!(heard.requests, [], "{what}: nothing was asked of it");
        if arguments.is_empty() {
            ran.assert_no_word(what);
        } else {
            ran.assert_error("the app is not running", what);
        }
        assert!(ran.took() < DEADLINE, "{what}: it took {:?}", ran.took());
    }
}

#[test]
fn the_program_reaches_a_socket_wanted_at_a_path_too_long_to_bind() {
    let mut world = World::with_a_long_path();
    world.keeper(Keeper::AnswersWith(b"{\"review\": \"x\"}".to_vec()));

    let ran = world.run(&[], Some("t1"), &a_payload());
    let heard = world.heard();

    assert_eq!(the_one_report(&heard).payload, a_payload());
    ran.assert_printed(b"{\"review\": \"x\"}", "a report at a long path");
}

#[test]
fn a_program_run_outside_the_apps_terminals_reports_no_terminal() {
    for terminal in [None, Some("")] {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(&[], terminal, &a_payload());
        let heard = world.heard();

        assert_eq!(the_one_report(&heard).terminal, None, "{terminal:?}");
        ran.assert_no_word("a report with no terminal");
    }
}

#[test]
fn a_payload_is_passed_on_unread_whatever_it_holds() {
    let huge = vec![b'x'; 1024 * 1024];
    let payloads: [(&str, &[u8]); 3] = [
        ("empty", b""),
        ("not JSON", b"not json at all \xff\xfe\n"),
        ("a megabyte", &huge),
    ];
    for (what, payload) in payloads {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(&[], Some("t1"), payload);
        let heard = world.heard();

        assert_eq!(the_one_report(&heard).payload, payload, "{what}");
        ran.assert_no_word(what);
    }
}

#[test]
fn a_payload_too_long_for_a_frame_is_left_unsent_and_a_tool_call_says_the_reply_was_not_kept() {
    let too_long = vec![b'x'; 17 * 1024 * 1024];
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(arguments, Some("t1"), &too_long);
        let heard = world.heard();

        assert_eq!(
            heard.requests,
            [],
            "{what}: the Link carries no frame so long"
        );
        if arguments.is_empty() {
            ran.assert_no_word(what);
        } else {
            ran.assert_error("the app did not keep the reply", what);
        }
    }
}

#[test]
fn what_the_keeper_tells_before_its_reply_is_read_past_to_the_reply_to_the_request() {
    let bytes = b"{\"review\": \"its own\"}".to_vec();
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::AnswersAfterOtherThings(bytes.clone()));

        let ran = world.run(arguments, Some("t1"), &a_payload());
        world.heard();

        ran.assert_printed(&bytes, what);
    }
}

#[test]
fn an_answer_that_is_in_the_socket_when_the_keeper_has_gone_is_still_printed() {
    let bytes = b"{\"review\": \"left behind\"}".to_vec();
    for (what, arguments) in MODES {
        let mut world = World::new();
        world.keeper(Keeper::AnswersAndLeaves(bytes.clone()));

        let ran = world.run(arguments, Some("t1"), &a_payload());
        world.heard();

        ran.assert_printed(&bytes, &format!("{what}, read after the Keeper left"));
    }
}

#[test]
fn a_program_that_was_answered_leaves_while_the_keeper_still_holds_its_end_open() {
    let bytes = b"{\"review\": \"the answer\"}".to_vec();
    for (what, arguments) in MODES {
        let mut world = World::new();
        let holding = world.keeper_that_holds(Reply::Answer {
            bytes: bytes.clone(),
        });

        let ran = world.run(arguments, Some("t1"), &a_payload());
        let answered = holding
            .answered
            .recv_timeout(PATIENCE)
            .expect("the Keeper answered before the program left");
        holding.release.send(()).expect("let the Keeper go");
        world.heard();

        ran.assert_printed(&bytes, what);
        let after_the_answer = ran.exited.saturating_duration_since(answered);
        assert!(
            after_the_answer < DEADLINE / 2,
            "{what}: it left {after_the_answer:?} after the answer"
        );
    }
}

#[test]
fn a_report_that_the_keeper_never_answers_holds_the_program_for_the_deadline_and_no_longer() {
    let mut world = World::new();
    world.keeper(Keeper::Silent);

    let ran = world.run(&[], Some("t1"), &a_payload());
    let heard = world.heard();

    assert_eq!(
        the_one_report(&heard).payload,
        a_payload(),
        "the request was sent and waited on"
    );
    ran.assert_no_word("a report nobody answered");
    assert!(
        ran.took() >= DEADLINE,
        "the program waited out its deadline, not less: {:?}",
        ran.took()
    );
    assert!(
        ran.took() < DEADLINE + MARGIN,
        "the program returns within the deadline and a margin: {:?}",
        ran.took()
    );
}

#[test]
fn an_answer_that_comes_after_the_deadline_is_not_printed() {
    let mut world = World::new();
    world.keeper(Keeper::AnswersTooLate);

    let ran = world.run(&[], Some("t1"), &a_payload());
    world.heard();

    ran.assert_no_word("a late answer");
    assert!(
        ran.took() >= DEADLINE,
        "the program waited: {:?}",
        ran.took()
    );
}

#[test]
fn a_keeper_that_keeps_telling_facts_cannot_hold_the_program_past_the_deadline() {
    let mut world = World::new();
    world.keeper(Keeper::Chatty);

    let ran = world.run(&[], Some("t1"), &a_payload());
    world.heard();

    ran.assert_no_word("a report among facts");
    assert!(
        ran.took() < DEADLINE + MARGIN,
        "the deadline runs from the start, not from each read: {:?}",
        ran.took()
    );
}

#[test]
fn a_call_of_the_reply_tool_that_the_keeper_never_answers_waits_ten_seconds_and_says_so() {
    assert_eq!(TOOL_DEADLINE, Duration::from_secs(10));
    let mut world = World::new();
    world.keeper(Keeper::Silent);

    let ran = world.run(&["--tool"], Some("t1"), &a_payload());
    let heard = world.heard();

    assert_eq!(the_one_report(&heard).payload, a_payload());
    ran.assert_error(
        "the app did not answer in time",
        "a tool call nobody answered",
    );
    assert!(
        ran.took() >= TOOL_DEADLINE,
        "the program waited out the longer deadline, not less: {:?}",
        ran.took()
    );
    assert!(
        ran.took() < TOOL_DEADLINE + MARGIN,
        "and no longer than it and a margin: {:?}",
        ran.took()
    );
}
