//! Component tests: a worktree made from a branch with an agent started in it in one
//! request, and the folder a user names for a repository's worktrees, through the surface a
//! Window uses, with the Keeper as the library on a thread of the test, real git and a real
//! pseudo-terminal beneath.
//!
//! The agent is a stand-in of the test's own: a script named like the CLI, which a `.profile`
//! of the test's home puts first on the search path of the user's login shell, that prints a
//! marker with its working folder and then waits on its terminal. No real `claude` or `pi`
//! runs and nothing of the user's home is read. The Keeper starts programs with the
//! environment of its process, so each test sets that environment as its first statement,
//! and these tests are run by `cargo nextest`, which gives each test a process of its own.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Base, BaseSource, Comparison, Fact, Head, Hello, Message, Program, Refusal, Reply, Repository,
    RepositoryId, Request, RequestId, Setting, TerminalId, TerminalInfo, TerminalSize, WorkspaceId,
    handshake, read_message, write_message,
};

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// How long a Keeper that ought to be waiting is given to show that it is not. Nothing a
/// passing run does has to end within it: a Keeper that leaves earlier is a failure, one
/// that is slower than this on a busy machine is missed, never wrongly blamed.
const SETTLE: Duration = Duration::from_secs(3);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// The two shells a test hands the Keeper as the user's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    /// `/bin/sh`: the login shell of the user, which reads the `.profile` of the home.
    Plain,
    /// A file with an execute bit whose interpreter is no program: it passes for a shell
    /// and cannot be started.
    Broken,
}

/// A folder of the test's own with a short path, removed when the test ends.
///
/// Under it: `home`, `tmp`, `data` (the Keeper's), `fake` (the stand-ins), `lair/repo` (the
/// repository, alone in its folder: the folder of its worktrees is made beside it), and the
/// Keeper's socket.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and sets the environment of this process for git and for the shell
    /// of a terminal: no configuration of the user or of the system, a fixed author, a home
    /// of the test's own, and the stand-ins first on the search path of `shell`. It has to
    /// be the first statement of a test.
    fn new(shell: Shell) -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and git and the Keeper name a worktree by its resolved
        // path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-worktrees-{}-{n}", std::process::id()));
        for folder in ["home", "tmp", "fake", "lair"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let scratch = Scratch(root);
        for agent in ["claude", "pi"] {
            scratch.program(
                &scratch.0.join("fake").join(agent),
                "printf 'STANDIN-RAN <%s> <%s>\\n' \"$(basename \"$0\")\" \"$(pwd -P)\"\n\
                 exec cat > /dev/null",
            );
        }
        // A login shell's system profile sets the search path anew or puts the system's
        // folders first, so the folder is put first again by the `.profile` of the home.
        std::fs::write(
            scratch.home().join(".profile"),
            format!(
                "PATH=\"{}:$PATH\"; export PATH\n",
                scratch.0.join("fake").display()
            ),
        )
        .expect("write the profile");
        let shell = match shell {
            Shell::Plain => PathBuf::from("/bin/sh"),
            Shell::Broken => {
                let broken = scratch.0.join("broken-shell");
                std::fs::write(
                    &broken,
                    format!("#!{}\n", scratch.0.join("no-such-interpreter").display()),
                )
                .expect("write the broken shell");
                std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755))
                    .expect("make it run");
                broken
            }
        };
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
            std::env::set_var("SHELL", &shell);
            std::env::set_var("HOME", scratch.home());
            // Where the Keeper's copies of git's lists of files go, and a program of a
            // terminal's too: a folder that goes with the test.
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

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// The folder the repository lies alone in.
    fn lair(&self) -> PathBuf {
        self.0.join("lair")
    }

    /// The folder the app keeps the worktrees of the repository in when no other is named.
    fn beside(&self) -> PathBuf {
        self.lair().join("repo.worktrees")
    }

    /// A script `#!/bin/sh` with `body`, which can be run.
    fn program(&self, path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make it run");
    }

    /// A `post-checkout` hook of the repository, which git runs once a worktree is made.
    fn hook(&self, repo: &Path, body: &str) {
        self.program(&repo.join(".git/hooks/post-checkout"), body);
    }

    /// What the Keeper of this test is started with.
    fn options(&self) -> Options {
        Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.home(),
        }
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = self.options();
        std::thread::spawn(move || run(options))
    }

    /// Attaches as a Window does, waiting for the Keeper to listen, and waiting until it
    /// serves this Window: a fact told before that is told to a Window that is not there
    /// yet, and the first reply is the proof that it is.
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
                let mut window = Window { stream, next_id: 0 };
                // The Keeper tells a fact to the Windows it serves: its first reply comes
                // after the Window is one of them. `ListClis` tells no fact and keeps no
                // state.
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

    /// The repository `lair/repo` with `main` at one commit, and a second branch `other`
    /// at the same commit.
    fn repository(&self) -> PathBuf {
        self.repository_at(&self.lair().join("repo"))
    }

    /// A repository like [`Scratch::repository`] in `folder`, which may lie anywhere in the
    /// scratch folder.
    fn repository_at(&self, folder: &Path) -> PathBuf {
        let folder = folder.to_owned();
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        std::fs::write(folder.join("a.txt"), "one\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        git(&folder, &["branch", "other"]);
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

/// What git prints for a read that may fail, with the exit code: the bytes a test of
/// "nothing is written" compares. No optional lock, so that a read never rewrites the
/// index of a repository committed to in the same second.
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

/// Every name below `folder`, with a letter for its kind (`l` link, `d` folder, `f` file),
/// relative to `below`.
fn walk(folder: &Path, below: &Path, found: &mut Vec<(PathBuf, char)>) {
    for entry in std::fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let kind = std::fs::symlink_metadata(&path).expect("kind").file_type();
        let letter = if kind.is_symlink() {
            'l'
        } else if kind.is_dir() {
            'd'
        } else {
            'f'
        };
        found.push((path.strip_prefix(below).expect("below").to_owned(), letter));
        if letter == 'd' {
            walk(&path, below, found);
        }
    }
}

/// What a refusal, and a worktree taken away, must leave as it was: the seven reads of the
/// repository, its objects with their sizes, and every name in the folder it lies in.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    reads: Vec<Vec<u8>>,
    objects: Vec<(PathBuf, u64)>,
    listing: Vec<(PathBuf, char)>,
}

