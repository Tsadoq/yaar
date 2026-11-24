//! Component tests: what the Keeper does with the recorded states, through the Link as a
//! Window speaks it: the room they take, the clean-up the user asks for, the sweep by the set
//! time at the Keeper's start and while it runs, and the files of long reviews that go with
//! them. A real Keeper runs in this process, on a database, a folder of state stores and
//! repositories the test makes, with a clock of the test's own.
//!
//! The turns are the agents' own: the hook program of Claude Code, built, is run with
//! recorded inputs moved into the test's worktree, as `tests/flow.rs` does, and a file of the
//! worktree is changed in each turn so that its end records a state of its own. The Keeper
//! reads the time from the test's clock, which never moves by itself: the test moves it past
//! the set time and waits for the fact the Keeper tells, never for a span to pass. The Keeper
//! runs git and the shells of its terminals with the environment of its process, so each test
//! sets that environment as its first statement, and these tests are run by `cargo nextest`,
//! which gives each test a process of its own.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use yaar_keeper::{Exit, Options, run_with_clock};
use yaar_link::{
    Fact, Hello, HistoryEntry, Message, Program, Refusal, Reply, Request, RequestId,
    SOCKET_VARIABLE, Setting, StateSizes, TERMINAL_VARIABLE, TerminalId, TerminalSize, TurnId,
    WorkspaceId, handshake, read_message, write_message,
};
use yaar_recorder::Clock;
use yaar_store::Store;
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord};

/// How long a test waits for the Keeper before it fails: longer than the plan's ten
/// seconds, as every test file of this crate has it, since a Keeper starts git and shells
/// of its own and a throttled machine runs them slowly.
const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const HOUR: Duration = Duration::from_secs(60 * 60);

/// The recording `claude/turn-tools.jsonl`: the lines that begin a session and a turn, the
/// one that ends the turn, the prompt Claude Code started itself sixteen milliseconds after
/// that stop, and the stop of that second turn.
const TOOLS: &str = "claude/turn-tools.jsonl";
const SESSION_AND_PROMPT: [usize; 2] = [1, 2];
const STOP: usize = 32;
const PROMPT_AGAIN: usize = 33;
const STOP_AGAIN: usize = 34;

/// `claude/review-file-read.jsonl`: a session's start (line 0) and its end (7), whose reason
/// is not `clear`.
const REVIEW_FILE: &str = "claude/review-file-read.jsonl";

/// A clock of the test's own: a base taken once, and an offset the test moves.
#[derive(Debug)]
struct TestClock {
    base: SystemTime,
    offset: Mutex<Duration>,
}

impl TestClock {
    /// 2026-01-01T00:00:00Z, the offset none.
    fn new() -> Arc<TestClock> {
        Arc::new(TestClock {
            base: UNIX_EPOCH + Duration::from_secs(1_767_225_600),
            offset: Mutex::new(Duration::ZERO),
        })
    }

    /// Moves the clock `by` forward.
    fn move_on(&self, by: Duration) {
        *self.offset.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }

    /// Sets the clock to the base and `offset`.
    fn set(&self, offset: Duration) {
        *self.offset.lock().unwrap_or_else(PoisonError::into_inner) = offset;
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        self.base + *self.offset.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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
            .join(format!("yaar-keeper-cleaning-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            fs::create_dir_all(root.join(folder)).expect("make the test's folder");
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

    /// The folder the Claude Code Adapter writes the files of long reviews into, one folder
    /// for each session by the CLI's own id of it.
    fn reviews(&self) -> PathBuf {
        self.data().join("agents").join("claude").join("reviews")
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
        fs::create_dir_all(&folder).expect("make the folder");
        let deadline = Instant::now() + DEADLINE;
        loop {
            let built = std::panic::catch_unwind(|| program("yaar-claude-hook"));
            if let Ok(built) = built
                && fs::copy(built, &copy).is_ok()
            {
                return folder;
            }
            assert!(Instant::now() < deadline, "the hook program is never there");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Starts the Keeper's library on a thread of this test, reading the time from `clock`,
    /// with the folder of the hook program as where the pieces of the agents lie.
    fn keeper(&self, clock: &Arc<TestClock>) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
            home: self.0.join("home"),
            login_shell: None,
        };
        let clock = Arc::clone(clock);
        std::thread::spawn(move || run_with_clock(options, clock))
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

    /// A repository in the folder `name` with one commit on `main` holding `notes.txt`, and
    /// an identity of its own, since git guesses none on every system.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        fs::write(folder.join("notes.txt"), "notes\n").expect("write a file");
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

    /// The Keeper's database, opened a second time to read what it kept.
    fn store(&self) -> Store {
        Store::open(&self.data().join(yaar_store::FILE_NAME)).expect("open the Keeper's database")
    }

    /// Runs the hook program with `payload` on its standard input, as Claude Code does,
    /// in the terminal `terminal` of the app. It must succeed.
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
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A `git` of the test's own, first on the search path: it runs the real git for every call
/// but one. The `prune` of the store named by [`PruneGate::hold_store`] is held, the first
/// time only, until [`PruneGate::release`], and `held` appears in the gate's folder once the
/// call has reached it. Its fifo is kept open by the test for reading and writing, so that a
/// release is never lost and a test that fails lets the held call go when the gate drops.
struct PruneGate {
    folder: PathBuf,
    go: fs::File,
}

impl PruneGate {
    /// Puts the git in front of the search path of this process. It has to be called right
    /// after [`Scratch::new`], before anything starts a thread.
    fn new(scratch: &Scratch) -> PruneGate {
        let folder = scratch.0.join("gate");
        fs::create_dir_all(&folder).expect("make the gate's folder");
        let fifo = folder.join("go");
        let made = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("start mkfifo");
        assert!(made.success(), "mkfifo failed");
        let go = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)
            .expect("open the fifo");
        let search = std::env::var_os("PATH").expect("a search path");
        let real = std::env::split_paths(&search)
            .map(|place| place.join("git"))
            .find(|candidate| candidate.is_file())
            .expect("a git on the search path");
        let script = format!(
            "#!/bin/sh\n\
             gate='{gate}'\n\
             if [ -f \"$gate/target\" ]; then\n\
             target=$(cat \"$gate/target\")\n\
             is_prune=0\n\
             is_target=0\n\
             for argument in \"$@\"; do\n\
             case \"$argument\" in\n\
             prune) is_prune=1 ;;\n\
             *\"$target\") is_target=1 ;;\n\
             esac\n\
             done\n\
             if [ $is_prune = 1 ] && [ $is_target = 1 ] && mkdir \"$gate/held\" 2>/dev/null; then\n\
             read -r _ < \"$gate/go\"\n\
             fi\n\
             fi\n\
             exec '{real}' \"$@\"\n",
            gate = folder.display(),
            real = real.display(),
        );
        let bin = scratch.0.join("gate-bin");
        fs::create_dir_all(&bin).expect("make the folder of the git");
        let program = bin.join("git");
        fs::write(&program, script).expect("write the git");
        fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("make the git a program");
        let mut places = vec![bin];
        places.extend(std::env::split_paths(&search));
        // SAFETY: as in `Scratch::new`: no other thread exists yet, and each test has a
        // process of its own.
        unsafe {
            std::env::set_var("PATH", std::env::join_paths(places).expect("a search path"));
        }
        PruneGate { folder, go }
    }

    /// Holds the first `prune` of the state store `store` (its folder's name, `<id>.git`).
    fn hold_store(&self, store: &str) {
        fs::write(self.folder.join("target"), store).expect("name the store");
    }

    /// Waits, up to the deadline, until a `prune` is held.
    fn wait_held(&self) {
        let held = self.folder.join("held");
        let deadline = Instant::now() + DEADLINE;
        while !held.exists() {
            assert!(Instant::now() < deadline, "no prune was held");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Lets the held `prune` go on. The fifo stays open until the gate drops, so that the
    /// word waits in it for a call that has not yet opened it.
    fn release(&self) {
        (&self.go).write_all(b"go\n").expect("let it go");
    }
}

/// Runs git in `folder` and gives what it printed, trimmed.
fn git(folder: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// What git prints for a read, with the exit code: the bytes a test of "nothing of the
/// user's is written" compares. No optional lock, so that a read never rewrites the index of
/// a repository committed to in the same second.
fn read_git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    let mut all = output.stdout;
    all.extend_from_slice(b"|");
    all.extend_from_slice(&output.status.code().unwrap_or(-1).to_le_bytes());
    all
}

/// Every file below `folder`, relative to it, with its size.
fn files_below(folder: &Path, below: &Path, found: &mut Vec<(PathBuf, u64)>) {
    for entry in fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let meta = fs::symlink_metadata(&path).expect("kind");
        if meta.is_dir() {
            files_below(&path, below, found);
        } else {
            found.push((
                path.strip_prefix(below).expect("below").to_owned(),
                meta.len(),
            ));
        }
    }
}

/// What a clean-up must leave as it was: the seven reads of the repository and every file
/// of its objects with its size (ruling (c)).
#[derive(Debug, PartialEq, Eq)]
struct Reads {
    reads: Vec<Vec<u8>>,
    objects: Vec<(PathBuf, u64)>,
}

fn reads(repo: &Path) -> Reads {
    let objects_folder = repo.join(".git/objects");
    let mut objects = Vec::new();
    files_below(&objects_folder, &objects_folder, &mut objects);
    objects.sort();
    Reads {
        reads: vec![
            read_git(
                repo,
                &[
                    "status",
                    "--porcelain=v2",
                    "--ignored",
                    "--untracked-files=all",
                ],
            ),
            read_git(repo, &["for-each-ref"]),
            read_git(repo, &["worktree", "list", "--porcelain"]),
            read_git(repo, &["stash", "list"]),
            read_git(repo, &["fsck", "--unreachable", "--no-reflogs"]),
            fs::read(repo.join(".git/index")).expect("the list of files"),
            fs::read(repo.join(".git/config")).expect("the configuration"),
        ],
        objects,
    }
}

/// Two objects no ref reaches, planted before the first read: a blob and a dangling commit.
/// Gives their ids, which `still_there` finds again.
fn plant(scratch: &Scratch, repo: &Path) -> Vec<String> {
    let file = scratch.0.join("blob.txt");
    fs::write(&file, "a blob nothing refers to\n").expect("write the blob's file");
    let blob = git(
        repo,
        &["hash-object", "-w", file.to_str().expect("a path of text")],
    );
    fs::write(repo.join("notes.txt"), "edited\n").expect("edit a file");
    let dangling = git(repo, &["stash", "create"]);
    git(repo, &["checkout", "--quiet", "--", "notes.txt"]);
    assert!(!dangling.is_empty());
    vec![blob, dangling]
}

/// `git cat-file -e` of each planted object succeeds.
fn still_there(repo: &Path, planted: &[String]) {
    for id in planted {
        git(repo, &["cat-file", "-e", id]);
    }
}

/// 64 KB that no other call with another seed gives and that no compression shrinks: a
/// xorshift over a `u64`, so that each state holds room of its own.
fn noise(seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut bytes = Vec::with_capacity(64 * 1024);
    while bytes.len() < 64 * 1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    bytes
}

/// The blocks a file or a folder and everything in it use on disk, links not followed: the
/// test's own count, as `du` makes it.
fn blocks_of(path: &Path) -> u64 {
    let meta = fs::symlink_metadata(path).expect("metadata");
    let mut bytes = meta.blocks() * 512;
    if meta.is_dir() {
        for entry in fs::read_dir(path).expect("read the folder") {
            bytes += blocks_of(&entry.expect("an entry").path());
        }
    }
    bytes
}

/// Sets the modification time of the file `path`.
fn set_modified(path: &Path, time: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open the file")
        .set_modified(time)
        .expect("set the modification time");
}

/// Writes `reviews/<cli>/r-1.md` of the Adapter's folder with `text`, last modified at
/// `modified`, and gives its path.
fn review_file(scratch: &Scratch, cli: &str, text: &str, modified: SystemTime) -> PathBuf {
    let folder = scratch.reviews().join(cli);
    fs::create_dir_all(&folder).expect("make the session's folder");
    let file = folder.join("r-1.md");
    fs::write(&file, text).expect("write the review's file");
    set_modified(&file, modified);
    file
}

/// One end of the Link, as a Window holds it. Everything it is sent is kept, so that a
/// test reads it in order, and what it has looked at is not looked at again.
struct Window {
    stream: UnixStream,
    next_id: u32,
    facts: Vec<Fact>,
    looked_at: usize,
    replies: Vec<(RequestId, Result<Reply, Refusal>)>,
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

    /// Reads one frame and keeps it. Fails the test at the deadline, with the facts that
    /// came.
    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply)),
            // The shell of a terminal prints; nobody here reads it.
            Ok(Message::Output { .. }) => {}
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    /// Sends a request and waits for its reply.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        self.reply_to(id)
    }

    /// Waits for the reply to the request `id`.
    fn reply_to(&mut self, id: RequestId) -> Result<Reply, Refusal> {
        loop {
            if let Some(at) = self.replies.iter().position(|(to, _)| *to == id) {
                return self.replies.remove(at).1;
            }
            self.read("a reply");
        }
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

    /// The turn of the next `AgentsChanged` of `workspace` that shows a running turn
    /// `wanted` takes.
    fn turn_until(
        &mut self,
        workspace: &WorkspaceId,
        what: &str,
        wanted: impl Fn(&TurnId) -> bool,
    ) -> TurnId {
        self.until(what, |fact| match fact {
            Fact::AgentsChanged(agents) if agents.workspace == *workspace => agents
                .sessions
                .iter()
                .find_map(|session| session.turn.clone().filter(&wanted)),
            _ => None,
        })
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

    /// The next `StatesDeleted` of `workspace` from here on.
    fn states_deleted(&mut self, workspace: &WorkspaceId) {
        self.until("StatesDeleted", |fact| match fact {
            Fact::StatesDeleted { workspace: lost } if lost == workspace => Some(()),
            _ => None,
        });
    }

    /// Every workspace a `StatesDeleted` named so far, in the order told.
    fn deleted_so_far(&self) -> Vec<WorkspaceId> {
        self.facts
            .iter()
            .filter_map(|fact| match fact {
                Fact::StatesDeleted { workspace } => Some(workspace.clone()),
                _ => None,
            })
            .collect()
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

    /// `GetStateSizes`.
    fn sizes(&mut self) -> StateSizes {
        match self.ask(Request::GetStateSizes) {
            Ok(Reply::StateSizes(sizes)) => sizes,
            other => panic!("no sizes: {other:?}"),
        }
    }

    /// `GetHistory` of `workspace`: its entries.
    fn history(&mut self, workspace: &WorkspaceId) -> Vec<HistoryEntry> {
        match self.ask(Request::GetHistory {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::History(history)) => history.entries,
            other => panic!("no history: {other:?}"),
        }
    }

    /// Sets the time states are kept, in days, and waits for the Keeper to say it is set.
    fn keep_for(&mut self, days: u32) {
        let set = self.ask(Request::SetSetting {
            setting: Setting::KeepStates { days },
        });
        assert_eq!(set, Ok(Reply::Done));
    }

    /// The barrier: `ListWorkspaces` is answered after everything the Keeper did for this
    /// Window before it.
    fn barrier(&mut self) {
        assert!(matches!(
            self.ask(Request::ListWorkspaces),
            Ok(Reply::Workspaces(_))
        ));
    }

    /// Asks the Keeper to stop and waits for it.
    fn stop(&mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// One repository with its workspace, a terminal of the app and the CLI session its turns
/// are reported for.
struct Work {
    repo: PathBuf,
    workspace: WorkspaceId,
    terminal: TerminalId,
    /// The CLI's id of the session.
    cli: String,
    /// The objects no ref reaches, planted before the Keeper first looked at the repository.
    planted: Vec<String>,
    /// How many turns of it ended.
    turns: u64,
}

/// The state a turn's end recorded, and the turn.
#[derive(Debug, Clone)]
struct Ended {
    turn: TurnId,
    state: String,
}

/// A Keeper with a Window attached, on a clock of the test's own.
struct World {
    scratch: Scratch,
    clock: Arc<TestClock>,
    keeper: Option<JoinHandle<std::io::Result<Exit>>>,
    window: Window,
    records: Vec<HookRecord>,
    /// How many repositories were made: each gets a CLI session id of its own.
    made: u32,
}

impl World {
    /// Has to be called with the scratch folder of a test that made it its first statement.
    fn on(scratch: Scratch) -> World {
        let clock = TestClock::new();
        let keeper = scratch.keeper(&clock);
        let window = scratch.window();
        World {
            scratch,
            clock,
            keeper: Some(keeper),
            window,
            records: recorded::hook_events(TOOLS),
            made: 0,
        }
    }

    /// Has to be the first statement of a test.
    fn new() -> World {
        World::on(Scratch::new())
    }

    /// Stops the Keeper, sets the clock to `offset` past its base and starts a Keeper again
    /// on the same data, with a new Window attached. The Keeper that starts makes no turn
    /// run: the sessions the old one held are cut off.
    fn restart_at(&mut self, offset: Duration) {
        let keeper = self.keeper.take().expect("a Keeper that runs");
        self.window.stop(keeper);
        self.clock.set(offset);
        self.keeper = Some(self.scratch.keeper(&self.clock));
        self.window = self.scratch.window();
    }

    /// A repository `name` opened as a workspace, with objects planted that no ref reaches,
    /// a terminal of the app, shown, and its first state recorded: a turn that starts before
    /// it has no state to count from. The state is taken by a thread of the Keeper after the
    /// watcher runs, and nothing of the Link tells of it, so it is looked for in the state
    /// store until the deadline.
    fn work(&mut self, name: &str) -> Work {
        let repo = self.scratch.repository(name);
        let planted = plant(&self.scratch, &repo);
        self.adopt(repo, planted)
    }

    /// [`World::work`] for a folder that is made and has its objects planted already.
    fn adopt(&mut self, repo: PathBuf, planted: Vec<String>) -> Work {
        self.made += 1;
        let workspace = self.window.open(&repo);
        let terminal = self.window.open_terminal(&workspace);
        let shown = self.window.ask(Request::ShowWorkspace {
            workspace: Some(workspace.clone()),
        });
        assert_eq!(shown, Ok(Reply::Done));
        let deadline = Instant::now() + DEADLINE;
        while self.scratch.states_of(&workspace).is_empty() {
            assert!(Instant::now() < deadline, "no state was recorded");
            std::thread::sleep(Duration::from_millis(10));
        }
        let n = self.made;
        Work {
            repo,
            workspace,
            terminal,
            cli: format!("{n:08}-0000-4000-8000-{n:012}"),
            planted,
            turns: 0,
        }
    }

    /// Runs the hook of the line `line` of the recording as the session of `work`.
    fn hooks(&self, work: &Work, lines: &[usize]) {
        for line in lines {
            let mut record: HookRecord = self.records[*line].in_project(&work.repo);
            record.input["session_id"] = work.cli.clone().into();
            self.scratch.hook(&work.terminal, &record.input_bytes());
        }
    }

    /// One turn of the session of `work`: it begins, a file of the worktree changes, it ends,
    /// and the state its end recorded is waited for.
    fn turn(&mut self, work: &mut Work, previous: Option<&TurnId>) -> Ended {
        match previous {
            None => self.hooks(work, &SESSION_AND_PROMPT),
            Some(_) => self.hooks(work, &[PROMPT_AGAIN]),
        }
        let turn = self
            .window
            .turn_until(&work.workspace, "the turn works", |turn| {
                Some(turn) != previous
            });
        work.turns += 1;
        let seed = work.turns + u64::from(self.made) * 100;
        fs::write(work.repo.join("turn.bin"), noise(seed)).expect("write a file");
        self.hooks(work, &[if previous.is_none() { STOP } else { STOP_AGAIN }]);
        let state = self
            .window
            .ended(&turn)
            .expect("a state is recorded at the turn's end");
        Ended { turn, state }
    }

    /// Two turns of `work`, one after the other.
    fn two_turns(&mut self, work: &mut Work) -> [Ended; 2] {
        let one = self.turn(work, None);
        let two = self.turn(work, Some(&one.turn));
        assert_ne!(one.state, two.state);
        [one, two]
    }

    /// Stops the Keeper that runs, at the end of a test.
    fn finish(mut self) -> Scratch {
        let keeper = self.keeper.take().expect("a Keeper that runs");
        self.window.stop(keeper);
        self.scratch
    }
}

/// Whether the history shows `turn` as expired.
fn is_expired(entries: &[HistoryEntry], turn: &TurnId) -> bool {
    entries
        .iter()
        .any(|entry| matches!(entry, HistoryEntry::ExpiredTurn { turn: t, .. } if t == turn))
}

/// The turns of the history in the order it lists them, each with whether it is expired.
fn turns_listed(entries: &[HistoryEntry]) -> Vec<(TurnId, bool)> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            HistoryEntry::Turn { turn, .. } => Some((turn.clone(), false)),
            HistoryEntry::ExpiredTurn { turn, .. } => Some((turn.clone(), true)),
            HistoryEntry::Commit(_) => None,
        })
        .collect()
}

/// The state the history gives `turn`, when it shows it with one.
fn state_in(entries: &[HistoryEntry], turn: &TurnId) -> Option<String> {
    entries.iter().find_map(|entry| match entry {
        HistoryEntry::Turn {
            turn: t,
            state: Some(state),
            ..
        } if t == turn => Some(state.clone()),
        _ => None,
    })
}

/// Waits, up to the deadline, until `gone` no longer exists: a look at a fact that stays true
/// once it is.
fn wait_until_gone(path: &Path) {
    let deadline = Instant::now() + DEADLINE;
    while path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} is still there",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

// The room the states take

#[test]
fn the_sizes_of_two_workspaces_come_with_a_total_that_holds_both_and_the_room_no_workspace_claims()
{
    let mut world = World::new();
    // Before any state is recorded: no workspace, nothing taken.
    let none = world.window.sizes();
    assert!(none.workspaces.is_empty());

    let mut first = world.work("first");
    let mut second = world.work("second");
    world.two_turns(&mut first);
    // One turn fewer, so that the two stores differ by the room of a state: a line given to
    // the wrong workspace would not match the folder counted again.
    world.turn(&mut second, None);

    // A store of no workspace the Keeper holds: it counts in the total and has no line.
    let stranger = world.scratch.data().join("states").join("stranger.git");
    fs::create_dir_all(&stranger).expect("make the stranger's folder");
    fs::write(stranger.join("blob"), noise(7)).expect("write the stranger's file");
    // And room that is no store at all, a plain file of the folder of states: no line holds
    // it, and the total is what the whole folder takes.
    fs::write(world.scratch.data().join("states").join("junk"), noise(8))
        .expect("write the junk file");

    let sizes = world.window.sizes();
    let named: Vec<&WorkspaceId> = sizes
        .workspaces
        .iter()
        .map(|size| &size.workspace)
        .collect();
    // In the order the workspaces were first kept.
    assert_eq!(named, [&first.workspace, &second.workspace]);
    assert!(sizes.workspaces.iter().all(|size| size.bytes > 0));
    let sum: u64 = sizes.workspaces.iter().map(|size| size.bytes).sum();
    assert!(
        sizes.total >= sum + 2 * 64 * 1024,
        "the total {} holds the {sum} of the workspaces and the 64 KB of each stranger",
        sizes.total
    );
    // Each line is what its store takes on disk: the test's own count of the folder.
    let states = world.scratch.data().join("states");
    let whole = blocks_of(&states);
    assert!(
        whole.abs_diff(sizes.total) <= 16 * 1024,
        "the total is told {} and the folder holds {whole} when counted again",
        sizes.total
    );
    for size in &sizes.workspaces {
        let walked = blocks_of(&states.join(format!("{}.git", size.workspace)));
        let tolerance = 16 * 1024;
        assert!(
            walked.abs_diff(size.bytes) <= tolerance,
            "{} is told {} and holds {walked} when counted again",
            size.workspace,
            size.bytes
        );
    }
    let _ = world.finish();
}

// The clean-up the user asks for

#[test]
fn a_clean_up_at_once_answers_sizes_no_larger_and_tells_both_windows_of_the_workspace_that_lost_a_state_and_not_of_the_one_that_lost_none()
 {
    let mut world = World::new();
    let mut second_window = world.scratch.window();
    // The first workspace is shown once and has its first state only, which is its newest
    // and stays; the second has had two turns, and what its first state and the start of its
    // first turn hold, nothing refers to.
    let quiet = world.work("quiet");
    let mut busy = world.work("busy");
    let [one, two] = world.two_turns(&mut busy);
    let before = world.window.sizes();
    let repo_before = reads(&busy.repo);
    let quiet_before = reads(&quiet.repo);

    // The states are older than the hour a recording keeps its newest state: the clean-up
    // at once keeps the ones younger than that.
    world.clock.move_on(HOUR);
    let answered = world.window.ask(Request::CleanUpStates);
    let Ok(Reply::StateSizes(after)) = answered else {
        panic!("no sizes: {answered:?}");
    };

    // The sizes it answers are no larger, and the busy workspace's is smaller.
    assert!(after.total <= before.total, "{after:?} against {before:?}");
    for size in &after.workspaces {
        let was = before
            .workspaces
            .iter()
            .find(|was| was.workspace == size.workspace)
            .expect("a workspace that was there");
        assert!(size.bytes <= was.bytes, "{size:?} against {was:?}");
    }
    let bytes_of = |sizes: &StateSizes, workspace: &WorkspaceId| {
        sizes
            .workspaces
            .iter()
            .find(|size| size.workspace == *workspace)
            .map(|size| size.bytes)
            .expect("the workspace has a store")
    };
    assert!(
        bytes_of(&after, &busy.workspace) + 32 * 1024 <= bytes_of(&before, &busy.workspace),
        "{after:?} against {before:?}"
    );

    // The facts follow the reply, in one batch, in the order the workspaces were kept: the
    // one that lost a state is told to both Windows, and the one that lost none, kept
    // first, would have been told before it.
    world.window.states_deleted(&busy.workspace);
    second_window.states_deleted(&busy.workspace);
    world.window.barrier();
    second_window.barrier();
    assert_eq!(world.window.deleted_so_far(), [busy.workspace.clone()]);
    assert_eq!(second_window.deleted_so_far(), [busy.workspace.clone()]);

    // What stays: the newest state of the busy workspace, and the first turn's now expired.
    let states = world.scratch.states_of(&busy.workspace);
    assert!(states.contains(&two.state), "{states:?}");
    assert!(!states.contains(&one.state), "{states:?}");
    let history = world.window.history(&busy.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));

    // Nothing of the user's is written.
    assert_eq!(reads(&busy.repo), repo_before);
    assert_eq!(reads(&quiet.repo), quiet_before);
    still_there(&busy.repo, &busy.planted);
    still_there(&quiet.repo, &quiet.planted);
    let _ = world.finish();
}

#[test]
fn a_clean_up_at_once_keeps_the_newest_state_of_a_worktree_that_is_there_and_takes_it_from_one_that_is_gone()
 {
    let mut world = World::new();
    // The one that stays is kept first, so that a fact told for it would come before the
    // other's. The other is a linked worktree of a repository, whose folder is removed.
    let there = world.work("there");
    let base = world.scratch.repository("base");
    let planted = plant(&world.scratch, &base);
    let linked = world.scratch.0.join("linked");
    git(
        &base,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().expect("a path"),
        ],
    );
    let gone = world.adopt(linked.clone(), planted);
    assert!(!world.scratch.states_of(&gone.workspace).is_empty());
    // Showing the first again lets the second go off screen; its folder is removed.
    let shown = world.window.ask(Request::ShowWorkspace {
        workspace: Some(there.workspace.clone()),
    });
    assert_eq!(shown, Ok(Reply::Done));
    fs::remove_dir_all(&linked).expect("remove the worktree's folder");
    let there_states = world.scratch.states_of(&there.workspace);
    let base_before = reads(&base);
    let there_before = reads(&there.repo);

    world.clock.move_on(HOUR);
    let cleaned = world.window.ask(Request::CleanUpStates);
    assert!(matches!(cleaned, Ok(Reply::StateSizes(_))), "{cleaned:?}");

    // The state of the worktree that is gone was nobody's and the worktree has no newest
    // state to keep; the one that is there keeps its newest.
    world.window.states_deleted(&gone.workspace);
    world.window.barrier();
    assert_eq!(world.window.deleted_so_far(), vec![gone.workspace.clone()]);
    assert!(world.scratch.states_of(&gone.workspace).is_empty());
    assert_eq!(world.scratch.states_of(&there.workspace), there_states);
    assert_eq!(reads(&base), base_before);
    assert_eq!(reads(&there.repo), there_before);
    still_there(&base, &gone.planted);
    still_there(&there.repo, &there.planted);
    let _ = world.finish();
}

/// How long a test lets the sweep's thread come due and look, which is a few of its own looks
/// at the clock: a wait for a fact that a correct Keeper never makes happen. Nothing tells
/// that a sweep is held at a lock, or that it has not begun, so there is no barrier to read
/// instead. A run that passes cannot fail on this wait, since what it waits for is let go
/// only after it; a break could be missed only on a machine that takes longer than this for
/// one sweep of a workspace.
const SWEEP_LOOKS: Duration = Duration::from_secs(5);

#[test]
fn a_clean_up_at_once_of_a_worktree_that_is_gone_is_not_cut_off_by_a_sweep_that_comes_due_while_it_packs()
 {
    let scratch = Scratch::new();
    let gate = PruneGate::new(&scratch);
    let mut world = World::on(scratch);
    // As in the test before: the one that stays is kept first; the other is a linked
    // worktree whose folder is removed.
    let there = world.work("there");
    let base = world.scratch.repository("base");
    let planted = plant(&world.scratch, &base);
    let linked = world.scratch.0.join("linked");
    git(
        &base,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().expect("a path"),
        ],
    );
    let gone = world.adopt(linked.clone(), planted);
    let shown = world.window.ask(Request::ShowWorkspace {
        workspace: Some(there.workspace.clone()),
    });
    assert_eq!(shown, Ok(Reply::Done));
    fs::remove_dir_all(&linked).expect("remove the worktree's folder");
    let base_before = reads(&base);
    let there_before = reads(&there.repo);
    let store = format!("{}.git", gone.workspace);
    let store_folder = world.scratch.data().join("states").join(&store);
    gate.hold_store(&store);

    // The clean-up deletes the state of the worktree that is gone and, with its turn let go,
    // packs the store: its `prune` is held.
    world.clock.move_on(HOUR);
    let asked = world.window.send(Request::CleanUpStates);
    gate.wait_held();
    // Now a sweep comes due. It finds the store empty, as the clean-up left it, and, if it
    // went on beside the clean-up, would remove the folder the held `prune` is to run in.
    // The wait is for that: a correct Keeper lets the sweep wait for the clean-up, so the
    // folder stays, and it is let go after `SWEEP_LOOKS` all the same.
    world.clock.move_on(HOUR);
    let started = Instant::now();
    while store_folder.exists() && started.elapsed() < SWEEP_LOOKS {
        std::thread::sleep(Duration::from_millis(10));
    }
    gate.release();
    let cleaned = world.window.reply_to(asked);
    assert!(
        matches!(cleaned, Ok(Reply::StateSizes(_))),
        "the clean-up that worked was told it failed: {cleaned:?}"
    );

    world.window.states_deleted(&gone.workspace);
    world.window.barrier();
    assert_eq!(world.window.deleted_so_far(), vec![gone.workspace.clone()]);
    assert!(world.scratch.states_of(&gone.workspace).is_empty());
    assert!(
        !store_folder.exists(),
        "the folder of the store of a worktree that is gone is left behind"
    );
    assert_eq!(reads(&base), base_before);
    assert_eq!(reads(&there.repo), there_before);
    still_there(&base, &gone.planted);
    still_there(&there.repo, &there.planted);
    let _ = world.finish();
}

// The files of long reviews

#[test]
fn the_files_of_long_reviews_of_a_session_that_ended_go_with_the_clean_up_and_those_of_one_that_runs_stay_byte_for_byte()
 {
    let mut world = World::new();
    let work = world.work("repo");
    let repo_before = reads(&work.repo);

    // Two sessions, as Claude Code reports them: the first ends, the second runs on.
    let ended = recorded::hook_events(REVIEW_FILE);
    let running = world.records[1].clone();
    let (cli_ended, cli_running) = (ended[0].session().to_owned(), running.session().to_owned());
    let hook = |record: &HookRecord| {
        world
            .scratch
            .hook(&work.terminal, &record.in_project(&work.repo).input_bytes());
    };
    hook(&ended[0]);
    hook(&running);
    assert_eq!(ended[7].event, "SessionEnd");
    hook(&ended[7]);
    let store = world.scratch.store();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let runs: Vec<String> = store
            .running_sessions()
            .expect("read the sessions")
            .into_iter()
            .map(|session| session.cli_session)
            .collect();
        if runs == [cli_running.clone()] {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the sessions that run are {runs:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // A file of each, written now: the clean-up at once asks for no age.
    let now = SystemTime::now();
    let gone = review_file(
        &world.scratch,
        &cli_ended,
        "long review of the ended\n",
        now,
    );
    let kept = review_file(
        &world.scratch,
        &cli_running,
        "long review of the one that runs\n",
        now,
    );
    let kept_bytes = fs::read(&kept).expect("read the file");

    let answered = world.window.ask(Request::CleanUpStates);
    assert!(matches!(answered, Ok(Reply::StateSizes(_))), "{answered:?}");
    assert!(!gone.parent().expect("its folder").exists());
    assert_eq!(fs::read(&kept).expect("read the file"), kept_bytes);
    assert_eq!(reads(&work.repo), repo_before);
    still_there(&work.repo, &work.planted);
    let _ = world.finish();
}

// The sweep by the set time

#[test]
fn at_its_start_the_keeper_deletes_the_states_nothing_refers_to_once_the_set_time_has_passed_and_the_history_marks_the_turn_expired()
 {
    let mut world = World::new();
    let mut work = world.work("repo");
    let [one, two] = world.two_turns(&mut work);
    let before = world.window.sizes();
    let repo_before = reads(&work.repo);

    // The Keeper stops, the clock moves past the set time, and a Keeper starts again: its
    // first sweep, after its first reply, deletes what nothing refers to.
    world.restart_at(31 * DAY + HOUR);
    world.window.barrier();
    world.window.states_deleted(&work.workspace);

    let history = world.window.history(&work.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));
    // The expired turn stays where it was: before the one that followed it.
    assert_eq!(
        turns_listed(&history),
        [(one.turn.clone(), true), (two.turn.clone(), false)]
    );
    let after = world.window.sizes();
    assert!(after.total < before.total, "{after:?} against {before:?}");
    let states = world.scratch.states_of(&work.workspace);
    assert!(states.contains(&two.state) && !states.contains(&one.state));
    assert_eq!(reads(&work.repo), repo_before);
    still_there(&work.repo, &work.planted);
    let _ = world.finish();
}

#[test]
fn at_its_start_the_first_sweep_waits_until_the_keeper_has_queued_its_first_reply() {
    let mut world = World::new();
    let mut work = world.work("repo");
    let [one, two] = world.two_turns(&mut work);
    let repo_before = reads(&work.repo);

    // The Keeper starts again with the clock past the set time, and the Window that attaches
    // asks nothing: the Keeper has no reply to queue. The wait is for a sweep that a correct
    // Keeper does not make, so there is nothing to wait for and the settle is one wait of
    // `SWEEP_LOOKS`, on a channel that never gets a message while its sender is held, and
    // the store is read once after it (a look at each tick would start a git each time).
    world.restart_at(31 * DAY + HOUR);
    let (hold, never) = std::sync::mpsc::channel::<()>();
    let _ = never.recv_timeout(SWEEP_LOOKS);
    let states = world.scratch.states_of(&work.workspace);
    drop(hold);
    assert!(
        states.contains(&one.state) && states.contains(&two.state),
        "a sweep ran before the Keeper had queued a reply: {states:?}"
    );

    // The first request is answered; the sweep then runs, and tells.
    world.window.barrier();
    world.window.states_deleted(&work.workspace);
    let history = world.window.history(&work.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));
    let states = world.scratch.states_of(&work.workspace);
    assert!(states.contains(&two.state) && !states.contains(&one.state));
    assert_eq!(reads(&work.repo), repo_before);
    still_there(&work.repo, &work.planted);
    let _ = world.finish();
}

#[test]
fn while_it_runs_the_keeper_deletes_the_states_nothing_refers_to_once_the_clock_passes_the_set_time()
 {
    let scratch = Scratch::new();
    // A file of an ended session's long review, last written 40 days before the base: the
    // first sweep, at the base, removes it, which says that sweep ran.
    let old = review_file(
        &scratch,
        "a-session-long-gone",
        "an old review\n",
        UNIX_EPOCH + Duration::from_secs(1_767_225_600) - 40 * DAY,
    );
    let mut world = World::on(scratch);
    let mut work = world.work("repo");
    let [one, two] = world.two_turns(&mut work);
    let before = world.window.sizes();
    let repo_before = reads(&work.repo);

    // The first sweep has run, at the base: it removes the files of ended sessions after it
    // has expired the states, so a file that is gone says the states were weighed and
    // every one was kept, as none was old. Now the clock passes the set time and the hour,
    // and nothing is asked.
    wait_until_gone(old.parent().expect("its folder"));
    let states = world.scratch.states_of(&work.workspace);
    assert!(states.contains(&one.state) && states.contains(&two.state));
    world.clock.move_on(31 * DAY + HOUR);
    world.window.states_deleted(&work.workspace);

    let history = world.window.history(&work.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));
    let after = world.window.sizes();
    assert!(after.total < before.total, "{after:?} against {before:?}");
    assert_eq!(reads(&work.repo), repo_before);
    still_there(&work.repo, &work.planted);
    let _ = world.finish();
}

