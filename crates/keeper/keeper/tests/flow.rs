//! Component tests: what the Keeper keeps for Flow, through what a Window and the hook
//! program of Claude Code use, with real repositories, a real database and real shells
//! beneath: the state a turn's change counts from, and the settings.
//!
//! The hook program is the built one, run with recorded inputs moved into the test's own
//! worktree, as Claude Code would run it; the Keeper is the library on a thread of the
//! test. The Keeper runs git and the shells of its terminals with the environment of its
//! process, so each test sets that environment as its first statement, and these tests are
//! run by `cargo nextest`, which gives each test a process of its own.

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
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

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

const TOOLS: &str = "claude/turn-tools.jsonl";

/// The lines of the recording that begin a session and a turn, that read a file, that edit
/// one, that write a new one, and that end a turn; and the prompt of the turn Claude Code
/// starts itself, sixteen milliseconds after that stop in the recording.
const SESSION_AND_PROMPT: [usize; 2] = [1, 2];
const READ: [usize; 2] = [3, 4];
const EDIT_AND_WRITE: [usize; 4] = [9, 10, 12, 13];
const STOP: usize = 32;
const PROMPT_AGAIN: usize = 33;
const STOP_AGAIN: usize = 34;

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

    /// The folder of the test's own copy of the built hook program, made at the first
    /// call. Every test process asks `program` to build, and cargo puts a program it built
    /// in place again each time: where tests run side by side, the program is gone for a
    /// moment, and one started from there fails. A copy is made at the first try that finds
    /// the program, and is the test's own.
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

    /// Starts the Keeper's library on a thread of this test, with the folder of the hook
    /// program as where the pieces of the agents lie, on the data folder of the test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
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
                return Window::new(stream);
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository in the folder `name` with one commit on `main` holding `notes.txt` and
    /// `src/calc.py`, and an identity of its own, since git guesses none on every system.
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

    /// The states the Keeper recorded for `workspace`, oldest first, by their full ids,
    /// read from the state store as git does.
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

    /// Runs the hook program with `payload` on its standard input, as Claude Code does,
    /// in the terminal `terminal` of the app. It must succeed and print nothing.
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

/// One end of the Link, as a Window holds it. Everything it is sent is kept, so that a
/// test reads it in order, and what it has looked at is not looked at again.
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

    /// Reads one frame and keeps it, a reply with the number of facts that came before it.
    /// Fails the test at the deadline, with the facts that came.
    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply, self.facts.len())),
            // The shell of a terminal prints; nobody here reads it.
            Ok(Message::Output { .. }) => {}
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    /// Sends a request and waits for its reply, which comes with how many facts this
    /// Window had been told when it came.
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

    /// Sends a request and waits for its reply.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_noting_facts(request).0
    }

    /// The first fact from here on that `wanted` takes, passing over the ones before it.
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

    /// The next `AgentsChanged` of `workspace` that `wanted` takes.
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

    /// The turn of the next `AgentsChanged` of `workspace` that shows a running turn
    /// `wanted` takes.
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

    /// The next `TurnEnded` of `turn`, with the state recorded.
    fn ended(&mut self, turn: &TurnId) -> Option<String> {
        self.until("the end of a turn", |fact| match fact {
            Fact::TurnEnded {
                turn: ended, state, ..
            } if ended == turn => Some(state.clone()),
            _ => None,
        })
    }

    /// The turn as the Keeper keeps it.
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

    /// Opens a terminal with a shell in it: what a CLI typed in that shell reports from.
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

    /// The next `SettingChanged` from here on.
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

/// A Keeper with two repositories open, a Window attached and a shell of the app open in
/// the second: what a CLI typed in that shell reports from. The first is there so that a
/// fact that names the first workspace for every session is told apart from one that names
/// the session's own.
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
    /// Has to be the first statement of a test.
    fn new() -> Setup {
        let scratch = Scratch::new();
        let first = scratch.repository("first");
        let repo = scratch.repository("repo");
        Setup::on(scratch, &first, repo)
    }

    /// The same on repositories the test made.
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

    /// Shows the workspace and waits until its first state is recorded: a turn that starts
    /// before it has no state to count from. The state is taken by a thread of the Keeper
    /// after the watcher runs, and nothing of the Link tells of it, so it is looked for in
    /// the state store until the deadline.
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

    /// Runs the hooks of the lines `lines` of the recording, moved into the repository, in
    /// the shell's terminal, in order.
    fn hooks(&self, lines: &[usize]) {
        for line in lines {
            let record: HookRecord = self.records[*line].in_project(&self.repo);
            self.scratch.hook(&self.terminal, &record.input_bytes());
        }
    }

    /// Stops the Keeper.
    fn finish(self) -> Scratch {
        self.window.stop(self.keeper);
        self.scratch
    }
}

/// What the live diff of `turn` lists: the changes from the turn's start to the files on
/// disk, as a Window asks for them.
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

/// The turn id of the first step heard, and the number of the step.
fn first_step(window: &mut Window) -> (TurnId, u32) {
    window.until("a step", |fact| match fact {
        Fact::Step { turn, number, .. } => Some((turn.clone(), *number)),
        _ => None,
    })
}