fn snapshot(scratch: &Scratch, repo: &Path) -> Snapshot {
    let objects_folder = repo.join(".git/objects");
    let mut files = Vec::new();
    walk(&objects_folder, &objects_folder, &mut files);
    let mut objects: Vec<(PathBuf, u64)> = files
        .into_iter()
        .filter(|(_, letter)| *letter == 'f')
        .map(|(path, _)| {
            let size = std::fs::metadata(objects_folder.join(&path))
                .expect("size")
                .len();
            (path, size)
        })
        .collect();
    objects.sort();
    let mut listing = Vec::new();
    walk(&scratch.lair(), &scratch.lair(), &mut listing);
    listing.sort();
    Snapshot {
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
            std::fs::read(repo.join(".git/index")).expect("the list of files"),
            std::fs::read(repo.join(".git/config")).expect("the configuration"),
        ],
        objects,
        listing,
    }
}

/// Two objects no ref reaches, planted before the first read: a blob and a dangling commit.
/// Gives their ids, which `still_there` finds again.
fn plant(scratch: &Scratch, repo: &Path) -> Vec<String> {
    let file = scratch.0.join("blob.txt");
    std::fs::write(&file, "a blob nothing refers to\n").expect("write the blob's file");
    let blob = git(
        repo,
        &["hash-object", "-w", file.to_str().expect("a path of text")],
    );
    std::fs::write(repo.join("a.txt"), "edited\n").expect("edit a file");
    let dangling = git(repo, &["stash", "create"]);
    git(repo, &["checkout", "--quiet", "--", "a.txt"]);
    assert!(!dangling.is_empty());
    vec![blob, dangling]
}

