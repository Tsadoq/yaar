//! Each test sets the process environment first, so each needs a process of its own (cargo
//! nextest).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, adapters, run};
use yaar_link::{
    Hello, Message, Program, Refusal, Reply, Request, RequestId, TerminalId, TerminalInfo,
    WorkspaceId, handshake, read_message, write_message,
};

mod support;

use support::{Environment, SIZE, Scratch, git};

const DEADLINE: Duration = Duration::from_secs(20);

impl Scratch {
    fn with_launcher_shell() -> Scratch {
        let scratch = Scratch::in_tmp("launchers", Environment::Full);
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
        scratch.set_env("SHELL", &shell);
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
