//! The reply tool's server speaks MCP as Claude Code does and hands the agent's replies to
//! the Keeper whole: the built `yaar-claude-tools` is run, the test writes it the messages
//! of the recorded trace of a Claude Code session, and plays the Keeper's end of the socket
//! to read what the server sent and to answer it.

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_companion_claude::{SERVER_NAME, TOOL, TOOL_DEADLINE};
use yaar_link::{
    AgentReport, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Request, SOCKET_VARIABLE,
    TERMINAL_VARIABLE, TerminalId, read_message, write_message,
};
use yaar_system_tests::recorded::{self, Direction};

/// The recorded trace of Claude Code speaking MCP with a tool server.
const TRACE: &str = "claude/reply-tool-mcp.jsonl";

/// What a test waits for at most when it does not expect to wait at all.
const PATIENCE: Duration = Duration::from_secs(20);

/// The session id the server's environment names.
const SESSION: &str = "s1";

/// A folder of this test's own, which is also the folder the server is started in.
struct Scratch {
    folder: PathBuf,
}

impl Scratch {
    /// A folder for the user alone, under a short path.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = PathBuf::from(format!("/tmp/yaar-tools-{}-{n}", std::process::id()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&folder)
            .expect("make a private folder");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("set its mode");
        Self { folder }
    }

    /// Where the Keeper is wanted.
    fn socket(&self) -> PathBuf {
        self.folder.join("keeper.sock")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// How the test's Keeper answers one report: none holds the connection and says nothing.
type Behaviour = Box<dyn Fn(&AgentReport) -> Option<Result<Reply, Refusal>> + Send + Sync>;

/// The Keeper's end of the socket: it accepts every connection, reads what the server
/// sent, and tells the test each report it got.
struct Keeper {
    reports: Receiver<AgentReport>,
    stop: Arc<AtomicBool>,
    socket: PathBuf,
    accepting: Option<JoinHandle<()>>,
}

impl Keeper {
    /// Binds the Keeper at `socket` and answers each report as `behaviour` says, with a
    /// hello of the protocol `protocol`.
    fn start(socket: &Path, protocol: u16, behaviour: Behaviour) -> Self {
        let listener = UnixListener::bind(socket).expect("bind the Keeper's socket");
        let (tell, reports) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let behaviour = Arc::new(behaviour);
        let stopped = Arc::clone(&stop);
        let accepting = thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let (tell, behaviour) = (tell.clone(), Arc::clone(&behaviour));
                thread::spawn(move || converse(stream, protocol, &tell, &behaviour));
            }
        });
        Self {
            reports,
            stop,
            socket: socket.to_owned(),
            accepting: Some(accepting),
        }
    }

    /// The next report the server sent, within [`PATIENCE`] and no longer.
    fn report(&self) -> AgentReport {
        self.reports
            .recv_timeout(PATIENCE)
            .expect("the server sent a report")
    }

    /// The reports that are there now and have not been taken.
    fn reports_waiting(&self) -> Vec<AgentReport> {
        self.reports.try_iter().collect()
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accepting thread waits in `accept`; a connection wakes it to see the flag.
        let _ = UnixStream::connect(&self.socket);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

/// One connection of the server: its hello, then its report, answered as `behaviour` says.
fn converse(
    mut stream: UnixStream,
    protocol: u16,
    tell: &mpsc::Sender<AgentReport>,
    behaviour: &Behaviour,
) {
    let Ok(Message::Hello(_)) = read_message(&mut stream) else {
        return;
    };
    let mut mine = Hello::for_this_process("test keeper");
    mine.protocol = protocol;
    if write_message(&mut stream, &Message::Hello(mine)).is_err() {
        return;
    }
    while let Ok(message) = read_message(&mut stream) {
        let Message::Request {
            id,
            request: Request::Report { report },
        } = message
        else {
            continue;
        };
        let answer = behaviour(&report);
        let _ = tell.send(report);
        if let Some(reply) = answer {
            let _ = write_message(&mut stream, &Message::Reply { to: id, reply });
        }
    }
}

/// The server, run as Claude Code runs it.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Server {
    /// Starts the built server in `scratch`'s folder with an environment of its own: the
    /// socket wanted, the terminal `t1`, and `session` as Claude Code's session id.
    fn start(scratch: &Scratch, session: Option<&str>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yaar-claude-tools"));
        command
            .env_clear()
            .env("HOME", &scratch.folder)
            .env("TMPDIR", &scratch.folder)
            .env(SOCKET_VARIABLE, scratch.socket())
            .env(TERMINAL_VARIABLE, "t1")
            .current_dir(&scratch.folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(session) = session {
            command.env("CLAUDE_CODE_SESSION_ID", session);
        }
        // Cargo takes the program out of its place for a moment at every build, and tests
        // that build run beside this one: a program that is not there is looked for again.
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(why) if why.kind() == ErrorKind::NotFound && Instant::now() < give_up => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(why) => panic!("start the tool server: {why}"),
            }
        };
        let stdin = child.stdin.take().expect("the server's standard input");
        let stdout = child.stdout.take().expect("the server's standard output");
        let (tell, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tell.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            lines,
        }
    }

    /// Writes `bytes` and a line end to the server.
    fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("the server's input is open");
        stdin.write_all(bytes).expect("write to the server");
        stdin.write_all(b"\n").expect("write to the server");
        stdin.flush().expect("flush to the server");
    }

    /// Writes one JSON-RPC message.
    fn send(&mut self, message: &Value) {
        self.send_raw(message.to_string().as_bytes());
    }

    /// The next line the server wrote, as JSON, within `patience`.
    fn next_within(&self, patience: Duration) -> Value {
        let line = self
            .lines
            .recv_timeout(patience)
            .expect("the server answered");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
    }

    /// The next line the server wrote, as JSON, within [`PATIENCE`].
    fn next(&self) -> Value {
        self.next_within(PATIENCE)
    }

    /// Ends the server's input, as Claude Code does when it stops, and waits for it to go.
    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("wait for the server")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The messages Claude Code sent in the recorded trace, by line.