/// `git cat-file -e` of each planted object succeeds.
fn still_there(repo: &Path, planted: &[String]) {
    for id in planted {
        git(repo, &["cat-file", "-e", id]);
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

    /// The next frame the Keeper sent, whatever it is. Fails the test at the deadline.
    fn next_any(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    /// The next frame that is not the output of a program: what a started agent writes
    /// comes when it comes, and no fact is waited for behind it.
    fn next(&mut self) -> Message {
        loop {
            match self.next_any() {
                Message::Output { .. } => {}
                other => return other,
            }
        }
    }

    /// The reply to `id` and every frame that came before it, in the order they came.
    fn reply_to(&mut self, id: RequestId) -> (Result<Reply, Refusal>, Vec<Message>) {
        let mut seen = Vec::new();
        loop {
            match self.next_any() {
                Message::Reply { to, reply } if to == id => return (reply, seen),
                Message::Reply { .. } => panic!("the reply to a request nobody sent"),
                other => seen.push(other),
            }
        }
    }

    /// Sends a request and gives its reply with what came before it.
    fn ask_seen(&mut self, request: Request) -> (Result<Reply, Refusal>, Vec<Message>) {
        let id = self.send(request);
        self.reply_to(id)
    }

    /// Sends a request and gives its reply, passing over what comes before it.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_seen(request).0
    }

    /// The barrier: `ListWorkspaces` is answered after everything the Keeper did for this
    /// Window before it, the facts it told included. Gives the list and what came before
    /// the reply.
    fn barrier(&mut self) -> (Vec<Repository>, Vec<Message>) {
        match self.ask_seen(Request::ListWorkspaces) {
            (Ok(Reply::Workspaces(list)), seen) => (list, seen),
            other => panic!("no list of workspaces: {other:?}"),
        }
    }

    /// Opens the folder as a workspace, and reads the fact that tells the list.
    fn open(&mut self, folder: &Path) -> (RepositoryId, WorkspaceId) {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened {
            repository,
            workspace,
        }) = opened
        else {
            panic!("not opened: {opened:?}");
        };
        assert!(matches!(self.next(), Message::Fact(Fact::Workspaces(_))));
        (repository, workspace)
    }

    /// The request the app sends for a worktree with an agent.
    fn create(
        &mut self,
        repository: &RepositoryId,
        from: &str,
        branch: &str,
        agent: &str,
    ) -> (Result<Reply, Refusal>, Vec<Message>) {
        self.ask_seen(Request::CreateWorktree {
            repository: repository.clone(),
            from: from.to_owned(),
            branch: branch.to_owned(),
            agent: agent.to_owned(),
            size: SIZE,
        })
    }

    /// Reads frames until the output of `terminal` holds `needle`, and gives all it read.
    /// The marker is looked for in what came, never at where a line begins: a shell may
    /// write its prompt after the echo of what it was told.
    fn output_holds(&mut self, terminal: &TerminalId, needle: &str) -> String {
        let mut text = String::new();
        loop {
            match self.next_any() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    if text.contains(needle) {
                        return text;
                    }
                }
                _ => {}
            }
        }
    }

    fn terminals(&mut self) -> Vec<TerminalInfo> {
        match self.ask(Request::ListTerminals) {
            Ok(Reply::Terminals(terminals)) => terminals,
            other => panic!("no list of terminals: {other:?}"),
        }
    }

    fn settings(&mut self) -> Vec<Setting> {
        match self.ask(Request::GetSettings) {
            Ok(Reply::Settings(settings)) => settings,
            other => panic!("no settings: {other:?}"),
        }
    }

    /// Sets the folder for worktrees of `repository` as a user does.
    fn set_folder(
        &mut self,
        repository: &RepositoryId,
        folder: Option<&Path>,
    ) -> Result<Reply, Refusal> {
        self.ask(Request::SetSetting {
            setting: worktrees_folder(repository, folder),
        })
    }

    /// The next frame, which has to tell `setting`.
    fn told(&mut self, setting: &Setting) {
        match self.next() {
            Message::Fact(Fact::SettingChanged(told)) => assert_eq!(&told, setting),
            other => panic!("not the setting {setting:?} told: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn worktrees_folder(repository: &RepositoryId, folder: Option<&Path>) -> Setting {
    Setting::WorktreesFolder {
        repository: repository.clone(),
        folder: folder.map(Path::to_owned),
    }
}

/// The workspace `id` among the lists of `repository` in the facts and the list: its folder
/// and what is checked out.
fn found_in(
    lists: &[Repository],
    repository: &RepositoryId,
    id: &WorkspaceId,
) -> Option<(PathBuf, Option<Head>)> {
    lists
        .iter()
        .filter(|one| one.id == *repository)
        .flat_map(|one| &one.workspaces)
        .find(|workspace| workspace.id == *id)
        .map(|workspace| (workspace.path.clone(), workspace.head.clone()))
}

/// The lists of repositories the facts among `seen` carry, in the order told.
fn lists_told(seen: &[Message]) -> Vec<Vec<Repository>> {
    seen.iter()
        .filter_map(|message| match message {
            Message::Fact(Fact::Workspaces(list)) => Some(list.clone()),
            _ => None,
        })
        .collect()
}

/// Whether a workspace on the branch `branch` is among the lists.
fn some_list_holds_a_workspace_on(lists: &[Vec<Repository>], branch: &str) -> bool {
    lists.iter().flatten().any(|repository| {
        repository
            .workspaces
            .iter()
            .any(|workspace| workspace.head == Some(Head::Branch(branch.to_owned())))
    })
}

/// The folders `git worktree list --porcelain` names, as git prints them.
fn worktree_folders(repo: &Path) -> Vec<PathBuf> {
    git(repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

/// The sentence of a refusal of Workspaces, as the Link carries it.
fn failed_with(refusal: &yaar_workspaces::Refusal) -> Refusal {
    Refusal::Failed {
        reason: refusal.to_string(),
    }
}

/// A worktree is made on a new branch from `main`, beside the repository, with `main` as its
/// base, and the agent `agent` runs in it, all from one request.
fn a_worktree_with_an_agent_in_one_request(agent: &str) {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, first) = window.open(&repo);
    let (mut second_window, main_commit) = (scratch.window(), git(&repo, &["rev-parse", "main"]));
    let expected = scratch.beside().join("task-one");

    let (reply, seen) = window.create(&repository, "main", "task/one", agent);

    let Ok(Reply::WorktreeCreated {
        workspace,
        terminal,
    }) = reply
    else {
        panic!("no worktree: {reply:?}");
    };
    assert_ne!(workspace, first);
    // The list that holds the workspace on its new branch came before the reply. The output
    // of the agent reaches the Window only after the reply (the step that lets it through is
    // returned with the outcome, as for `OpenTerminal`), but this is a stated clause and
    // not a shown one: no surface can hold the Keeper between the opening of the terminal
    // and the queueing of the reply, and the shell writes later than the Keeper queues, so
    // that the check below would pass without the step too. Slice 7's opening of a terminal
    // is read the same way.
    assert!(
        seen.iter()
            .all(|message| matches!(message, Message::Fact(_))),
        "{seen:?}"
    );
    let told = lists_told(&seen);
    assert_eq!(
        told.last()
            .and_then(|list| found_in(list, &repository, &workspace)),
        Some((expected.clone(), Some(Head::Branch("task/one".to_owned())))),
        "{told:?}"
    );
    // The CLI ran in the new folder, in the terminal the reply names. Read first: a request
    // that is asked meanwhile passes over the output that comes between it and its reply.
    let marker = format!("STANDIN-RAN <{agent}> <{}>", expected.display());
    window.output_holds(&terminal, &marker);
    // The other Window is told the same list.
    let heard = second_window.next();
    assert!(
        matches!(&heard, Message::Fact(Fact::Workspaces(list))
            if found_in(list, &repository, &workspace).is_some()),
        "{heard:?}"
    );

    // Made by git, on the new branch, at the commit `main` names.
    assert!(
        worktree_folders(&repo).contains(&expected),
        "{:?}",
        worktree_folders(&repo)
    );
    assert_eq!(git(&expected, &["branch", "--show-current"]), "task/one");
    assert_eq!(git(&repo, &["rev-parse", "task/one"]), main_commit);
    // Its base is `main`, as the user would have set it.
    let changes = window.ask(Request::GetChanges {
        workspace: workspace.clone(),
    });
    let Ok(Reply::Changes(changes)) = changes else {
        panic!("no changes: {changes:?}");
    };
    assert_eq!(
        changes.comparison,
        Comparison::SinceBase {
            base: Base {
                name: "main".to_owned(),
                source: BaseSource::User,
            },
        }
    );

    assert_eq!(
        window.terminals(),
        [TerminalInfo {
            id: terminal,
            workspace,
            program: Program::Agent {
                agent: agent.to_owned(),
            },
        }]
    );
    window.stop(keeper);
}

#[test]
fn a_worktree_is_made_from_a_branch_and_the_agent_is_started_in_it_by_one_request() {
    a_worktree_with_an_agent_in_one_request("claude");
}

#[test]
fn a_worktree_is_made_from_a_branch_and_the_agent_is_started_in_it_by_one_request_as_pi() {
    a_worktree_with_an_agent_in_one_request("pi");
}

#[test]
fn each_refusal_makes_nothing_tells_nothing_and_runs_no_git() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    // A folder that is where a worktree would go, and a file in the place of a folder the
    // user named.
    std::fs::create_dir_all(scratch.beside().join("taken")).expect("make the folder");
    let planted = plant(&scratch, &repo);
    // The hook is put after the planting, whose checkout would run it. It writes where only
    // a git that made a worktree could have.
    let ran = scratch.0.join("hook-ran");
    scratch.hook(&repo, &format!("echo ran >> '{}'", ran.display()));
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let (listed_before, _) = window.barrier();

    use yaar_workspaces::Refusal as Refused;
    let other_repository = RepositoryId::new("no-such-repository");
    let cases: Vec<(&str, RepositoryId, &str, &str, &str, Refusal)> = vec![
        (
            "an agent no Adapter has",
            repository.clone(),
            "main",
            "one",
            "otheragent",
            Refusal::Failed {
                reason: "there is no agent CLI named otheragent".to_owned(),
            },
        ),
        (
            "a repository the Keeper does not hold",
            other_repository.clone(),
            "main",
            "one",
            "claude",
            failed_with(&Refused::UnknownRepository(
                yaar_workspaces::RepositoryId::new(other_repository.as_str()),
            )),
        ),
        (
            "a branch that is checked out",
            repository.clone(),
            "main",
            "main",
            "claude",
            failed_with(&Refused::BranchExists("main".to_owned())),
        ),
        (
            "a branch that exists and is not checked out",
            repository.clone(),
            "main",
            "other",
            "claude",
            failed_with(&Refused::BranchExists("other".to_owned())),
        ),
        (
            "a name git does not take for a branch",
            repository.clone(),
            "main",
            "a..b",
            "claude",
            failed_with(&Refused::NotABranchName("a..b".to_owned())),
        ),
        (
            "a name that starts with a dash",
            repository.clone(),
            "main",
            "-x",
            "claude",
            failed_with(&Refused::NotABranchName("-x".to_owned())),
        ),
        (
            "a branch to start from that is none",
            repository.clone(),
            "no-such-branch",
            "one",
            "claude",
            Refusal::UnknownRevision {
                revision: "no-such-branch".to_owned(),
            },
        ),
        (
            "a folder that is there",
            repository.clone(),
            "main",
            "taken",
            "claude",
            failed_with(&Refused::FolderExists(scratch.beside().join("taken"))),
        ),
    ];
    for (what, repository, from, branch, agent, refusal) in cases {
        // What is compared is read before the request is sent.
        let before = snapshot(&scratch, &repo);
        let (reply, seen) = window.create(&repository, from, branch, agent);
        // The barrier: the list is answered after anything the request told.
        let (listed, after_the_reply) = window.barrier();
        assert_eq!(reply, Err(refusal), "{what}");
        assert!(seen.is_empty(), "{what}: {seen:?}");
        assert!(after_the_reply.is_empty(), "{what}: {after_the_reply:?}");
        assert_eq!(listed, listed_before, "{what}");
        assert_eq!(snapshot(&scratch, &repo), before, "{what}");
        assert!(!ran.exists(), "{what}: git ran a hook");
        still_there(&repo, &planted);
    }

    // The user names a folder that is a file: kept when it is set, refused when a worktree
    // is to be made in it.
    let a_file = scratch.0.join("a-file");
    std::fs::write(&a_file, "not a folder\n").expect("write the file");
    let named = worktrees_folder(&repository, Some(&a_file));
    assert!(matches!(
        window.set_folder(&repository, Some(&a_file)),
        Ok(Reply::Done)
    ));
    window.told(&named);
    let (listed_before, _) = window.barrier();
    let before = snapshot(&scratch, &repo);
    let (reply, seen) = window.create(&repository, "main", "one", "claude");
    let (listed, after_the_reply) = window.barrier();
    assert_eq!(reply, Err(Refusal::NoSuchFolder { path: a_file }));
    assert!(seen.is_empty() && after_the_reply.is_empty());
    assert_eq!(listed, listed_before);
    assert_eq!(snapshot(&scratch, &repo), before);
    assert!(!ran.exists());
    window.stop(keeper);
}

/// A `post-checkout` hook that waits on a gate the test holds: while it waits, git is in the
/// middle of making a worktree.
///
/// The hook reads a pipe until the test closes it, and the test cannot open the pipe for
/// writing before the hook has it open for reading: that is how the test knows the hook
/// runs.
struct Gate {
    running: mpsc::Receiver<()>,
    opening: JoinHandle<std::fs::File>,
}

impl Gate {
    /// Puts the hook into the repository, to be put there before the first request.
    fn hook(scratch: &Scratch, repo: &Path) -> Gate {
        let pipe = scratch.0.join("gate");
        assert!(
            Command::new("mkfifo")
                .arg(&pipe)
                .status()
                .expect("start mkfifo")
                .success()
        );
        scratch.hook(repo, &format!("cat '{}' > /dev/null", pipe.display()));
        let (running, hook_runs) = mpsc::channel();
        let opening = std::thread::spawn(move || {
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .open(pipe)
                .expect("open the gate");
            running.send(()).expect("tell the test");
            opened
        });
        Gate {
            running: hook_runs,
            opening,
        }
    }

    /// Waits until the hook runs.
    fn hook_runs(&self) {
        self.running
            .recv_timeout(DEADLINE)
            .expect("the hook never started");
    }

    /// Lets the hook go.
    fn let_go(self) {
        drop(self.opening.join().expect("the opening of the gate"));
    }
}

#[test]
fn a_request_does_not_hold_up_the_keeper_while_git_runs_the_hook_of_the_user() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let gate = Gate::hook(&scratch, &repo);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let mut other = scratch.window();

    let id = window.send(Request::CreateWorktree {
        repository,
        from: "main".to_owned(),
        branch: "slow".to_owned(),
        agent: "claude".to_owned(),
        size: SIZE,
    });
    gate.hook_runs();

    // The hook runs, so git is in the middle of the add: another Window's list, which
    // waits for the Keeper's turn, is answered, and the request has not been.
    let (list, _) = other.barrier();
    assert_eq!(list.len(), 1);
    window
        .stream
        .set_nonblocking(true)
        .expect("do not wait for the reply");
    let silent = (&window.stream).read(&mut [0u8; 1]);
    window
        .stream
        .set_nonblocking(false)
        .expect("wait for the reply again");
    assert_eq!(
        silent.expect_err("a frame before the hook ended").kind(),
        std::io::ErrorKind::WouldBlock
    );

    // The gate is let go: the hook ends and the reply comes.
    gate.let_go();
    let (reply, _) = window.reply_to(id);
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    window.stop(keeper);
}

/// A stand-in `claude` that tells the test when its program ends: it prints a marker once
/// its traps are set and then reads its terminal, and writes to the fifo `ended` as it
/// exits, which the hang-up that ends a terminal brings about. The test reads the fifo from
/// a thread: the channel it gives holds a message once the program has ended.
fn an_agent_that_tells_when_it_ends(scratch: &Scratch) -> mpsc::Receiver<()> {
    let pipe = scratch.0.join("ended");
    assert!(
        Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("start mkfifo")
            .success()
    );
    scratch.program(
        &scratch.0.join("fake").join("claude"),
        &format!(
            "trap 'echo ended > \"{pipe}\"' EXIT\n\
             trap 'exit 0' HUP TERM\n\
             echo AGENT-READY\n\
             cat > /dev/null",
            pipe = pipe.display()
        ),
    );
    let (ended, hears) = mpsc::channel();
    std::thread::spawn(move || {
        // Opened for reading once the program opens it for writing, and read to its end.
        let mut said = String::new();
        std::fs::File::open(pipe)
            .and_then(|mut file| file.read_to_string(&mut said))
            .expect("read what the agent said");
        let _ = ended.send(());
    });
    hears
}

#[test]
fn a_keeper_that_is_asked_to_stop_while_git_makes_the_worktree_waits_for_it_and_takes_it_away() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    // Planted before the hook is put, as planting checks a file out and a checkout runs it.
    let planted = plant(&scratch, &repo);
    let gate = Gate::hook(&scratch, &repo);
    let agent_ended = an_agent_that_tells_when_it_ends(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, first) = window.open(&repo);
    let mut other = scratch.window();
    // An agent runs in the repository's own workspace: the end of its program says that the
    // Keeper has ended its parts.
    let opened = window.ask(Request::OpenTerminal {
        workspace: first,
        program: Program::Agent {
            agent: "claude".to_owned(),
        },
        size: SIZE,
    });
    let Ok(Reply::TerminalOpened { terminal }) = opened else {
        panic!("no terminal: {opened:?}");
    };
    window.output_holds(&terminal, "AGENT-READY");
    // Read before the request is sent.
    let before = snapshot(&scratch, &repo);

    window.send(Request::CreateWorktree {
        repository,
        from: "main".to_owned(),
        branch: "slow".to_owned(),
        agent: "claude".to_owned(),
        size: SIZE,
    });
    gate.hook_runs();
    let made = scratch.beside().join("slow");
    assert!(
        worktree_folders(&repo).contains(&made),
        "git has not made the worktree yet"
    );

    // The Keeper is asked to stop while the hook still runs. It lets every Window go and
    // takes the parts away before it waits for the work in flight, which ends the program of
    // the terminal: that is read before the window of time is given, so that the work finds
    // the parts gone whenever it is let go.
    write_message(&mut other.stream, &Message::Shutdown).expect("ask for the stop");
    let (stopped, stops) = mpsc::channel();
    std::thread::spawn(move || stopped.send(keeper.join()).expect("tell the test"));
    agent_ended
        .recv_timeout(DEADLINE)
        .expect("the Keeper did not end the program of the terminal");
    // The one timed negative: a Keeper that does not wait for the work leaves nothing to
    // look at while the work is held. A passing run cannot fail on it, since the hook is let
    // go only after the window; a Keeper that leaves later than this on a busy machine is
    // missed, never wrongly blamed.
    let left_early = stops.recv_timeout(SETTLE);
    // The lock of the Keeper is held until the work ends, as the next Keeper may start as
    // soon as it is let go: another one started now on the same data steps back. A Keeper
    // that let the lock go before the wait would start, and not answer within the deadline.
    let (second, seconds) = mpsc::channel();
    let options = scratch.options();
    std::thread::spawn(move || {
        let _ = second.send(run(options));
    });
    let second = seconds.recv_timeout(DEADLINE);
    gate.let_go();
    assert!(
        matches!(left_early, Err(mpsc::RecvTimeoutError::Timeout)),
        "the Keeper left while git still made the worktree"
    );
    assert!(
        matches!(second, Ok(Ok(Exit::AlreadyRunning))),
        "a second Keeper was not turned away while the first one waited: {second:?}"
    );
    let exit = stops
        .recv_timeout(DEADLINE)
        .expect("the Keeper did not leave once the work had ended");
    assert_eq!(exit.expect("join").expect("run"), Exit::Shutdown);

    // The work found the parts gone and took the worktree away before `run` returned, so
    // nothing is waited for here: the repository is as it was, with no worktree, no branch
    // and no folder.
    assert_eq!(worktree_folders(&repo), std::slice::from_ref(&repo));
    assert_eq!(git(&repo, &["branch", "--list", "slow"]), "");
    assert!(!made.exists(), "the folder of the worktree is left");
    assert_eq!(snapshot(&scratch, &repo), before);
    still_there(&repo, &planted);
}

