//! Component tests: the terminals of the Keeper, through the surface a Window uses, with
//! a real shell on a real pseudo-terminal beneath.
//!
//! The Keeper starts programs with the environment of its process. Each test therefore
//! sets that environment as its first statement, with the shell `/bin/sh` and a home of
//! its own, and these tests are run by `cargo nextest`, which gives each test a process
//! of its own. A test ends its programs before it returns.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Ending, Fact, Hello, Message, PROTOCOL_VERSION, Program, Refusal, Reply, Request, RequestId,
    TerminalId, TerminalSize, WorkspaceId, handshake, read_message, write_message,
};

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and sets the environment of this process for git and for the
    /// shell of a terminal: no configuration of the user or of the system, a fixed
    /// author, the shell `/bin/sh`, an empty home. It has to be the first statement of a
    /// test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-terminals-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let settings = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "A Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
            ("GIT_COMMITTER_NAME", "A Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
            ("SHELL", "/bin/sh"),
        ];
        // SAFETY: changing the environment is sound only while no other thread reads it,
        // as a thread that starts a process does. This runs as the first statement of a
        // test, and these tests are run by `cargo nextest`, which gives each test a
        // process of its own: no other thread exists yet.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            // Where the Keeper's copies of git's lists of files go, and a program of a
            // terminal's too: a folder that goes with the test.
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "ENV", "PS1"] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self, linger: Duration) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger,
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
        };
        std::thread::spawn(move || run(options))
    }

    /// Attaches as a Window does, waiting for the Keeper to listen.
    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && handshake(&mut stream, &mine).is_ok()
            {
                stream
                    .set_read_timeout(Some(DEADLINE))
                    .expect("set the deadline of a read");
                return Window { stream, next_id: 0 };
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository in the folder `name` with one commit on `main`.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        std::fs::write(folder.join("a.txt"), "one\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(folder: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One end of the Link, as a Window holds it.
struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    /// The next frame the Keeper sent, whatever it is. Fails the test at the deadline.
    fn next(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    /// Sends a request and expects its reply as the very next frame.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        match self.next() {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    /// Sends a request and waits for its reply, passing over the output of a program that
    /// is still writing: that may come between the request and the reply.
    fn ask_amid_output(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        loop {
            match self.next() {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Output { .. } => {}
                other => panic!("not the reply to {id:?}: {other:?}"),
            }
        }
    }

    /// Opens the folder as a workspace and reads the fact that tells the list.
    fn open(&mut self, folder: &Path) -> WorkspaceId {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened { workspace, .. }) = opened else {
            panic!("not opened: {opened:?}");
        };
        assert!(matches!(self.next(), Message::Fact(Fact::Workspaces(_))));
        workspace
    }

    /// Opens a terminal with a shell in it, and expects the reply as the very next frame:
    /// the output of the shell comes after it.
    fn open_terminal(&mut self, workspace: &WorkspaceId, size: TerminalSize) -> TerminalId {
        let opened = self.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("no terminal: {opened:?}");
        };
        terminal
    }

    fn type_into(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        write_message(
            &mut self.stream,
            &Message::Input {
                terminal: terminal.clone(),
                bytes: bytes.to_vec(),
            },
        )
        .expect("send the bytes");
    }

    /// Reads what the program of `terminal` writes, until `wanted` accepts all of it so
    /// far, and returns it. Frames of other kinds, or of another terminal, fail the test.
    fn output_until(&mut self, terminal: &TerminalId, wanted: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let mut heard = Vec::new();
        while !wanted(&heard) {
            match self.next() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => heard.extend(bytes),
                other => panic!(
                    "after {:?}, not output of {terminal}: {other:?}",
                    String::from_utf8_lossy(&heard)
                ),
            }
        }
        heard
    }

    /// Reads until the fact that the program of `terminal` ended, and returns how it
    /// did, with the output that came before it.
    fn until_ended(&mut self, terminal: &TerminalId) -> (Vec<u8>, Ending) {
        let mut heard = Vec::new();
        loop {
            match self.next() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => heard.extend(bytes),
                Message::Fact(Fact::TerminalEnded {
                    terminal: from,
                    ending,
                }) if from == *terminal => return (heard, ending),
                other => panic!("waiting for the end of {terminal}, got {other:?}"),
            }
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// The number in the answer to `echo shell-$$`, once its line has ended. The echo of the
/// typed line holds `$$`, never digits, and the prompt of the shell may stand in front of
/// the answer on its line.
fn shell_in(heard: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(heard);
    text.split_inclusive(['\r', '\n']).find_map(|line| {
        let line = line.strip_suffix(['\r', '\n'])?;
        line[line.rfind("shell-")? + "shell-".len()..].parse().ok()
    })
}

fn contains(bytes: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(bytes).contains(text)
}

#[test]
fn the_temporary_files_of_the_keeper_of_a_test_go_in_a_folder_of_the_test() {
    let scratch = Scratch::new();
    let temporary = std::env::temp_dir();
    assert!(temporary.starts_with(&scratch.0), "{temporary:?}");
    assert!(
        temporary.is_dir(),
        "{temporary:?} is made with the test's folder"
    );
}

#[test]
fn a_terminal_answers_what_is_typed_and_tells_its_end_after_the_last_output_to_every_window() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut opener = scratch.window();
    let mut other = scratch.window();
    let workspace = opener.open(&repo);

    // The reply is the very next frame, though the shell writes its prompt at once. The
    // command is answered with what it computes, which the echo of the typed line does
    // not hold.
    let terminal = opener.open_terminal(&workspace, SIZE);
    assert_eq!(terminal.as_str().len(), 32, "{terminal}");
    opener.type_into(&terminal, b"echo yaar-$((40 + 2))\r");
    opener.output_until(&terminal, |heard| contains(heard, "yaar-42"));

    // The program ends: the fact comes after everything it wrote, and after it nothing
    // more of the terminal.
    opener.type_into(&terminal, b"exit 3\r");
    let (_, ending) = opener.until_ended(&terminal);
    assert_eq!(ending, Ending::Exited(3));
    assert_eq!(
        opener
            .ask(Request::ListWorkspaces)
            .map(|list| matches!(list, Reply::Workspaces(_))),
        Ok(true),
        "something of the terminal came after its end"
    );

    // The other Window opened nothing, and so was sent nothing of the program: of the
    // frames it got, the list of workspaces and the end are all. The end reached it too.
    assert!(matches!(other.next(), Message::Fact(Fact::Workspaces(_))));
    assert_eq!(
        other.next(),
        Message::Fact(Fact::TerminalEnded {
            terminal,
            ending: Ending::Exited(3)
        })
    );
    other.stop(keeper);
}

