//! System tests of a worktree made with an agent in one request: the built Keeper and Dock,
//! the stand-in agent started by the launcher the Keeper opens in the new worktree, read
//! through the Link as a Window reads it and through the files the stand-in leaves.
//!
//! The stand-in runs as Claude Code and, in the second function of the row, as pi, through
//! the same body with the same assertions. It is started from a launcher (the CLI's name
//! typed into a shell of the app), so it follows the world's script named `default`.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

use yaar_link::{
    Fact, Head, LinkError, Message, Refusal, Reply, RepositoryId, Request, RequestId, TerminalSize,
    WorkspaceId, read_message, write_message,
};
use yaar_system_tests::standin::{Did, Script, Step};
use yaar_system_tests::{DEADLINE, Dialect, Repo, World, attach, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// The file the stand-in's one step writes, in the folder it works in.
const MARKER_FILE: &str = "agent-was-here";

/// A world whose stand-in follows a script of one step: it writes [`MARKER_FILE`] into its
/// working folder. Pi is installed and allowed in both runs, so that a Keeper that started
/// the other CLI than the one the request names fails at the sentence that says which
/// started, and not at a wait that runs out.
fn world_for() -> (World, Repo) {
    let script = Script(vec![Step::Write(PathBuf::from(MARKER_FILE), "yes".into())]);
    World::with_standin_and_pi("default", &script)
}

/// One request sent on a connection of its own and read as that Window reads it: every fact
/// that came before the reply, in order, and the reply.
struct Asked {
    facts: Vec<Fact>,
    reply: Result<Reply, Refusal>,
}

/// A connection to the Keeper that reads its frames in the order they come, which a
/// `Client` does not show: it keeps the facts that came while a reply was waited for, but
/// not whether a later fact came before or after the reply.
///
/// A thread of its own reads the frames and hands each one over a channel, so that the wait
/// for a reply is a `recv_timeout` against one deadline and no option is set on a socket
/// whose Keeper may have gone.
struct Window {
    stream: UnixStream,
    frames: Receiver<Result<Message, LinkError>>,
    next_id: u32,
}

impl Window {
    fn attach(world: &World) -> Window {
        let (stream, _) = attach(&world.socket());
        let mut reading = stream.try_clone().expect("clone the connection to read it");
        let (hand_over, frames) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let frame = read_message(&mut reading);
                let ended = frame.is_err();
                // The thread ends with the stream, or when the Window is gone.
                if hand_over.send(frame).is_err() || ended {
                    break;
                }
            }
        });
        Window {
            stream,
            frames,
            next_id: 0,
        }
    }

    /// Sends `request` and reads until its reply. Panics, naming the request, when the reply
    /// does not come within one deadline for the whole wait, however many frames come first.
    fn ask(&mut self, request: Request) -> Asked {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        let kind = request.kind();
        write_message(&mut self.stream, &Message::Request { id, request })
            .unwrap_or_else(|e| panic!("the request {kind:?} could not be sent: {e}"));
        let mut facts = Vec::new();
        let end = Instant::now() + DEADLINE;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.frames.recv_timeout(left) {
                Ok(Ok(Message::Reply { to, reply })) if to == id => return Asked { facts, reply },
                Ok(Ok(Message::Fact(fact))) => facts.push(fact),
                // What the program of a terminal writes comes to the Window that asked.
                Ok(Ok(Message::Output { .. })) => {}
                Ok(Ok(other)) => {
                    panic!("waiting for the reply to {kind:?}, the Keeper sent {other:?}")
                }
                Ok(Err(e)) => panic!("no reply to {kind:?}: {e}; facts before it: {facts:?}"),
                Err(_) => {
                    panic!("no reply to {kind:?} within {DEADLINE:?}; facts before it: {facts:?}")
                }
            }
        }
    }
}