#[test]
fn a_git_that_fails_makes_nothing_and_tells_no_window() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let planted = plant(&scratch, &repo);
    // A hook that fails ends the user's git with a failure after it made the worktree, and
    // what it says on its error output is git's own words, which the reply must carry.
    scratch.hook(&repo, "echo hook-said-no >&2\nexit 1");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let (listed_before, _) = window.barrier();
    // Read before the request is sent.
    let before = snapshot(&scratch, &repo);

    let (reply, seen) = window.create(&repository, "main", "failing", "claude");

    let Err(Refusal::Failed { reason }) = reply else {
        panic!("not refused: {reply:?}");
    };
    assert!(reason.contains("hook-said-no"), "{reason}");
    let (listed, after_the_reply) = window.barrier();
    assert_eq!(listed, listed_before);
    assert!(
        !some_list_holds_a_workspace_on(&lists_told(&seen), "failing")
            && !some_list_holds_a_workspace_on(&lists_told(&after_the_reply), "failing"),
        "{seen:?} {after_the_reply:?}"
    );
    assert!(window.terminals().is_empty());
    assert_eq!(git(&repo, &["branch", "--list", "failing"]), "");
    assert!(!scratch.beside().exists());
    assert_eq!(snapshot(&scratch, &repo), before);
    still_there(&repo, &planted);
    window.stop(keeper);
}

