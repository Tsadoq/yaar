//! pi's extension, loaded by a stand-in for pi's extension interface under Node: the
//! extension the Adapter installs, the built reporting program, and a Keeper's end of the
//! socket that the test plays. This is the one test of the app that needs Node (22.18 or
//! later, whose type stripping runs the TypeScript extension as it is); on a machine without
//! it every test here fails and says so.
//!
//! The stand-in is `companions/pi/test/harness.ts` (its header holds its protocol) and the
//! module of pi's that the extension imports is `companions/pi/test/pi-ai.mjs`. Nothing of
//! pi is installed and nothing under a real home is read: the harness runs with a home and
//! a temporary folder of the test's own.
//!
//! Every handler of the extension awaits its report to the program's exit, and the Keeper's
//! end hands a report to the test before it answers it. So when the harness has written the
//! result of an event, the Keeper's end has already heard the report of it, and a report
//! that is not there then is a report that was not made.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_adapter_pi::{
    EXTENSION, EXTENSION_FILE, INSTRUCTION, NAME, Pi, REPLY_EVENT, REPLY_TOOL, REPORT_VARIABLE,
    TOOL_DEADLINE,
};
use yaar_agent::{
    Adapter, AgentReply, Answer, Asks, CliSession, Event, Phase, Rendered, Report, Reported,
    ShellEntry, Start, TurnEnd, WriteTool,
};
use yaar_link::{
    AgentReport, Hello, Message, Reply, Request, SOCKET_VARIABLE, TERMINAL_VARIABLE, TerminalId,
    read_message, write_message,
};

/// How long the test waits for a fact it expects, such as the harness starting up or a
/// report arriving: a deadline, never a pause.
const PATIENCE: Duration = Duration::from_secs(10);

/// The terminal of the app the harness runs in.
const TERMINAL: &str = "t1";

/// pi's own id of the session the harness plays.
const SESSION: &str = "s1";

/// The events the extension reports, in the order it registers them.
const REPORTED: [&str; 7] = [
    "session_start",
    "session_shutdown",
    "input",
    "agent_start",
    "tool_call",
    "tool_result",
    "agent_settled",
];

/// The folder of the repository that holds pi's companion.
fn companion() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../companions/pi")
}

/// Fails the test, in a plain sentence, unless Node 22.18 or later is on the search path.
fn need_node() {
    let found = match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        Ok(output) => format!("a node that exits with {}", output.status),
        Err(why) => format!("no program named node on the search path ({why})"),
    };
    let mut numbers = found
        .strip_prefix('v')
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor) = (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
    assert!(
        (major, minor) >= (22, 18),
        "PIX-01 needs Node 22.18 or later on this machine, for the test of pi's extension only: found {found}"
    );
}