/// A state goes once the set time and the hour have passed since it was recorded, not a day
/// before and not a day after. Two workspaces have two turns each, `early` at the base and
/// `late` an hour after it, and the Keeper starts again with the clock half an hour past
/// `early`'s edge: `early`'s states are past it and `late`'s, an hour younger, half an hour
/// short of it. The clock then moves an hour on, past `late`'s edge too. `keep` is the days
/// the user set, none for the default of 30.
fn a_state_goes_when_the_set_time_and_the_hour_have_passed_and_not_before(keep: Option<u32>) {
    let days = keep.unwrap_or(30);
    let mut world = World::new();
    if let Some(days) = keep {
        world.window.keep_for(days);
    }
    let mut early = world.work("early");
    let [early_one, early_two] = world.two_turns(&mut early);
    world.clock.move_on(HOUR);
    let mut late = world.work("late");
    let [late_one, late_two] = world.two_turns(&mut late);
    let early_before = reads(&early.repo);
    let late_before = reads(&late.repo);

    world.restart_at(DAY * days + HOUR + HOUR / 2);
    // The first sweep waits for the Keeper's first reply, which this request is. The fact
    // of `early` is the barrier: a sweep ran, and it told its facts in one batch after it
    // had weighed every workspace, `late` included.
    world.window.barrier();
    world.window.states_deleted(&early.workspace);
    world.window.barrier();
    assert_eq!(world.window.deleted_so_far(), [early.workspace.clone()]);
    let history = world.window.history(&early.workspace);
    assert!(is_expired(&history, &early_one.turn), "{history:?}");
    assert_eq!(state_in(&history, &early_two.turn), Some(early_two.state));
    // Half an hour short of its edge, `late` keeps everything.
    let history = world.window.history(&late.workspace);
    assert_eq!(
        state_in(&history, &late_one.turn),
        Some(late_one.state.clone())
    );
    assert_eq!(
        state_in(&history, &late_two.turn),
        Some(late_two.state.clone())
    );
    let states = world.scratch.states_of(&late.workspace);
    assert!(states.contains(&late_one.state) && states.contains(&late_two.state));

    // An hour on it is past the edge, and the sweep that is due tells it with nothing asked.
    world.clock.move_on(HOUR);
    world.window.states_deleted(&late.workspace);
    let history = world.window.history(&late.workspace);
    assert!(is_expired(&history, &late_one.turn), "{history:?}");
    assert_eq!(state_in(&history, &late_two.turn), Some(late_two.state));
    assert_eq!(reads(&early.repo), early_before);
    assert_eq!(reads(&late.repo), late_before);
    still_there(&early.repo, &early.planted);
    still_there(&late.repo, &late.planted);
    let _ = world.finish();
}

