//! Component tests: the export of an old side, through the Link as a Window speaks it. A
//! real Keeper runs in this process on a database and a folder of state stores the test
//! makes, over real repositories; git runs as its user's does, with the environment of this
//! process, so each test sets that environment as its first statement, and these tests are
//! run by `cargo nextest`, which gives each test a process of its own.
//!
//! The app's cache folder is where `yaar_link::cache_dir()` says in this process, which the
//! test has pointed into its own scratch folder: the Keeper finds the same folder by the same
//! rule.

use std::collections::BTreeMap;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, CommentId, DraftOn, Hello, Lines, Message, Refusal, Reply, Request, RequestId, Side,
    ThreadId, WorkspaceId, handshake, read_message, write_message,
};

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and sets the environment of this process for git and for the cache
    /// folder: no configuration of the user or of the system, a fixed author, an empty home,
    /// and a cache folder inside the scratch folder. It has to be the first statement of a
    /// test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-exports-{}-{n}", std::process::id()));
        for folder in ["home", "tmp", "cache"] {
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
            // Where the Keeper's copies of git's lists of files go: a folder that goes
            // with the test.
            std::env::set_var("TMPDIR", root.join("tmp"));
            // The cache folder on Linux; macOS keeps it under the home.
            std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// The app's folder of the Keeper of this test.
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: None,
            home: self.0.join("home"),
            login_shell: None,
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

    /// A repository in the folder `name`, empty, on `main`, with an identity of its own:
    /// git on Linux guesses none.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git_bytes(folder: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        // What the test reads of the repository must not write it, as a refresh of the
        // list of files would.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git(folder: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(folder, args))
        .expect("git printed text")
        .trim()
        .to_owned()
}

fn commit_all(folder: &Path, message: &str) -> String {
    git(folder, &["add", "--all"]);
    git(folder, &["commit", "--quiet", "-m", message]);
    git(folder, &["rev-parse", "HEAD"])
}

fn write(folder: &Path, name: &str, text: &str) {
    let file = folder.join(name);
    std::fs::create_dir_all(file.parent().expect("a folder above")).expect("make the folders");
    std::fs::write(file, text).expect("write a file");
}

fn make_executable(folder: &Path, name: &str) {
    std::fs::set_permissions(folder.join(name), std::fs::Permissions::from_mode(0o755))
        .expect("make it run");
}

/// One end of the Link, as a Window holds it.
struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    /// Sends a request and gives its reply, passing over the facts that come before it:
    /// opening a folder and saving a draft are told to every Window.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        loop {
            match read_message(&mut self.stream).expect("a frame from the Keeper") {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Fact(_) => {}
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
        workspace
    }

    /// Records a state of the worktree as its files are on disk, by saving the first
    /// comment of a new thread on line 1 of `a.txt`, and gives the state's id as the
    /// thread's anchor names it.
    fn record_state(&mut self, workspace: &WorkspaceId, first_line: &str) -> String {
        let reviews = match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        };
        let review = reviews.reviews[0].id.clone();
        let thread = ThreadId::new(format!("{:032x}", 0x1000));
        let saved = self.ask(Request::SaveDraft {
            review,
            thread: thread.clone(),
            comment: CommentId::new(format!("{:032x}", 0x2000)),
            on: DraftOn::NewThread {
                anchor: Anchor {
                    tree: None,
                    path: PathBuf::from("a.txt"),
                    side: Side::New,
                    lines: Lines { first: 1, last: 1 },
                    text: first_line.to_owned(),
                },
            },
            text: "a word".to_owned(),
        });
        assert_eq!(saved, Ok(Reply::Done));
        let held = match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        };
        let anchor = &held.reviews[0]
            .threads
            .iter()
            .find(|held| held.id == thread)
            .expect("the thread is kept")
            .anchor;
        anchor
            .tree
            .clone()
            .expect("a thread on the files on disk names the state recorded for it")
    }

    fn export(
        &mut self,
        workspace: &WorkspaceId,
        version: &str,
        into: &Path,
    ) -> Result<Reply, Refusal> {
        self.ask(Request::ExportVersion {
            workspace: workspace.clone(),
            version: version.to_owned(),
            into: into.to_owned(),
        })
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// What a folder holds at one path, as an export must reproduce it.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File { bytes: Vec<u8>, executable: bool },
    Link(PathBuf),
}

fn file(text: &str) -> Entry {
    Entry::File {
        bytes: text.as_bytes().to_vec(),
        executable: false,
    }
}

fn program(text: &str) -> Entry {
    Entry::File {
        bytes: text.as_bytes().to_vec(),
        executable: true,
    }
}

