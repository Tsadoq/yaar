//! Each test sets the process environment first, so each needs a process of its own (nextest).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Ending, Fact, Hello, Message, Program, Refusal, Reply, Request, RequestId, TerminalId,
    TerminalInfo, TerminalSize, WorkspaceId, handshake, read_message, write_message,
};

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-attaching-{}-{n}", std::process::id()));
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
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
                "ENV",
                "PS1",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
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

    fn ask_amid_output(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        loop {
            match self.next() {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Output { .. } | Message::Fact(_) => {}
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
        let opened = self.ask_amid_output(Request::OpenTerminal {
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
        self.hear_until(terminal, &mut heard, wanted);
        heard
    }

    fn hear_until(
        &mut self,
        terminal: &TerminalId,
        heard: &mut Vec<u8>,
        wanted: impl Fn(&[u8]) -> bool,
    ) {
        while !wanted(heard) {
            match self.next() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => heard.extend(bytes),
                other => panic!(
                    "after {:?}, not output of {terminal}: {other:?}",
                    String::from_utf8_lossy(heard)
                ),
            }
        }
    }

    fn attach(&mut self, terminal: &TerminalId, size: TerminalSize) -> Result<Reply, Refusal> {
        self.ask(Request::AttachTerminal {
            terminal: terminal.clone(),
            size,
        })
    }

    fn until_ended(&mut self, terminal: &TerminalId) {
        loop {
            match self.next() {
                Message::Fact(Fact::TerminalEnded {
                    terminal: ended, ..
                }) if ended == *terminal => {
                    return;
                }
                Message::Output { .. } | Message::Fact(_) => {}
                other => panic!("waiting for the end of {terminal}, got {other:?}"),
            }
        }
    }

    fn terminals(&mut self) -> Vec<TerminalInfo> {
        match self.ask_amid_output(Request::ListTerminals) {
            Ok(Reply::Terminals(terminals)) => terminals,
            other => panic!("asked for the terminals, got {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn contains(bytes: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(bytes).contains(text)
}

/// Whether the last 256 bytes hold `text`: a wait on megabytes of output reads none of it again
/// at each frame.
fn tail_holds(bytes: &[u8], text: &str) -> bool {
    contains(&bytes[bytes.len().saturating_sub(256)..], text)
}

fn count(bytes: &[u8], text: &str) -> usize {
    String::from_utf8_lossy(bytes).matches(text).count()
}

/// The first line may be cut where the kept output starts and the last where the reading stopped: a
/// line with no line end is not a number yet.
fn numbers_in(heard: &[u8]) -> Vec<u32> {
    let text = String::from_utf8_lossy(heard);
    let mut lines: Vec<&str> = text.split('\n').skip(1).collect();
    lines.pop();
    lines
        .into_iter()
        .filter_map(|line| line.trim_end_matches('\r').parse().ok())
        .collect()
}

#[test]
fn a_window_that_attaches_is_sent_the_reply_then_the_kept_output_then_what_the_program_writes() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut opener = scratch.window();
    let workspace = opener.open(&repo);
    let terminal = opener.open_terminal(&workspace, SIZE);
    let mut second = scratch.window();

    opener.type_into(&terminal, b"echo yaar-$((40 + 2))\r");
    opener.output_until(&terminal, |heard| contains(heard, "yaar-42"));

    assert_eq!(
        second.attach(&terminal, SIZE),
        Ok(Reply::Attached { cut: false })
    );
    let kept = second.output_until(&terminal, |heard| contains(heard, "yaar-42"));
    assert_eq!(
        count(&kept, "yaar-42"),
        1,
        "{}",
        String::from_utf8_lossy(&kept)
    );

    second.type_into(&terminal, b"echo yaar-$((50 + 5))\r");
    let live = second.output_until(&terminal, |heard| contains(heard, "yaar-55"));
    assert_eq!(count(&live, "yaar-42"), 0, "the kept output came twice");
    opener.output_until(&terminal, |heard| contains(heard, "yaar-55"));

    opener.type_into(&terminal, b"exit 3\r");
    for window in [&mut opener, &mut second] {
        loop {
            match window.next() {
                Message::Output { .. } => {}
                Message::Fact(Fact::TerminalEnded { ending, .. }) => {
                    assert_eq!(ending, Ending::Exited(3));
                    break;
                }
                other => panic!("waiting for the end, got {other:?}"),
            }
        }
    }
    second.stop(keeper);
}

#[test]
fn a_window_that_attaches_in_the_middle_of_a_flood_hears_one_unbroken_run_with_nothing_lost_or_doubled()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut opener = scratch.window();
    let workspace = opener.open(&repo);
    let terminal = opener.open_terminal(&workspace, SIZE);
    let mut second = scratch.window();

    // About 2.6 MB, more than the Keeper keeps, in about a second, so that the second Window
    // attaches long before the end. Typed before the shell writes its prompt, which can land where
    // the run starts, so the line begins with a line end.
    opener.type_into(
        &terminal,
        b"echo; i=0; while [ $i -lt 400 ]; do seq $((i * 1000 + 1)) $((i * 1000 + 1000)); i=$((i + 1)); done\r",
    );
    let mut heard_by_opener = Vec::new();
    opener.hear_until(&terminal, &mut heard_by_opener, |heard| {
        contains(heard, "\r\n500\r\n")
    });
    // Both read at once, as Windows do: one that reads nothing for five seconds is let go.
    let reading = {
        let terminal = terminal.clone();
        std::thread::spawn(move || {
            opener.hear_until(&terminal, &mut heard_by_opener, |heard| {
                tail_holds(heard, "\r\n400000\r\n")
            });
            (opener, heard_by_opener)
        })
    };
    assert_eq!(
        second.attach(&terminal, SIZE),
        Ok(Reply::Attached { cut: false })
    );
    let heard = second.output_until(&terminal, |heard| tail_holds(heard, "\r\n400000\r\n"));
    let numbers = numbers_in(&heard);
    assert!(numbers.len() > 1000, "{} numbers", numbers.len());
    assert!(
        numbers.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "the run breaks where the kept output ends and the live output begins"
    );
    assert_eq!(numbers.last(), Some(&400_000));

    let (opener, heard_by_opener) = reading.join().expect("the Window that opened it");
    assert_eq!(
        numbers_in(&heard_by_opener),
        (1..=400_000).collect::<Vec<u32>>()
    );

    let mut late = scratch.window();
    assert_eq!(
        late.attach(&terminal, SIZE),
        Ok(Reply::Attached { cut: true })
    );
    let kept = late.output_until(&terminal, |heard| tail_holds(heard, "\r\n400000\r\n"));
    let numbers = numbers_in(&kept);
    assert!(
        (100_000..400_000).contains(&numbers.len()),
        "{} numbers",
        numbers.len()
    );
    assert!(numbers.windows(2).all(|pair| pair[1] == pair[0] + 1));
    assert_eq!(numbers.last(), Some(&400_000));
    opener.stop(keeper);
}

#[test]
fn window_after_window_that_attaches_and_goes_during_a_flood_hears_an_unbroken_run_each_time() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut opener = scratch.window();
    let workspace = opener.open(&repo);
    let terminal = opener.open_terminal(&workspace, SIZE);
    opener.type_into(
        &terminal,
        b"echo; i=0; while [ $i -lt 400 ]; do seq $((i * 1000 + 1)) $((i * 1000 + 1000)); i=$((i + 1)); done\r",
    );
    let mut heard_by_opener = Vec::new();
    opener.hear_until(&terminal, &mut heard_by_opener, |heard| {
        contains(heard, "\r\n500\r\n")
    });
    let reading = {
        let terminal = terminal.clone();
        std::thread::spawn(move || {
            opener.hear_until(&terminal, &mut heard_by_opener, |heard| {
                tail_holds(heard, "\r\n400000\r\n")
            });
            opener
        })
    };

    let mut joins = 0;
    for _ in 0..25 {
        let mut window = scratch.window();
        let Ok(Reply::Attached { .. }) = window.attach(&terminal, SIZE) else {
            panic!("not attached");
        };
        let mut heard = Vec::new();
        window.hear_until(&terminal, &mut heard, |heard| {
            heard.len() > (1 << 20) + 50_000 || tail_holds(heard, "\r\n400000\r\n")
        });
        let numbers = numbers_in(&heard);
        assert!(
            numbers.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "a run that breaks where the kept output ends and the live output begins"
        );
        let ended = tail_holds(&heard, "\r\n400000\r\n");
        joins += usize::from(!ended);
        drop(window);
        // The loop ends with the first Window that hears the end: every later attach has the shell
        // draw its prompt again, and after a few the last 256 bytes of the kept output no longer
        // hold the last number.
        if ended {
            break;
        }
    }
    assert!(
        joins > 0,
        "the flood was over before a Window attached to it"
    );
    let opener = reading.join().expect("the Window that opened it");
    opener.stop(keeper);
}

#[test]
fn a_terminal_that_is_not_held_or_has_ended_cannot_be_attached() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);

    let never = TerminalId::new("0".repeat(32));
    assert_eq!(
        window.attach(&never, SIZE),
        Err(Refusal::UnknownTerminal { terminal: never })
    );

    let terminal = window.open_terminal(&workspace, SIZE);
    window.type_into(&terminal, b"exit 5\r");
    window.until_ended(&terminal);
    assert_eq!(
        window.attach(&terminal, SIZE),
        Err(Refusal::UnknownTerminal {
            terminal: terminal.clone()
        })
    );
    window.stop(keeper);
}

