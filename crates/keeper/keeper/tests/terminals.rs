//! Component tests of the terminals, with a real shell on a real pseudo-terminal. Each test
//! sets the process environment first, so nextest runs it in a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Ending, Fact, Hello, Message, PROTOCOL_VERSION, Program, Refusal, Reply, Request, RequestId,
    TerminalId, TerminalSize, WorkspaceId, handshake, read_message, write_message,
};

mod support;

use support::{Environment, SIZE, Scratch, contains, git};

const DEADLINE: Duration = Duration::from_secs(20);

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("terminals", Environment::Full)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn keeper(&self, linger: Duration) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger,
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

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

    fn next(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        match self.next() {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    /// A program still writing may put output between the request and the reply.
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

/// The number the shell answers to `echo shell-$$`, once its line has ended. The echo of
/// the typed line holds `$$`, never digits, and the shell's prompt may stand before it.
fn shell_in(heard: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(heard);
    text.split_inclusive(['\r', '\n']).find_map(|line| {
        let line = line.strip_suffix(['\r', '\n'])?;
        line[line.rfind("shell-")? + "shell-".len()..].parse().ok()
    })
}

#[test]
fn the_scratch_folder_drops_the_configuration_a_shell_hands_git_in_its_environment() {
    // SAFETY: this is the first statement of a test, and nextest gives each test a process
    // of its own, so no other thread exists yet.
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", "1");
        std::env::set_var("GIT_CONFIG_KEY_0", "alias.status");
        std::env::set_var("GIT_CONFIG_VALUE_0", "log");
        std::env::set_var("GIT_CONFIG_PARAMETERS", "'alias.status'='log'");
        std::env::set_var("GIT_CONFIG", "/nonexistent/config");
    }
    let _scratch = Scratch::new();
    for name in ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"] {
        assert_eq!(std::env::var_os(name), None, "{name} is still set");
    }
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

    let terminal = opener.open_terminal(&workspace, SIZE);
    assert_eq!(terminal.as_str().len(), 32, "{terminal}");
    opener.type_into(&terminal, b"echo yaar-$((40 + 2))\r");
    opener.output_until(&terminal, |heard| contains(heard, "yaar-42"));

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

    // The line is typed before the shell has written its prompt, so the prompt can land where the
    // output starts: the program writes a line end first.
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

    drop(first);
    let waited = Instant::now();
    while waited.elapsed() < linger * 4 {
        assert!(
            !keeper.is_finished(),
            "the Keeper left with a program running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut second = scratch.window();
    second.type_into(&terminal, b"exit 5\r");
    assert_eq!(
        second.next(),
        Message::Fact(Fact::TerminalEnded {
            terminal,
            ending: Ending::Exited(5)
        })
    );

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

    // The Keeper returns when the programs are gone.
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

#[test]
fn close_terminal_stops_one_running_terminal_and_refuses_an_unknown_one() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper(Duration::from_secs(60));
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let first = window.open_terminal(&workspace, SIZE);
    let Ok(Reply::TerminalOpened { terminal: second }) =
        window.ask_amid_output(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size: SIZE,
        })
    else {
        panic!("expected second terminal to open");
    };

    assert_eq!(
        window.ask_amid_output(Request::CloseTerminal {
            terminal: first.clone(),
        }),
        Ok(Reply::Done)
    );

    let list_id = window.send(Request::ListTerminals);
    let mut saw_ended = false;
    let listed = loop {
        match window.next() {
            Message::Reply { to, reply } if to == list_id => match reply {
                Ok(Reply::Terminals(listed)) => break listed,
                other => panic!("expected terminals list: {other:?}"),
            },
            Message::Fact(Fact::TerminalEnded { terminal, .. }) if terminal == first => {
                saw_ended = true;
            }
            Message::Output { .. } => {}
            other => panic!("unexpected frame while listing terminals: {other:?}"),
        }
    };
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, second);

    while !saw_ended {
        match window.next() {
            Message::Fact(Fact::TerminalEnded { terminal, .. }) if terminal == first => {
                saw_ended = true;
            }
            Message::Output { .. } => {}
            other => panic!("unexpected frame while waiting for closed terminal to end: {other:?}"),
        }
    }

    assert_eq!(
        window.ask_amid_output(Request::CloseTerminal {
            terminal: first.clone(),
        }),
        Err(Refusal::UnknownTerminal { terminal: first })
    );
    let unknown = TerminalId::new("0".repeat(32));
    assert_eq!(
        window.ask_amid_output(Request::CloseTerminal {
            terminal: unknown.clone(),
        }),
        Err(Refusal::UnknownTerminal { terminal: unknown })
    );
    window.stop(keeper);
}
