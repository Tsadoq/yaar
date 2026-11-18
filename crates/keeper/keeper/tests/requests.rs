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
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, Fact, FileSides,
    FileStatus, Head, Hello, MAX_CONTENT, Message, PROTOCOL_VERSION, Refusal, Reply, Repository,
    Request, RequestId, RequestKind, Workspace, WorkspaceId, handshake, read_message,
    write_message,
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
            head: Head::Branch("feature".to_owned()),
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
            head: Head::Branch("second".to_owned()),
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

/// The processes this test process has started and not yet collected.
fn children() -> Vec<u32> {
    let me = std::process::id();
    if cfg!(target_os = "linux") {
        // The parent is the second field after the name, and the name ends at the
        // last `)`.
        std::fs::read_dir("/proc")
            .expect("read /proc")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        let (_, rest) = stat.rsplit_once(')')?;
                        rest.split_whitespace().nth(1)?.parse::<u32>().ok()
                    })
                    == Some(me)
            })
            .collect()
    } else {
        let listing = Command::new("ps")
            .args(["-axo", "pid=,ppid="])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start ps");
        // `ps` is a child of this process too, while it lists.
        let ps = listing.id();
        let output = listing.wait_with_output().expect("wait for ps");
        assert!(output.status.success(), "ps failed");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse::<u32>().ok()?;
                let parent = fields.next()?.parse::<u32>().ok()?;
                (parent == me && pid != ps).then_some(pid)
            })
            .collect()
    }
}

#[test]
fn the_git_process_kept_for_file_contents_has_ended_when_the_keeper_returns() {
    let scratch = Scratch::new();
    let (repo, first) = scratch.repository("repo");
    assert_eq!(children(), Vec::<u32>::new(), "nothing runs yet");
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
    assert_eq!(children().len(), 1, "the one git process of the worktree");

    window.stop(keeper);
    assert_eq!(children(), Vec::<u32>::new(), "the Keeper has returned");
}