#[test]
fn what_a_program_writes_in_quantity_arrives_whole_and_in_order() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let terminal = window.open_terminal(&workspace, SIZE);

    // 20,000 numbers, far more than one read of the program's terminal takes. The line is
    // typed before the shell has written its prompt, so the prompt can land right where
    // the program's output starts: the first thing the program writes is a line end, which
    // leaves the first number alone on its line.
    window.type_into(&terminal, b"echo; seq 1 20000\r");
    let heard = window.output_until(&terminal, |heard| contains(heard, "\r\n20000\r\n"));
    let numbers: Vec<u32> = String::from_utf8_lossy(&heard)
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').parse().ok())
        .collect();
    assert_eq!(numbers, (1..=20_000).collect::<Vec<u32>>());
    window.stop(keeper);
}

#[test]
fn a_terminal_has_the_size_it_was_opened_with_and_the_one_it_is_resized_to() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut window = scratch.window();
    let workspace = window.open(&repo);

    // A size with a 0 in it is taken as 1.
    let terminal = window.open_terminal(
        &workspace,
        TerminalSize {
            columns: 0,
            rows: 0,
        },
    );
    // The size is told in a line of the program's own making: the prompt of the shell can
    // stand in front of it, and the echo of the typed line holds no digits.
    window.type_into(&terminal, b"echo size-$(stty size)\r");
    window.output_until(&terminal, |heard| contains(heard, "size-1 1\r\n"));

    assert_eq!(
        window.ask_amid_output(Request::ResizeTerminal {
            terminal: terminal.clone(),
            size: TerminalSize {
                columns: 100,
                rows: 30,
            },
        }),
        Ok(Reply::Done)
    );
    window.type_into(&terminal, b"echo size-$(stty size)\r");
    window.output_until(&terminal, |heard| contains(heard, "size-30 100\r\n"));
    window.stop(keeper);
}