/// Makes the recording of a state slow until the test says: a file named `slow.txt` goes
/// through a clean filter that makes the file `held` in the scratch folder and waits for the
/// file `gate` there, and for as long as the folder is there, so that a filter of a test
/// that failed does not outlive it.
/// A state is recorded through the repository's own filters, as `git add` makes it, so the
/// recording of a turn's end that changed `slow.txt` is held until the test opens the gate:
/// what must not wait for it is shown to run during it, whatever the machine.
fn hold_recordings_at_a_gate(scratch: &Scratch, repo: &Path) {
    let root = scratch.0.display();
    let filter = format!(
        "touch {root}/held; while [ -d {root} ] && [ ! -e {root}/gate ]; do sleep 0.05; done; cat"
    );
    git(repo, &["config", "filter.slow.clean", &filter]);
    std::fs::write(repo.join(".gitattributes"), "slow.txt filter=slow\n").expect("write it");
    git(repo, &["add", ".gitattributes"]);
    git(repo, &["commit", "--quiet", "-m", "attributes"]);
}

/// Waits until a recording is held at the gate: git has named the paths it stages, and a
/// file made now is in no state that recording takes. Nothing of the Link tells of it, so the
/// mark the filter makes is looked for until the deadline.
fn wait_until_a_recording_is_held(scratch: &Scratch) {
    let deadline = Instant::now() + DEADLINE;
    while !scratch.0.join("held").exists() {
        assert!(Instant::now() < deadline, "no recording was held");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn open_the_gate(scratch: &Scratch) {
    std::fs::write(scratch.0.join("gate"), "").expect("open the gate");
}

// A turn's start

#[test]
fn a_turn_counts_from_the_newest_state_when_it_starts_and_its_live_diff_holds_what_it_wrote() {
    let mut setup = Setup::new();
    // A change made by hand before the first state is in that state: the turn did not
    // make it, and its live diff must not hold it.
    std::fs::write(setup.repo.join("notes.txt"), "notes, by hand\n").expect("write a file");
    let first = setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    setup.hooks(&READ);
    let (turn, number) = first_step(&mut setup.window);
    assert_eq!(number, 1);

    // The turn's first step is heard: its start is kept, and is that state.
    let running = setup.window.turn(&turn);
    assert_eq!(running.start.as_deref(), Some(first.as_str()));
    assert_eq!(running.ended, None);

    // The agent edits a file and writes a new one; it reads one it does not change.
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

    // Counted from that start, the files on disk differ in the files the turn wrote and in
    // no other: not in the file changed by hand before it, not in the one it only read.
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

    // The newest of two states, not the first.
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
fn a_turn_that_starts_while_the_turn_before_is_recorded_waits_for_its_state_and_counts_from_it() {
    let scratch = Scratch::new();
    let first = scratch.repository("first");
    let repo = scratch.repository("repo");
    hold_recordings_at_a_gate(&scratch, &repo);
    let mut setup = Setup::on(scratch, &first, repo);
    setup.show_and_wait_for_the_first_state();

    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    // The file the gate holds up, made at the turn's end, and the stop and the next prompt
    // with nothing between them, as the recording has them.
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    setup.hooks(&[STOP, PROMPT_AGAIN]);
    let two = setup
        .window
        .turn_until(&setup.workspace, "the next turn works", |turn| *turn != one);

    // The recording of the first turn's state is held at the gate, so the second turn
    // started during it: it has no start yet, and its predecessor's end is not told.
    assert!(
        setup
            .window
            .facts
            .iter()
            .all(|fact| !matches!(fact, Fact::TurnEnded { .. })),
        "the turn's end was told while its recording was held"
    );
    assert_eq!(setup.window.turn(&two).start, None);

    // The recording ends: before its end is told, the turn that waited has its start,
    // which is that state, and the Window that reads the turn at that fact finds it.
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
fn a_turn_that_ends_while_it_waits_leaves_the_turn_after_it_the_state_of_its_own_end() {
    let scratch = Scratch::new();
    let first = scratch.repository("first");
    let repo = scratch.repository("repo");
    hold_recordings_at_a_gate(&scratch, &repo);
    let mut setup = Setup::on(scratch, &first, repo);
    setup.show_and_wait_for_the_first_state();

    // Three turns back to back while the first one's state is held: the second starts and
    // ends, and the third starts, each with the recording of the turn before still to come.
    setup.hooks(&SESSION_AND_PROMPT);
    let one = setup
        .window
        .turn_until(&setup.workspace, "the first turn works", |_| true);
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    setup.hooks(&[STOP]);
    wait_until_a_recording_is_held(&setup.scratch);
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
    // Each counts from the state of the turn before it, the second not from its own.
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

// The settings

/// A Keeper with two repositories open and two Windows attached.
struct Settings {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    second: Window,
    first: WorkspaceId,
    other: WorkspaceId,
}

impl Settings {
    /// Has to be the first statement of a test.
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
}

#[test]
fn a_setting_is_kept_told_to_every_window_after_the_reply_and_listed_in_the_order_of_the_workspaces()
 {
    let mut settings = Settings::new();
    // The second workspace's first, so that the order of the list is not the order of the
    // calls.
    let later = Settings::flow(&settings.other, FlowForm::LiveDiff);
    let (reply, facts_before) = settings.window.ask_noting_facts(Request::SetSetting {
        setting: later.clone(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    // Both Windows are told, the one that set it after its reply.
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

    // A form set again takes the place of the one before: one setting, the new value.
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

    // The barrier: a setting that is kept is told after the refused one would have been, to
    // the same Windows, so that nothing told about the first can still be on its way.
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
fn a_setting_the_link_has_no_word_for_is_kept_and_left_out_of_what_a_window_reads() {
    let mut settings = Settings::new();
    let chosen = Settings::flow(&settings.first, FlowForm::LiveDiff);
    let done = settings.window.ask(Request::SetSetting {
        setting: chosen.clone(),
    });
    assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    settings.window.stop(settings.keeper);

    // The settings screen of a later slice sets these; the Store holds them now.
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
    assert_eq!(window.settings(), [chosen]);
    window.stop(keeper);
}
