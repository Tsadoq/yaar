//! Each test sets the process environment first, so each needs a process of its own (cargo
//! nextest).

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    ChangedFile, Fact, FileStatus, FlowForm, Hello, Message, Program, Refusal, Reply, Request,
    RequestId, SOCKET_VARIABLE, Setting, Step, TERMINAL_VARIABLE, TerminalId, TerminalSize, Turn,
    TurnId, WorkspaceAgents, WorkspaceId, handshake, read_message, write_message,
};
use yaar_store::Store;
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord};

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

const TOOLS: &str = "claude/turn-tools.jsonl";

/// Lines of the recording. PROMPT_AGAIN is the turn Claude Code starts itself, 16 ms after STOP.
const SESSION_AND_PROMPT: [usize; 2] = [1, 2];
const READ: [usize; 2] = [3, 4];
const EDIT_AND_WRITE: [usize; 4] = [9, 10, 12, 13];
const STOP: usize = 32;
const PROMPT_AGAIN: usize = 33;
const STOP_AGAIN: usize = 34;

struct Scratch(PathBuf);

impl Scratch {
    /// Sets the process environment; must be a test's first statement.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-flow-{}-{n}", std::process::id()));
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

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// The test's own copy of the built hook program: cargo replaces it on each build, so under
    /// parallel tests it is briefly missing.
    fn companions(&self) -> PathBuf {
        let folder = self.0.join("companions");
        let copy = folder.join(yaar_adapter_claude::HOOK_PROGRAM);
        if copy.is_file() {
            return folder;
        }
        std::fs::create_dir_all(&folder).expect("make the folder");
        let deadline = Instant::now() + DEADLINE;
        loop {
            let built = std::panic::catch_unwind(|| program("yaar-claude-hook"));
            if let Ok(built) = built
                && std::fs::copy(built, &copy).is_ok()
            {
                return folder;
            }
            assert!(Instant::now() < deadline, "the hook program is never there");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

    /// Waits for a first reply, the proof that the Keeper serves this Window: a fact told before
    /// then goes to no Window.
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
                let mut window = Window::new(stream);
                window
                    .ask(Request::ListClis)
                    .expect("the Keeper serves the Window");
                return window;
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
        std::fs::create_dir_all(folder.join("src")).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(folder.join("notes.txt"), "notes\n").expect("write a file");
        std::fs::write(folder.join("src/calc.py"), "def f(x):\n    return x\n")
            .expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    fn states_of(&self, workspace: &WorkspaceId) -> Vec<String> {
        let store = self.data().join("states").join(format!("{workspace}.git"));
        if !store.exists() {
            return Vec::new();
        }
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&store)
            .args([
                "for-each-ref",
                "--format=%(refname)%00%(objectname)",
                "refs/yaar/states/",
            ])
            .stdin(Stdio::null())
            .output()
            .expect("start git");
        let mut numbered: Vec<(u64, String)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (name, id) = line.split_once('\0')?;
                let number = name.rsplit('/').next()?.parse().ok()?;
                Some((number, id.to_owned()))
            })
            .collect();
        numbered.sort();
        numbered.into_iter().map(|(_, id)| id).collect()
    }

    fn hook(&self, terminal: &TerminalId, payload: &[u8]) {
        let mut child = Command::new(self.companions().join(yaar_adapter_claude::HOOK_PROGRAM))
            .env(SOCKET_VARIABLE, self.socket())
            .env(TERMINAL_VARIABLE, terminal.as_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the hook program");
        child
            .stdin
            .take()
            .expect("its standard input")
            .write_all(payload)
            .expect("hand it the hook's input");
        let output = child.wait_with_output().expect("wait for the hook program");
        assert!(output.status.success(), "the hook program failed");
        assert!(output.stdout.is_empty(), "the hook program printed");
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
    facts: Vec<Fact>,
    looked_at: usize,
    replies: Vec<(RequestId, Result<Reply, Refusal>, usize)>,
}

impl Window {
    fn new(stream: UnixStream) -> Window {
        Window {
            stream,
            next_id: 0,
            facts: Vec::new(),
            looked_at: 0,
            replies: Vec::new(),
        }
    }

    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply, self.facts.len())),
            Ok(Message::Output { .. }) => {}
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    fn ask_noting_facts(&mut self, request: Request) -> (Result<Reply, Refusal>, usize) {
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, ..)| *to == id) {
                let (_, reply, facts) = self.replies.remove(at);
                return (reply, facts);
            }
            self.read("a reply");
        }
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_noting_facts(request).0
    }

    fn until<T>(&mut self, what: &str, mut wanted: impl FnMut(&Fact) -> Option<T>) -> T {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Some(found) = wanted(&self.facts[self.looked_at - 1]) {
                    return found;
                }
            }
            self.read(what);
        }
    }

    fn agents_until(
        &mut self,
        workspace: &WorkspaceId,
        what: &str,
        wanted: impl Fn(&WorkspaceAgents) -> bool,
    ) -> WorkspaceAgents {
        self.until(what, |fact| match fact {
            Fact::AgentsChanged(agents) if agents.workspace == *workspace && wanted(agents) => {
                Some(agents.clone())
            }
            _ => None,
        })
    }

    fn turn_until(
        &mut self,
        workspace: &WorkspaceId,
        what: &str,
        wanted: impl Fn(&TurnId) -> bool,
    ) -> TurnId {
        let agents = self.agents_until(workspace, what, |agents| {
            agents
                .sessions
                .iter()
                .any(|session| session.turn.as_ref().is_some_and(&wanted))
        });
        agents
            .sessions
            .into_iter()
            .find_map(|session| session.turn.filter(&wanted))
            .expect("the turn that was waited for")
    }

    fn ended(&mut self, turn: &TurnId) -> Option<String> {
        self.until("the end of a turn", |fact| match fact {
            Fact::TurnEnded {
                turn: ended, state, ..
            } if ended == turn => Some(state.clone()),
            _ => None,
        })
    }

    fn turn(&mut self, turn: &TurnId) -> Turn {
        match self.ask(Request::GetTurn { turn: turn.clone() }) {
            Ok(Reply::Turn(turn)) => turn,
            other => panic!("no turn: {other:?}"),
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

    fn open_terminal(&mut self, workspace: &WorkspaceId) -> TerminalId {
        let opened = self.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size: SIZE,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("no terminal: {opened:?}");
        };
        terminal
    }

    fn settings(&mut self) -> Vec<Setting> {
        match self.ask(Request::GetSettings) {
            Ok(Reply::Settings(settings)) => settings,
            other => panic!("no settings: {other:?}"),
        }
    }

    fn setting_changed(&mut self) -> Setting {
        self.until("a setting changed", |fact| match fact {
            Fact::SettingChanged(setting) => Some(setting.clone()),
            _ => None,
        })
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// A Keeper with two repositories open and a shell of the app in the second. The first lets a
/// test tell a fact naming the first workspace for every session from one naming the session's own.
struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    other: WorkspaceId,
    workspace: WorkspaceId,
    terminal: TerminalId,
    records: Vec<HookRecord>,
}

impl Setup {
    fn new() -> Setup {
        let scratch = Scratch::new();
        let first = scratch.repository("first");
        let repo = scratch.repository("repo");
        Setup::on(scratch, &first, repo)
    }

    fn on(scratch: Scratch, first: &Path, repo: PathBuf) -> Setup {
        let keeper = scratch.keeper();
        let mut window = scratch.window();
        let other = window.open(first);
        let workspace = window.open(&repo);
        let terminal = window.open_terminal(&workspace);
        Setup {
            scratch,
            keeper,
            window,
            repo,
            other,
            workspace,
            terminal,
            records: recorded::hook_events(TOOLS),
        }
    }

    /// A turn that starts before the first state has none to count from. Nothing on the Link tells
    /// when it is taken, so it is looked for in the state store until the deadline.
    fn show_and_wait_for_the_first_state(&mut self) -> String {
        let shown = self.window.ask(Request::ShowWorkspace {
            workspace: Some(self.workspace.clone()),
        });
        assert!(matches!(shown, Ok(Reply::Done)), "not shown: {shown:?}");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(first) = self.scratch.states_of(&self.workspace).first() {
                return first.clone();
            }
            assert!(Instant::now() < deadline, "no state was recorded");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn hooks(&self, lines: &[usize]) {
        for line in lines {
            let record: HookRecord = self.records[*line].in_project(&self.repo);
            self.scratch.hook(&self.terminal, &record.input_bytes());
        }
    }

    fn hook_of_cli(&self, terminal: &TerminalId, cli: u32, line: usize) {
        let mut record: HookRecord = self.records[line].in_project(&self.repo);
        record.input["session_id"] = format!("{cli:08}-0000-4000-8000-{cli:012}").into();
        self.scratch.hook(terminal, &record.input_bytes());
    }

    fn finish(self) -> Scratch {
        self.window.stop(self.keeper);
        self.scratch
    }
}

fn live_diff(window: &mut Window, workspace: &WorkspaceId, turn: &Turn) -> Vec<ChangedFile> {
    let from = turn.start.clone().expect("the turn has a start");
    match window.ask(Request::CompareVersions {
        workspace: workspace.clone(),
        from,
        to: None,
    }) {
        Ok(Reply::Compared(files)) => files,
        other => panic!("no comparison: {other:?}"),
    }
}

fn first_step(window: &mut Window) -> (TurnId, u32) {
    window.until("a step", |fact| match fact {
        Fact::Step { turn, number, .. } => Some((turn.clone(), *number)),
        _ => None,
    })
}

struct Held(Receiver<()>);

impl Held {
    /// Waits until a recording is held: git has named the paths it stages, so a file made now is in
    /// no state that recording takes.
    fn wait(&self) {
        self.0
            .recv_timeout(DEADLINE)
            .expect("no recording was held");
    }
}

/// Makes recording a state slow until the test opens the gate: `slow.txt` goes through a clean
/// filter that writes once into a pipe (heard by `Held`), then waits for the file `gate`, or until
/// the folder is gone, so a failed test's filter does not outlive it.
fn hold_recordings_at_a_gate(scratch: &Scratch, repo: &Path) -> Held {
    let root = scratch.0.display();
    let pipe = scratch.0.join("held.pipe");
    let made = Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .expect("start mkfifo");
    assert!(made.success(), "mkfifo failed");
    // The filter runs for every staged file and the runs overlap: only the one that makes the
    // folder `held` writes into the pipe, since a second writer would wait for a reader for ever.
    let filter = format!(
        "mkdir {root}/held 2>/dev/null && echo > {pipe}; \
         while [ -d {root} ] && [ ! -e {root}/gate ]; do sleep 0.05; done; cat",
        pipe = pipe.display()
    );
    git(repo, &["config", "filter.slow.clean", &filter]);
    std::fs::write(repo.join(".gitattributes"), "slow.txt filter=slow\n").expect("write it");
    git(repo, &["add", ".gitattributes"]);
    git(repo, &["commit", "--quiet", "-m", "attributes"]);

    // Opening a pipe waits for the other end: a thread does it, so a filter that writes finds a
    // reader.
    let (told, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok(mut reading) = std::fs::File::open(&pipe) else {
            return;
        };
        let mut byte = [0u8; 1];
        if reading.read(&mut byte).is_ok() {
            let _ = told.send(());
        }
    });
    Held(heard)
}

fn open_the_gate(scratch: &Scratch) {
    std::fs::write(scratch.0.join("gate"), "").expect("open the gate");
}

#[test]
fn a_turn_counts_from_the_newest_state_when_it_starts_and_its_live_diff_holds_what_it_wrote() {
    let mut setup = Setup::new();
    std::fs::write(setup.repo.join("notes.txt"), "notes, by hand\n").expect("write a file");
    let first = setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    setup.hooks(&READ);
    let (turn, number) = first_step(&mut setup.window);
    assert_eq!(number, 1);

    let running = setup.window.turn(&turn);
    assert_eq!(running.start.as_deref(), Some(first.as_str()));
    assert_eq!(running.ended, None);

    std::fs::write(
        setup.repo.join("src/calc.py"),
        "def f(x):\n    return x + 1\n",
    )
    .expect("write a file");
    std::fs::write(setup.repo.join("new.txt"), "hello").expect("write a file");
    setup.hooks(&EDIT_AND_WRITE);
    setup.window.until("the write", |fact| match fact {
        Fact::Step {
            number: 3,
            step: Step::Write { .. },
            ..
        } => Some(()),
        _ => None,
    });

    let files = live_diff(&mut setup.window, &setup.workspace, &running);
    let listed: Vec<(&Path, &FileStatus)> = files
        .iter()
        .map(|file| (file.path.as_path(), &file.status))
        .collect();
    assert_eq!(
        listed,
        [
            (Path::new("new.txt"), &FileStatus::Added),
            (Path::new("src/calc.py"), &FileStatus::Modified),
        ]
    );
    let _ = setup.finish();
}

#[test]
fn every_step_names_the_workspace_of_its_session() {
    let mut setup = Setup::new();
    setup.show_and_wait_for_the_first_state();
    assert_ne!(setup.other, setup.workspace);

    setup.hooks(&SESSION_AND_PROMPT);
    setup.hooks(&READ);
    setup.hooks(&EDIT_AND_WRITE);
    setup.hooks(&[15]);
    setup.window.until("the command", |fact| match fact {
        Fact::Step {
            step: Step::Command { .. },
            ..
        } => Some(()),
        _ => None,
    });

    let steps: Vec<(&WorkspaceId, u32)> = setup
        .window
        .facts
        .iter()
        .filter_map(|fact| match fact {
            Fact::Step {
                workspace, number, ..
            } => Some((workspace, *number)),
            _ => None,
        })
        .collect();
    let numbers: Vec<u32> = steps.iter().map(|(_, number)| *number).collect();
    assert_eq!(numbers, [1, 2, 3, 4]);
    assert!(
        steps
            .iter()
            .all(|(workspace, _)| **workspace == setup.workspace),
        "the steps name {:?}, the session works in {:?}",
        steps,
        setup.workspace
    );
    let _ = setup.finish();
}

#[test]
fn a_turn_that_starts_after_the_turn_before_was_recorded_counts_from_that_turns_state() {
    let mut setup = Setup::new();
    let first = setup.show_and_wait_for_the_first_state();
    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    std::fs::write(setup.repo.join("new.txt"), "hello").expect("write a file");
    setup.hooks(&[STOP]);
    let state = setup
        .window
        .ended(&one)
        .expect("a state is recorded at the turn's end");
    assert_ne!(state, first);

    setup.hooks(&[PROMPT_AGAIN]);
    let two = setup
        .window
        .turn_until(&setup.workspace, "the next turn works", |turn| *turn != one);
    assert_eq!(
        setup.window.turn(&two).start.as_deref(),
        Some(state.as_str())
    );
    let _ = setup.finish();
}

#[test]
fn a_turn_that_starts_in_the_very_step_that_ends_the_turn_before_counts_from_that_turns_state() {
    let mut setup = Setup::new();
    let first = setup.show_and_wait_for_the_first_state();
    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    std::fs::write(setup.repo.join("new.txt"), "hello").expect("write a file");

    // A prompt typed while the turn is open: the Switchboard ends the turn and starts the next in
    // one go, while the first turn's state is still being recorded.
    setup.hooks(&[PROMPT_AGAIN]);
    let two = setup
        .window
        .turn_until(&setup.workspace, "the next turn works", |turn| *turn != one);
    let state = setup
        .window
        .ended(&one)
        .expect("a state is recorded at the turn's end");
    assert_ne!(state, first);
    assert_eq!(
        setup.window.turn(&two).start.as_deref(),
        Some(state.as_str())
    );
    let _ = setup.finish();
}

#[test]
fn a_turn_that_starts_while_the_turn_before_is_recorded_waits_for_its_state_and_counts_from_it() {
    let scratch = Scratch::new();
    let first = scratch.repository("first");
    let repo = scratch.repository("repo");
    let _held = hold_recordings_at_a_gate(&scratch, &repo);
    let mut setup = Setup::on(scratch, &first, repo);
    setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    setup.hooks(&[STOP, PROMPT_AGAIN]);
    let two = setup
        .window
        .turn_until(&setup.workspace, "the next turn works", |turn| *turn != one);

    assert!(
        setup
            .window
            .facts
            .iter()
            .all(|fact| !matches!(fact, Fact::TurnEnded { .. })),
        "the turn's end was told while its recording was held"
    );
    assert_eq!(setup.window.turn(&two).start, None);

    open_the_gate(&setup.scratch);
    let state = setup
        .window
        .ended(&one)
        .expect("a state is recorded at the turn's end");
    assert_eq!(
        setup.window.turn(&two).start.as_deref(),
        Some(state.as_str())
    );
    let _ = setup.finish();
}

#[test]
fn every_turn_that_waited_for_a_recording_has_its_start_when_the_end_of_the_turn_before_is_heard() {
    const WAITING: u32 = 6;
    let scratch = Scratch::new();
    let first = scratch.repository("first");
    let repo = scratch.repository("repo");
    let _held = hold_recordings_at_a_gate(&scratch, &repo);
    let mut setup = Setup::on(scratch, &first, repo);
    setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    let terminals: Vec<TerminalId> = (0..WAITING)
        .map(|_| setup.window.open_terminal(&setup.workspace))
        .collect();
    for (cli, terminal) in (2..).zip(&terminals) {
        setup.hook_of_cli(terminal, cli, 1);
    }
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    setup.hooks(&[STOP]);

    let mut waiting: Vec<TurnId> = Vec::new();
    for (cli, terminal) in (2..).zip(&terminals) {
        setup.hook_of_cli(terminal, cli, 2);
        let seen = waiting.clone();
        waiting.push(
            setup
                .window
                .turn_until(&setup.workspace, "another turn works", |turn| {
                    *turn != one && !seen.contains(turn)
                }),
        );
    }
    open_the_gate(&setup.scratch);
    let state = setup
        .window
        .ended(&one)
        .expect("a state is recorded at the turn's end");

    // A read is answered without waiting for writes the Keeper still has to make; reading the turn
    // kept last first makes a Keeper that tells the end before keeping the starts likelier to fail.
    for turn in waiting.iter().rev() {
        assert_eq!(
            setup.window.turn(turn).start.as_deref(),
            Some(state.as_str()),
            "a turn that waited has no start when the end it waited for is heard"
        );
    }
    let _ = setup.finish();
}

#[test]
fn a_turn_that_ends_while_it_waits_leaves_the_turn_after_it_the_state_of_its_own_end() {
    let scratch = Scratch::new();
    let first = scratch.repository("first");
    let repo = scratch.repository("repo");
    let held = hold_recordings_at_a_gate(&scratch, &repo);
    let mut setup = Setup::on(scratch, &first, repo);
    setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    setup.hooks(&[STOP]);
    held.wait();
    setup.hooks(&[PROMPT_AGAIN]);
    let two = setup
        .window
        .turn_until(&setup.workspace, "the second turn works", |turn| {
            *turn != one
        });
    // Made after the first recording named its paths: only the second state holds it.
    std::fs::write(setup.repo.join("new.txt"), "hello").expect("write a file");
    setup.hooks(&[STOP_AGAIN, PROMPT_AGAIN]);
    let three = setup
        .window
        .turn_until(&setup.workspace, "the third turn works", |turn| {
            *turn != one && *turn != two
        });
    assert_eq!(setup.window.turn(&two).start, None);
    assert_eq!(setup.window.turn(&three).start, None);

    open_the_gate(&setup.scratch);
    let after_one = setup.window.ended(&one).expect("the first state");
    let after_two = setup.window.ended(&two).expect("the second state");
    assert_ne!(after_one, after_two);
    assert_eq!(
        setup.window.turn(&two).start.as_deref(),
        Some(after_one.as_str())
    );
    assert_eq!(
        setup.window.turn(&three).start.as_deref(),
        Some(after_two.as_str())
    );
    let _ = setup.finish();
}

struct Settings {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    second: Window,
    first: WorkspaceId,
    other: WorkspaceId,
}

impl Settings {
    fn new() -> Settings {
        let scratch = Scratch::new();
        let (a, b) = (scratch.repository("a"), scratch.repository("b"));
        let keeper = scratch.keeper();
        let mut window = scratch.window();
        let first = window.open(&a);
        let other = window.open(&b);
        let second = scratch.window();
        Settings {
            scratch,
            keeper,
            window,
            second,
            first,
            other,
        }
    }

    fn flow(workspace: &WorkspaceId, form: FlowForm) -> Setting {
        Setting::FlowForm {
            workspace: workspace.clone(),
            form,
        }
    }
}

#[test]
fn a_new_keeper_lists_no_setting() {
    let mut settings = Settings::new();
    assert_eq!(settings.window.settings(), []);
    settings.window.stop(settings.keeper);
}

#[test]
fn a_setting_is_kept_told_to_every_window_after_the_reply_and_listed_in_the_order_of_the_workspaces()
 {
    let mut settings = Settings::new();
    let later = Settings::flow(&settings.other, FlowForm::LiveDiff);
    let (reply, facts_before) = settings.window.ask_noting_facts(Request::SetSetting {
        setting: later.clone(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    assert_eq!(settings.window.setting_changed(), later);
    assert_eq!(settings.second.setting_changed(), later);
    let told_at = settings
        .window
        .facts
        .iter()
        .position(|fact| matches!(fact, Fact::SettingChanged(_)))
        .expect("the fact");
    assert!(
        told_at >= facts_before,
        "the fact came before the reply it follows"
    );

    let earlier = Settings::flow(&settings.first, FlowForm::LiveDiff);
    let done = settings.second.ask(Request::SetSetting {
        setting: earlier.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    assert_eq!(settings.window.setting_changed(), earlier);
    assert_eq!(settings.window.settings(), [earlier.clone(), later.clone()]);

    let back = Settings::flow(&settings.first, FlowForm::Timeline);
    let done = settings.window.ask(Request::SetSetting {
        setting: back.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    assert_eq!(settings.second.setting_changed(), earlier);
    assert_eq!(settings.second.setting_changed(), back);
    assert_eq!(settings.second.settings(), [back, later]);
    settings.window.stop(settings.keeper);
}

#[test]
fn a_setting_of_a_workspace_the_keeper_does_not_hold_is_refused_and_tells_nobody() {
    let mut settings = Settings::new();
    let unknown = WorkspaceId::new("0123456789abcdef0123456789abcdef");
    let refused = settings.window.ask(Request::SetSetting {
        setting: Settings::flow(&unknown, FlowForm::LiveDiff),
    });
    assert!(
        matches!(&refused, Err(Refusal::UnknownWorkspace { workspace }) if *workspace == unknown),
        "{refused:?}"
    );

    let kept = Settings::flow(&settings.other, FlowForm::LiveDiff);
    let done = settings.window.ask(Request::SetSetting {
        setting: kept.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    assert_eq!(settings.second.setting_changed(), kept);
    assert_eq!(settings.window.setting_changed(), kept);
    let told = |window: &Window| {
        window
            .facts
            .iter()
            .filter(|fact| matches!(fact, Fact::SettingChanged(_)))
            .count()
    };
    assert_eq!((told(&settings.window), told(&settings.second)), (1, 1));
    assert_eq!(settings.window.settings(), [kept]);
    settings.window.stop(settings.keeper);
}

#[test]
fn the_settings_are_listed_by_a_keeper_started_again_on_the_same_data_folder() {
    let mut settings = Settings::new();
    let chosen = Settings::flow(&settings.other, FlowForm::LiveDiff);
    let done = settings.window.ask(Request::SetSetting {
        setting: chosen.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    settings.window.stop(settings.keeper);

    let keeper = settings.scratch.keeper();
    let mut window = settings.scratch.window();
    assert_eq!(window.settings(), [chosen]);
    window.stop(keeper);
}

#[test]
fn settings_the_store_holds_that_no_window_set_are_read_in_the_links_words_the_apps_first() {
    let mut settings = Settings::new();
    let chosen = Settings::flow(&settings.first, FlowForm::LiveDiff);
    let done = settings.window.ask(Request::SetSetting {
        setting: chosen.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    settings.window.stop(settings.keeper);

    let store = Store::open(&settings.scratch.data().join(yaar_store::FILE_NAME)).expect("open");
    store
        .set_setting(&yaar_store::Setting::Theme(yaar_store::Theme::Dark))
        .expect("keep the theme");
    store
        .set_setting(&yaar_store::Setting::StopAfter { seconds: 90 })
        .expect("keep the stop time");
    drop(store);

    let keeper = settings.scratch.keeper();
    let mut window = settings.scratch.window();
    assert_eq!(
        window.settings(),
        [
            yaar_link::Setting::StopAfter { seconds: 90 },
            yaar_link::Setting::Theme(yaar_link::ThemeChoice::Named("Dark".to_owned())),
            chosen
        ]
    );
    window.stop(keeper);
}
