//! The hook program answers a permission request with no decision of its own: the built
//! `yaar-claude-hook` is run with recorded inputs, and the test plays the Keeper's end of
//! the socket only to read what the program sent and to see what it printed.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use yaar_companion_claude::DEADLINE;
use yaar_link::{
    Fact, Hello, LinkError, Message, PROTOCOL_VERSION, Refusal, Reply, Request, RequestId,
    SOCKET_PATH_MAX, SOCKET_VARIABLE, TERMINAL_VARIABLE, TerminalId, WorkspaceId, read_message,
    socket_address, write_message,
};
use yaar_system_tests::recorded::{self, HookRecord};

/// The recording of the dialogs: the permission requests are its lines 4 and 12.
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

/// The lines of [`DIALOGS`] that are permission requests: a Write and a Bash.
const REQUESTS: [usize; 2] = [4, 12];

/// How much longer than [`DEADLINE`] the program may take, to start and to leave, on a
/// machine that is busy.
const MARGIN: Duration = Duration::from_secs(3);

/// What a test waits for at most when it does not expect to wait at all.
const PATIENCE: Duration = Duration::from_secs(20);

/// A folder of this test's own, removed with whatever else the test made outside it.
struct Scratch {
    folder: PathBuf,
    also: Vec<PathBuf>,
}

impl Scratch {
    /// A folder for the user alone, under a short path.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = PathBuf::from(format!("/tmp/yaar-hook-{}-{n}", std::process::id()));
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

/// Makes `folder` closed to everybody but the user, whatever the mask of this process.
fn make_private(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
        .expect("make a private folder");
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).expect("set its mode");
}

/// What the Keeper's end saw of the program.
#[derive(Default)]
struct Heard {
    /// The program's hello, when it sent one.
    hello: Option<Hello>,
    /// Every request it sent, in order.
    requests: Vec<(RequestId, Request)>,
    /// Whether it hung up while the test's Keeper still owed it its reply.
    hung_up_early: bool,
}

/// How the test's Keeper behaves once the program is connected.
#[derive(Clone)]
enum Keeper {
    /// Answers the request with `Done`.
    Answers,
    /// Answers the request with a refusal.
    Refuses,
    /// Tells a fact and a reply to another request, then checks the program has not gone,
    /// and only then answers.
    TellsOtherThingsFirst,
    /// Reads the request and never answers.
    Silent,
    /// Reads the request and tells a fact every so often, never answering.
    Chatty,
    /// Answers the hello with one of another protocol and waits for the program to go.
    OfAnotherProtocol,
    /// Accepts the connection and hangs up.
    HangsUp,
}

/// A fact to tell, one that no test of the program reads.
fn a_fact() -> Message {
    Message::Fact(Fact::ChangesChanged {
        workspace: WorkspaceId::new("w1"),
    })
}

/// The place a socket is wanted at, the Keeper bound there, and the folder it lies in.
struct World {
    scratch: Scratch,
    wanted: PathBuf,
    /// The Keeper's thread, when one is running.
    keeper: Option<JoinHandle<Heard>>,
}

impl World {
    /// A world whose socket is wanted at a path short enough to bind.
    fn new() -> Self {
        let scratch = Scratch::new();
        let wanted = scratch.folder.join("keeper.sock");
        Self {
            scratch,
            wanted,
            keeper: None,
        }
    }

    /// A world whose socket is wanted at a path too long to bind, so that the program has
    /// to reach it at the short place the Link computes.
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

    /// Binds the Keeper at the place the Link says, in a folder for the user alone as the
    /// real Keeper makes it, and serves one connection as `behaviour` says.
    fn keeper(&mut self, behaviour: Keeper) {
        let address = socket_address(&self.wanted);
        if address != self.wanted {
            let folder = address.parent().expect("the short place has a folder");
            make_private(folder);
            self.scratch.also.push(folder.to_owned());
        }
        let listener = UnixListener::bind(&address).expect("bind the Keeper's socket");
        self.keeper = Some(thread::spawn(move || serve(&listener, &behaviour)));
    }