/// Runs git in `folder` as a reader of a repository does: with no optional lock, so that
/// reading never rewrites the index, and with nothing of the user's configuration.
fn read_git(world: &World, folder: &Path, args: &[&str]) -> Vec<u8> {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(search) = std::env::var_os("PATH") {
        git.env("PATH", search);
    }
    let output = git
        .current_dir(folder)
        .args(args)
        .env("HOME", world.home())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
        .env("GIT_COMMITTER_NAME", "A Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z")
        .env("LC_ALL", "C")
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

fn read_git_text(world: &World, folder: &Path, args: &[&str]) -> String {
    String::from_utf8(read_git(world, folder, args))
        .expect("git printed text")
        .trim_end()
        .to_owned()
}

/// What `git worktree list --porcelain` of `repo` says about the worktree at `folder`: its
/// commit and the branch it has checked out, none when it is not listed.
fn listed(world: &World, repo: &Repo, folder: &Path) -> Option<(String, Option<String>)> {
    let listing = read_git_text(world, repo.path(), &["worktree", "list", "--porcelain"]);
    listing.split("\n\n").find_map(|record| {
        let field = |key: &str| {
            record
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .map(str::to_owned)
        };
        let at = PathBuf::from(field("worktree ")?);
        (at.canonicalize().unwrap_or(at) == folder).then(|| {
            (
                field("HEAD ").expect("a worktree lists its commit"),
                field("branch "),
            )
        })
    })
}

/// Every name under `folder`, recursively, with whether it is a folder, a link or a file.
fn listing(folder: &Path) -> BTreeSet<(PathBuf, char)> {
    fn walk(root: &Path, folder: &Path, into: &mut BTreeSet<(PathBuf, char)>) {
        for entry in fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            let kind = fs::symlink_metadata(&path).expect("the kind").file_type();
            let letter = if kind.is_symlink() {
                'l'
            } else if kind.is_dir() {
                'd'
            } else {
                'f'
            };
            into.insert((path.strip_prefix(root).expect("below").to_owned(), letter));
            if kind.is_dir() {
                walk(root, &path, into);
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(folder, folder, &mut found);
    found
}

/// What a reader of the repository sees, and every name in the folder the repository lies
/// in: the user's repository, which a refused request leaves as it was.
#[derive(Debug, PartialEq, Eq)]
struct Reads {
    git: Vec<Bytes>,
    objects: Vec<(PathBuf, u64)>,
    around: BTreeSet<(PathBuf, char)>,
}

/// What a read gave, compared byte for byte and shown as text when two differ.
#[derive(PartialEq, Eq)]
struct Bytes(Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = String::from_utf8_lossy(&self.0);
        write!(f, "{} bytes: {text:?}", self.0.len())
    }
}

fn reads(world: &World, repo: &Repo) -> Reads {
    let git_folder = repo.path().join(".git");
    let objects = listing(&git_folder.join("objects"))
        .into_iter()
        .filter(|(_, kind)| *kind == 'f')
        .map(|(path, _)| {
            let size = fs::metadata(git_folder.join("objects").join(&path))
                .expect("the size of an object")
                .len();
            (path, size)
        })
        .collect();
    Reads {
        git: [
            &[
                "status",
                "--porcelain=v2",
                "--ignored",
                "--untracked-files=all",
            ][..],
            &["for-each-ref"],
            &["worktree", "list", "--porcelain"],
            &["stash", "list"],
            &["fsck", "--unreachable", "--no-reflogs"],
        ]
        .iter()
        .map(|args| read_git(world, repo.path(), args))
        .chain([
            fs::read(git_folder.join("index")).expect("the index"),
            fs::read(git_folder.join("config")).expect("the configuration"),
        ])
        .map(Bytes)
        .collect(),
        objects,
        around: listing(repo.path().parent().expect("the repository has a parent")),
    }
}

/// Two objects no ref reaches, planted before the first read: a blob and a dangling commit.
/// Gives their ids.
fn plant(world: &World, repo: &Repo) -> Vec<String> {
    let file = world.home().join("planted.txt");
    fs::write(&file, "a blob nothing refers to\n").expect("write the blob's file");
    let blob = repo.git(&["hash-object", "-w", file.to_str().expect("a path of text")]);
    repo.write("README.md", "edited\n");
    let dangling = repo.git(&["stash", "create"]);
    repo.git(&["checkout", "--quiet", "--", "README.md"]);
    assert!(!dangling.is_empty());
    vec![blob, dangling]
}

fn a_worktree_is_made_with_an_agent_in_one_request(dialect: Dialect) {
    let (world, repo) = world_for();
    // The workspace on screen is on another branch, one commit ahead of `main`: the worktree
    // is made from the branch the request names and not from what is checked out.
    let main_commit = repo.git(&["rev-parse", "main"]);
    repo.git(&["checkout", "--quiet", "-b", "other"]);
    repo.write("other.txt", "ahead of main\n");
    assert_ne!(repo.commit("Go ahead of main"), main_commit);
    let mut watcher = world.client();
    let (repository, _) = watcher
        .open_folder(repo.path())
        .expect("open the folder of the repository");
    let parent = repo
        .path()
        .parent()
        .expect("the repository has a parent")
        .canonicalize()
        .expect("resolve the parent");
    let folder = parent.join("app.worktrees").join("agent-task");
    assert!(!folder.exists(), "nothing lies there before the request");

    // One request, from a Window of its own, which reads the frames in the order they come.
    let mut window = Window::attach(&world);
    let asked = window.ask(Request::CreateWorktree {
        repository: repository.clone(),
        from: "main".to_owned(),
        branch: "agent-task".to_owned(),
        agent: dialect.name().to_owned(),
        size: SIZE,
    });
    let Ok(Reply::WorktreeCreated {
        workspace,
        terminal,
    }) = asked.reply
    else {
        panic!("the Keeper answered {:?}", asked.reply);
    };

    // A worktree from a branch name: git lists it at the folder beside the repository, on the
    // new branch, made at the commit `main` is at.
    let folder = folder
        .canonicalize()
        .expect("the worktree's folder is there");
    assert_eq!(
        listed(&world, &repo, &folder),
        Some((
            main_commit.clone(),
            Some("refs/heads/agent-task".to_owned())
        ))
    );
    assert_eq!(
        repo.git(&["rev-parse", "refs/heads/agent-task"]),
        main_commit
    );
    // The Window was told the list that holds the workspace, before the reply.
    assert!(
        asked
            .facts
            .iter()
            .any(|fact| names_the_worktree(fact, &repository, &workspace, &folder)),
        "no list with the new workspace before the reply: {:?}",
        asked.facts
    );

    // And the agent starts in it: a session of the CLI, in the new workspace, on the
    // terminal the reply names.
    let agents = watcher.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|session| session.terminal.as_ref() == Some(&terminal))
    });
    let session = agents
        .sessions
        .iter()
        .find(|session| session.terminal.as_ref() == Some(&terminal))
        .expect("the wait accepted a session on the terminal");
    assert_eq!(session.agent, dialect.name());
    assert_eq!(session.workspace, workspace);

    // The stand-in wrote its file where it works, which is the new worktree.
    let written = folder.join(MARKER_FILE);
    assert!(
        wait_until(DEADLINE, || written.exists()),
        "the stand-in wrote no {MARKER_FILE} in {}: {:?}",
        folder.display(),
        world.agent_log("default")
    );
    assert_eq!(fs::read_to_string(&written).expect("read it"), "yes");
    // Its one step is done, so it cannot write the file anywhere else now.
    assert!(
        wait_until(DEADLINE, || world
            .agent_log("default")
            .did
            .iter()
            .any(|did| matches!(did, Did::Done { step: 0, .. }))),
        "the step did not end: {:?}",
        world.agent_log("default")
    );
    assert!(
        !repo.path().join(MARKER_FILE).exists(),
        "the stand-in wrote into the repository's own folder"
    );
}