fn link(target: &str) -> Entry {
    Entry::Link(PathBuf::from(target))
}

/// Every file and link below `folder` by its path, read as the file system holds it:
/// folders are not entries, `.git` would be.
fn entries(folder: &Path) -> BTreeMap<String, Entry> {
    fn walk(root: &Path, folder: &Path, into: &mut BTreeMap<String, Entry>) {
        for child in std::fs::read_dir(folder).expect("read the folder") {
            let child = child.expect("an entry").path();
            let kind = std::fs::symlink_metadata(&child).expect("the entry's kind");
            let name = child
                .strip_prefix(root)
                .expect("below the root")
                .to_str()
                .expect("a path of text")
                .to_owned();
            if kind.is_dir() {
                walk(root, &child, into);
            } else if kind.is_symlink() {
                into.insert(
                    name,
                    Entry::Link(std::fs::read_link(&child).expect("a link")),
                );
            } else {
                into.insert(
                    name,
                    Entry::File {
                        bytes: std::fs::read(&child).expect("read the file"),
                        executable: kind.permissions().mode() & 0o111 != 0,
                    },
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

fn expected<const N: usize>(wanted: [(&str, Entry); N]) -> BTreeMap<String, Entry> {
    wanted
        .into_iter()
        .map(|(path, entry)| (path.to_owned(), entry))
        .collect()
}

/// What the repository answers to the five questions the row asks, and the bytes of its
/// list of files: every one is read-only, and none writes the list.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    status: String,
    worktrees: String,
    refs: String,
    stash: String,
    index: Vec<u8>,
}

fn snapshot(repo: &Path) -> Snapshot {
    Snapshot {
        status: git(
            repo,
            &[
                "status",
                "--porcelain=v2",
                "--ignored",
                "--untracked-files=all",
            ],
        ),
        worktrees: git(repo, &["worktree", "list", "--porcelain"]),
        refs: git(repo, &["for-each-ref"]),
        stash: git(repo, &["stash", "list"]),
        index: std::fs::read(repo.join(".git/index")).expect("the list of files"),
    }
}

/// The repository every test but the filter test uses: a commit C1, a second commit, an
/// uncommitted change, an untracked file, an ignored file, a stash entry and a second
/// worktree.
struct Repo {
    folder: PathBuf,
    c1: String,
    c2: String,
}

/// The files of C1.
fn files_of_c1() -> BTreeMap<String, Entry> {
    expected([
        (".gitignore", file("ignored/\n")),
        ("a.txt", file("one\n")),
        ("bin/run.sh", program("#!/bin/sh\necho one\n")),
        ("link", link("a.txt")),
        ("src/lib.rs", file("fn one() {}\n")),
    ])
}

/// The files of the worktree once the test has changed it: the state recorded then.
fn files_of_the_state() -> BTreeMap<String, Entry> {
    expected([
        (".gitignore", file("ignored/\n")),
        ("a.txt", file("three\n")),
        ("bin/run.sh", program("#!/bin/sh\necho one\n")),
        ("later.txt", file("later\n")),
        ("link", link("a.txt")),
        ("src/main.rs", file("fn main() {}\n")),
        ("untracked.txt", file("not added\n")),
    ])
}

fn repository_in_every_state(scratch: &Scratch) -> Repo {
    let folder = scratch.repository("repo");
    write(&folder, ".gitignore", "ignored/\n");
    write(&folder, "a.txt", "one\n");
    write(&folder, "bin/run.sh", "#!/bin/sh\necho one\n");
    make_executable(&folder, "bin/run.sh");
    write(&folder, "src/lib.rs", "fn one() {}\n");
    symlink("a.txt", folder.join("link")).expect("make a link");
    let c1 = commit_all(&folder, "one");

    write(&folder, "a.txt", "two\n");
    write(&folder, "later.txt", "later\n");
    std::fs::remove_file(folder.join("src/lib.rs")).expect("delete");
    write(&folder, "src/main.rs", "fn main() {}\n");
    let c2 = commit_all(&folder, "two");

    // A stash entry, then the changes the worktree holds when the state is recorded.
    write(&folder, "later.txt", "later, stashed\n");
    git(&folder, &["stash", "push", "--quiet"]);
    write(&folder, "a.txt", "three\n");
    write(&folder, "untracked.txt", "not added\n");
    write(&folder, "ignored/cache.bin", "left out of every version\n");
    git(
        &folder,
        &["worktree", "add", "--quiet", "-b", "other", "../second"],
    );
    Repo { folder, c1, c2 }
}

/// Where an export goes, in the app's cache folder as the Navigator names it.
fn old_side(n: u32) -> PathBuf {
    yaar_link::cache_dir().join(format!("navigator/1/old/{n}"))
}

#[test]
fn after_use_of_the_old_side_the_status_the_worktrees_and_the_refs_of_the_repository_are_unchanged()
{
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);

    // Read before the Keeper starts and before any request is sent. The repository has
    // each thing the row names, so that the five equal each other for a reason.
    let before = snapshot(&repo.folder);
    assert!(before.status.contains("? untracked.txt"), "{before:?}");
    assert!(before.status.contains("! ignored/"), "{before:?}");
    assert!(before.status.contains("a.txt"), "{before:?}");
    assert!(
        before.worktrees.matches("worktree ").count() == 2,
        "{before:?}"
    );
    assert!(before.refs.contains("refs/heads/other"), "{before:?}");
    assert!(before.stash.contains("stash@{0}"), "{before:?}");

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);
    let state = window.record_state(&workspace, "three");

    // Neither folder is there yet, nor is the cache folder: the Keeper makes the folders
    // above them.
    assert!(!yaar_link::cache_dir().exists());
    assert_eq!(
        window.export(&workspace, &repo.c1, &old_side(1)),
        Ok(Reply::Done)
    );
    assert_eq!(
        window.export(&workspace, &state, &old_side(2)),
        Ok(Reply::Done)
    );

    // Each folder holds that version's files and no `.git`: the second commit's files are
    // not C1's, and the state's are the worktree's but for what git ignores.
    assert_eq!(entries(&old_side(1)), files_of_c1());
    assert_eq!(entries(&old_side(2)), files_of_the_state());
    assert!(!old_side(1).join(".git").exists());
    assert!(!old_side(2).join(".git").exists());
    assert_ne!(repo.c1, repo.c2);

    // After use of the old side: the five, read again, equal what they were.
    assert_eq!(snapshot(&repo.folder), before);
    // The worktree's own files are as the test left them.
    assert_eq!(
        std::fs::read_to_string(repo.folder.join("a.txt")).expect("read"),
        "three\n"
    );
    window.stop(keeper);
}

#[test]
fn an_export_of_the_second_commit_holds_that_commits_files() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);

    assert_eq!(
        window.export(&workspace, &repo.c2, &old_side(1)),
        Ok(Reply::Done)
    );
    assert_eq!(
        entries(&old_side(1)),
        expected([
            (".gitignore", file("ignored/\n")),
            ("a.txt", file("two\n")),
            ("bin/run.sh", program("#!/bin/sh\necho one\n")),
            ("later.txt", file("later\n")),
            ("link", link("a.txt")),
            ("src/main.rs", file("fn main() {}\n")),
        ])
    );
    window.stop(keeper);
}