    /// Binds a socket and lets it go, which leaves the file where nothing listens.
    fn stale_socket(&self) {
        drop(UnixListener::bind(&self.wanted).expect("bind a socket"));
    }

    /// What the Keeper's end saw, once the program has gone.
    fn heard(&mut self) -> Heard {
        self.keeper
            .take()
            .expect("a Keeper was started")
            .join()
            .expect("the Keeper's thread ends")
    }

    /// Runs the built program on `input`, in a world of its own: nothing of this process's
    /// environment, but the socket wanted and the terminal named.
    fn run(&self, terminal: Option<&str>, input: &[u8]) -> Ran {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yaar-claude-hook"));
        command
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
        // Cargo takes the program out of its place for a moment at every build, and tests
        // that build run beside this one: a program that is not there is looked for again.
        let give_up = Instant::now() + Duration::from_secs(5);
        let (mut child, started) = loop {
            let started = Instant::now();
            match command.spawn() {
                Ok(child) => break (child, started),
                Err(why) if why.kind() == ErrorKind::NotFound && Instant::now() < give_up => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(why) => panic!("start the hook program: {why}"),
            }
        };
        let mut stdin = child.stdin.take().expect("the program's standard input");
        // The program reads its input whole before anything else; a break of the pipe
        // means it left without, which the test sees from what it sent.
        let _ = stdin.write_all(input);
        drop(stdin);
        let output = child.wait_with_output().expect("wait for the program");
        Ran {
            status: output.status,
            printed: output.stdout,
            took: started.elapsed(),
        }
    }
}

/// What the program did, as its caller sees it.
struct Ran {
    status: ExitStatus,
    printed: Vec<u8>,
    took: Duration,
}

impl Ran {
    /// The program went as the CLI wants it to: it succeeded and said nothing.
    fn assert_succeeded_with_no_word(&self) {
        assert_eq!(self.status.code(), Some(0), "the program exits 0");
        assert_eq!(
            self.printed,
            Vec::<u8>::new(),
            "the program prints nothing: no decision, no field, no byte"
        );
    }
}

/// Serves one connection as `behaviour` says, and tells what came in.
fn serve(listener: &UnixListener, behaviour: &Keeper) -> Heard {
    let mut heard = Heard::default();
    let Some(mut stream) = accept_within(listener, PATIENCE) else {
        return heard;
    };
    if matches!(behaviour, Keeper::HangsUp) {
        return heard;
    }
    stream
        .set_read_timeout(Some(PATIENCE))
        .expect("set the Keeper's read timeout");
    let Ok(Message::Hello(hello)) = read_message(&mut stream) else {
        return heard;
    };
    let mut mine = Hello::for_this_process("test keeper");
    if matches!(behaviour, Keeper::OfAnotherProtocol) {
        mine.protocol = PROTOCOL_VERSION + 1;
    }
    write_message(&mut stream, &Message::Hello(mine)).expect("answer the hello");
    heard.hello = Some(hello);
    while let Ok(message) = read_message(&mut stream) {
        let Message::Request { id, request } = message else {
            continue;
        };
        heard.requests.push((id, request));
        let reply = |reply| Message::Reply { to: id, reply };
        match behaviour {
            Keeper::Answers => {
                write_message(&mut stream, &reply(Ok(Reply::Done))).expect("answer");
                return heard;
            }
            Keeper::Refuses => {
                let refusal = Refusal::Failed {
                    reason: "no Adapter of this name".to_owned(),
                };
                write_message(&mut stream, &reply(Err(refusal))).expect("refuse");
                return heard;
            }
            Keeper::TellsOtherThingsFirst => {
                write_message(&mut stream, &a_fact()).expect("tell a fact");
                let other = Message::Reply {
                    to: RequestId(id.0 + 7),
                    reply: Ok(Reply::Done),
                };
                write_message(&mut stream, &other).expect("answer another request");
                // What the program does with what it was told shows in whether it is still
                // there: it has to read on until its own reply.
                heard.hung_up_early = !stays_connected(&mut stream);
                write_message(&mut stream, &reply(Ok(Reply::Done))).expect("answer");
                return heard;
            }
            Keeper::Silent => {}
            Keeper::Chatty => {
                let until = Instant::now() + PATIENCE;
                while Instant::now() < until && write_message(&mut stream, &a_fact()).is_ok() {
                    thread::sleep(Duration::from_millis(200));
                }
                return heard;
            }
            Keeper::OfAnotherProtocol | Keeper::HangsUp => {}
        }
    }
    heard
}