fn recorded_in(line: usize) -> Value {
    let message = recorded::mcp_messages(TRACE).swap_remove(line);
    assert_eq!(message.direction, Direction::In, "line {line} of the trace");
    message.message
}

/// Line 7 of the trace, a call of the reply tool, with `arguments.body` set to `body`: the
/// one field changed.
fn call_with_body(body: &str) -> Value {
    let mut call = recorded_in(7);
    call["params"]["arguments"]["body"] = Value::String(body.to_owned());
    call
}

/// A result of the kind the Keeper's Adapter writes for a kept reply, for a body of
/// `characters` characters.
fn kept(characters: usize) -> Value {
    json!({"content": [{"type": "text",
        "text": format!("recorded reply to T1 (addressed, {characters} characters)")}]})
}

/// A Keeper that answers every report with a result that names the body's length.
fn keeper_that_keeps(scratch: &Scratch) -> Keeper {
    Keeper::start(
        &scratch.socket(),
        PROTOCOL_VERSION,
        Box::new(|report| {
            let payload: Value = serde_json::from_slice(&report.payload).expect("a JSON payload");
            let body = payload["arguments"]["body"].as_str().unwrap_or_default();
            let bytes = serde_json::to_vec(&kept(body.chars().count())).expect("a result");
            Some(Ok(Reply::Answer { bytes }))
        }),
    )
}

/// A Keeper that answers every report with `reply`.
fn keeper_that_answers(scratch: &Scratch, reply: Result<Reply, Refusal>) -> Keeper {
    Keeper::start(
        &scratch.socket(),
        PROTOCOL_VERSION,
        Box::new(move |_| Some(reply.clone())),
    )
}

/// What an error result of the agent's tool says, when `answer` is one.
fn error_text(answer: &Value) -> String {
    assert_eq!(
        answer["result"]["isError"], true,
        "an error result: {answer}"
    );
    let content = answer["result"]["content"]
        .as_array()
        .expect("the error has content");
    assert_eq!(content.len(), 1, "{answer}");
    assert_eq!(content[0]["type"], "text", "{answer}");
    content[0]["text"]
        .as_str()
        .expect("the error says why")
        .to_owned()
}