#[test]
fn an_into_outside_the_cache_folder_is_refused_and_nothing_is_written() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);

    // A folder of the app's cache that is a link to somewhere else leads out of it.
    let elsewhere = scratch.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("make the folder");
    std::fs::create_dir_all(yaar_link::cache_dir()).expect("make the cache folder");
    symlink(&elsewhere, yaar_link::cache_dir().join("navigator")).expect("make a link");

    let cache = yaar_link::cache_dir();
    let refused = [
        // Outside: a folder beside the cache folder, the cache folder itself, a path that
        // is not absolute, one that climbs out, and one that goes through the link.
        scratch.0.join("somewhere/old/1"),
        cache.clone(),
        PathBuf::from("old/1"),
        cache.join("other/../inside"),
        cache.join("../escaped"),
        cache.join("navigator/1/old/1"),
    ];
    for into in refused {
        let answer = window.export(&workspace, &repo.c1, &into);
        assert!(
            matches!(answer, Err(Refusal::Failed { .. })),
            "{into:?}: {answer:?}"
        );
    }
    // Nothing was made by any of them: not the folders above, not a file below.
    assert_eq!(std::fs::read_dir(&elsewhere).expect("read").count(), 0);
    assert!(!scratch.0.join("somewhere").exists());
    assert!(!cache.join("../escaped").exists());
    assert!(!cache.join("inside").exists());
    assert!(!cache.join("other").exists());
    assert!(!Path::new("old").exists());
    assert_eq!(std::fs::read_dir(&cache).expect("read").count(), 1);
    window.stop(keeper);
}