/// Whether the other end is still there after a short while, which is to say that a read
/// of it times out and does not find it closed.
fn stays_connected(stream: &mut UnixStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .expect("set a short read timeout");
    matches!(
        read_message(stream),
        Err(LinkError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
    )
}

/// The first connection to `listener`, or none within `patience`.
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

/// The request the program sent, which has to be the one and only.
fn the_one_report(heard: &Heard) -> &yaar_link::AgentReport {
    let [(id, Request::Report { report })] = heard.requests.as_slice() else {
        panic!("the program sends one report, got {:?}", heard.requests);
    };
    assert_eq!(*id, RequestId(0), "the report is request 0");
    report
}

/// The record of line `line` of the dialogs, which is a permission request.
fn permission_request(line: usize) -> HookRecord {
    let record = recorded::hook_events(DIALOGS).swap_remove(line);
    assert_eq!(record.event, "PermissionRequest", "line {line}");
    record
}

#[test]
fn the_hook_program_answers_a_permission_request_with_no_decision_of_its_own() {
    for line in REQUESTS {
        let record = permission_request(line);
        let terminal = record.terminal().expect("the record names its terminal");
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(Some(terminal), &record.input_bytes());
        let heard = world.heard();

        let report = the_one_report(&heard);
        assert_eq!(report.agent, "claude", "line {line}");
        assert_eq!(
            report.terminal,
            Some(TerminalId::new(terminal)),
            "line {line}"
        );
        assert_eq!(
            report.pid,
            std::process::id(),
            "line {line}: the parent of the program is this test, its caller"
        );
        assert_eq!(
            report.payload,
            record.input_bytes(),
            "line {line}: the input, byte for byte"
        );
        let hello = heard.hello.expect("the program says hello first");
        assert_eq!(hello.protocol, PROTOCOL_VERSION);
        assert!(
            hello.app_version.starts_with("yaar-claude-hook "),
            "{}",
            hello.app_version
        );
        ran.assert_succeeded_with_no_word();
    }
}

#[test]
fn the_hook_program_reaches_a_socket_wanted_at_a_path_too_long_to_bind() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::with_a_long_path();
    world.keeper(Keeper::Answers);

    let ran = world.run(record.terminal(), &record.input_bytes());
    let heard = world.heard();

    assert_eq!(the_one_report(&heard).payload, record.input_bytes());
    ran.assert_succeeded_with_no_word();
}

#[test]
fn a_hook_run_outside_the_apps_terminals_reports_no_terminal() {
    let record = permission_request(REQUESTS[0]);
    // The variable unset, and the variable empty, which a shell can leave behind.
    for terminal in [None, Some("")] {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(terminal, &record.input_bytes());
        let heard = world.heard();

        assert_eq!(the_one_report(&heard).terminal, None, "{terminal:?}");
        ran.assert_succeeded_with_no_word();
    }
}

#[test]
fn an_input_is_passed_on_unread_whatever_it_holds() {
    let huge = vec![b'x'; 1024 * 1024];
    let inputs: [(&str, &[u8]); 3] = [
        ("empty", b""),
        ("not JSON", b"not json at all \xff\xfe\n"),
        ("a megabyte", &huge),
    ];
    for (what, input) in inputs {
        let mut world = World::new();
        world.keeper(Keeper::Answers);

        let ran = world.run(Some("t1"), input);
        let heard = world.heard();

        assert_eq!(the_one_report(&heard).payload, input, "{what}");
        ran.assert_succeeded_with_no_word();
    }
}