#[test]
fn a_request_for_a_terminal_or_a_workspace_the_keeper_does_not_hold_is_refused_with_its_name() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut window = scratch.window();
    let workspace = window.open(&repo);

    let unknown = WorkspaceId::new("f".repeat(32));
    assert_eq!(
        window.ask(Request::OpenTerminal {
            workspace: unknown.clone(),
            program: Program::Shell,
            size: SIZE,
        }),
        Err(Refusal::UnknownWorkspace { workspace: unknown })
    );

    // A terminal whose program has ended is held no more.
    let terminal = window.open_terminal(&workspace, SIZE);
    window.type_into(&terminal, b"exit 5\r");
    let (_, ending) = window.until_ended(&terminal);
    assert_eq!(ending, Ending::Exited(5));
    assert_eq!(
        window.ask(Request::ResizeTerminal {
            terminal: terminal.clone(),
            size: SIZE,
        }),
        Err(Refusal::UnknownTerminal {
            terminal: terminal.clone()
        })
    );

    // Bytes for it are dropped, and nothing answers them: the next frame is the reply to
    // the next request.
    window.type_into(&terminal, b"ls\r");
    window.type_into(&TerminalId::new("0".repeat(32)), b"ls\r");
    assert_eq!(
        window.ask(Request::ResizeTerminal {
            terminal: TerminalId::new("0".repeat(32)),
            size: SIZE,
        }),
        Err(Refusal::UnknownTerminal {
            terminal: TerminalId::new("0".repeat(32))
        })
    );
    window.stop(keeper);
}

#[test]
fn the_keeper_stays_while_a_program_runs_with_no_window_and_leaves_after_the_linger_once_it_ended()
{
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let linger = Duration::from_millis(400);
    let keeper = scratch.keeper(linger);
    let mut first = scratch.window();
    let workspace = first.open(&repo);
    let terminal = first.open_terminal(&workspace, SIZE);

    // The Window that opened the terminal goes. Its program runs on, and so does the
    // Keeper, for far longer than the linger.
    drop(first);
    let waited = Instant::now();
    while waited.elapsed() < linger * 4 {
        assert!(
            !keeper.is_finished(),
            "the Keeper left with a program running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Another Window is told the end of the program, though the output was not its own.
    let mut second = scratch.window();
    second.type_into(&terminal, b"exit 5\r");
    assert_eq!(
        second.next(),
        Message::Fact(Fact::TerminalEnded {
            terminal,
            ending: Ending::Exited(5)
        })
    );

    // With no Window and no program, the Keeper leaves once the linger has passed.
    let closed = Instant::now();
    drop(second);
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Idle);
    assert!(closed.elapsed() >= linger, "it lingers before it leaves");
}

#[test]
fn a_window_that_asks_for_the_stop_ends_the_programs_of_the_terminals_it_leaves() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let terminal = window.open_terminal(&workspace, SIZE);
    window.type_into(&terminal, b"echo shell-$$\r");
    let heard = window.output_until(&terminal, |heard| shell_in(heard).is_some());
    let shell = shell_in(&heard).expect("the shell's id");
    // SAFETY: signal 0 sends nothing; it only asks whether the process is there.
    assert_eq!(unsafe { libc::kill(shell as libc::pid_t, 0) }, 0);

    // The Keeper returns when the programs are gone, and a program that has ended and was
    // collected is known to the system no more.
    window.stop(keeper);
    // SAFETY: as above.
    assert_ne!(unsafe { libc::kill(shell as libc::pid_t, 0) }, 0);
}

#[test]
fn a_window_of_another_version_cannot_type_into_a_terminal() {
    let scratch = Scratch::new();
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut mine = Hello::for_this_process("window-from-the-future");
    mine.protocol = PROTOCOL_VERSION + 1;
    let mut stream = {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&scratch.socket())
                && handshake(&mut stream, &mine).is_ok()
            {
                break stream;
            }
            assert!(Instant::now() < deadline, "no Keeper");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    stream
        .set_read_timeout(Some(DEADLINE))
        .expect("set the deadline of a read");

    // Its first frame of input ends the connection, as its first request would.
    write_message(
        &mut stream,
        &Message::Input {
            terminal: TerminalId::new("0".repeat(32)),
            bytes: b"ls\r".to_vec(),
        },
    )
    .expect("send the bytes");
    assert!(matches!(
        read_message(&mut stream),
        Err(yaar_link::LinkError::Closed)
    ));
    let window = scratch.window();
    window.stop(keeper);
}