#[test]
fn a_window_that_attaches_to_a_terminal_it_opened_is_sent_its_kept_output_once_more_and_then_each_byte_once()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let terminal = window.open_terminal(&workspace, SIZE);
    window.type_into(&terminal, b"echo yaar-$((40 + 2))\r");
    window.output_until(&terminal, |heard| contains(heard, "yaar-42"));

    let id = window.send(Request::AttachTerminal {
        terminal: terminal.clone(),
        size: SIZE,
    });
    loop {
        match window.next() {
            Message::Reply { to, reply } if to == id => {
                assert_eq!(reply, Ok(Reply::Attached { cut: false }));
                break;
            }
            Message::Output { .. } => {}
            other => panic!("waiting for the reply, got {other:?}"),
        }
    }
    let kept = window.output_until(&terminal, |heard| contains(heard, "yaar-42"));
    assert_eq!(count(&kept, "yaar-42"), 1);
    window.type_into(&terminal, b"echo yaar-$((50 + 5))\r");
    let live = window.output_until(&terminal, |heard| contains(heard, "yaar-55"));
    assert_eq!(count(&live, "yaar-55"), 1);
    window.stop(keeper);
}

#[test]
fn a_terminal_is_given_the_size_of_the_window_that_attaches_and_asked_to_draw_itself_again() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut opener = scratch.window();
    let workspace = opener.open(&repo);
    let terminal = opener.open_terminal(&workspace, SIZE);
    let mut second = scratch.window();

    // The program redraws on a signal, which the Keeper sends at an attach also when the size is
    // the same.
    opener.type_into(
        &terminal,
        b"sh -c 'trap \"echo drawn-$((20 + 2))\" WINCH; echo ready-$((1 + 1)); while :; do sleep 0.05; done'\r",
    );
    opener.output_until(&terminal, |heard| contains(heard, "ready-2"));
    assert_eq!(
        second.attach(&terminal, SIZE),
        Ok(Reply::Attached { cut: false })
    );
    // The whole line: its line end may come in a frame of its own and would then stand in front of
    // the reply to the next request.
    second.output_until(&terminal, |heard| contains(heard, "drawn-22\r\n"));

    let wide = TerminalSize {
        columns: 100,
        rows: 30,
    };
    assert_eq!(
        second.attach(&terminal, wide),
        Ok(Reply::Attached { cut: false })
    );
    second.output_until(&terminal, |heard| contains(heard, "drawn-22\r\n"));
    opener.type_into(&terminal, b"\x03");
    opener.type_into(&terminal, b"echo size-$(stty size)\r");
    opener.output_until(&terminal, |heard| contains(heard, "size-30 100\r\n"));
    opener.stop(keeper);
}

