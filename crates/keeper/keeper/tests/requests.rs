//! Component tests: what the Keeper answers and tells, through the surface a Window
//! uses, with real repositories the tests make and a real database beneath.
//!
//! The Keeper runs git as its user does, with the environment of its process. Each test
//! therefore cleans that environment as its first statement, and these tests are run by
//! `cargo nextest`, which gives each test a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, Base, BaseSource, ChangedFile, Changes, CommentId, Commit, Comparison, Content,
    DraftOn, Fact, FileSides, FileStatus, Head, Hello, Lines, MAX_CONTENT, Message,
    PROTOCOL_VERSION, Program, Refusal, Reply, Repository, Request, RequestId, RequestKind,
    ReviewId, Side, TerminalId, TerminalSize, ThreadId, Workspace, WorkspaceId, WorkspaceReviews,
    handshake, read_message, write_message,
};

/// What the one file of a new repository holds.
const FIVE_LINES: &str = "one\ntwo\nthree\nfour\nfive\n";

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and cleans the environment of this process for git: no
    /// configuration of the user or of the system, a fixed author and date, an empty
    /// home. It has to be the first statement of a test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-requests-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).expect("make the test's folder");
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
        // SAFETY: changing the environment is sound only while no other thread reads it,
        // as a thread that starts a process does. This runs as the first statement of a
        // test, and these tests are run by `cargo nextest`, which gives each test a
        // process of its own: no other thread exists yet.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
        };
        std::thread::spawn(move || run(options))
    }

    /// Attaches as a Window does, waiting for the Keeper to listen.
    fn window(&self) -> Window {
        self.window_speaking(PROTOCOL_VERSION)
    }

    fn window_speaking(&self, protocol: u16) -> Window {
        let mut mine = Hello::for_this_process("test-window");
        mine.protocol = protocol;
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

    /// A repository in the folder `name` with one commit on `main`. Returns the folder
    /// and the commit.
    fn repository(&self, name: &str) -> (PathBuf, String) {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        std::fs::write(folder.join("a.txt"), FIVE_LINES).expect("write a file");
        let first = commit_all(&folder, "first");
        (folder, first)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
    String::from_utf8(output.stdout)
        .expect("git printed text")
        .trim()
        .to_owned()
}

fn commit_all(folder: &Path, message: &str) -> String {
    git(folder, &["add", "--all"]);
    git(folder, &["commit", "--quiet", "-m", message]);
    git(folder, &["rev-parse", "HEAD"])
}

/// One end of the Link, as a Window holds it.
struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    /// Sends a request and returns its id, without waiting for anything.
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

    /// Expects a fact as the very next frame.
    fn fact(&mut self) -> Fact {
        match self.next() {
            Message::Fact(fact) => fact,
            other => panic!("not a fact: {other:?}"),
        }
    }

    fn open(&mut self, folder: &Path) -> WorkspaceId {
        match self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        }) {
            Ok(Reply::Opened { workspace, .. }) => workspace,
            other => panic!("not opened: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn added(path: &str) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status: FileStatus::Added,
    }
}

