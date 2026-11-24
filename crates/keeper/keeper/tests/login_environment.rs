//! The Keeper is a staged copy of the built program; only the test's login shell (a script named by
//! `SHELL`) can put `login-only` on its search path.

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

use yaar_link::login::SHELL_DEADLINE;
use yaar_link::{
    Hello, LinkError, Message, Refusal, Reply, RepositoryId, Request, RequestId, TerminalSize,
    WorkspaceId, handshake, read_message, write_message,
};

const DEADLINE: Duration = Duration::from_secs(60);

/// Less than `SHELL_DEADLINE` (ten seconds), more than any read takes.
const AT_ONCE: Duration = Duration::from_secs(8);

const LEAVES_BEFORE: Duration = Duration::from_secs(SHELL_DEADLINE.as_secs() - 1);

const GAP: u64 = 8;

/// How much earlier than `SHELL_DEADLINE` a creation that waits for a held shell may be answered:
/// the time the shell took to start and the test to see it.
const EARLY: Duration = Duration::from_secs(1);

/// How much later than `SHELL_DEADLINE` a creation that waited for a held shell may be answered:
/// the shell's end, git and the hook.
const LATE: Duration = Duration::from_secs(3);

/// How long a creation is watched before the held shell is let go, the one wait here not on a
/// fact: a creation that does not wait for the shell is answered well within it.
const WATCHED_FOR: Duration = Duration::from_millis(500);

/// The shell sets it to another value; the Keeper's own environment is the base of the take, and
/// wins.
const MARK: &str = "YAAR_TEST_MARK";

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    Held,
    Failing,
}

struct Scratch {
    root: PathBuf,
    /// Kept open so that what is written stays in the pipe until the shell reads it.
    fifo: Option<File>,
    keeper: Option<Child>,
}

impl Scratch {
    fn new(shell: Shell) -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and git and the Keeper name a worktree by its resolved path.
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
        // A login shell's system profile may set the search path anew, so the home's `.profile`
        // puts the stand-in agent's folder first again. `login-only` is not put back.
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

    fn worktree(&self) -> PathBuf {
        self.root.join("lair/repo.worktrees/task-one")
    }

    fn shell(&self) -> PathBuf {
        self.root.join("login-shell")
    }

    fn witness_log(&self) -> PathBuf {
        self.root.join("witness.log")
    }

    fn hook_log(&self) -> PathBuf {
        self.root.join("hook.log")
    }

    fn errors(&self) -> PathBuf {
        self.root.join("keeper.err")
    }

    fn held_note(&self) -> PathBuf {
        self.root.join("held")
    }

    fn released_note(&self) -> PathBuf {
        self.root.join("released")
    }

    fn hook_environment(&self) -> PathBuf {
        self.root.join("hook-environment")
    }

    fn keeper_path(&self) -> OsString {
        let search = std::env::var_os("PATH").expect("a search path");
        let mut path = self.root.join("fake").into_os_string();
        path.push(":");
        path.push(&search);
        path
    }

    fn program(&self, path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make it run");
    }