#[test]
fn the_terminals_are_listed_in_the_order_they_were_opened_with_their_workspace_and_program_until_they_end()
 {
    let scratch = Scratch::new();
    let one = scratch.repository("one");
    let two = scratch.repository("two");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut other = scratch.window();
    assert_eq!(window.terminals(), Vec::new());
    let first_workspace = window.open(&one);
    let second_workspace = window.open(&two);
    let first = window.open_terminal(&first_workspace, SIZE);
    let second = window.open_terminal(&second_workspace, SIZE);
    let third = window.open_terminal(&first_workspace, SIZE);

    let expected = |terminals: &[(&TerminalId, &WorkspaceId)]| -> Vec<TerminalInfo> {
        terminals
            .iter()
            .map(|(id, workspace)| TerminalInfo {
                id: (*id).clone(),
                workspace: (*workspace).clone(),
                program: Program::Shell,
            })
            .collect()
    };
    let all = expected(&[
        (&first, &first_workspace),
        (&second, &second_workspace),
        (&third, &first_workspace),
    ]);
    assert_eq!(window.terminals(), all);
    assert_eq!(other.terminals(), all);

    window.type_into(&second, b"exit 0\r");
    window.until_ended(&second);
    assert_eq!(
        other.terminals(),
        expected(&[(&first, &first_workspace), (&third, &first_workspace)])
    );
    window.stop(keeper);
}