#[test]
fn the_server_answers_claude_codes_opening_messages_and_lists_one_tool_that_cannot_resolve() {
    let scratch = Scratch::new();
    let keeper = keeper_that_keeps(&scratch);
    let mut server = Server::start(&scratch, Some(SESSION));

    // Line 0, the probe for a protocol the server does not speak.
    server.send(&recorded_in(0));
    let probe = server.next();
    assert_eq!(probe["id"], recorded_in(0)["id"]);
    assert_eq!(probe["error"]["code"], -32601, "{probe}");
    assert!(probe.get("result").is_none(), "{probe}");

    // Line 2, `initialize`: the version Claude Code asked for, the server's name.
    server.send(&recorded_in(2));
    let initialized = server.next();
    assert_eq!(initialized["jsonrpc"], "2.0");
    assert_eq!(initialized["id"], 0);
    assert_eq!(
        initialized["result"]["protocolVersion"],
        recorded_in(2)["params"]["protocolVersion"]
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], SERVER_NAME);
    assert_eq!(SERVER_NAME, "yaar");
    assert!(
        initialized["result"]["capabilities"]["tools"].is_object(),
        "it says it serves tools: {initialized}"
    );

    // The same line with its one field `params.protocolVersion` changed to a version the
    // server has no reason to name unasked: it comes back as asked, not as a constant.
    let mut other_version = recorded_in(2);
    other_version["params"]["protocolVersion"] = json!("2026-07-28");
    assert_ne!(
        other_version["params"]["protocolVersion"],
        recorded_in(2)["params"]["protocolVersion"]
    );
    server.send(&other_version);
    let asked_again = server.next();
    assert_eq!(asked_again["id"], 0);
    assert_eq!(asked_again["result"]["protocolVersion"], "2026-07-28");

    // Line 4 is a notification: nothing is written for it. The barrier is the answer to
    // the next message, which is the next line the server writes.
    server.send(&recorded_in(4));
    server.send(&recorded_in(5));
    let listed = server.next();
    assert_eq!(listed["id"], 1, "nothing was written for the notification");

    // Line 5, `tools/list`: exactly the one tool, with exactly the three arguments.
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("a list of tools");
    assert_eq!(tools.len(), 1, "{listed}");
    assert_eq!(tools[0]["name"], TOOL);
    assert_eq!(TOOL, "reply_to_thread");
    assert!(
        tools[0]["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "the agent reads what the tool does"
    );
    let schema = &tools[0]["inputSchema"];
    assert_eq!(schema["type"], "object");
    let mut named: Vec<&String> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .collect();
    named.sort();
    assert_eq!(named, ["addressed", "body", "thread"], "no way to resolve");
    assert_eq!(schema["properties"]["addressed"]["type"], "boolean");
    assert_eq!(schema["properties"]["thread"]["type"], "string");
    assert_eq!(schema["properties"]["body"]["type"], "string");
    let mut required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|name| name.as_str().expect("a name"))
        .collect();
    required.sort_unstable();
    assert_eq!(required, ["body", "thread"]);

    // Listing is not a call: the Keeper heard nothing, as the next call shows is the first.
    server.send(&call_with_body("first"));
    server.next();
    let first = keeper.report();
    let payload: Value = serde_json::from_slice(&first.payload).expect("JSON");
    assert_eq!(payload["arguments"]["body"], "first");
    assert!(keeper.reports_waiting().is_empty());
    assert!(server.finish().success(), "the server ends with its input");
}

#[test]
fn a_ping_is_answered_empty_an_unknown_method_is_not_found_and_a_line_that_is_no_json_is_skipped() {
    let scratch = Scratch::new();
    let keeper = keeper_that_keeps(&scratch);
    let mut server = Server::start(&scratch, Some(SESSION));

    server.send_raw(b"this is not json at all \xff\xfe");
    server.send_raw(b"");
    server.send_raw(b"[1, 2]");
    server.send(&json!({"jsonrpc": "2.0", "id": "ping-1", "method": "ping"}));
    let ping = server.next();
    assert_eq!(ping["id"], "ping-1", "a string id comes back as it was");
    assert_eq!(ping["result"], json!({}));

    server.send(&json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}));
    let unknown = server.next();
    assert_eq!(unknown["id"], 9);
    assert_eq!(unknown["error"]["code"], -32601, "{unknown}");

    // A call of a tool the server does not list is an error and reaches no Keeper; the
    // barrier is the answer to a ping sent after it.
    let mut other = call_with_body("x");
    other["params"]["name"] = json!("resolve_thread");
    other["id"] = json!(10);
    server.send(&other);
    let refused = server.next();
    assert_eq!(refused["id"], 10);
    assert!(refused["error"].is_object(), "{refused}");
    server.send(&json!({"jsonrpc": "2.0", "id": 11, "method": "ping"}));
    assert_eq!(server.next()["id"], 11);
    assert!(
        keeper.reports_waiting().is_empty(),
        "nothing reached the Keeper"
    );
}