#[test]
fn a_launcher_that_cannot_be_opened_takes_the_worktree_and_the_branch_away_again() {
    let scratch = Scratch::new(Shell::Broken);
    let repo = scratch.repository();
    let planted = plant(&scratch, &repo);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    // Read before the request is sent.
    let before = snapshot(&scratch, &repo);
    let (listed_before, _) = window.barrier();

    let (reply, seen) = window.create(&repository, "main", "broken", "claude");

    // The worktree was made, and the opening of the launcher failed: the refusal is the
    // Dock's sentence about the shell.
    let broken = scratch.0.join("broken-shell");
    let Err(Refusal::Failed { reason }) = reply else {
        panic!("not refused: {reply:?}");
    };
    assert!(
        reason.starts_with(&format!("{} could not be started", broken.display())),
        "{reason}"
    );
    // Nothing is left, and no Window heard of the workspace, before the barrier's reply.
    let (listed, after_the_reply) = window.barrier();
    assert_eq!(listed, listed_before);
    assert!(
        !some_list_holds_a_workspace_on(&lists_told(&seen), "broken")
            && !some_list_holds_a_workspace_on(&lists_told(&after_the_reply), "broken"),
        "{seen:?} {after_the_reply:?}"
    );
    assert!(window.terminals().is_empty());
    assert_eq!(git(&repo, &["branch", "--list", "broken"]), "");
    assert!(!scratch.beside().exists());
    assert_eq!(snapshot(&scratch, &repo), before);
    still_there(&repo, &planted);
    window.stop(keeper);
}