#[test]
fn an_into_that_exists_is_refused_and_left_as_it_was() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);

    let taken = old_side(1);
    std::fs::create_dir_all(&taken).expect("make the folder");
    std::fs::write(taken.join("mine.txt"), "not the Keeper's\n").expect("write");
    let answer = window.export(&workspace, &repo.c1, &taken);
    assert!(matches!(answer, Err(Refusal::Failed { .. })), "{answer:?}");
    assert_eq!(
        entries(&taken),
        expected([("mine.txt", file("not the Keeper's\n"))])
    );

    // A file or a link where the folder would be is a taken name too.
    let file_there = old_side(2);
    std::fs::write(&file_there, "a file\n").expect("write");
    let answer = window.export(&workspace, &repo.c1, &file_there);
    assert!(matches!(answer, Err(Refusal::Failed { .. })), "{answer:?}");
    assert_eq!(std::fs::read(&file_there).expect("read"), b"a file\n");
    let dangling = old_side(3);
    symlink(scratch.0.join("nowhere"), &dangling).expect("make a link");
    let answer = window.export(&workspace, &repo.c1, &dangling);
    assert!(matches!(answer, Err(Refusal::Failed { .. })), "{answer:?}");
    assert!(!scratch.0.join("nowhere").exists());
    window.stop(keeper);
}

#[test]
fn a_version_that_is_no_commit_and_no_state_is_refused_and_nothing_is_made() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);

    let tree = git(&repo.folder, &["rev-parse", "HEAD^{tree}"]);
    let not_there = "1234567890abcdef1234567890abcdef12345678".to_owned();
    for version in [tree, not_there, "main".to_owned(), String::new()] {
        let answer = window.export(&workspace, &version, &old_side(1));
        assert_eq!(
            answer,
            Err(Refusal::UnknownRevision {
                revision: version.clone()
            }),
            "{version:?}"
        );
    }
    // Not even the folders above the one asked for.
    assert!(!yaar_link::cache_dir().exists());
    window.stop(keeper);
}

#[test]
fn a_workspace_the_keeper_does_not_know_is_refused() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    window.open(&repo.folder);

    let nobody = WorkspaceId::new("no-such-workspace");
    assert_eq!(
        window.export(&nobody, &repo.c1, &old_side(1)),
        Err(Refusal::UnknownWorkspace { workspace: nobody })
    );
    assert!(!yaar_link::cache_dir().exists());
    window.stop(keeper);
}

#[test]
fn a_recorded_state_is_exported_with_the_repository_gone() {
    let scratch = Scratch::new();
    let repo = repository_in_every_state(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo.folder);
    let state = window.record_state(&workspace, "three");

    // The state store holds the state whole: the repository is not needed to write it.
    std::fs::remove_dir_all(&repo.folder).expect("remove the repository");
    assert_eq!(
        window.export(&workspace, &state, &old_side(1)),
        Ok(Reply::Done)
    );
    assert_eq!(entries(&old_side(1)), files_of_the_state());
    // A commit the state store never copied cannot be written without the repository.
    assert_eq!(
        window.export(&workspace, &repo.c1, &old_side(2)),
        Err(Refusal::UnknownRevision {
            revision: repo.c1.clone()
        })
    );
    assert!(!old_side(2).exists());
    window.stop(keeper);
}

#[test]
fn a_repository_whose_filter_driver_would_fail_exports_the_files_as_git_stores_them() {
    let scratch = Scratch::new();
    let repo = scratch.repository("filtered");
    // The driver is in the user's own configuration, as a large-file driver is, and in the
    // repository's too; it fails, as one does when no server answers.
    let global = scratch.0.join("gitconfig");
    let driver = "[filter \"big\"]\n\tclean = cat\n\tsmudge = false\n\trequired = true\n";
    std::fs::write(&global, driver).expect("write the configuration");
    // SAFETY: as in `Scratch::new`: nothing else runs in this process yet.
    unsafe { std::env::set_var("GIT_CONFIG_GLOBAL", &global) };
    git(&repo, &["config", "filter.big.smudge", "false"]);
    git(&repo, &["config", "filter.big.required", "true"]);
    write(&repo, ".gitattributes", "*.bin filter=big\n");
    write(
        &repo,
        "a.bin",
        "version https://git-lfs.example/spec/v1\noid sha256:00\n",
    );
    write(&repo, "a.txt", "one\n");
    let commit = commit_all(&repo, "one");

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let state = window.record_state(&workspace, "one");
    for (n, version) in [&commit, &state].into_iter().enumerate() {
        let into = old_side(n as u32 + 1);
        assert_eq!(window.export(&workspace, version, &into), Ok(Reply::Done));
        assert_eq!(
            entries(&into),
            expected([
                (".gitattributes", file("*.bin filter=big\n")),
                (
                    "a.bin",
                    file("version https://git-lfs.example/spec/v1\noid sha256:00\n")
                ),
                ("a.txt", file("one\n")),
            ]),
            "{version}"
        );
    }
    window.stop(keeper);
}