/// Bodies a shell, a JSON writer or a line reader could each mishandle.
fn hostile_bodies() -> Vec<(&'static str, String)> {
    vec![
        ("both kinds of quote", "say \"hi\" and 'bye'".to_owned()),
        ("backticks", "run `rm -rf /` or ``` fenced ```".to_owned()),
        ("command substitution", "$(touch PWNED_1)".to_owned()),
        ("shell metacharacters", "\"; touch PWNED_2; #".to_owned()),
        ("a line break", "one\ntwo\n\nthree\n".to_owned()),
        (
            "a carriage return and line feed",
            "one\r\ntwo\r\n".to_owned(),
        ),
        ("a trailing backslash", "path\\to\\file\\".to_owned()),
        ("a tab", "a\tb\t\tc".to_owned()),
        (
            "a letter with an accent and an emoji",
            "caf\u{e9} \u{1F980} \u{2028}".to_owned(),
        ),
        ("a body of 70,000 characters", "0123456789".repeat(7_000)),
    ]
}

#[test]
fn reply_bodies_with_quotes_backticks_line_breaks_and_shell_syntax_arrive_byte_for_byte() {
    let scratch = Scratch::new();
    let keeper = keeper_that_keeps(&scratch);
    let mut server = Server::start(&scratch, Some(SESSION));
    server.send(&recorded_in(2));
    server.next();

    for (what, body) in hostile_bodies() {
        server.send(&call_with_body(&body));
        let answer = server.next();

        let report = keeper.report();
        assert_eq!(report.agent, "claude", "{what}");
        assert_eq!(report.terminal, Some(TerminalId::new("t1")), "{what}");
        assert_eq!(
            report.pid,
            std::process::id(),
            "{what}: its parent is this test"
        );
        let payload: Value = serde_json::from_slice(&report.payload)
            .unwrap_or_else(|e| panic!("{what}: the payload is not JSON: {e}"));
        assert_eq!(payload["tool_server"], "yaar", "{what}");
        assert_eq!(payload["session_id"], SESSION, "{what}");
        assert_eq!(payload["name"], TOOL, "{what}");
        // `/tmp` is a link on macOS: the process knows its folder by the real path.
        assert_eq!(
            fs::canonicalize(payload["cwd"].as_str().expect("a folder")).ok(),
            fs::canonicalize(&scratch.folder).ok(),
            "{what}: the folder the server was started in"
        );
        assert_eq!(
            payload["arguments"]["body"].as_str(),
            Some(body.as_str()),
            "{what}: the body, byte for byte"
        );
        assert_eq!(payload["arguments"]["thread"], "T1", "{what}");
        assert_eq!(payload["arguments"]["addressed"], true, "{what}");

        assert_eq!(answer["id"], 2, "{what}");
        assert_eq!(
            answer["result"],
            kept(body.chars().count()),
            "{what}: the result the Keeper handed back, as it came"
        );
    }

    assert!(keeper.reports_waiting().is_empty(), "one report per call");
    let made: Vec<_> = fs::read_dir(&scratch.folder)
        .expect("read the scratch folder")
        .map(|entry| entry.expect("an entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("PWNED"))
        .collect();
    assert!(made.is_empty(), "a body ran as a command: {made:?}");
}

#[test]
fn the_result_the_keeper_hands_back_reaches_the_agent_whole_an_error_result_too() {
    // The kinds of result the Keeper's Adapter writes, and one with a key of its own: the
    // agent is told what the Keeper said, an `isError` of a refused reply above all.
    let results = [
        (
            "a reply that was refused",
            json!({"isError": true, "content": [{"type": "text",
                "text": "thread T9 belongs to the review of another session"}]}),
        ),
        (
            "a result with a key no one planned for",
            json!({"content": [{"type": "text", "text": "kept"}], "isError": false,
                "structuredContent": {"thread": "T1", "kept": true}, "_meta": {"n": 3}}),
        ),
        ("a result with no content", json!({})),
    ];
    for (what, result) in results {
        let scratch = Scratch::new();
        let bytes = serde_json::to_vec(&result).expect("a result");
        let keeper = keeper_that_answers(&scratch, Ok(Reply::Answer { bytes }));
        let mut server = Server::start(&scratch, Some(SESSION));

        server.send(&call_with_body("hello"));
        let answer = server.next();

        keeper.report();
        assert_eq!(answer["id"], 2, "{what}");
        assert_eq!(answer["result"], result, "{what}: as the Keeper gave it");
    }
}

#[test]
fn a_call_names_no_session_when_claude_codes_variable_is_not_set() {
    let scratch = Scratch::new();
    let keeper = keeper_that_keeps(&scratch);
    let mut server = Server::start(&scratch, None);

    server.send(&call_with_body("hello"));
    server.next();

    let payload: Value = serde_json::from_slice(&keeper.report().payload).expect("JSON");
    assert_eq!(payload["session_id"], Value::Null);
}

#[test]
fn with_no_keeper_listening_a_call_is_an_error_the_agent_reads() {
    let scratch = Scratch::new();
    let mut server = Server::start(&scratch, Some(SESSION));

    // No socket at all, then a socket file nobody listens at.
    server.send(&call_with_body("one"));
    let nothing_there = server.next();
    drop(UnixListener::bind(scratch.socket()).expect("bind a socket"));
    server.send(&call_with_body("two"));
    let nobody_behind_the_file = server.next();

    for answer in [nothing_there, nobody_behind_the_file] {
        assert_eq!(answer["id"], 2);
        assert_eq!(error_text(&answer), "the app is not running");
    }
}

#[test]
fn a_keeper_of_another_protocol_is_no_keeper_to_the_server() {
    let scratch = Scratch::new();
    let keeper = Keeper::start(
        &scratch.socket(),
        PROTOCOL_VERSION + 1,
        Box::new(|_| Some(Ok(Reply::Done))),
    );
    let mut server = Server::start(&scratch, Some(SESSION));

    server.send(&call_with_body("hello"));
    let answer = server.next();

    assert_eq!(error_text(&answer), "the app is not running");
    assert!(
        keeper.reports_waiting().is_empty(),
        "no report was sent to it"
    );
}

#[test]
fn a_keeper_that_answers_done_gives_an_error_that_says_the_reply_was_not_kept() {
    let scratch = Scratch::new();
    let keeper = keeper_that_answers(&scratch, Ok(Reply::Done));
    let mut server = Server::start(&scratch, Some(SESSION));

    server.send(&call_with_body("hello"));
    let answer = server.next();

    keeper.report();
    assert_eq!(error_text(&answer), "the app did not keep the reply");
}

#[test]
fn an_answer_that_is_no_result_is_an_error_too() {
    let scratch = Scratch::new();
    let keeper = keeper_that_answers(
        &scratch,
        Ok(Reply::Answer {
            bytes: b"not a result \xff".to_vec(),
        }),
    );
    let mut server = Server::start(&scratch, Some(SESSION));

    server.send(&call_with_body("hello"));
    let answer = server.next();

    keeper.report();
    assert_eq!(error_text(&answer), "the app did not keep the reply");
}

#[test]
fn a_keeper_that_refuses_the_call_gives_its_reason() {
    let scratch = Scratch::new();
    let keeper = keeper_that_answers(
        &scratch,
        Err(Refusal::Failed {
            reason: "thread T9 belongs to the review of another session".to_owned(),
        }),
    );
    let mut server = Server::start(&scratch, Some(SESSION));

    server.send(&call_with_body("hello"));
    let answer = server.next();

    keeper.report();
    assert_eq!(
        error_text(&answer),
        "thread T9 belongs to the review of another session"
    );
}

#[test]
fn a_keeper_that_never_answers_is_waited_for_until_the_tool_deadline_and_no_less() {
    let scratch = Scratch::new();
    let keeper = Keeper::start(&scratch.socket(), PROTOCOL_VERSION, Box::new(|_| None));
    let mut server = Server::start(&scratch, Some(SESSION));

    let started = Instant::now();
    server.send(&call_with_body("hello"));
    let answer = server.next_within(TOOL_DEADLINE + PATIENCE);

    keeper.report();
    assert!(
        started.elapsed() >= TOOL_DEADLINE,
        "the server waited out its deadline: {:?}",
        started.elapsed()
    );
    assert_eq!(error_text(&answer), "the app did not answer in time");
    // The server goes on: the next message is answered.
    server.send(&json!({"jsonrpc": "2.0", "id": 5, "method": "ping"}));
    assert_eq!(server.next()["id"], 5);
}