    fn write_shell(&self, shell: Shell) {
        let hold = match shell {
            Shell::Held => format!(
                "printf '%s\\n' \"$$\" > '{note}'\nexec 3<> '{fifo}'\nread go <&3\nprintf released > '{released}'\n",
                note = self.held_note().display(),
                fifo = self.root.join("fifo").display(),
                released = self.released_note().display()
            ),
            Shell::Failing => String::new(),
        };
        let end = match shell {
            Shell::Failing => &format!(
                "printf '%s' 'yaar-environment-follows'\nenv -0\ndate +%s > '{}'\nexit 3\n",
                self.root.join("shell-ended").display()
            ),
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
                 PATH='{login_only}':\"$PATH\"\n\
                 {MARK}=shell\n\
                 export PATH {MARK}\n\
                 {end}",
                login_only = self.root.join("login-only").display()
            ),
        );
    }

    fn release(&mut self) {
        if let Some(fifo) = &mut self.fifo {
            fifo.write_all(b"go\n").expect("let the shell go");
        }
    }

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
                "date +%s > '{at}'\nprintf '%s\\n%s\\n' \"${MARK}\" \"$PATH\" > '{seen}'\necho ran >> '{log}'\nwitness || echo 'no witness' >> '{log}'\nexit 0",
                log = self.hook_log().display(),
                at = self.root.join("hook-ran-at").display(),
                seen = self.hook_environment().display()
            ),
        );
    }

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

    fn start_keeper(&mut self) -> Window {
        let path = self.keeper_path();
        assert!(
            !std::env::split_paths(&path).any(|folder| folder == self.root.join("login-only")),
            "the folder only the login shell knows is on the search path of the Keeper"
        );
        let child = Command::new(yaar_system_tests::program("yaar-keeper"))
            .arg("--socket")
            .arg(self.socket())
            .env_clear()
            .env("PATH", path)
            .env("SHELL", self.shell())
            .env(MARK, "keeper")
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

    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket()) {
                stream
                    .set_read_timeout(Some(DEADLINE))
                    .expect("set the deadline of a read");
                if handshake(&mut stream, &mine).is_ok() {
                    return Window { stream, next_id: 0 };
                }
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_text(&self, file: &Path, needle: &str) -> String {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "{} never held {needle:?}; it held {text:?}; the Keeper said {:?}",
                file.display(),
                std::fs::read_to_string(self.errors()).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_held(&self) -> (libc::pid_t, Instant) {
        let text = self.wait_for_text(&self.held_note(), "\n");
        let pid = text.trim().parse().expect("the process id of the shell");
        (pid, Instant::now())
    }

    fn wait_until_gone(&self, pid: libc::pid_t) {
        let deadline = Instant::now() + DEADLINE;
        while exists(pid) {
            assert!(
                Instant::now() < deadline,
                "the shell {pid} was still there after the Keeper ended"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

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
        self.release();
        if let Some(mut child) = self.keeper.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
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

    fn create(&mut self, repository: &RepositoryId) -> RequestId {
        self.send(Request::CreateWorktree {
            repository: repository.clone(),
            from: "main".to_owned(),
            branch: "task-one".to_owned(),
            agent: "claude".to_owned(),
            size: SIZE,
        })
    }

    fn reply_within(&mut self, id: RequestId, within: Duration) -> Option<Result<Reply, Refusal>> {
        let deadline = Instant::now() + within;
        let found = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break None;
            }
            self.stream
                .set_read_timeout(Some(left))
                .expect("set the deadline of a read");
            match read_message(&mut self.stream) {
                Ok(Message::Reply { to, reply }) if to == id => break Some(reply),
                Ok(Message::Reply { .. }) => panic!("the reply to a request nobody sent"),
                Ok(_) => {}
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

fn exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn the_hook_of_a_worktree_finds_a_program_only_the_login_shells_search_path_holds() {
    let mut scratch = Scratch::new(Shell::Held);
    scratch.repository();
    let mut window = scratch.start_keeper();
    scratch.wait_for_held();
    let (repository, _) = window.open(&scratch.repo());

    let id = window.create(&repository);

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

    let seen = std::fs::read_to_string(scratch.hook_environment()).expect("what the hook saw");
    let [mark, path] = lines(&seen)[..] else {
        panic!("the hook saw {seen:?}");
    };
    assert_eq!(mark, "keeper", "the shell's value won over the Keeper's");
    let mut expected: Vec<PathBuf> = std::env::split_paths(&scratch.keeper_path()).collect();
    expected.push(scratch.root.join("login-only"));
    let found: Vec<PathBuf> = std::env::split_paths(path).collect();
    assert!(
        found.ends_with(&expected),
        "the hook's search path {found:?} does not end with the Keeper's own, then the shell's folder: {expected:?}"
    );
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

    let reply = window.reply_within(id, DEADLINE).expect("a reply");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "no worktree: {reply:?}"
    );
    let hook = scratch.wait_for_text(&scratch.hook_log(), "no witness");
    assert_eq!(lines(&hook), ["ran", "no witness"]);
    let seconds = |name: &str| -> u64 {
        std::fs::read_to_string(scratch.root.join(name))
            .unwrap_or_else(|e| panic!("no {name}: {e}"))
            .trim()
            .parse()
            .expect("a time in seconds")
    };
    let waited = seconds("hook-ran-at").saturating_sub(seconds("shell-ended"));
    assert!(
        waited < GAP,
        "the hook ran {waited} s after the shell ended: the take was waited for to its deadline"
    );
    assert!(
        !scratch.witness_log().exists(),
        "the witness ran with the environment of a shell that failed"
    );
    let expected = format!(
        "yaar-keeper: the login shell {} gave no environment: it ended with status 3",
        scratch.shell().display()
    );
    scratch.wait_for_text(&scratch.errors(), &expected);
    // Counted once the Keeper has ended, so a line written again as it leaves is seen.
    scratch.stop(window);
    let errors = std::fs::read_to_string(scratch.errors()).expect("the Keeper's errors");
    assert_eq!(
        errors.lines().filter(|line| *line == expected).count(),
        1,
        "{errors}"
    );
}

#[test]
fn a_read_is_answered_while_the_login_shell_is_held_and_never_waits_for_it() {
    let mut scratch = Scratch::new(Shell::Held);
    scratch.repository();
    let mut window = scratch.start_keeper();
    let (shell, held_at) = scratch.wait_for_held();

    let (repository, workspace) = window.open(&scratch.repo());
    let id = window.send(Request::GetChanges { workspace });

    let reply = window
        .reply_within(id, AT_ONCE)
        .expect("a read waited for the login shell");
    assert!(
        matches!(reply, Ok(Reply::Changes(_))),
        "no changes: {reply:?}"
    );
    assert!(
        held_at.elapsed() < AT_ONCE,
        "the answer came {:?} after the shell began to wait",
        held_at.elapsed()
    );
    assert!(
        exists(shell),
        "the shell had ended when the read was answered"
    );
    assert!(
        !scratch.released_note().exists(),
        "the shell was let go when the read was answered"
    );

    // A creation waits for the held shell and is in flight when the Keeper is asked to stop: the
    // Keeper must end the shell instead of waiting for its deadline.
    let creation = window.create(&repository);
    assert!(
        window.reply_within(creation, WATCHED_FOR).is_none(),
        "a creation was answered while the login shell was held"
    );

    scratch.stop(window);
    assert!(
        held_at.elapsed() < LEAVES_BEFORE,
        "the Keeper ended {:?} after the shell began to wait: it waited for the shell",
        held_at.elapsed()
    );
    scratch.wait_until_gone(shell);
    assert!(
        !scratch.released_note().exists(),
        "the shell went on after it was let go, not ended"
    );
    let errors = std::fs::read_to_string(scratch.errors()).expect("the Keeper's errors");
    assert!(
        !errors.contains("gave no environment"),
        "a shell ended for the Keeper's leave was said to have failed: {errors}"
    );
}

#[test]
fn a_shell_held_past_the_deadline_the_program_hands_is_given_up_and_the_creation_answers_then() {
    let mut scratch = Scratch::new(Shell::Held);
    scratch.repository();
    let mut window = scratch.start_keeper();
    let (shell, held_at) = scratch.wait_for_held();
    let (repository, _) = window.open(&scratch.repo());

    let id = window.create(&repository);

    let reply = window
        .reply_within(id, (SHELL_DEADLINE + LATE).saturating_sub(held_at.elapsed()),
        )
        .unwrap_or_else(|| {
            panic!(
                "no reply {:?} after the shell began to wait: the deadline the program hands is longer than {SHELL_DEADLINE:?}",
                held_at.elapsed()
            )
        });
    let answered = held_at.elapsed();
    assert!(
        answered >= SHELL_DEADLINE - EARLY,
        "the creation was answered {answered:?} after the shell began to wait, before its deadline of {SHELL_DEADLINE:?}"
    );
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "no worktree: {reply:?}"
    );
    assert!(
        !scratch.released_note().exists(),
        "the shell was let go and not ended"
    );
    scratch.wait_until_gone(shell);
    let hook = scratch.wait_for_text(&scratch.hook_log(), "no witness");
    assert_eq!(lines(&hook), ["ran", "no witness"]);
    assert!(
        !scratch.witness_log().exists(),
        "the witness ran with the environment of a shell that never gave one"
    );
    let expected = format!(
        "yaar-keeper: the login shell {} gave no environment: it took longer than {} s",
        scratch.shell().display(),
        SHELL_DEADLINE.as_secs()
    );
    scratch.wait_for_text(&scratch.errors(), &expected);
    scratch.stop(window);
    let errors = std::fs::read_to_string(scratch.errors()).expect("the Keeper's errors");
    assert_eq!(
        errors.lines().filter(|line| *line == expected).count(),
        1,
        "{errors}"
    );
}