#[test]
fn the_set_time_the_user_set_is_the_one_the_sweep_counts_by() {
    a_state_goes_when_the_set_time_and_the_hour_have_passed_and_not_before(Some(7));
}

#[test]
fn the_default_set_time_is_thirty_days_and_a_state_goes_when_it_and_the_hour_have_passed() {
    a_state_goes_when_the_set_time_and_the_hour_have_passed_and_not_before(None);
}

#[test]
fn a_longer_set_time_keeps_what_the_default_would_have_deleted() {
    let mut world = World::new();
    world.window.keep_for(60);
    let mut old = world.work("old");
    world.two_turns(&mut old);
    // The second workspace's turns come 30 days after the first's.
    world.clock.move_on(30 * DAY);
    let mut recent = world.work("recent");
    let [one, two] = world.two_turns(&mut recent);
    let repo_before = reads(&recent.repo);
    let old_before = reads(&old.repo);

    // At 61 days and an hour the old workspace's states are past the 60 days and the
    // hour, which says a sweep ran with the time set; the recent one's are 31 days and an
    // hour old, past the default's 30 days and the hour, and are kept.
    world.restart_at(61 * DAY + HOUR);
    world.window.barrier();
    world.window.states_deleted(&old.workspace);
    let history = world.window.history(&recent.workspace);
    assert_eq!(state_in(&history, &one.turn), Some(one.state.clone()));
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));
    let states = world.scratch.states_of(&recent.workspace);
    assert!(states.contains(&one.state) && states.contains(&two.state));
    assert_eq!(reads(&recent.repo), repo_before);
    assert_eq!(reads(&old.repo), old_before);
    still_there(&recent.repo, &recent.planted);
    still_there(&old.repo, &old.planted);
    let _ = world.finish();
}

