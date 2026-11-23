//! Component tests: the Keeper takes the environment of the user's login shell, and the
//! user's own git runs in it. A worktree is made through the Link in a repository whose
//! `post-checkout` hook runs a program that only the login shell's search path holds.
//!
//! The Keeper is the built program, started with an environment the test gives it and
//! nothing else: its search path is the test's own, which has no `login-only` folder, so
//! that the folder can come only from the login shell. The shell is a script of the test's
//! own, named by `SHELL` as the program takes the user's (the user's own is never run, and
//! no start-up file of anyone is read). It acts only when it is told to print the
//! environment (`-l -i -c` and a command that starts with `printf`) and is the plain `/bin/sh`
//! for the terminal of the agent the request starts. Its standard error is a file the test
//! reads, which a Keeper of the test's own process could not give.
//!
//! The environment goes to each `Command`: nothing in this file changes that of the test.

use std::ffi::{CString, OsString};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use yaar_link::{
    Hello, LinkError, Message, Refusal, Reply, RepositoryId, Request, RequestId, TerminalSize,
    WorkspaceId, handshake, read_message, write_message,
};

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// How long a Keeper that ought to answer at once is given: more than any answer of a
/// machine at background priority takes, and less than the ten seconds the program lets
/// its shell take (`yaar_link::login::SHELL_DEADLINE`), so that a run that waited for a shell
/// that is held, or that never ended its take, can be told from one that did not.
const AT_ONCE: Duration = Duration::from_secs(8);

/// How long a worktree is watched for before the shell it waits for is let go: the one wait
/// of these tests that is not on a fact. Git has nothing else to wait for, so a run that
/// does not wait makes its worktree within it on any machine the tests run on; one that does
/// is slower than this by the whole of the hold.
const WATCHED_FOR: Duration = Duration::from_millis(500);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// How the test's shell behaves when it is told to print its environment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    /// Adds `login-only` to its search path and prints the environment, but waits first on a
    /// fifo until [`Scratch::release`].
    Held,
    /// Prints the environment with `login-only` in it, then ends with status 3.
    Failing,
}

/// A folder of the test's own with a short path, removed when the test ends.
///
/// Under it: `home`, `tmp`, `data` (the Keeper's), `fake` (the stand-in agent), `login-only`
/// (the program `witness`), `lair/repo` (the repository), and the Keeper's socket.
struct Scratch {
    root: PathBuf,
    /// The test's end of the fifo the held shell waits on, kept open so that what is written
    /// stays in the pipe until the shell reads it.
    fifo: Option<File>,
    keeper: Option<Child>,
}

impl Scratch {
    fn new(shell: Shell) -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and git and the Keeper name a worktree by its resolved
        // path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-login-{}-{n}", std::process::id()));
        for folder in ["home", "tmp", "fake", "login-only", "lair"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let mut scratch = Scratch {
            root,
            fifo: None,
            keeper: None,
        };
        // The agent the request starts: a program that waits on its terminal.
        scratch.program(&scratch.root.join("fake/claude"), "exec cat > /dev/null");
        scratch.program(
            &scratch.root.join("login-only/witness"),
            &format!(
                "printf 'witness ran in %s\\n' \"$(pwd -P)\" >> '{}'",
                scratch.witness_log().display()
            ),
        );
        if shell == Shell::Held {
            let fifo = scratch.root.join("fifo");
            let path = CString::new(fifo.as_os_str().as_bytes()).expect("a path with no NUL");
            // SAFETY: `mkfifo` reads the NUL-terminated path it is given.
            assert_eq!(
                unsafe { libc::mkfifo(path.as_ptr(), 0o600) },
                0,
                "make a fifo"
            );
            // Read and write, so that opening it blocks for nobody.
            scratch.fifo = Some(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&fifo)
                    .expect("open the fifo"),
            );
        }
        // A login shell's system profile may set the search path anew, so the folder of the
        // stand-in agent is put first again by the `.profile` of the home. `login-only` is
        // not: only the shell of the take adds it.
        std::fs::write(
            scratch.home().join(".profile"),
            format!(
                "PATH=\"{}:$PATH\"; export PATH\n",
                scratch.root.join("fake").display()
            ),
        )
        .expect("write the profile");
        scratch.write_shell(shell);
        scratch
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn socket(&self) -> PathBuf {
        self.root.join("k.sock")
    }