#[test]
fn the_window_that_asked_is_answered_first_and_then_every_window_is_told_what_changed() {
    let scratch = Scratch::new();
    let (repo, first) = scratch.repository("repo");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    std::fs::write(repo.join("b.txt"), "new\n").expect("write a file");
    let second = commit_all(&repo, "second");
    let keeper = scratch.keeper();
    let mut asking = scratch.window();
    let mut other = scratch.window();

    // Opening a folder: the reply, then the list, to both Windows.
    let id = asking.send(Request::OpenFolder { path: repo.clone() });
    let Message::Reply {
        to,
        reply: Ok(Reply::Opened {
            repository,
            workspace,
        }),
    } = asking.next()
    else {
        panic!("the first frame is not the reply");
    };
    assert_eq!(to, id);
    let list = vec![Repository {
        id: repository,
        name: "repo".to_owned(),
        workspaces: vec![Workspace {
            id: workspace.clone(),
            path: repo.clone(),
            head: Some(Head::Branch("feature".to_owned())),
        }],
    }];
    assert_eq!(asking.fact(), Fact::Workspaces(list.clone()));
    assert_eq!(other.fact(), Fact::Workspaces(list));

    // A worktree made with git, outside the app, is in no fact: no Window has heard of
    // it. Opening it tells the list that holds it, though the Keeper, which reads the
    // list from git, finds the same list before and after.
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "second", "../linked"],
    );
    let linked = scratch.0.join("linked");
    let second_workspace = asking.open(&linked);
    let Fact::Workspaces(told) = asking.fact() else {
        panic!("the list is not told");
    };
    assert_eq!(other.fact(), Fact::Workspaces(told.clone()));
    assert_eq!(
        told[0].workspaces[1],
        Workspace {
            id: second_workspace,
            path: linked,
            head: Some(Head::Branch("second".to_owned())),
        }
    );
    // The list is told for a folder that was open already, too.
    assert_eq!(asking.open(&repo), workspace);
    assert_eq!(asking.fact(), Fact::Workspaces(told.clone()));
    assert_eq!(other.fact(), Fact::Workspaces(told.clone()));
    assert_eq!(
        asking.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(told))
    );

    // Setting the base, from the other Window: its reply, then the fact to both.
    assert_eq!(
        other.ask(Request::SetBase {
            workspace: workspace.clone(),
            base: "HEAD~1".to_owned(),
        }),
        Ok(Reply::Done)
    );
    let changed = Fact::ChangesChanged {
        workspace: workspace.clone(),
    };
    assert_eq!(other.fact(), changed);
    assert_eq!(asking.fact(), changed);

    // And the changes are counted from where the base was put.
    assert_eq!(
        asking.ask(Request::GetChanges {
            workspace: workspace.clone(),
        }),
        Ok(Reply::Changes(Changes {
            workspace,
            comparison: Comparison::SinceBase {
                base: Base {
                    name: first.clone(),
                    source: BaseSource::User,
                },
            },
            counted_from: Some(first),
            commits: vec![Commit {
                id: second,
                summary: "second".to_owned(),
            }],
            files: vec![added("b.txt")],
        }))
    );
    asking.stop(keeper);
}

