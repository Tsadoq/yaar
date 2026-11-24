//! Each test sets the process environment first, so each needs a process of its own (cargo
//! nextest).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, adapters, run};
use yaar_link::{
    Hello, Message, Program, Refusal, Reply, Request, RequestId, TerminalId, TerminalInfo,
    TerminalSize, WorkspaceId, handshake, read_message, write_message,
};

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

struct Scratch(PathBuf);

impl Scratch {
    /// Sets the process environment; must be a test's first statement.
    fn with_launcher_shell() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-launchers-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let scratch = Scratch(root);
        let shell = scratch.0.join("shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec cat > /dev/null\n",
                scratch.shell_log().display()
            ),
        )
        .expect("write the shell");
        let mut mode = std::fs::metadata(&shell).expect("the shell").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
        std::fs::set_permissions(&shell, mode).expect("make it run");
        let settings = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "A Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
            ("GIT_COMMITTER_NAME", "A Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
        ];
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("SHELL", &shell);
            std::env::set_var("HOME", scratch.0.join("home"));
            std::env::set_var("TMPDIR", scratch.0.join("tmp"));
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
        scratch
    }

    fn shell_log(&self) -> PathBuf {
        self.0.join("shell-arguments")
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn forget_what_the_shell_was_run_with(&self) {
        match std::fs::remove_file(self.shell_log()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("the log of the shell could not be taken away: {e}"),
        }
    }

    fn launched_with(&self) -> Vec<String> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let written = std::fs::read_to_string(self.shell_log()).unwrap_or_default();
            let lines: Vec<String> = written.lines().map(str::to_owned).collect();
            if lines.len() == 4 {
                return lines;
            }
            assert!(
                Instant::now() < deadline,
                "the shell never ran: {written:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.home(),
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
    /// Skips the facts and program output that arrive between request and reply.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        loop {
            match read_message(&mut self.stream).expect("a frame from the Keeper") {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Reply { .. } => panic!("the reply to a request nobody sent"),
                Message::Fact(_) | Message::Output { .. } => {}
                other => panic!("a frame nobody sends: {other:?}"),
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
        workspace
    }

    fn terminals(&mut self) -> Vec<TerminalInfo> {
        match self.ask(Request::ListTerminals) {
            Ok(Reply::Terminals(terminals)) => terminals,
            other => panic!("no list of terminals: {other:?}"),
        }
    }

    fn clis(&mut self) -> Vec<yaar_link::AgentCli> {
        match self.ask(Request::ListClis) {
            Ok(Reply::Clis(clis)) => clis,
            other => panic!("no list of CLIs: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

#[test]
fn a_launcher_starts_the_start_command_its_adapter_names_in_a_new_terminal() {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let home = scratch.home();
    let adapters = adapters(&home);

    let clis = window.clis();
    let names: Vec<&str> = clis.iter().map(|cli| cli.agent.as_str()).collect();
    assert_eq!(names, ["claude", "pi"]);
    assert_eq!(
        names,
        adapters.iter().map(|a| a.name()).collect::<Vec<_>>(),
        "the CLIs named are the Adapters the Keeper holds"
    );

    let mut commands = Vec::new();
    let mut seen: Vec<TerminalId> = window.terminals().into_iter().map(|t| t.id).collect();
    assert!(seen.is_empty());
    for cli in &clis {
        scratch.forget_what_the_shell_was_run_with();
        let program = Program::Agent {
            agent: cli.agent.clone(),
        };
        let opened = window.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: program.clone(),
            size: SIZE,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("{}: no terminal: {opened:?}", cli.agent);
        };
        assert!(
            !seen.contains(&terminal),
            "{}: the id {terminal} was a terminal's before",
            cli.agent
        );
        seen.push(terminal.clone());
        let listed = window.terminals();
        assert_eq!(
            listed.last(),
            Some(&TerminalInfo {
                id: terminal,
                workspace: workspace.clone(),
                program,
            }),
            "{}: {listed:?}",
            cli.agent
        );

        let command = adapters
            .iter()
            .find(|adapter| adapter.name() == cli.agent)
            .expect("the Adapter of the CLI")
            .command();
        let launched = scratch.launched_with();
        assert_eq!(launched[..3], ["-l", "-i", "-c"], "{}", cli.agent);
        assert_eq!(launched[3], command, "{}", cli.agent);
        commands.push(command);
    }
    assert_eq!(commands.len(), 2);
    assert_ne!(
        commands[0], commands[1],
        "each CLI has a command of its own"
    );

    let before = window.terminals();
    let refused = window.ask(Request::OpenTerminal {
        workspace: workspace.clone(),
        program: Program::Agent {
            agent: "otheragent".to_owned(),
        },
        size: SIZE,
    });
    assert!(
        matches!(refused, Err(Refusal::Failed { .. })),
        "{refused:?}"
    );
    assert_eq!(window.terminals(), before);
    assert_eq!(before.len(), clis.len());
    window.stop(keeper);
}