/// A folder of the test's own, closed to everybody else, which holds the socket, the home,
/// the project the harness works in and the Adapter's folder of the app; it is removed
/// with the test.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // A short path, for a socket to be bound in; `/tmp` is a link on macOS, and the
        // paths the extension is given are the resolved ones.
        let root = PathBuf::from(format!("/tmp/yaar-pix-{}-{n}", std::process::id()));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .expect("make the scratch folder");
        let root = root.canonicalize().expect("resolve the scratch folder");
        for folder in ["home", "project"] {
            std::fs::create_dir_all(root.join(folder)).expect("make a folder");
        }
        Scratch { root }
    }

    fn socket(&self) -> PathBuf {
        self.root.join("keeper.sock")
    }

    /// A socket nobody listens at.
    fn missing_socket(&self) -> PathBuf {
        self.root.join("nobody.sock")
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// The folder the harness's session works in.
    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    /// The Adapter's own folder of the app, with a space in its name: the extension is
    /// loaded from a path that is not made for a shell to read.
    fn app_folder(&self) -> PathBuf {
        self.root.join("app data").join("agents").join("pi")
    }

    /// The session's own file, which pi would have written.
    fn session_file(&self) -> PathBuf {
        self.root.join("sessions").join("s1.jsonl")
    }

    /// The context of every event the test fires: the session pi would hold.
    fn context(&self) -> Value {
        json!({
            "sessionId": SESSION,
            "sessionFile": self.session_file(),
            "cwd": self.project(),
        })
    }

    /// Whether a file of this name lies anywhere in the scratch folder.
    fn holds_a_file_named(&self, name: &str) -> bool {
        fn walk(folder: &Path, name: &str) -> bool {
            std::fs::read_dir(folder).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    entry.file_name() == name
                        || (entry.path().is_dir() && walk(&entry.path(), name))
                })
            })
        }
        walk(&self.root, name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The folder the Adapter installed the extension into, made as pi finds it: the
/// extension the Adapter wrote, a `package.json` that says its files are modules, and the
/// stand-in for the one module of pi's that it imports where Node looks for it. Gives the
/// extension's file and the reporting program the Adapter named for the shells.
fn install(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let folder = scratch.app_folder();
    let program = Path::new(env!("CARGO_BIN_EXE_yaar-pi-report"));
    let companions = program.parent().expect("the program's folder");
    let entries = Pi::in_home(&scratch.home())
        .install(&folder, companions)
        .expect("install the extension");
    let [ShellEntry::Variable { name, value }] = entries.as_slice() else {
        panic!("the Adapter gives one variable, got {entries:?}");
    };
    assert_eq!(name.to_str(), Some(REPORT_VARIABLE));
    let program = PathBuf::from(value);
    assert!(
        program.is_file(),
        "{} is the reporting program",
        program.display()
    );

    // Node takes a `.ts` file for a module by the nearest `package.json` above it.
    std::fs::write(folder.join("package.json"), "{\"type\": \"module\"}\n").expect("package.json");
    let module = folder.join("node_modules/@earendil-works/pi-ai");
    std::fs::create_dir_all(&module).expect("make the module's folder");
    std::fs::write(
        module.join("package.json"),
        "{\"name\": \"@earendil-works/pi-ai\", \"type\": \"module\", \"exports\": \"./index.mjs\"}\n",
    )
    .expect("the module's package.json");
    std::fs::copy(companion().join("test/pi-ai.mjs"), module.join("index.mjs"))
        .expect("the stand-in for pi-ai");
    (folder.join(EXTENSION).join(EXTENSION_FILE), program)
}

/// What a Keeper's end does with one report.
enum Answering {
    /// Answers `Done`: it asks nothing of the extension.
    Done,
    /// Answers with these bytes.
    Answer(Vec<u8>),
    /// Holds the answer until the gate is opened, then answers `Done`.
    HoldThenDone(Arc<Gate>),
    /// Holds the answer until the program that reported has given up and left, which the
    /// connection shows by ending, then opens the gate and answers `Done` to nobody.
    HoldUntilTheProgramLeaves(Arc<Gate>),
}

/// A gate the test opens when it wants a held answer to go.
struct Gate {
    opened: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn new() -> Arc<Gate> {
        Arc::new(Gate {
            opened: Mutex::new(false),
            changed: Condvar::new(),
        })
    }

    fn open(&self) {
        *self.opened.lock().expect("the gate") = true;
        self.changed.notify_all();
    }

    /// Waits until the gate is open, or the deadline.
    fn wait(&self) {
        self.wait_up_to(PATIENCE);
    }

    /// Waits until the gate is open, or `limit`, and says whether it is open.
    fn wait_up_to(&self, limit: Duration) -> bool {
        let opened = self.opened.lock().expect("the gate");
        let (opened, _) = self
            .changed
            .wait_timeout_while(opened, limit, |opened| !*opened)
            .expect("the gate");
        *opened
    }
}

/// How the Keeper's end decides what to answer a report.
type Policy = Box<dyn FnMut(&AgentReport) -> Answering + Send>;

/// The Keeper's end of the socket: every connection is one report, as the reporting
/// program makes them. A report is given to the test before it is answered.
struct Keeper {
    socket: PathBuf,
    heard: Receiver<AgentReport>,
    policy: Arc<Mutex<Policy>>,
    stopped: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
}

impl Keeper {
    /// Listens at `socket` and answers every report `Done` until told otherwise.
    fn start(socket: &Path) -> Keeper {
        let listener = UnixListener::bind(socket).expect("bind the Keeper's socket");
        let (tell, heard) = mpsc::channel();
        let policy: Arc<Mutex<Policy>> = Arc::new(Mutex::new(Box::new(|_| Answering::Done)));
        let stopped = Arc::new(AtomicBool::new(false));
        let accepting = {
            let (policy, stopped) = (policy.clone(), stopped.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { return };
                    let (policy, tell) = (policy.clone(), tell.clone());
                    // One thread for each connection, so that a held answer holds no other.
                    thread::spawn(move || serve(stream, &policy, &tell));
                }
            })
        };
        Keeper {
            socket: socket.to_owned(),
            heard,
            policy,
            stopped,
            accepting: Some(accepting),
        }
    }

    /// From now on, `policy` decides what each report is answered.
    fn answer_with(&self, policy: impl FnMut(&AgentReport) -> Answering + Send + 'static) {
        *self.policy.lock().expect("the policy") = Box::new(policy);
    }

    /// The next report the Keeper's end heard, in the order it heard them.
    fn next_report(&self) -> AgentReport {
        match self.heard.recv_timeout(PATIENCE) {
            Ok(report) => report,
            Err(why) => panic!("the Keeper's end heard no report: {why}"),
        }
    }

    /// Whether no report is waiting to be read.
    fn hears_nothing(&self) -> bool {
        matches!(self.heard.try_recv(), Err(TryRecvError::Empty))
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        // A connection wakes the thread that waits to accept one.
        let _ = UnixStream::connect(&self.socket);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

/// Serves one connection of the reporting program: the hello, the one report, and its
/// answer.
fn serve(mut stream: UnixStream, policy: &Mutex<Policy>, tell: &mpsc::Sender<AgentReport>) {
    // Set now, while the program is surely there: a socket whose other end has gone
    // refuses an option on macOS.
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let Ok(Message::Hello(_)) = read_message(&mut stream) else {
        return;
    };
    if write_message(
        &mut stream,
        &Message::Hello(Hello::for_this_process("test keeper")),
    )
    .is_err()
    {
        return;
    }
    let (id, report) = loop {
        match read_message(&mut stream) {
            Ok(Message::Request {
                id,
                request: Request::Report { report },
            }) => break (id, report),
            Ok(_) => {}
            Err(_) => return,
        }
    };
    let answering = (policy.lock().expect("the policy"))(&report);
    // Told before the answer: a report that reached the harness's handler is known to the
    // test by the time the handler returns.
    let _ = tell.send(report);
    let bytes = match answering {
        Answering::Done => None,
        Answering::Answer(bytes) => Some(bytes),
        Answering::HoldThenDone(gate) => {
            gate.wait();
            None
        }
        Answering::HoldUntilTheProgramLeaves(gone) => {
            // The program sends nothing more: the read ends when it closes its end, or at
            // the deadline of this thread's own.
            while read_message(&mut stream).is_ok() {}
            gone.open();
            None
        }
    };
    let reply = match bytes {
        Some(bytes) => Reply::Answer { bytes },
        None => Reply::Done,
    };
    let _ = write_message(
        &mut stream,
        &Message::Reply {
            to: id,
            reply: Ok(reply),
        },
    );
}

/// The stand-in for pi's extension interface, started on the installed extension.
struct Harness {
    child: Child,
    stdin: Option<ChildStdin>,
    /// The harness's standard output, one end of a socket pair, read by the test's own
    /// thread, so that "no line yet" is a read that would block and nothing else.
    output: BufReader<UnixStream>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    /// What the harness said it had registered.
    registered: Value,
}

/// What a harness left behind once it ended.
struct Finished {
    status: ExitStatus,
    /// Every line of its standard output after the ones the test read.
    unread: Vec<String>,
    stderr: Vec<u8>,
}

impl Harness {
    /// Starts Node on the harness and the extension, with an environment of the test's own,
    /// `report` being what `YAAR_PI_REPORT` is, none for a variable that is not set, and
    /// waits for the line that says what the extension registered.
    fn start(scratch: &Scratch, extension: &Path, report: Option<&Path>) -> Harness {
        let (ours, theirs) = UnixStream::pair().expect("a socket pair for the harness's output");
        // Set now, while the harness's end is surely open: a socket whose other end has
        // gone refuses an option on macOS.
        ours.set_read_timeout(Some(PATIENCE))
            .expect("a deadline for the harness's output");
        let mut command = Command::new("node");
        command
            // The warning that type stripping is experimental, on the Node versions that
            // still print it, is Node's, not the extension's.
            .arg("--disable-warning=ExperimentalWarning")
            .arg(companion().join("test/harness.ts"))
            .arg(extension)
            .current_dir(scratch.project())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", scratch.home())
            .env("TMPDIR", &scratch.root)
            .env(SOCKET_VARIABLE, scratch.socket())
            .env(TERMINAL_VARIABLE, TERMINAL)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(OwnedFd::from(theirs)))
            .stderr(Stdio::piped());
        if let Some(report) = report {
            command.env(REPORT_VARIABLE, report);
        }
        let mut child = command.spawn().expect("start node");
        // The command holds the harness's end of the socket pair: with it dropped, the
        // harness's own is the only one left, and the test's end reads to the end when
        // the harness ends.
        drop(command);
        let stdin = child.stdin.take();
        let mut stderr = child.stderr.take().expect("the harness's standard error");
        let stderr = Some(thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stderr, &mut bytes);
            bytes
        }));
        let mut harness = Harness {
            child,
            stdin,
            output: BufReader::new(ours),
            stderr,
            registered: Value::Null,
        };
        let first = harness.line();
        harness.registered = first["registered"].clone();
        assert!(
            harness.registered.is_object(),
            "the harness's first line says what was registered: {first}"
        );
        harness
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Writes one line to the harness.
    fn send(&mut self, line: &Value) {
        let stdin = self.stdin.as_mut().expect("the harness's standard input");
        let mut text = line.to_string();
        text.push('\n');
        stdin
            .write_all(text.as_bytes())
            .and_then(|()| stdin.flush())
            .expect("write to the harness");
    }

    /// The next line the harness wrote, as JSON.
    fn line(&mut self) -> Value {
        let mut text = String::new();
        match self.output.read_line(&mut text) {
            Ok(0) => panic!("the harness ended without writing a line"),
            Ok(_) => serde_json::from_str(&text).expect("a line of the harness is JSON"),
            Err(why) => panic!("the harness wrote no line: {why}"),
        }
    }

    /// The next line, if the harness has written one already: a read of its output that
    /// does not wait, by the test's own thread.
    fn line_ready(&mut self) -> Option<Value> {
        self.output
            .get_ref()
            .set_nonblocking(true)
            .expect("a read that does not wait");
        let ready = match self.output.fill_buf() {
            Ok([]) => panic!("the harness ended without writing a line"),
            Ok(_) => true,
            Err(why) if why.kind() == ErrorKind::WouldBlock => false,
            Err(why) => panic!("the harness's output cannot be read: {why}"),
        };
        self.output
            .get_ref()
            .set_nonblocking(false)
            .expect("reads that wait again");
        ready.then(|| self.line())
    }

    /// The next line, if the harness writes one before `by`: a time ahead of the clock.
    fn line_before(&mut self, by: Instant) -> Option<Value> {
        loop {
            if let Some(line) = self.line_ready() {
                return Some(line);
            }
            if Instant::now() >= by {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Plays one event to the handlers and gives what each returned.
    fn fire(&mut self, event: &str, body: Value, context: &Value) -> Vec<Value> {
        self.send_fire(event, body, context);
        let said = self.line();
        said["results"]
            .as_array()
            .unwrap_or_else(|| panic!("firing {event} gives results, got {said}"))
            .clone()
    }

    /// Plays one event and does not wait for the harness to say what became of it.
    fn send_fire(&mut self, event: &str, body: Value, context: &Value) {
        let mut body = body;
        body["type"] = json!(event);
        self.send(&json!({"fire": event, "event": body, "ctx": context}));
    }

    /// Calls the reply tool as pi calls it and gives what the harness said: `{"result"}`
    /// or `{"error"}`.
    fn call(&mut self, params: &Value, context: &Value) -> Value {
        self.send(&json!({"call": REPLY_TOOL, "params": params, "ctx": context}));
        self.line()
    }

    /// Sets, or with none takes away, variables of the harness's own environment.
    fn set_env(&mut self, variables: Value) {
        self.send(&json!({"env": variables}));
        assert_eq!(self.line(), json!({"env": "set"}));
    }

    /// Closes the harness's input, which ends it, and gives what it left.
    fn finish(&mut self) -> Finished {
        drop(self.stdin.take());
        let mut unread = Vec::new();
        loop {
            let mut text = String::new();
            match self.output.read_line(&mut text) {
                Ok(0) => break,
                Ok(_) => unread.push(text.trim_end_matches('\n').to_owned()),
                Err(why) => panic!("the harness did not end: {why}"),
            }
        }
        let status = self.child.wait().expect("wait for the harness");
        let stderr = self
            .stderr
            .take()
            .map(|reader| reader.join().expect("the harness's standard error"))
            .unwrap_or_default();
        Finished {
            status,
            unread,
            stderr,
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // A test that failed leaves no node running.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The scratch folder, the Keeper's end, and the harness on the installed extension.
struct World {
    harness: Harness,
    keeper: Keeper,
    scratch: Scratch,
    program: PathBuf,
}

impl World {
    /// The extension installed, a Keeper's end listening, and the harness started with the
    /// reporting program named, as inside the app.
    fn new() -> World {
        need_node();
        let scratch = Scratch::new();
        let (extension, program) = install(&scratch);
        let keeper = Keeper::start(&scratch.socket());
        let harness = Harness::start(&scratch, &extension, Some(&program));
        World {
            harness,
            keeper,
            scratch,
            program,
        }
    }

    /// The same, with no reporting program named, as outside the app.
    fn outside_the_app() -> World {
        need_node();
        let scratch = Scratch::new();
        let (extension, program) = install(&scratch);
        let keeper = Keeper::start(&scratch.socket());
        let harness = Harness::start(&scratch, &extension, None);
        World {
            harness,
            keeper,
            scratch,
            program,
        }
    }

    fn context(&self) -> Value {
        self.scratch.context()
    }

    /// Plays one event of the session.
    fn fire(&mut self, event: &str, body: Value) -> Vec<Value> {
        let context = self.context();
        self.harness.fire(event, body, &context)
    }
}

/// The payload of a report, as the JSON object it is.
fn payload(report: &AgentReport) -> Value {
    serde_json::from_slice(&report.payload).expect("the payload is JSON")
}

/// The `event` a report names.
fn event_of(report: &AgentReport) -> String {
    payload(report)["event"]
        .as_str()
        .expect("a payload names its event")
        .to_owned()
}

/// What the Adapter makes of a report, as the Keeper hands it over.
fn as_handed(report: &AgentReport) -> Report<'_> {
    Report {
        terminal: report.terminal.as_ref().map(|terminal| terminal.as_str()),
        pid: report.pid,
        payload: &report.payload,
    }
}

/// The event as the Adapter tells it for the harness's session.
fn told(world: &World, event: Event) -> Reported {
    Reported {
        session: CliSession(SESSION.to_owned()),
        terminal: Some(TERMINAL.to_owned()),
        pid: world.harness.pid(),
        folder: Some(world.scratch.project()),
        cli_record: Some(world.scratch.session_file()),
        event,
    }
}

/// A user's message, as pi's `input` event holds it.
fn message(text: &str, streaming: Value) -> Value {
    json!({
        "text": text,
        "images": [],
        "source": "interactive",
        "streamingBehavior": streaming,
    })
}

#[test]
fn the_extension_registers_the_seven_events_and_the_one_reply_tool_and_nothing_for_a_dialog() {
    let world = World::new();

    let registered = &world.harness.registered;
    assert_eq!(registered["events"], json!(REPORTED));
    for dialog in ["project_trust", "before_agent_start"] {
        assert!(
            !registered["events"]
                .as_array()
                .expect("events")
                .contains(&json!(dialog)),
            "{dialog} is for the user to answer or is not the route of the review"
        );
    }
    assert!(
        registered["events"]
            .as_array()
            .expect("events")
            .iter()
            .all(|event| !event.as_str().unwrap_or_default().starts_with("ui_")),
        "no dialog of pi's is handled"
    );

    let tools = registered["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1, "one tool: {tools:?}");
    let tool = &tools[0];
    assert_eq!(tool["name"], REPLY_TOOL);
    assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
    let parameters = &tool["parameters"];
    assert_eq!(parameters["type"], "object");
    // The tool has no way to resolve a thread: the three arguments are all it takes.
    let properties = parameters["properties"].as_object().expect("properties");
    let mut names: Vec<_> = properties.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["addressed", "body", "thread"]);
    assert_eq!(properties["thread"]["type"], "string");
    assert_eq!(properties["body"]["type"], "string");
    assert_eq!(properties["addressed"]["type"], "boolean");
    let mut required: Vec<_> = parameters["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|name| name.as_str().expect("a name"))
        .collect();
    required.sort_unstable();
    assert_eq!(required, ["body", "thread"]);
}

#[test]
fn each_event_of_a_turn_is_reported_in_pi_s_order_and_is_read_as_the_event_of_the_table() {
    let mut world = World::new();
    let project = world.scratch.project();
    let outside = world.scratch.root.join("elsewhere").join("absolute.txt");
    let secret_text = "the user's words, which the Adapter does not read";
    let secret_content = "what a write wrote, which the Adapter does not read";
    let patch = "--- a/src/calc.py\n+++ b/src/calc.py\n@@ -1,2 +1,3 @@\n a\n+b\n c\n@@ -10 +11,2 @@\n-x\n+y\n+z\n";

    let call = |tool: &str, id: &str, input: Value| json!({"toolName": tool, "toolCallId": id, "input": input});
    let result = |tool: &str, id: &str, input: Value, details: Value| json!({"toolName": tool, "toolCallId": id, "input": input, "content": [], "details": details, "isError": false});
    let read = json!({"path": "src/calc.py"});
    let edit =
        json!({"path": "src/calc.py", "edits": [{"oldText": secret_content, "newText": "n"}]});
    let write = json!({"path": outside, "content": secret_content});
    let bash = json!({"command": "echo x > bash_made.txt"});

    let mut message = message(secret_text, json!(null));
    message["images"] = json!([{"type": "image", "data": secret_content}]);
    world.fire("session_start", json!({"reason": "startup"}));
    world.fire("input", message);
    world.fire("agent_start", json!({}));
    world.fire("tool_call", call("read", "c1", read.clone()));
    world.fire("tool_result", result("read", "c1", read, Value::Null));
    world.fire("tool_call", call("edit", "c2", edit.clone()));
    world.fire(
        "tool_result",
        result("edit", "c2", edit, json!({"patch": patch})),
    );
    world.fire("tool_call", call("write", "c3", write.clone()));
    world.fire("tool_result", result("write", "c3", write, Value::Null));
    world.fire("tool_call", call("bash", "c4", bash.clone()));
    world.fire("tool_result", result("bash", "c4", bash, Value::Null));
    world.fire("agent_settled", json!({"aborted": false}));
    world.fire("session_shutdown", json!({"reason": "quit"}));

    let mut heard = Vec::new();
    for _ in 0..13 {
        heard.push(world.keeper.next_report());
    }
    assert!(
        world.keeper.hears_nothing(),
        "one report for each event, and no more"
    );
    let events: Vec<_> = heard.iter().map(event_of).collect();
    assert_eq!(
        events,
        [
            "session_start",
            "input",
            "agent_start",
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "agent_settled",
            "session_shutdown",
        ],
        "the reports come in the order pi gave the events"
    );
    for report in &heard {
        assert_eq!(report.agent, NAME);
        assert_eq!(report.terminal, Some(TerminalId::new(TERMINAL)));
        assert_eq!(
            report.pid,
            world.harness.pid(),
            "the program's parent, the one that reports, is the process of pi"
        );
        let payload = payload(report);
        assert_eq!(payload["sessionId"], SESSION);
        assert_eq!(payload["sessionFile"], json!(world.scratch.session_file()));
        assert_eq!(payload["cwd"], json!(project));
        let text = String::from_utf8_lossy(&report.payload);
        assert!(
            !text.contains(secret_text) && !text.contains(secret_content),
            "what the Adapter does not read is not sent: {text}"
        );
    }

    // The events of the crate doc's table, given to an Adapter in the order they came.
    let mut adapter = Pi::new();
    let told_by_the_adapter: Vec<Reported> = heard
        .iter()
        .flat_map(|report| adapter.report(&as_handed(report)).expect("a report"))
        .collect();
    let command = "echo x > bash_made.txt".to_owned();
    assert_eq!(
        told_by_the_adapter,
        [
            told(&world, Event::SessionStarted { how: Start::New }),
            told(&world, Event::TurnStarted),
            told(
                &world,
                Event::FileRead {
                    path: project.join("src/calc.py")
                }
            ),
            told(
                &world,
                Event::FileWritten {
                    path: project.join("src/calc.py"),
                    tool: WriteTool::Edit,
                    lines: Some(vec![1..=3, 11..=12]),
                }
            ),
            told(
                &world,
                Event::FileWritten {
                    path: outside,
                    tool: WriteTool::Write,
                    lines: None
                }
            ),
            told(
                &world,
                Event::Command {
                    phase: Phase::Began,
                    command: command.clone()
                }
            ),
            told(
                &world,
                Event::Command {
                    phase: Phase::Ended,
                    command
                }
            ),
            told(
                &world,
                Event::TurnEnded {
                    how: TurnEnd::Finished
                }
            ),
            told(&world, Event::SessionEnded),
        ]
    );
}

#[test]
fn a_tool_that_failed_is_reported_as_failed_and_is_no_file_read_or_written() {
    let mut world = World::new();
    let project = world.scratch.project();
    let patch =
        "--- a/src/calc.py\n+++ b/src/calc.py\n@@ -1 +1,2 @@\n a\n+b\n@@ -9 +10,2 @@\n c\n+d\n";
    let failed = |tool: &str, id: &str, input: Value, details: Value| json!({"toolName": tool, "toolCallId": id, "input": input, "content": [], "details": details, "isError": true});
    let worked = |tool: &str, id: &str, input: Value, details: Value| json!({"toolName": tool, "toolCallId": id, "input": input, "content": [], "details": details, "isError": false});
    let read = json!({"path": "src/missing.py"});
    let edit = json!({"path": "src/calc.py", "edits": []});
    let write = json!({"path": "src/new.py", "content": "x"});

    // Each failed result is followed by the same tool's result that worked, so that what
    // the Adapter tells for the second shows that it read the reports at all.
    world.fire(
        "tool_result",
        failed("read", "c1", read.clone(), Value::Null),
    );
    world.fire(
        "tool_result",
        worked("read", "c2", json!({"path": "src/calc.py"}), Value::Null),
    );
    world.fire(
        "tool_result",
        failed("edit", "c3", edit.clone(), json!({"patch": patch})),
    );
    world.fire(
        "tool_result",
        worked("edit", "c4", edit, json!({"patch": patch})),
    );
    world.fire(
        "tool_result",
        failed("write", "c5", write.clone(), Value::Null),
    );
    world.fire("tool_result", worked("write", "c6", write, Value::Null));
    let heard: Vec<_> = (0..6).map(|_| world.keeper.next_report()).collect();

    let says_failed: Vec<_> = heard
        .iter()
        .map(|report| payload(report)["isError"].clone())
        .collect();
    assert_eq!(
        says_failed,
        [true, false, true, false, true, false].map(Value::Bool),
        "a result says whether the tool failed"
    );
    let mut adapter = Pi::new();
    let told_by_the_adapter: Vec<Event> = heard
        .iter()
        .flat_map(|report| adapter.report(&as_handed(report)).expect("a report"))
        .map(|reported| reported.event)
        .collect();
    assert_eq!(
        told_by_the_adapter,
        [
            Event::FileRead {
                path: project.join("src/calc.py")
            },
            Event::FileWritten {
                path: project.join("src/calc.py"),
                tool: WriteTool::Edit,
                lines: Some(vec![1..=2, 10..=11]),
            },
            Event::FileWritten {
                path: project.join("src/new.py"),
                tool: WriteTool::Write,
                lines: None,
            },
        ],
        "a failed read is no file read, a failed edit or write no file written"
    );
}

#[test]
fn a_session_pi_keeps_in_no_file_is_reported_with_a_null_file() {
    let mut world = World::new();
    let mut context = world.context();
    context["sessionFile"] = Value::Null;

    world.harness.fire("agent_start", json!({}), &context);

    let report = world.keeper.next_report();
    assert_eq!(payload(&report)["sessionFile"], Value::Null);
    let events = Pi::new().report(&as_handed(&report)).expect("a report");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cli_record, None);
}

#[test]
fn a_handler_returns_only_after_the_app_answered_its_report() {
    let mut world = World::new();
    let gate = Gate::new();
    let held = gate.clone();
    world.keeper.answer_with(move |report| {
        if payload(report)["toolName"] == "bash" {
            Answering::HoldThenDone(held.clone())
        } else {
            Answering::Done
        }
    });
    let context = world.context();

    world.harness.send_fire(
        "tool_call",
        json!({"toolName": "bash", "toolCallId": "c1", "input": {"command": "true"}}),
        &context,
    );
    // The report is at the Keeper's end, which holds its answer: a handler that had not
    // waited for it would have returned long before the program was started.
    let report = world.keeper.next_report();
    assert_eq!(event_of(&report), "tool_call");
    assert!(
        world.harness.line_ready().is_none(),
        "the handler returned before the report was answered"
    );

    gate.open();
    assert_eq!(world.harness.line(), json!({"results": [null]}));
}

#[test]
fn reports_that_are_made_together_reach_the_app_one_at_a_time_in_order() {
    let mut world = World::new();
    // Opened when the program of the first report has left: the connection of the first
    // report has ended.
    let first_has_left = Gate::new();
    let first = first_has_left.clone();
    // Whether the first report's program had left when the second report arrived.
    let seen_leaving = Arc::new(AtomicBool::new(false));
    let seen = seen_leaving.clone();
    world.keeper.answer_with(move |report| {
        match event_of(report).as_str() {
            "session_start" => Answering::HoldUntilTheProgramLeaves(first.clone()),
            "agent_start" => {
                // The program leaves before the next one starts, so the look is at a fact
                // that is already true; the short wait is for this thread's own lag.
                seen.store(first.wait_up_to(Duration::from_secs(1)), Ordering::SeqCst);
                Answering::Done
            }
            _ => Answering::Done,
        }
    });
    let context = world.context();
    let started = json!({"fire": "session_start", "event": {"type": "session_start", "reason": "startup"}, "ctx": context});
    let begun = json!({"fire": "agent_start", "event": {"type": "agent_start"}, "ctx": context});
    world
        .harness
        .send(&json!({"fireTogether": [started, begun]}));

    // The first is held, and its program gives up by itself at its short deadline. The
    // second report is not started until it has: when it arrives, the first's connection
    // has ended. Reports run at the same time would have the second here within moments
    // of the first, while the first's connection was still open.
    assert_eq!(event_of(&world.keeper.next_report()), "session_start");
    assert_eq!(event_of(&world.keeper.next_report()), "agent_start");
    assert!(
        seen_leaving.load(Ordering::SeqCst),
        "a report was started before the one before it ended"
    );
    assert_eq!(world.harness.line(), json!({"results": [[null], [null]]}));
}

#[test]
fn an_ordinary_report_is_given_up_after_the_short_deadline_and_only_the_reply_tool_waits_the_long_one()
 {
    let mut world = World::new();
    let gate = Gate::new();
    let held = gate.clone();
    world
        .keeper
        .answer_with(move |_| Answering::HoldThenDone(held.clone()));
    let context = world.context();
    // Well short of the long deadline, well past the short one, and ahead of the clock.
    let by = Instant::now() + TOOL_DEADLINE / 2;

    world.harness.send_fire("agent_start", json!({}), &context);

    assert_eq!(event_of(&world.keeper.next_report()), "agent_start");
    let said = world.harness.line_before(by);
    // The app has still not answered when pi is let go: the program gave up by itself.
    gate.open();
    assert_eq!(
        said,
        Some(json!({"results": [null]})),
        "pi waited for an answer the app did not give past the short deadline"
    );
}

/// A text of the characters a review holds, after the instruction and a blank line: some
/// of two and of three bytes, some outside the basic plane, quotes, backslashes, line
/// breaks and a control.
fn review(characters: usize) -> String {
    let filler = "## Thread t1\n\nComment: \"naïve\" `x` \\ → 𝒳 😀 日本語\u{1}\t\r\n";
    let mut text = String::from(INSTRUCTION);
    text.push_str("\n\n");
    text.extend(filler.chars().cycle().take(characters));
    text
}

/// The bytes the Adapter answers an `input` report with when a review is attached: what
/// the Keeper's end gives back for that report.
fn handing_out(report: &AgentReport, folder: &Path, review: &str) -> Vec<u8> {
    let adapter = Pi::new();
    let asked = adapter
        .asked(folder, &as_handed(report))
        .expect("the report is readable")
        .expect("an input asks");
    adapter
        .delivery(folder, &asked)
        .hand_out(&Rendered {
            text: review,
            review: "R",
            round: 1,
            threads: 3,
        })
        .expect("the review is handed out")
}

#[test]
fn a_review_of_one_and_a_half_million_characters_goes_whole_into_the_users_message_with_the_instruction()
 {
    let mut world = World::new();
    let review = review(1_500_000);
    let folder = world.scratch.project();
    let attached = review.clone();
    world.keeper.answer_with(move |report| {
        if event_of(report) == "input" {
            Answering::Answer(handing_out(report, &folder, &attached))
        } else {
            Answering::Done
        }
    });
    let mut message = message("please look at the comments", json!(null));
    message["images"] = json!([{"type": "image", "data": "AAAA", "mimeType": "image/png"}]);
    let images = message["images"].clone();

    let results = world.fire("input", message);

    let [returned] = results.as_slice() else {
        panic!("one handler of input answered, got {results:?}");
    };
    assert_eq!(returned["action"], "transform");
    let mut keys: Vec<_> = returned.as_object().expect("an object").keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, ["action", "images", "text"]);
    assert_eq!(returned["images"], images, "the user's images go on");
    let text = returned["text"].as_str().expect("the new text");
    let expected = format!("please look at the comments\n\n{review}");
    // Not printed whole when it fails: it is a million and a half characters long.
    assert!(
        text == expected,
        "the user's text, a blank line and the review, whole: got {} characters, expected {}",
        text.chars().count(),
        expected.chars().count()
    );
    assert!(
        text.contains(INSTRUCTION) && INSTRUCTION.contains(REPLY_TOOL),
        "the instruction on how to answer goes with it"
    );
}

#[test]
fn a_message_the_app_hands_no_review_for_goes_on_as_the_user_wrote_it() {
    let mut world = World::new();
    let answers: [Answering; 4] = [
        Answering::Done,
        Answering::Answer(b"this is not JSON".to_vec()),
        Answering::Answer(b"{\"review\": 5}".to_vec()),
        Answering::Answer(Vec::new()),
    ];
    for answering in answers {
        let mut once = Some(answering);
        world
            .keeper
            .answer_with(move |_| once.take().unwrap_or(Answering::Done));

        let results = world.fire("input", message("hello", json!(null)));

        assert_eq!(results, [json!({"action": "continue"})]);
        world.keeper.next_report();
    }
}

#[test]
fn the_report_of_a_message_asks_for_a_review_whether_pi_is_idle_or_queues_the_message() {
    let mut world = World::new();
    world.fire("session_start", json!({"reason": "startup"}));
    world.fire("input", message("idle", json!(null)));
    world.fire("agent_settled", json!({"aborted": false}));
    // A run is open from here: the next two are what the user types while pi works.
    world.fire("agent_start", json!({}));
    world.fire("input", message("steered", json!("steer")));
    world.fire("input", message("followed up", json!("followUp")));
    let heard: Vec<_> = (0..6).map(|_| world.keeper.next_report()).collect();

    let mut adapter = Pi::new();
    let folder = world.scratch.project();
    let mut asks = Vec::new();
    let mut turns_started = 0;
    for report in &heard {
        let events = adapter.report(&as_handed(report)).expect("a report");
        turns_started += events
            .iter()
            .filter(|reported| reported.event == Event::TurnStarted)
            .count();
        if event_of(report) == "input" {
            let asked = adapter
                .asked(&folder, &as_handed(report))
                .expect("readable")
                .expect("a message asks");
            asks.push((payload(report)["streamingBehavior"].clone(), asked.asks));
        }
    }

    assert_eq!(
        asks,
        [
            (json!(null), Asks::Message),
            (json!("steer"), Asks::Message),
            (json!("followUp"), Asks::Message),
        ],
        "the message of an idle pi and the two pi queues all make the app send"
    );
    assert_eq!(
        turns_started, 2,
        "a message queued while a run is open starts no turn: the run is the turn"
    );
}

#[test]
fn a_review_reaches_the_message_pi_queues_as_it_does_the_one_of_an_idle_pi() {
    let mut world = World::new();
    let review = review(2_000);
    let folder = world.scratch.project();
    let attached = review.clone();
    world.keeper.answer_with(move |report| {
        if event_of(report) == "input" {
            Answering::Answer(handing_out(report, &folder, &attached))
        } else {
            Answering::Done
        }
    });
    world.fire("session_start", json!({"reason": "startup"}));
    let idle = world.fire("input", message("idle", json!(null)));
    world.fire("agent_start", json!({}));
    // A run is open from here: the next two are what the user types while pi works.
    let steered = world.fire("input", message("steered", json!("steer")));
    let followed = world.fire("input", message("followed up", json!("followUp")));

    for (text, results) in [
        ("idle", idle),
        ("steered", steered),
        ("followed up", followed),
    ] {
        assert_eq!(
            results,
            [json!({"action": "transform", "text": format!("{text}\n\n{review}"), "images": []})],
            "the message `{text}` carries the review"
        );
    }
}

#[test]
fn the_report_of_a_message_says_whose_it_is_and_a_message_another_extension_sent_asks_for_nothing()
{
    let mut world = World::new();
    let folder = world.scratch.project();
    world.fire("input", message("the user's", json!(null)));
    let mut another = message("an extension's", json!(null));
    another["source"] = json!("extension");
    world.fire("input", another);
    let (users, others) = (world.keeper.next_report(), world.keeper.next_report());

    assert_eq!(payload(&users)["source"], "interactive");
    assert_eq!(payload(&others)["source"], "extension");
    let adapter = Pi::new();
    let asked = adapter
        .asked(&folder, &as_handed(&users))
        .expect("readable")
        .expect("the user's message asks");
    assert_eq!(asked.asks, Asks::Message);
    assert!(
        adapter
            .asked(&folder, &as_handed(&others))
            .expect("readable")
            .is_none(),
        "a message another extension sent carries no review and marks no comment sent"
    );
}

#[test]
fn a_session_pi_brings_back_and_a_run_the_user_interrupted_are_reported_as_such() {
    let mut world = World::new();
    world.fire("session_start", json!({"reason": "resume"}));
    world.fire("agent_start", json!({}));
    world.fire("agent_settled", json!({"aborted": true}));
    world.fire("session_shutdown", json!({"reason": "reload"}));
    world.fire("session_start", json!({"reason": "reload"}));
    world.fire("session_shutdown", json!({"reason": "new"}));
    let heard: Vec<_> = (0..6).map(|_| world.keeper.next_report()).collect();

    let mut adapter = Pi::new();
    let told_by_the_adapter: Vec<Event> = heard
        .iter()
        .flat_map(|report| adapter.report(&as_handed(report)).expect("a report"))
        .map(|reported| reported.event)
        .collect();

    assert_eq!(
        told_by_the_adapter,
        [
            Event::SessionStarted {
                how: Start::Resumed
            },
            Event::TurnStarted,
            Event::TurnEnded {
                how: TurnEnd::Interrupted
            },
            // A reload is the same session, whose start and shutdown are told to nobody.
            Event::SessionEnded,
        ]
    );
}

/// A body of a reply that a shell, a quoting or a line-based format would not survive.
const HOSTILE: &str = "it \"works\", `id` $(touch PWNED) 'x'\nsecond line, \u{1f600} \\";

/// What the Keeper's end answers a call of the reply tool as the Adapter does: kept.
fn replied(report: &AgentReport, folder: &Path) -> Answering {
    let adapter = Pi::new();
    let asked = adapter
        .asked(folder, &as_handed(report))
        .expect("readable")
        .expect("a reply asks");
    let Asks::Reply(AgentReply {
        thread,
        text,
        addressed,
    }) = &asked.asks
    else {
        panic!("a call of the tool asks for a reply: {asked:?}");
    };
    Answering::Answer(adapter.answer(
        &asked,
        &Answer::Replied {
            thread,
            addressed: *addressed,
            characters: text.chars().count(),
        },
    ))
}

#[test]
fn the_reply_tool_reaches_the_app_as_it_was_called_and_gives_the_agent_what_the_app_answered() {
    let mut world = World::new();
    let folder = world.scratch.project();
    world
        .keeper
        .answer_with(move |report| replied(report, &folder));
    let context = world.context();

    let said = world.harness.call(
        &json!({"thread": "T1", "body": HOSTILE, "addressed": true}),
        &context,
    );

    let report = world.keeper.next_report();
    assert_eq!(report.agent, NAME);
    assert_eq!(report.terminal, Some(TerminalId::new(TERMINAL)));
    assert_eq!(report.pid, world.harness.pid());
    let payload = payload(&report);
    assert_eq!(payload["event"], REPLY_EVENT);
    assert_eq!(
        payload["arguments"]["body"], HOSTILE,
        "the body, byte for byte"
    );
    let asked = Pi::new()
        .asked(&world.scratch.project(), &as_handed(&report))
        .expect("readable")
        .expect("a reply asks");
    assert_eq!(
        asked.asks,
        Asks::Reply(AgentReply {
            thread: "T1".to_owned(),
            text: HOSTILE.to_owned(),
            addressed: true,
        })
    );
    let characters = HOSTILE.chars().count();
    assert_eq!(
        said["result"]["content"],
        json!([{
            "type": "text",
            "text": format!("recorded reply to T1 (addressed, {characters} characters)"),
        }]),
        "the agent is told what the app answered, as the tool's result"
    );
    assert!(
        !world.scratch.holds_a_file_named("PWNED"),
        "no shell read the body"
    );

    // A reply that does not say it addressed the thread is an open one.
    let said = world
        .harness
        .call(&json!({"thread": "T2", "body": "no"}), &context);
    let report = world.keeper.next_report();
    let asked = Pi::new()
        .asked(&world.scratch.project(), &as_handed(&report))
        .expect("readable")
        .expect("a reply asks");
    assert!(matches!(&asked.asks, Asks::Reply(reply) if !reply.addressed));
    assert_eq!(
        said["result"]["content"][0]["text"],
        "recorded reply to T2 (open, 2 characters)"
    );
}

#[test]
fn a_reply_the_app_did_not_keep_or_could_not_hear_is_a_failure_of_the_tool() {
    let mut world = World::new();
    let context = world.context();
    let params = json!({"thread": "T1", "body": "x", "addressed": false});

    // The app answers nothing: it did not keep the reply.
    let said = world.harness.call(&params, &context);
    world.keeper.next_report();
    assert_eq!(said, json!({"error": "the app did not keep the reply"}));

    // The app refuses it, and says why.
    world.keeper.answer_with(|_| {
        let adapter = Pi::new();
        Answering::Answer(adapter.answer(
            &yaar_agent::Asked {
                session: CliSession(SESSION.to_owned()),
                terminal: None,
                folder: None,
                asks: Asks::OwnTool,
            },
            &Answer::Refused {
                why: "the review holds no thread T9",
            },
        ))
    });
    let said = world.harness.call(&params, &context);
    world.keeper.next_report();
    assert_eq!(said, json!({"error": "the review holds no thread T9"}));

    // No app to hear it.
    let missing = world.scratch.missing_socket();
    world.harness.set_env(json!({ SOCKET_VARIABLE: missing }));
    let said = world.harness.call(&params, &context);
    assert_eq!(said, json!({"error": "the app is not running"}));
    assert!(world.keeper.hears_nothing());
}

#[test]
fn loaded_with_no_reporting_program_named_the_extension_registers_nothing_and_prints_nothing() {
    let mut world = World::outside_the_app();

    assert_eq!(
        world.harness.registered,
        json!({"events": [], "tools": []}),
        "no handler and no tool"
    );
    // pi plays its events to nobody.
    assert_eq!(
        world.fire("input", message("hi", json!(null))),
        Vec::<Value>::new()
    );
    assert_eq!(
        world.fire("session_start", json!({"reason": "startup"})),
        Vec::<Value>::new()
    );
    let finished = world.harness.finish();

    assert_eq!(finished.status.code(), Some(0));
    assert!(
        finished.unread.is_empty(),
        "nothing on the standard output but the harness's own lines: {:?}",
        finished.unread
    );
    assert!(
        finished.stderr.is_empty(),
        "nothing on the standard error: {}",
        String::from_utf8_lossy(&finished.stderr)
    );
    // The harness has ended: whatever it was going to report has reached the Keeper's end.
    assert!(
        world.keeper.hears_nothing(),
        "the extension reported nothing"
    );
    assert!(world.program.is_file());
}

#[test]
fn the_reporting_program_is_looked_for_again_at_each_report() {
    let mut world = World::new();
    let program = world.program.clone();
    let context = world.context();

    // Taken away after the extension loaded: it reports nothing and the message goes on.
    world.harness.set_env(json!({ REPORT_VARIABLE: null }));
    let results = world.fire("input", message("hi", json!(null)));
    assert_eq!(results, [json!({"action": "continue"})]);
    let said = world
        .harness
        .call(&json!({"thread": "T1", "body": "x"}), &context);
    assert_eq!(said, json!({"error": "the app is not running"}));

    // Named again, in the same process and the same chain of reports.
    world.harness.set_env(json!({ REPORT_VARIABLE: program }));
    world.fire("agent_start", json!({}));

    // The first report the Keeper's end ever hears is this one: the two before made none.
    assert_eq!(event_of(&world.keeper.next_report()), "agent_start");
    assert!(world.keeper.hears_nothing());
}

#[test]
fn a_report_that_cannot_be_made_never_makes_a_handler_throw() {
    let mut world = World::new();
    let program = world.program.clone();
    let context = world.context();
    let params = json!({"thread": "T1", "body": "x"});

    // A program that is not there.
    let gone = world.scratch.root.join("no-such-program");
    world.harness.set_env(json!({ REPORT_VARIABLE: gone }));
    assert_eq!(
        world.fire("input", message("hi", json!(null))),
        [json!({"action": "continue"})]
    );
    assert_eq!(
        world.fire("session_start", json!({"reason": "startup"})),
        [Value::Null]
    );
    assert_eq!(
        world.harness.call(&params, &context),
        json!({"error": "the app is not running"})
    );

    // A program that finds no Keeper.
    let nobody = world.scratch.missing_socket();
    world
        .harness
        .set_env(json!({ REPORT_VARIABLE: program, SOCKET_VARIABLE: nobody }));
    assert_eq!(
        world.fire("input", message("hi", json!(null))),
        [json!({"action": "continue"})]
    );
    assert_eq!(
        world.fire("agent_settled", json!({"aborted": true})),
        [Value::Null]
    );

    let finished = world.harness.finish();
    assert_eq!(finished.status.code(), Some(0));
    assert!(
        finished.stderr.is_empty(),
        "the extension printed: {}",
        String::from_utf8_lossy(&finished.stderr)
    );
    assert!(world.keeper.hears_nothing());
}