#[test]
fn the_changes_of_a_branch_and_the_versions_of_its_files_come_in_the_words_of_the_link() {
    let scratch = Scratch::new();
    let (repo, first) = scratch.repository("repo");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    let six_lines = format!("{FIVE_LINES}six\n");
    std::fs::write(repo.join("a.txt"), &six_lines).expect("change a file");
    git(&repo, &["mv", "a.txt", "moved.txt"]);
    let second = commit_all(&repo, "second");
    // Not committed: something that is not text, and something too large to send.
    std::fs::write(repo.join("image.bin"), [1, 2, 0, 3]).expect("write a file");
    let large = vec![b'a'; MAX_CONTENT as usize + 1];
    std::fs::write(repo.join("large.txt"), &large).expect("write a file");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    window.fact();

    let changes = match window.ask(Request::GetChanges {
        workspace: workspace.clone(),
    }) {
        Ok(Reply::Changes(changes)) => changes,
        other => panic!("no changes: {other:?}"),
    };
    assert_eq!(
        changes,
        Changes {
            workspace: workspace.clone(),
            comparison: Comparison::SinceBase {
                base: Base {
                    name: "main".to_owned(),
                    source: BaseSource::Default,
                },
            },
            counted_from: Some(first.clone()),
            commits: vec![Commit {
                id: second,
                summary: "second".to_owned(),
            }],
            files: vec![
                added("image.bin"),
                added("large.txt"),
                ChangedFile {
                    path: PathBuf::from("moved.txt"),
                    status: FileStatus::Renamed {
                        from: PathBuf::from("a.txt"),
                    },
                },
            ],
        }
    );

    let mut read = |since: Option<&str>, old_path: &str, path: &str| {
        window.ask(Request::ReadFile {
            workspace: workspace.clone(),
            since: since.map(str::to_owned),
            old_path: PathBuf::from(old_path),
            path: PathBuf::from(path),
        })
    };
    let sides = |old, new| Ok(Reply::File(FileSides { old, new }));
    assert_eq!(
        read(Some(&first), "a.txt", "moved.txt"),
        sides(
            Content::Text(FIVE_LINES.into()),
            Content::Text(six_lines.into_bytes())
        )
    );
    assert_eq!(
        read(None, "image.bin", "image.bin"),
        sides(Content::Absent, Content::Binary { size: 4 })
    );
    assert_eq!(
        read(None, "large.txt", "large.txt"),
        sides(
            Content::Absent,
            Content::TooLarge {
                size: MAX_CONTENT + 1
            }
        )
    );

    // What is refused keeps its kind, so that a Window can say what to mend.
    assert_eq!(
        read(Some(&first), "a.txt", "../repo/a.txt"),
        Err(Refusal::OutsideWorkspace {
            path: PathBuf::from("../repo/a.txt")
        })
    );
    let nowhere = "0".repeat(40);
    assert_eq!(
        read(Some(&nowhere), "a.txt", "moved.txt"),
        Err(Refusal::UnknownRevision {
            revision: nowhere.clone()
        })
    );
    assert_eq!(
        window.ask(Request::SetBase {
            workspace: workspace.clone(),
            base: "no-such-branch".to_owned(),
        }),
        Err(Refusal::UnknownRevision {
            revision: "no-such-branch".to_owned()
        })
    );
    let nobody = WorkspaceId::new("f".repeat(32));
    assert_eq!(
        window.ask(Request::GetChanges {
            workspace: nobody.clone(),
        }),
        Err(Refusal::UnknownWorkspace { workspace: nobody })
    );
    assert_eq!(
        window.ask(Request::OpenFolder {
            path: scratch.0.join("home"),
        }),
        Err(Refusal::NotARepository {
            path: scratch.0.join("home")
        })
    );
    // A path counted from wherever the Keeper happens to run names no folder.
    assert_eq!(
        window.ask(Request::OpenFolder {
            path: PathBuf::from("repo"),
        }),
        Err(Refusal::NoSuchFolder {
            path: PathBuf::from("repo")
        })
    );
    // None of the refusals told anybody anything: the stop is taken at once.
    window.stop(keeper);
}

#[test]
fn a_keeper_that_finds_no_git_says_who_it_is_and_refuses_each_request_with_the_reason() {
    let scratch = Scratch::new();
    let empty = scratch.0.join("no-programs");
    std::fs::create_dir_all(&empty).expect("make an empty folder");
    // SAFETY: as in `Scratch::new`: still the only thread of this process.
    unsafe { std::env::set_var("PATH", &empty) };
    let keeper = scratch.keeper();

    // `window` has exchanged hellos, or it would not have come back.
    let mut window = scratch.window();
    let workspace = WorkspaceId::new("0".repeat(32));
    for kind in RequestKind::ALL {
        let request = match kind {
            RequestKind::ListWorkspaces => Request::ListWorkspaces,
            RequestKind::OpenFolder => Request::OpenFolder {
                path: scratch.0.clone(),
            },
            RequestKind::SetBase => Request::SetBase {
                workspace: workspace.clone(),
                base: "main".to_owned(),
            },
            RequestKind::GetChanges => Request::GetChanges {
                workspace: workspace.clone(),
            },
            RequestKind::ReadFile => Request::ReadFile {
                workspace: workspace.clone(),
                since: None,
                old_path: PathBuf::from("a.txt"),
                path: PathBuf::from("a.txt"),
            },
            RequestKind::OpenTerminal => Request::OpenTerminal {
                workspace: workspace.clone(),
                program: Program::Shell,
                size: TerminalSize {
                    columns: 80,
                    rows: 24,
                },
            },
            RequestKind::ResizeTerminal => Request::ResizeTerminal {
                terminal: TerminalId::new("0".repeat(32)),
                size: TerminalSize {
                    columns: 80,
                    rows: 24,
                },
            },
            RequestKind::GetReviews => Request::GetReviews {
                workspace: workspace.clone(),
            },
            RequestKind::SaveDraft => Request::SaveDraft {
                review: ReviewId::new("0".repeat(32)),
                thread: ThreadId::new("0".repeat(32)),
                comment: CommentId::new("0".repeat(32)),
                on: DraftOn::Thread,
                text: "a".to_owned(),
            },
            RequestKind::DeleteDraft => Request::DeleteDraft {
                review: ReviewId::new("0".repeat(32)),
                comment: CommentId::new("0".repeat(32)),
            },
            RequestKind::CopyReview => Request::CopyReview {
                review: ReviewId::new("0".repeat(32)),
            },
            RequestKind::MarkReviewed => Request::MarkReviewed {
                review: ReviewId::new("0".repeat(32)),
            },
            RequestKind::SetViewed => Request::SetViewed {
                workspace: workspace.clone(),
                path: PathBuf::from("a.txt"),
                viewed: true,
            },
            RequestKind::Commit => Request::Commit {
                workspace: workspace.clone(),
                message: "Commit".to_owned(),
            },
        };
        assert_eq!(
            window.ask(request),
            Err(Refusal::Failed {
                reason: "git was not found".to_owned()
            }),
            "{kind:?}"
        );
    }
    window.stop(keeper);
}

