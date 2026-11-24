//! Component tests of a worktree made with an agent started in it. The agent is a stand-in script
//! named like the CLI, put first on the search path by the home's `.profile`. Each test sets the
//! process environment first, so nextest runs it in a process of its own.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Base, BaseSource, Comparison, Fact, Head, Hello, Message, Program, Refusal, Reply, Repository,
    RepositoryId, Request, RequestId, Setting, TerminalId, TerminalInfo, WorkspaceId, handshake,
    read_message, write_message,
};

mod support;

use support::{Environment, SIZE, Scratch};

const DEADLINE: Duration = Duration::from_secs(20);

/// A Keeper that ought to be waiting is given this long to show it is not; a slower one is missed,
/// never wrongly blamed.
const SETTLE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    Plain,
    /// Has an execute bit but its interpreter is no program: cannot be started.
    Broken,
}

/// A folder of the test's own, holding `home`, `tmp`, `data`, `fake` (the stand-ins),
/// `lair/repo` (alone, as its worktrees' folder is made beside it) and the Keeper's socket.
impl Scratch {
    fn new(shell: Shell) -> Scratch {
        let scratch = Scratch::in_tmp("worktrees", Environment::Full);
        for folder in ["fake", "lair"] {
            std::fs::create_dir_all(scratch.0.join(folder)).expect("make the test's folder");
        }
        for agent in ["claude", "pi"] {
            scratch.program(
                &scratch.0.join("fake").join(agent),
                "printf 'STANDIN-RAN <%s> <%s>\\n' \"$(basename \"$0\")\" \"$(pwd -P)\"\n\
                 exec cat > /dev/null",
            );
        }
        // A login shell's system profile resets the search path, so the home's `.profile` puts the
        // stand-ins first again.
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
        scratch.set_env("SHELL", &shell);
        scratch
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn lair(&self) -> PathBuf {
        self.0.join("lair")
    }

    fn beside(&self) -> PathBuf {
        self.lair().join("repo.worktrees")
    }

    fn program(&self, path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make it run");
    }

    fn hook(&self, repo: &Path, body: &str) {
        self.program(&repo.join(".git/hooks/post-checkout"), body);
    }

    fn options(&self) -> Options {
        Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.home(),
            login_shell: None,
        }
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = self.options();
        std::thread::spawn(move || run(options))
    }

    /// Waits until the Keeper serves this Window, which the first reply proves: a fact told
    /// before then never reaches this Window.
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