#[test]
fn every_shell_of_the_app_is_told_the_socket_it_was_asked_for_and_the_id_of_its_terminal() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let terminal = window.open_terminal(&workspace, SIZE);

    window.type_into(
        &terminal,
        b"printf 'socket-%s\\n' \"$YAAR_SOCKET\"; printf 'terminal-%s\\n' \"$YAAR_TERMINAL\"\r",
    );
    let heard = window.output_until(&terminal, |heard| {
        contains(heard, &format!("terminal-{terminal}\r\n"))
    });
    assert!(
        contains(
            &heard,
            &format!("socket-{}\r\n", scratch.socket().display())
        ),
        "{}",
        String::from_utf8_lossy(&heard)
    );

    let other = window.open_terminal(&workspace, SIZE);
    assert_ne!(other, terminal);
    window.type_into(&other, b"printf 'terminal-%s\\n' \"$YAAR_TERMINAL\"\r");
    window.output_until(&other, |heard| {
        contains(heard, &format!("terminal-{other}\r\n"))
    });
    window.stop(keeper);
}

#[test]
fn a_window_that_goes_while_a_terminal_writes_takes_nothing_from_the_one_that_stays() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut stays = scratch.window();
    let workspace = stays.open(&repo);
    let terminal = stays.open_terminal(&workspace, SIZE);
    let mut goes = scratch.window();
    assert_eq!(
        goes.attach(&terminal, SIZE),
        Ok(Reply::Attached { cut: false })
    );

    stays.type_into(&terminal, b"seq 1 100000\r");
    drop(goes);
    let heard = stays.output_until(&terminal, |heard| tail_holds(heard, "\r\n100000\r\n"));
    assert_eq!(numbers_in(&heard).last(), Some(&100_000));
    assert_eq!(stays.terminals().len(), 1);
    stays.stop(keeper);
}