#[test]
fn a_worktree_that_cannot_be_taken_away_again_is_named_in_the_refusal_and_stays_listed() {
    let scratch = Scratch::new(Shell::Broken);
    let repo = scratch.repository();
    // The user's hook locks the worktree git just made, so that git refuses to remove it:
    // the opening of the launcher fails, and the take-away after it fails too. Put after
    // nothing that checks out, as a hook runs for every checkout.
    scratch.hook(&repo, "git worktree lock --reason held \"$(pwd -P)\"");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let (mut second_window, stays) = (scratch.window(), scratch.beside().join("broken"));

    let (reply, seen) = window.create(&repository, "main", "broken", "claude");

    // The Dock's sentence first, then which worktree stays.
    let broken = scratch.0.join("broken-shell");
    let Err(Refusal::Failed { reason }) = reply else {
        panic!("not refused: {reply:?}");
    };
    assert!(
        reason.starts_with(&format!("{} could not be started", broken.display())),
        "{reason}"
    );
    assert!(
        reason.contains(&format!(
            "the worktree at {} could not be taken away",
            stays.display()
        )),
        "{reason}"
    );
    // It stays in git's list, and no terminal was made.
    assert!(worktree_folders(&repo).contains(&stays));
    assert!(window.terminals().is_empty());
    // What it left is no secret from the Windows: the list they were told last is the one
    // they would be given now, and it holds the workspace the folder stays as.
    let (listed, after_the_reply) = window.barrier();
    let told = lists_told(&seen);
    assert_eq!(told.last(), Some(&listed), "{seen:?}");
    assert!(after_the_reply.is_empty(), "{after_the_reply:?}");
    assert!(some_list_holds_a_workspace_on(&[listed], "broken"));
    let heard = second_window.next();
    assert!(
        matches!(&heard, Message::Fact(Fact::Workspaces(list))
            if some_list_holds_a_workspace_on(std::slice::from_ref(list), "broken")),
        "{heard:?}"
    );

    // The scratch is cleaned: the lock lifted, the worktree and its branch taken away.
    window.stop(keeper);
    git(
        &repo,
        &[
            "worktree",
            "unlock",
            stays.to_str().expect("a path of text"),
        ],
    );
    git(
        &repo,
        &[
            "worktree",
            "remove",
            "--force",
            stays.to_str().expect("a path of text"),
        ],
    );
    git(&repo, &["branch", "-D", "broken"]);
}