    fn repo(&self) -> PathBuf {
        self.root.join("lair/repo")
    }

    /// Where the worktree `task-one` of the repository is made.
    fn worktree(&self) -> PathBuf {
        self.root.join("lair/repo.worktrees/task-one")
    }

    fn shell(&self) -> PathBuf {
        self.root.join("login-shell")
    }

    /// What `witness` writes into, one line for each run.
    fn witness_log(&self) -> PathBuf {
        self.root.join("witness.log")
    }

    /// What the hook writes into, one line for each run, whether or not it found `witness`.
    fn hook_log(&self) -> PathBuf {
        self.root.join("hook.log")
    }

    /// Where the Keeper's standard error goes.
    fn errors(&self) -> PathBuf {
        self.root.join("keeper.err")
    }

    /// Where the shell notes that it has begun to wait.
    fn held_note(&self) -> PathBuf {
        self.root.join("held")
    }

    /// A script `#!/bin/sh` with `body`, which can be run.
    fn program(&self, path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make it run");
    }

    /// The user's login shell of this test. The take runs it as `-l -i -c COMMAND`, and so
    /// does the launcher of an agent, whose command is the agent's name.
    fn write_shell(&self, shell: Shell) {
        let hold = match shell {
            Shell::Held => format!(
                "printf held > '{note}'\nexec 3<> '{fifo}'\nread go <&3\n",
                note = self.held_note().display(),
                fifo = self.root.join("fifo").display()
            ),
            Shell::Failing => String::new(),
        };
        let end = match shell {
            Shell::Failing => "printf '%s' 'yaar-environment-follows'\nenv -0\nexit 3\n",
            Shell::Held => "exec /bin/sh -c \"$4\"\n",
        };
        self.program(
            &self.shell(),
            &format!(
                "case \"$4\" in\n\
                 printf*) ;;\n\
                 *) exec /bin/sh \"$@\" ;;\n\
                 esac\n\
                 {hold}\
                 PATH='{login_only}':\"$PATH\"\nexport PATH\n\
                 {end}",
                login_only = self.root.join("login-only").display()
            ),
        );
    }

    /// Lets a held shell go on.
    fn release(&mut self) {
        if let Some(fifo) = &mut self.fifo {
            fifo.write_all(b"go\n").expect("let the shell go");
        }
    }