#[test]
fn an_answer_too_large_for_one_frame_is_refused_with_the_reason_and_the_window_is_served_on() {
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    // New files whose paths alone are more than a frame holds: 18,000 paths of 950
    // bytes. The folders are as deep as the longest path of macOS allows.
    let deep = repo
        .join("d".repeat(249))
        .join("e".repeat(249))
        .join("f".repeat(249));
    std::fs::create_dir_all(&deep).expect("make the folders");
    for n in 0..18_000 {
        std::fs::write(deep.join(format!("{}{n:07}", "g".repeat(193))), "").expect("write");
    }
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    window.fact();

    let refused = window.ask(Request::GetChanges {
        workspace: workspace.clone(),
    });
    let Err(Refusal::Failed { reason }) = &refused else {
        panic!("not refused: {refused:?}");
    };
    assert!(reason.contains("does not fit in one frame"), "{reason}");

    // The connection is whole: the next request is answered as ever.
    assert!(matches!(
        window.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(list)) if list.len() == 1
    ));
    window.stop(keeper);
}

#[test]
fn a_window_that_stops_reading_is_let_go_and_the_others_are_told_all_the_same() {
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    // More than the buffers of a socket hold on any system, and still text that travels.
    std::fs::write(repo.join("long.txt"), vec![b'a'; 3 << 20]).expect("write a file");
    let keeper = scratch.keeper();
    let mut stuck = scratch.window();
    let workspace = stuck.open(&repo);
    stuck.fact();
    let mut other = scratch.window();

    // One Window asks for more than fits in the buffers and reads none of it.
    stuck.send(Request::ReadFile {
        workspace: workspace.clone(),
        since: None,
        old_path: PathBuf::from("long.txt"),
        path: PathBuf::from("long.txt"),
    });

    // The other is answered meanwhile, and is told the fact that is owed to both: the
    // Keeper does not wait for the first for ever.
    assert_eq!(
        other.ask(Request::SetBase {
            workspace: workspace.clone(),
            base: "main".to_owned(),
        }),
        Ok(Reply::Done)
    );
    assert_eq!(other.fact(), Fact::ChangesChanged { workspace });

    // The first is let go. Reading from it here would be taking what it was sent, so
    // the test writes to it until the Keeper has closed the connection: a hello, which
    // the Keeper passes over when it reads one.
    let hello = Message::Hello(Hello::for_this_process("test-window"));
    let deadline = Instant::now() + DEADLINE;
    while write_message(&mut stuck.stream, &hello).is_ok() {
        assert!(Instant::now() < deadline, "the Window is still attached");
        std::thread::sleep(Duration::from_millis(10));
    }
    // What was written to it before ends inside the reply it did not take: the fact
    // may have reached it whole, the reply never.
    loop {
        match read_message(&mut stuck.stream) {
            Ok(Message::Fact(_)) => {}
            Ok(other) => panic!("a Window that took nothing was sent all of {other:?}"),
            Err(_) => break,
        }
    }

    // The Keeper serves on.
    assert!(matches!(
        other.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(list)) if list.len() == 1
    ));
    other.stop(keeper);
}