#[test]
fn a_shortened_time_is_in_force_at_once() {
    let mut world = World::new();
    let mut old = world.work("old");
    world.two_turns(&mut old);
    world.clock.move_on(29 * DAY);
    let mut recent = world.work("recent");
    let [one, two] = world.two_turns(&mut recent);
    let repo_before = reads(&recent.repo);
    let old_before = reads(&old.repo);

    // At 31 days and an hour the old workspace's states are past the default; the recent
    // one's are two days and an hour old. A sweep ran, and it kept them.
    world.clock.move_on(2 * DAY + HOUR);
    world.window.states_deleted(&old.workspace);
    let history = world.window.history(&recent.workspace);
    assert_eq!(state_in(&history, &one.turn), Some(one.state.clone()));
    let states = world.scratch.states_of(&recent.workspace);
    assert!(states.contains(&one.state) && states.contains(&two.state));

    // The clock stays where it is, and the time is shortened to a day: the same two days
    // and an hour are past a day and the hour.
    world.window.keep_for(1);
    world.window.states_deleted(&recent.workspace);
    let history = world.window.history(&recent.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    assert_eq!(state_in(&history, &two.turn), Some(two.state.clone()));
    assert_eq!(reads(&recent.repo), repo_before);
    assert_eq!(reads(&old.repo), old_before);
    still_there(&recent.repo, &recent.planted);
    still_there(&old.repo, &old.planted);
    let _ = world.finish();
}

#[test]
fn a_sweep_removes_a_review_file_of_an_ended_session_by_its_age_and_never_one_of_a_session_that_runs()
 {
    let mut world = World::new();
    let mut work = world.work("repo");
    let [one, _] = world.two_turns(&mut work);

    // One session ends and one runs on.
    let ended = recorded::hook_events(REVIEW_FILE);
    let running = world.records[1].clone();
    let (cli_ended, cli_running) = (ended[0].session().to_owned(), running.session().to_owned());
    let hook = |record: &HookRecord| {
        world
            .scratch
            .hook(&work.terminal, &record.in_project(&work.repo).input_bytes());
    };
    hook(&ended[0]);
    hook(&running);
    hook(&ended[7]);
    let store = world.scratch.store();
    let deadline = Instant::now() + DEADLINE;
    while store.running_sessions().expect("read the sessions").len() != 1 {
        assert!(Instant::now() < deadline, "the first session did not end");
        std::thread::sleep(Duration::from_millis(10));
    }

    // The sweep's time will be 31 days and an hour on the clock, so the set time of 30
    // days reaches back to a day and an hour past the base. Files, each of a session of its
    // own that is not held as running: one written at the clock's own time then, one a day
    // before it and one at the very edge of the set time (all inside it, so kept), one a
    // second past the edge and one 31 days before it (outside it, so gone), and one of the
    // session that runs, as old as that.
    let then = world.clock.now() + 31 * DAY + HOUR;
    let fresh = review_file(
        &world.scratch,
        &cli_ended,
        "written at the clock's now\n",
        then,
    );
    let inside = [
        review_file(
            &world.scratch,
            "an-ended-session-a-day-old",
            "written a day before\n",
            then - DAY,
        ),
        review_file(
            &world.scratch,
            "an-ended-session-at-the-edge",
            "written exactly one set time before\n",
            then - 30 * DAY,
        ),
    ];
    let past_the_edge = review_file(
        &world.scratch,
        "an-ended-session-past-the-edge",
        "written a second more than the set time before\n",
        then - 30 * DAY - Duration::from_secs(1),
    );
    let old_of_ended = review_file(
        &world.scratch,
        "an-ended-session",
        "written 31 days before\n",
        then - 31 * DAY,
    );
    let old_of_running = review_file(
        &world.scratch,
        &cli_running,
        "belongs to a session that runs\n",
        then - 31 * DAY,
    );
    let running_bytes = fs::read(&old_of_running).expect("read the file");
    let repo_before = reads(&work.repo);

    world.clock.move_on(31 * DAY + HOUR);
    world.window.states_deleted(&work.workspace);
    // The sweep removes the files (its step 2) before it tells its fact (step 3): that order
    // is the barrier every assertion below rests on, for no surface of the Keeper holds a
    // sweep between the two. A Keeper that told first would be caught here only if the
    // removal lost the race against the round trip of `GetHistory`.
    let history = world.window.history(&work.workspace);
    assert!(is_expired(&history, &one.turn), "{history:?}");
    // A session's files go by their age and by the set time, not by the sweep's own time:
    // one resumed the next day still finds what it wrote.
    assert!(fresh.exists(), "a file written at the clock's now went");
    for file in &inside {
        assert!(
            file.exists(),
            "{} is inside the set time and went",
            file.display()
        );
    }
    assert!(
        !past_the_edge.exists(),
        "a file a second older than the set time stayed"
    );
    assert!(!old_of_ended.exists(), "a file 31 days old stayed");
    assert_eq!(fs::read(&old_of_running).expect("read it"), running_bytes);
    assert_eq!(reads(&work.repo), repo_before);
    still_there(&work.repo, &work.planted);
    let _ = world.finish();
}