    /// The repository with `main` at one commit, and the hook that records that it ran and
    /// then runs `witness`, which only the login shell's search path holds. The hook never
    /// fails, so that the creation is answered either way.
    fn repository(&self) {
        let repo = self.repo();
        std::fs::create_dir_all(&repo).expect("make the repository's folder");
        self.git(&repo, &["init", "--quiet", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").expect("write a file");
        self.git(&repo, &["add", "--all"]);
        self.git(&repo, &["commit", "--quiet", "-m", "first"]);
        self.program(
            &repo.join(".git/hooks/post-checkout"),
            &format!(
                "echo ran >> '{log}'\nwitness || echo 'no witness' >> '{log}'\nexit 0",
                log = self.hook_log().display()
            ),
        );
    }

    /// The environment of every run of git of the test, and of the Keeper's: no
    /// configuration of the user or of the system, an author, a home of the test's own.
    fn git_variables(&self) -> Vec<(&'static str, OsString)> {
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
        let mut all: Vec<_> = settings
            .into_iter()
            .map(|(name, value)| (name, OsString::from(value)))
            .collect();
        all.push(("HOME", self.home().into_os_string()));
        all
    }

    fn git(&self, folder: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").expect("a search path"))
            .envs(self.git_variables())
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

    /// Starts the Keeper program in an environment of the test's making: its own search
    /// path with the stand-in agent first (no `login-only`), the test's shell as the user's,
    /// and the settings of git. Attaches as a Window does, waiting for it to listen.
    fn start_keeper(&mut self) -> Window {
        let search = std::env::var_os("PATH").expect("a search path");
        let mut path = self.root.join("fake").into_os_string();
        path.push(":");
        path.push(&search);
        assert!(
            !std::env::split_paths(&path).any(|folder| folder == self.root.join("login-only")),
            "the folder only the login shell knows is on the search path of the Keeper"
        );
        let child = Command::new(env!("CARGO_BIN_EXE_yaar-keeper"))
            .arg("--socket")
            .arg(self.socket())
            .env_clear()
            .env("PATH", path)
            .env("SHELL", self.shell())
            .env("TMPDIR", self.root.join("tmp"))
            .env("YAAR_KEEPER_LINGER_MS", "60000")
            .envs(self.git_variables())
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(self.errors()).expect("make the file of the errors"))
            .spawn()
            .expect("start the Keeper");
        self.keeper = Some(child);
        self.window()
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

    /// Waits for `file` to hold `needle`, and gives what it held.
    fn wait_for_text(&self, file: &Path, needle: &str) -> String {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "{} never held {needle:?}; it held {text:?}",
                file.display()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The Keeper's end: asked for over the Link, and waited for.
    fn stop(&mut self, mut window: Window) {
        write_message(&mut window.stream, &Message::Shutdown).expect("ask for the stop");
        let mut child = self.keeper.take().expect("a Keeper");
        let (ended_tx, ended) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let status = child.wait().expect("wait for the Keeper");
            let _ = ended_tx.send(());
            status
        });
        ended
            .recv_timeout(DEADLINE)
            .expect("the Keeper did not end when it was asked to");
        assert!(waiter.join().expect("the waiter ended").success());
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A shell still held goes on and ends, and so does the Keeper, so that a test that
        // failed leaves nothing running. The Keeper is the one this test started.
        self.release();
        if let Some(mut child) = self.keeper.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
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

    /// Opens the folder as a workspace, which reads git and so must not wait for a shell
    /// that is held: it is answered within [`AT_ONCE`].
    fn open(&mut self, folder: &Path) -> (RepositoryId, WorkspaceId) {
        let id = self.send(Request::OpenFolder {
            path: folder.to_owned(),
        });
        match self
            .reply_within(id, AT_ONCE)
            .expect("a read waited for the login shell")
        {
            Ok(Reply::Opened {
                repository,
                workspace,
            }) => (repository, workspace),
            other => panic!("not opened: {other:?}"),
        }
    }

    /// Asks for a worktree on the new branch `task-one` from `main`, with the agent `claude`
    /// in it, and does not wait for the reply.
    fn create(&mut self, repository: &RepositoryId) -> RequestId {
        self.send(Request::CreateWorktree {
            repository: repository.clone(),
            from: "main".to_owned(),
            branch: "task-one".to_owned(),
            agent: "claude".to_owned(),
            size: SIZE,
        })
    }

    /// The reply to `id`, within `within`; none when there was none by then. A connection
    /// that fails is no silence and fails the test.
    fn reply_within(&mut self, id: RequestId, within: Duration) -> Option<Result<Reply, Refusal>> {
        self.stream
            .set_read_timeout(Some(within))
            .expect("set the deadline of a read");
        let deadline = Instant::now() + within;
        let found = loop {
            match read_message(&mut self.stream) {
                Ok(Message::Reply { to, reply }) if to == id => break Some(reply),
                Ok(Message::Reply { .. }) => panic!("the reply to a request nobody sent"),
                Ok(_) => {
                    if Instant::now() >= deadline {
                        break None;
                    }
                }
                Err(LinkError::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break None;
                }
                Err(other) => panic!("the Link failed: {other}"),
            }
        };
        self.stream
            .set_read_timeout(Some(DEADLINE))
            .expect("set the deadline of a read");
        found
    }
}

/// The lines of `text`.
fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn the_hook_of_a_worktree_finds_a_program_only_the_login_shells_search_path_holds() {
    let mut scratch = Scratch::new(Shell::Held);
    scratch.repository();
    let mut window = scratch.start_keeper();
    // The shell is under way and held: the take has begun and has not ended.
    scratch.wait_for_text(&scratch.held_note(), "held");
    let (repository, _) = window.open(&scratch.repo());

    let id = window.create(&repository);

    // The user's own git waits for the take: with the shell held, no worktree is made and
    // the hook has not run. A run that did not wait would have made it within this time.
    assert!(
        window.reply_within(id, WATCHED_FOR).is_none(),
        "the worktree was made before the login shell had given its environment"
    );
    assert!(
        !scratch.hook_log().exists(),
        "the hook ran before the login shell had given its environment"
    );
    scratch.release();
    let reply = window.reply_within(id, DEADLINE).expect("a reply");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "no worktree: {reply:?}"
    );

    // The hook ran once and found `witness`, which ran once, in the new worktree.
    assert_eq!(
        lines(&scratch.wait_for_text(&scratch.hook_log(), "ran")),
        ["ran"]
    );
    let witness = scratch.wait_for_text(&scratch.witness_log(), "witness ran in");
    assert_eq!(
        lines(&witness),
        [format!("witness ran in {}", scratch.worktree().display())]
    );
    assert!(scratch.worktree().is_dir());
    scratch.stop(window);
}

#[test]
fn a_shell_that_fails_still_lets_the_creation_answer_runs_no_witness_and_is_said_on_standard_error()
{
    let mut scratch = Scratch::new(Shell::Failing);
    scratch.repository();
    let mut window = scratch.start_keeper();
    let (repository, _) = window.open(&scratch.repo());

    let id = window.create(&repository);

    // Answered well before the shell's own deadline, which a take that never ended would
    // have made it wait for.
    let reply = window.reply_within(id, AT_ONCE).expect("a reply in time");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "no worktree: {reply:?}"
    );
    // The hook ran, and so had its chance to run the witness, which it did not find.
    let hook = scratch.wait_for_text(&scratch.hook_log(), "no witness");
    assert_eq!(lines(&hook), ["ran", "no witness"]);
    assert!(
        !scratch.witness_log().exists(),
        "the witness ran with the environment of a shell that failed"
    );
    // One line on the Keeper's standard error, naming the shell and why.
    let expected = format!(
        "yaar-keeper: the login shell {} gave no environment: it ended with status 3",
        scratch.shell().display()
    );
    let errors = scratch.wait_for_text(&scratch.errors(), &expected);
    assert_eq!(
        errors.lines().filter(|line| *line == expected).count(),
        1,
        "{errors}"
    );
    scratch.stop(window);
}

#[test]
fn a_read_is_answered_while_the_login_shell_is_held_and_never_waits_for_it() {
    let mut scratch = Scratch::new(Shell::Held);
    scratch.repository();
    let mut window = scratch.start_keeper();
    // The shell is under way and held until the test lets it go, after the answer.
    scratch.wait_for_text(&scratch.held_note(), "held");

    let (_, workspace) = window.open(&scratch.repo());
    let id = window.send(Request::GetChanges { workspace });

    // A run that waited for the shell would answer only at the shell's own deadline, ten
    // seconds on, and the held shell does not end before: this is within eight.
    let reply = window
        .reply_within(id, AT_ONCE)
        .expect("a read waited for the login shell");
    assert!(
        matches!(reply, Ok(Reply::Changes(_))),
        "no changes: {reply:?}"
    );
    scratch.release();
    scratch.stop(window);
}