#[test]
fn a_window_of_another_version_is_told_no_fact_and_is_let_go_when_it_asks_for_more_than_the_stop() {
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut newer = scratch.window_speaking(PROTOCOL_VERSION + 1);
    let mut window = scratch.window();

    // A fact goes out to every Window that speaks this version.
    window.open(&repo);
    assert!(matches!(window.fact(), Fact::Workspaces(_)));

    // The other Window asks, which it may not. Its connection ends, and the end is the
    // first thing it reads: no fact was sent to it before.
    newer.send(Request::ListWorkspaces);
    assert!(matches!(
        read_message(&mut newer.stream),
        Err(yaar_link::LinkError::Closed)
    ));
    window.stop(keeper);
}

/// The git processes this test process has started and not yet collected.
///
/// Only those that run git by name are counted. While a process is started, and for a
/// moment after one has ended, macOS lists a child under the name of its parent that is
/// on its way out; such a one is no git process that runs.
fn git_children() -> Vec<u32> {
    let me = std::process::id();
    let is_git = |name: &str| name == "git" || name.ends_with("/git");
    if cfg!(target_os = "linux") {
        // The name stands between brackets, and the parent is the second field after
        // the last `)`.
        std::fs::read_dir("/proc")
            .expect("read /proc")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        let (before, rest) = stat.rsplit_once(')')?;
                        let parent = rest.split_whitespace().nth(1)?.parse::<u32>().ok()?;
                        Some(parent == me && is_git(before.split_once('(')?.1))
                    })
                    == Some(true)
            })
            .collect()
    } else {
        let listing = Command::new("ps")
            .args(["-axo", "pid=,ppid=,comm="])
            .stdin(Stdio::null())
            .output()
            .expect("start ps");
        assert!(listing.status.success(), "ps failed");
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse::<u32>().ok()?;
                let parent = fields.next()?.parse::<u32>().ok()?;
                (parent == me && is_git(fields.next()?)).then_some(pid)
            })
            .collect()
    }
}