#[test]
fn an_input_too_long_for_a_frame_is_left_unsent_and_the_program_still_says_nothing() {
    let too_long = vec![b'x'; 17 * 1024 * 1024];
    let mut world = World::new();
    world.keeper(Keeper::Answers);

    let ran = world.run(Some("t1"), &too_long);
    let heard = world.heard();

    assert_eq!(heard.requests, [], "the Link carries no frame so long");
    ran.assert_succeeded_with_no_word();
}

#[test]
fn what_the_keeper_tells_before_its_reply_is_read_past() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::new();
    world.keeper(Keeper::TellsOtherThingsFirst);

    let ran = world.run(record.terminal(), &record.input_bytes());
    let heard = world.heard();

    assert!(
        !heard.hung_up_early,
        "the program waits for the reply to its request, past a fact and the reply to another"
    );
    ran.assert_succeeded_with_no_word();
}

#[test]
fn a_keeper_that_refuses_the_report_changes_nothing_of_the_answer() {
    let record = permission_request(REQUESTS[1]);
    let mut world = World::new();
    world.keeper(Keeper::Refuses);

    let ran = world.run(record.terminal(), &record.input_bytes());
    let heard = world.heard();

    assert_eq!(the_one_report(&heard).payload, record.input_bytes());
    ran.assert_succeeded_with_no_word();
}

#[test]
fn with_no_keeper_listening_the_program_says_nothing_and_does_not_wait() {
    let record = permission_request(REQUESTS[0]);
    let world = World::new();

    let nothing_there = world.run(record.terminal(), &record.input_bytes());
    world.stale_socket();
    let nobody_behind_the_file = world.run(record.terminal(), &record.input_bytes());

    for (what, ran) in [
        ("no socket at all", nothing_there),
        ("a socket file nobody listens at", nobody_behind_the_file),
    ] {
        ran.assert_succeeded_with_no_word();
        assert!(ran.took < DEADLINE, "{what}: it took {:?}", ran.took);
    }
}

#[test]
fn a_keeper_that_hangs_up_changes_nothing_of_the_answer() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::new();
    world.keeper(Keeper::HangsUp);

    let ran = world.run(record.terminal(), &record.input_bytes());
    world.heard();

    ran.assert_succeeded_with_no_word();
    assert!(ran.took < DEADLINE, "it took {:?}", ran.took);
}

#[test]
fn a_keeper_of_another_protocol_is_sent_no_request_and_the_program_says_nothing() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::new();
    world.keeper(Keeper::OfAnotherProtocol);

    let ran = world.run(record.terminal(), &record.input_bytes());
    let heard = world.heard();

    assert!(heard.hello.is_some(), "the program did say hello");
    // The Keeper's end reads until the program goes: a request would have been read.
    assert_eq!(heard.requests, []);
    ran.assert_succeeded_with_no_word();
    assert!(ran.took < DEADLINE, "it took {:?}", ran.took);
}

#[test]
fn a_keeper_that_never_answers_holds_the_program_for_the_deadline_and_no_longer() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::new();
    world.keeper(Keeper::Silent);

    let ran = world.run(record.terminal(), &record.input_bytes());
    let heard = world.heard();

    assert_eq!(
        the_one_report(&heard).payload,
        record.input_bytes(),
        "the request was sent and waited on"
    );
    ran.assert_succeeded_with_no_word();
    assert!(
        ran.took >= DEADLINE,
        "the program waited out its deadline, not less: {:?}",
        ran.took
    );
    assert!(
        ran.took < DEADLINE + MARGIN,
        "the program returns within the deadline and a margin: {:?}",
        ran.took
    );
}

#[test]
fn a_keeper_that_keeps_telling_facts_cannot_hold_the_program_past_the_deadline() {
    let record = permission_request(REQUESTS[0]);
    let mut world = World::new();
    world.keeper(Keeper::Chatty);

    let ran = world.run(record.terminal(), &record.input_bytes());
    world.heard();

    ran.assert_succeeded_with_no_word();
    assert!(
        ran.took < DEADLINE + MARGIN,
        "the deadline runs from the start, not from each read: {:?}",
        ran.took
    );
}