    fn repository(&self) -> PathBuf {
        self.repository_at(&self.lair().join("repo"))
    }

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

/// No optional lock, so a read never rewrites the index of a repository committed to in the same
/// second.
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

fn still_there(repo: &Path, planted: &[String]) {
    for id in planted {
        git(repo, &["cat-file", "-e", id]);
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

    fn next_any(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    fn next(&mut self) -> Message {
        loop {
            match self.next_any() {
                Message::Output { .. } => {}
                other => return other,
            }
        }
    }

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

    fn ask_seen(&mut self, request: Request) -> (Result<Reply, Refusal>, Vec<Message>) {
        let id = self.send(request);
        self.reply_to(id)
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_seen(request).0
    }

    /// `ListWorkspaces` is answered after everything the Keeper did for this Window before it.
    fn barrier(&mut self) -> (Vec<Repository>, Vec<Message>) {
        match self.ask_seen(Request::ListWorkspaces) {
            (Ok(Reply::Workspaces(list)), seen) => (list, seen),
            other => panic!("no list of workspaces: {other:?}"),
        }
    }

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

    fn set_folder(
        &mut self,
        repository: &RepositoryId,
        folder: Option<&Path>,
    ) -> Result<Reply, Refusal> {
        self.ask(Request::SetSetting {
            setting: worktrees_folder(repository, folder),
        })
    }

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

fn lists_told(seen: &[Message]) -> Vec<Vec<Repository>> {
    seen.iter()
        .filter_map(|message| match message {
            Message::Fact(Fact::Workspaces(list)) => Some(list.clone()),
            _ => None,
        })
        .collect()
}

fn some_list_holds_a_workspace_on(lists: &[Vec<Repository>], branch: &str) -> bool {
    lists.iter().flatten().any(|repository| {
        repository
            .workspaces
            .iter()
            .any(|workspace| workspace.head == Some(Head::Branch(branch.to_owned())))
    })
}

fn worktree_folders(repo: &Path) -> Vec<PathBuf> {
    git(repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

fn failed_with(refusal: &yaar_workspaces::Refusal) -> Refusal {
    Refusal::Failed {
        reason: refusal.to_string(),
    }
}

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
    // Read first: a request asked meanwhile passes over the output between it and its reply.
    let marker = format!("STANDIN-RAN <{agent}> <{}>", expected.display());
    window.output_holds(&terminal, &marker);
    let heard = second_window.next();
    assert!(
        matches!(&heard, Message::Fact(Fact::Workspaces(list))
            if found_in(list, &repository, &workspace).is_some()),
        "{heard:?}"
    );

    assert!(
        worktree_folders(&repo).contains(&expected),
        "{:?}",
        worktree_folders(&repo)
    );
    assert_eq!(git(&expected, &["branch", "--show-current"]), "task/one");
    assert_eq!(git(&repo, &["rev-parse", "task/one"]), main_commit);
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
    std::fs::create_dir_all(scratch.beside().join("taken")).expect("make the folder");
    let planted = plant(&scratch, &repo);
    // The hook is put after the planting, whose checkout would run it.
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
        let before = snapshot(&scratch, &repo);
        let (reply, seen) = window.create(&repository, from, branch, agent);
        let (listed, after_the_reply) = window.barrier();
        assert_eq!(reply, Err(refusal), "{what}");
        assert!(seen.is_empty(), "{what}: {seen:?}");
        assert!(after_the_reply.is_empty(), "{what}: {after_the_reply:?}");
        assert_eq!(listed, listed_before, "{what}");
        assert_eq!(snapshot(&scratch, &repo), before, "{what}");
        assert!(!ran.exists(), "{what}: git ran a hook");
        still_there(&repo, &planted);
    }

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

/// A `post-checkout` hook that waits on a pipe the test holds: while it waits, git is in
/// the middle of making a worktree. The test's open of the pipe for writing returns only
/// once the hook has it open for reading, which proves the hook runs.
struct Gate {
    running: mpsc::Receiver<()>,
    opening: JoinHandle<std::fs::File>,
}

impl Gate {
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

    fn hook_runs(&self) {
        self.running
            .recv_timeout(DEADLINE)
            .expect("the hook never started");
    }

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

    gate.let_go();
    let (reply, _) = window.reply_to(id);
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    window.stop(keeper);
}

/// Makes a stand-in `claude` that prints `AGENT-READY` once its traps are set, reads its
/// terminal, and writes to the fifo `ended` as it exits, as a hang-up makes it do.
/// The returned channel gets a message once it has ended.
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
    let planted = plant(&scratch, &repo);
    let gate = Gate::hook(&scratch, &repo);
    let agent_ended = an_agent_that_tells_when_it_ends(&scratch);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, first) = window.open(&repo);
    let mut other = scratch.window();
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

    write_message(&mut other.stream, &Message::Shutdown).expect("ask for the stop");
    let (stopped, stops) = mpsc::channel();
    std::thread::spawn(move || stopped.send(keeper.join()).expect("tell the test"));
    agent_ended
        .recv_timeout(DEADLINE)
        .expect("the Keeper did not end the program of the terminal");
    // The one timed negative: a Keeper that leaves while git is held is caught within
    // `SETTLE`. The hook is let go only after it, so a passing run cannot fail here.
    let left_early = stops.recv_timeout(SETTLE);
    // The Keeper's lock is held until the work ends: another Keeper started now on the same
    // data steps back, and one that let go early would start and not answer in the deadline.
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
    scratch.hook(&repo, "echo hook-said-no >&2\nexit 1");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let (listed_before, _) = window.barrier();
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
    let before = snapshot(&scratch, &repo);
    let (listed_before, _) = window.barrier();

    let (reply, seen) = window.create(&repository, "main", "broken", "claude");

    let broken = scratch.0.join("broken-shell");
    let Err(Refusal::Failed { reason }) = reply else {
        panic!("not refused: {reply:?}");
    };
    assert!(
        reason.starts_with(&format!("{} could not be started", broken.display())),
        "{reason}"
    );
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
    // The hook locks the worktree git just made, so git refuses to take it away when the
    // broken shell fails to start. A hook runs on every checkout, so it is set after the setup.
    scratch.hook(&repo, "git worktree lock --reason held \"$(pwd -P)\"");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (repository, _) = window.open(&repo);
    let (mut second_window, stays) = (scratch.window(), scratch.beside().join("broken"));

    let (reply, seen) = window.create(&repository, "main", "broken", "claude");

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
    assert!(worktree_folders(&repo).contains(&stays));
    assert!(window.terminals().is_empty());
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

    let (reply, _) = window.create(&repository, "main", "first", "claude");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    let _ = other.next();

    let named = worktrees_folder(&repository, Some(&places));
    assert!(matches!(
        window.set_folder(&repository, Some(&places)),
        Ok(Reply::Done)
    ));
    window.told(&named);
    other.told(&named);
    assert!(window.settings().contains(&named));
    assert!(!places.exists(), "setting the folder made it");

    window.stop(keeper);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut other = scratch.window();
    assert!(window.settings().contains(&named));

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
    assert!(matches!(other.next(), Message::Fact(Fact::Workspaces(_))));

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
        let folders: Vec<Setting> = window
            .settings()
            .into_iter()
            .filter(|setting| matches!(setting, Setting::WorktreesFolder { .. }))
            .collect();
        assert_eq!(folders, std::slice::from_ref(&named), "{what}");
    }

    // Barrier: the next setting kept is the next fact the other Window sees, so the
    // refusals before it told nothing.
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

    let (reply, _) = window.create(&first_id, "main", "one", "claude");
    assert!(
        matches!(reply, Ok(Reply::WorktreeCreated { .. })),
        "{reply:?}"
    );
    assert!(worktree_folders(&repo).contains(&places.join("one")));
    window.stop(keeper);
}