/// Whether `fact` lists `workspace` of `repository` at `folder` on the branch `agent-task`.
fn names_the_worktree(
    fact: &Fact,
    repository: &RepositoryId,
    workspace: &WorkspaceId,
    folder: &Path,
) -> bool {
    let Fact::Workspaces(repositories) = fact else {
        return false;
    };
    repositories
        .iter()
        .filter(|listed| listed.id == *repository)
        .flat_map(|listed| &listed.workspaces)
        .any(|listed| {
            listed.id == *workspace
                && listed.path.canonicalize().ok().as_deref() == Some(folder)
                && listed.head == Some(Head::Branch("agent-task".to_owned()))
        })
}

#[test]
fn one_request_makes_a_worktree_from_a_branch_name_and_starts_claude_code_in_it() {
    a_worktree_is_made_with_an_agent_in_one_request(Dialect::ClaudeCode);
}

#[test]
fn one_request_makes_a_worktree_from_a_branch_name_and_starts_the_agent_in_it_as_pi() {
    a_worktree_is_made_with_an_agent_in_one_request(Dialect::Pi);
}

#[test]
fn a_refused_request_makes_nothing_and_leaves_the_repository_as_it_was() {
    let (world, repo) = world_for();
    let mut client = world.client();
    let (repository, _) = client
        .open_folder(repo.path())
        .expect("open the folder of the repository");
    let planted = plant(&world, &repo);
    let before = reads(&world, &repo);
    let workspaces = client.workspaces();
    let terminals = client.terminals();

    let create = |from: &str, branch: &str, agent: &str| Request::CreateWorktree {
        repository: repository.clone(),
        from: from.to_owned(),
        branch: branch.to_owned(),
        agent: agent.to_owned(),
        size: SIZE,
    };
    // An agent no Adapter has, a branch that exists, and a branch to start from that is none.
    let no_agent = client.request(create("main", "task", "otheragent"));
    let taken = client.request(create("main", "main", Dialect::ClaudeCode.name()));
    let no_base = client.request(create("nowhere", "task", Dialect::ClaudeCode.name()));
    assert!(
        matches!(&no_agent, Err(Refusal::Failed { reason })
            if reason == "there is no agent CLI named otheragent"),
        "{no_agent:?}"
    );
    // The Keeper's own check says it, not git's failure after it.
    assert!(
        matches!(&taken, Err(Refusal::Failed { reason })
            if reason == "a branch named main exists already"),
        "{taken:?}"
    );
    assert!(
        matches!(no_base, Err(Refusal::UnknownRevision { .. })),
        "{no_base:?}"
    );

    // The barrier: the Keeper answered each, and then a list; whatever it made it made by
    // now. Nothing is listed, no terminal opened, and the repository is as it was.
    assert_eq!(client.workspaces(), workspaces);
    assert_eq!(client.terminals(), terminals);
    assert_eq!(reads(&world, &repo), before);
    for id in &planted {
        repo.git(&["cat-file", "-e", id]);
    }
    assert!(
        !repo
            .path()
            .parent()
            .expect("the repository has a parent")
            .join("app.worktrees")
            .exists()
    );
}