#[test]
fn the_folder_a_user_names_is_kept_told_and_used_until_none_is_named() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let mut other = scratch.window();
    let places = scratch.0.join("places");

    // No folder is named: the worktree lies beside the repository.
    let (reply, _) = window.create(&repository, "main", "first", "claude");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    let _ = other.next();

    // The folder, which is not there yet, is named. Both Windows are told, and it is listed.
    let named = worktrees_folder(&repository, Some(&places));
    assert!(matches!(
        window.set_folder(&repository, Some(&places)),
        Ok(Reply::Done)
    ));
    window.told(&named);
    other.told(&named);
    assert!(window.settings().contains(&named));
    assert!(!places.exists(), "setting the folder made it");

    // A Keeper started again on the same data lists it.
    window.stop(keeper);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut other = scratch.window();
    assert!(window.settings().contains(&named));

    // The next worktree lies in it, which the creation made.
    let (reply, seen) = window.create(&repository, "main", "in-place", "claude");
    let Ok(Reply::WorktreeCreated { workspace, .. }) = reply else {
        panic!("no worktree: {reply:?}");
    };
    let expected = places.join("in-place");
    assert_eq!(
        lists_told(&seen)
            .last()
            .and_then(|list| found_in(list, &repository, &workspace)),
        Some((expected.clone(), Some(Head::Branch("in-place".to_owned())))),
    );
    assert!(worktree_folders(&repo).contains(&expected));
    assert!(expected.join("a.txt").is_file());
    // The list that came with it is not what the next fact is waited for behind.
    assert!(matches!(other.next(), Message::Fact(Fact::Workspaces(_))));

    // None is the way back: told, listed as none, and the worktrees lie beside the
    // repository again (the worktrees now lie in two folders, so the rule beside it).
    let back = worktrees_folder(&repository, None);
    assert!(matches!(
        window.set_folder(&repository, None),
        Ok(Reply::Done)
    ));
    window.told(&back);
    other.told(&back);
    assert!(window.settings().contains(&back));
    assert!(!window.settings().contains(&named));
    let (reply, seen) = window.create(&repository, "main", "beside", "claude");
    let Ok(Reply::WorktreeCreated { workspace, .. }) = reply else {
        panic!("no worktree: {reply:?}");
    };
    assert_eq!(
        lists_told(&seen)
            .last()
            .and_then(|list| found_in(list, &repository, &workspace)),
        Some((
            scratch.beside().join("beside"),
            Some(Head::Branch("beside".to_owned()))
        )),
    );
    window.stop(keeper);
}