/// Whether the system still knows a process by this id: one that runs, or one that has
/// ended and that nobody has collected.
fn is_known(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; it only asks whether the process is there.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[test]
fn the_git_process_kept_for_file_contents_has_ended_when_the_keeper_returns() {
    let scratch = Scratch::new();
    let (repo, first) = scratch.repository("repo");
    assert_eq!(git_children(), vec![], "no git runs yet");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    window.fact();

    // The old version of a file is read through a git process that stays.
    let read = window.ask(Request::ReadFile {
        workspace,
        since: Some(first),
        old_path: PathBuf::from("a.txt"),
        path: PathBuf::from("a.txt"),
    });
    assert!(read.is_ok(), "{read:?}");
    let kept = git_children();
    let [kept] = kept[..] else {
        panic!("not the one git process of the worktree: {kept:?}");
    };
    assert!(is_known(kept));

    // When the Keeper has returned, that process has ended and was collected: the
    // system knows it no more.
    window.stop(keeper);
    assert!(!is_known(kept), "the git process outlived the Keeper");
}

/// The ids of a thread and of its comment that the reviews tests draw: 32 digits each,
/// and no two alike, since the Keeper takes an id once whatever it names.
fn ids(n: u8) -> (ThreadId, CommentId) {
    (
        ThreadId::new(format!("{n:02x}").repeat(16)),
        CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
    )
}

/// Where a new thread sits: line 1 of `a.txt` in the files on disk.
fn on_the_first_line() -> DraftOn {
    DraftOn::NewThread {
        anchor: Anchor {
            tree: None,
            path: PathBuf::from("a.txt"),
            side: Side::New,
            lines: Lines { first: 1, last: 1 },
            text: "one".to_owned(),
        },
    }
}

impl Window {
    /// The reviews of a workspace.
    fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn save(
        &mut self,
        review: &ReviewId,
        thread: &ThreadId,
        comment: &CommentId,
        on: DraftOn,
        text: &str,
    ) -> Result<Reply, Refusal> {
        self.ask(Request::SaveDraft {
            review: review.clone(),
            thread: thread.clone(),
            comment: comment.clone(),
            on,
            text: text.to_owned(),
        })
    }
}

#[test]
fn a_draft_is_made_edited_copied_and_deleted_and_every_window_is_told_of_what_changed() {
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut typing = scratch.window();
    let workspace = typing.open(&repo);
    assert!(matches!(typing.fact(), Fact::Workspaces(_)));
    let mut watching = scratch.window();

    // A workspace has one review, of no agent, from the first time it is asked for, and it
    // is the same one the next time.
    let reviews = typing.reviews(&workspace);
    assert_eq!(reviews.workspace, workspace);
    assert_eq!(reviews.viewed, Vec::<PathBuf>::new());
    let [review] = &reviews.reviews[..] else {
        panic!("not one review: {:?}", reviews.reviews);
    };
    assert_eq!(review.agent, None);
    let review = review.id.clone();
    assert_eq!(typing.reviews(&workspace).reviews[0].id, review);

    // The first save makes the thread and is told to both Windows; one that changes the
    // text is told to nobody, so the next reply is the next frame.
    let (thread, comment) = ids(1);
    let changed = Fact::ReviewsChanged {
        workspace: workspace.clone(),
    };
    assert_eq!(
        typing.save(&review, &thread, &comment, on_the_first_line(), "o"),
        Ok(Reply::Done)
    );
    assert_eq!(typing.fact(), changed);
    assert_eq!(watching.fact(), changed);
    assert_eq!(
        typing.save(&review, &thread, &comment, DraftOn::Thread, "ok"),
        Ok(Reply::Done)
    );
    let held = typing.reviews(&workspace);
    let kept = &held.reviews[0].threads[0];
    assert_eq!(kept.id, thread);
    assert_eq!(kept.anchor.path, PathBuf::from("a.txt"));
    assert_eq!(kept.comments[0].text, "ok");
    assert_eq!(kept.comments[0].round, None);

    // A reply in a thread with nothing sent, and a save in a thread the review lacks.
    let (_, reply) = ids(2);
    assert!(matches!(
        typing.save(&review, &thread, &reply, DraftOn::Thread, "no"),
        Err(Refusal::Failed { .. })
    ));
    let (nowhere, other) = ids(3);
    assert_eq!(
        typing.save(&review, &nowhere, &other, DraftOn::Thread, "no"),
        Err(Refusal::UnknownThread { thread: nowhere })
    );
    let nobody = ReviewId::new("0".repeat(32));
    assert_eq!(
        typing.save(&nobody, &thread, &other, on_the_first_line(), "no"),
        Err(Refusal::UnknownReview {
            review: nobody.clone()
        })
    );

    // A draft is deleted, and the thread goes with it; another is copied.
    let (second_thread, second) = ids(4);
    assert_eq!(
        typing.save(&review, &second_thread, &second, on_the_first_line(), "x"),
        Ok(Reply::Done)
    );
    assert_eq!(typing.fact(), changed);
    assert_eq!(watching.fact(), changed);
    assert_eq!(
        typing.ask(Request::DeleteDraft {
            review: review.clone(),
            comment: second.clone(),
        }),
        Ok(Reply::Done)
    );
    assert_eq!(typing.fact(), changed);
    assert_eq!(watching.fact(), changed);
    assert_eq!(typing.reviews(&workspace).reviews[0].threads.len(), 1);
    assert_eq!(
        typing.ask(Request::DeleteDraft {
            review: review.clone(),
            comment: second.clone(),
        }),
        Err(Refusal::UnknownComment {
            comment: second.clone()
        })
    );

    let Ok(Reply::Copied { text, round }) = typing.ask(Request::CopyReview {
        review: review.clone(),
    }) else {
        panic!("not copied");
    };
    assert_eq!(round, 1);
    assert!(text.contains("ok") && text.contains("a.txt"), "{text}");
    assert_eq!(typing.fact(), changed);
    assert_eq!(watching.fact(), changed);

    // What was sent is not a draft any more, and a second copy has nothing to send.
    assert_eq!(
        typing.save(&review, &thread, &comment, DraftOn::Thread, "ok!"),
        Err(Refusal::NotADraft {
            comment: comment.clone()
        })
    );
    assert_eq!(
        typing.ask(Request::DeleteDraft {
            review: review.clone(),
            comment: comment.clone(),
        }),
        Err(Refusal::NotADraft { comment })
    );
    assert_eq!(
        typing.ask(Request::CopyReview {
            review: review.clone()
        }),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );
    let sent = typing.reviews(&workspace);
    assert_eq!(sent.reviews[0].rounds.len(), 1);
    assert_eq!(sent.reviews[0].threads[0].comments[0].round, Some(1));

    // What the Keeper does not hold.
    for request in [
        Request::CopyReview {
            review: nobody.clone(),
        },
        Request::MarkReviewed {
            review: nobody.clone(),
        },
        Request::DeleteDraft {
            review: nobody.clone(),
            comment: second,
        },
    ] {
        assert_eq!(
            typing.ask(request),
            Err(Refusal::UnknownReview {
                review: nobody.clone()
            })
        );
    }
    let unknown = WorkspaceId::new("0".repeat(32));
    assert_eq!(
        typing.ask(Request::GetReviews {
            workspace: unknown.clone()
        }),
        Err(Refusal::UnknownWorkspace {
            workspace: unknown.clone()
        })
    );
    typing.stop(keeper);
}

#[test]
fn a_mark_of_reviewed_a_viewed_file_and_a_commit_are_answered_and_told() {
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    assert!(matches!(window.fact(), Fact::Workspaces(_)));
    let review = window.reviews(&workspace).reviews[0].id.clone();
    let changed = Fact::ReviewsChanged {
        workspace: workspace.clone(),
    };

    // Marked reviewed: the review holds the state it was marked at.
    assert_eq!(window.reviews(&workspace).reviews[0].reviewed, None);
    assert_eq!(
        window.ask(Request::MarkReviewed {
            review: review.clone()
        }),
        Ok(Reply::Done)
    );
    assert_eq!(window.fact(), changed);
    assert!(window.reviews(&workspace).reviews[0].reviewed.is_some());

    // A file is marked viewed for its content, and loses the mark when it changes; a path
    // that leads out of the worktree is refused.
    let view = |window: &mut Window, path: &str, viewed| {
        window.ask(Request::SetViewed {
            workspace: workspace.clone(),
            path: PathBuf::from(path),
            viewed,
        })
    };
    assert_eq!(view(&mut window, "a.txt", true), Ok(Reply::Done));
    assert_eq!(window.fact(), changed);
    assert_eq!(
        window.reviews(&workspace).viewed,
        vec![PathBuf::from("a.txt")]
    );
    assert_eq!(
        view(&mut window, "../outside.txt", true),
        Err(Refusal::OutsideWorkspace {
            path: PathBuf::from("../outside.txt")
        })
    );
    std::fs::write(repo.join("a.txt"), "changed\n").expect("change the file");
    assert_eq!(window.reviews(&workspace).viewed, Vec::<PathBuf>::new());
    assert_eq!(view(&mut window, "a.txt", true), Ok(Reply::Done));
    assert_eq!(window.fact(), changed);
    assert_eq!(view(&mut window, "a.txt", false), Ok(Reply::Done));
    assert_eq!(window.fact(), changed);
    assert_eq!(window.reviews(&workspace).viewed, Vec::<PathBuf>::new());

    // The commit: the changes of the workspace changed, and the review is as it was.
    let reviewed = window.reviews(&workspace).reviews[0].reviewed.clone();
    let Ok(Reply::Committed { commit }) = window.ask(Request::Commit {
        workspace: workspace.clone(),
        message: "Change the file".to_owned(),
    }) else {
        panic!("not committed");
    };
    assert_eq!(commit, git(&repo, &["rev-parse", "HEAD"]));
    assert_eq!(
        window.fact(),
        Fact::ChangesChanged {
            workspace: workspace.clone()
        }
    );
    assert_eq!(window.reviews(&workspace).reviews[0].reviewed, reviewed);
    assert_eq!(
        window.ask(Request::Commit {
            workspace: workspace.clone(),
            message: "Nothing".to_owned(),
        }),
        Err(Refusal::NothingToCommit {
            workspace: workspace.clone()
        })
    );

    // A hook that refuses the commit: git's words come back, nothing is committed.
    std::fs::write(repo.join("a.txt"), "again\n").expect("change the file");
    let hook = repo.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho no-commits-today >&2\nexit 1\n").expect("hook");
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make the hook run");
    let Err(Refusal::Failed { reason }) = window.ask(Request::Commit {
        workspace: workspace.clone(),
        message: "Again".to_owned(),
    }) else {
        panic!("not refused");
    };
    assert!(reason.contains("no-commits-today"), "{reason}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), commit);

    let unknown = WorkspaceId::new("0".repeat(32));
    assert_eq!(
        window.ask(Request::Commit {
            workspace: unknown.clone(),
            message: "No".to_owned(),
        }),
        Err(Refusal::UnknownWorkspace { workspace: unknown })
    );
    window.stop(keeper);
}

#[test]
fn a_worktree_that_is_gone_keeps_its_reviews_and_the_text_of_its_drafts_and_refuses_what_needs_it()
{
    let scratch = Scratch::new();
    let (repo, _) = scratch.repository("repo");
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "linked", "../linked"],
    );
    let linked = scratch.0.join("linked");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&linked);
    assert!(matches!(window.fact(), Fact::Workspaces(_)));
    let review = window.reviews(&workspace).reviews[0].id.clone();
    let (thread, comment) = ids(1);
    assert_eq!(
        window.save(&review, &thread, &comment, on_the_first_line(), "k"),
        Ok(Reply::Done)
    );
    assert!(matches!(window.fact(), Fact::ReviewsChanged { .. }));

    // The worktree is removed with git.
    git(&repo, &["worktree", "remove", "--force", "../linked"]);

    // The text of a draft needs no worktree, and the reviews are read as they were.
    assert_eq!(
        window.save(&review, &thread, &comment, DraftOn::Thread, "kept"),
        Ok(Reply::Done)
    );
    let held = window.reviews(&workspace);
    assert_eq!(held.reviews[0].threads[0].comments[0].text, "kept");
    assert_eq!(held.viewed, Vec::<PathBuf>::new());

    // What needs the files is refused with the words of the engine.
    let gone = Refusal::Failed {
        reason: "the worktree is gone".to_owned(),
    };
    let (other_thread, other) = ids(2);
    for request in [
        Request::CopyReview {
            review: review.clone(),
        },
        Request::MarkReviewed {
            review: review.clone(),
        },
        Request::SetViewed {
            workspace: workspace.clone(),
            path: PathBuf::from("a.txt"),
            viewed: true,
        },
        Request::Commit {
            workspace: workspace.clone(),
            message: "No".to_owned(),
        },
        Request::SaveDraft {
            review: review.clone(),
            thread: other_thread,
            comment: other,
            on: on_the_first_line(),
            text: "no".to_owned(),
        },
    ] {
        assert_eq!(
            window.ask(request.clone()),
            Err(gone.clone()),
            "{request:?}"
        );
    }
    window.stop(keeper);
}