#[test]
fn a_folder_that_is_refused_is_not_kept_and_not_told() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let mut other = scratch.window();
    let kept = scratch.0.join("kept");
    let named = worktrees_folder(&repository, Some(&kept));
    assert!(matches!(
        window.set_folder(&repository, Some(&kept)),
        Ok(Reply::Done)
    ));
    window.told(&named);
    other.told(&named);

    use yaar_workspaces::Refusal as Refused;
    let unknown = RepositoryId::new("no-such-repository");
    let relative = PathBuf::from("places");
    let below = repo.join("sub");
    let in_git = repo.join(".git/x");
    let cases = [
        (
            "a folder that is not absolute",
            repository.clone(),
            relative.clone(),
            failed_with(&Refused::FolderNotAbsolute(relative)),
        ),
        (
            "a folder below the repository",
            repository.clone(),
            below.clone(),
            failed_with(&Refused::FolderInsideRepository(below)),
        ),
        (
            "a folder below its git folder",
            repository.clone(),
            in_git.clone(),
            failed_with(&Refused::FolderInsideRepository(in_git)),
        ),
        (
            "a repository the Keeper does not know",
            unknown.clone(),
            scratch.0.join("elsewhere"),
            Refusal::Failed {
                reason: format!("the repository {unknown} is not known"),
            },
        ),
    ];
    for (what, repository, folder, refusal) in cases {
        let reply = window.set_folder(&repository, Some(&folder));
        assert_eq!(reply, Err(refusal), "{what}");
        // Still what was kept before, and nothing of the kind listed for the other.
        let folders: Vec<Setting> = window
            .settings()
            .into_iter()
            .filter(|setting| matches!(setting, Setting::WorktreesFolder { .. }))
            .collect();
        assert_eq!(folders, std::slice::from_ref(&named), "{what}");
    }

    // The barrier: the next setting kept is the next fact the other Window sees, so that
    // nothing was told for the refusals before it.
    let newer = worktrees_folder(&repository, Some(&scratch.0.join("newer")));
    assert!(matches!(
        window.set_folder(&repository, Some(&scratch.0.join("newer"))),
        Ok(Reply::Done)
    ));
    other.told(&newer);
    window.told(&newer);
    assert!(window.settings().contains(&newer));
    window.stop(keeper);
}

#[test]
fn the_folder_named_for_one_repository_is_used_for_no_other() {
    let scratch = Scratch::new(Shell::Plain);
    let repo = scratch.repository();
    let second = scratch.repository_at(&scratch.0.join("lair-two/second"));
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (first_id, _) = window.open(&repo);
    let (second_id, _) = window.open(&second);
    let places = scratch.0.join("places");
    assert!(matches!(
        window.set_folder(&first_id, Some(&places)),
        Ok(Reply::Done)
    ));

    let (reply, seen) = window.create(&second_id, "main", "two", "claude");
    let Ok(Reply::WorktreeCreated { workspace, .. }) = reply else {
        panic!("no worktree: {reply:?}");
    };
    let beside_second = scratch.0.join("lair-two/second.worktrees/two");
    assert_eq!(
        lists_told(&seen)
            .last()
            .and_then(|list| found_in(list, &second_id, &workspace)),
        Some((beside_second.clone(), Some(Head::Branch("two".to_owned())))),
    );
    assert!(worktree_folders(&second).contains(&beside_second));
    assert!(!places.exists());

    // The folder is still the first repository's.
    let (reply, _) = window.create(&first_id, "main", "one", "claude");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    assert!(worktree_folders(&repo).contains(&places.join("one")));
    window.stop(keeper);
}
