//! System tests of a worktree made with an agent in one request, run with the stand-in as Claude
//! Code and as pi.

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

const MARKER_FILE: &str = "agent-was-here";

/// Pi is installed and allowed in both runs, so a Keeper that started the other CLI fails at the
/// sentence that says which started.
fn world_for() -> (World, Repo) {
    let script = Script(vec![Step::Write(PathBuf::from(MARKER_FILE), "yes".into())]);
    World::with_standin_and_pi("default", &script)
}

struct Asked {
    facts: Vec<Fact>,
    reply: Result<Reply, Refusal>,
}

/// A connection that shows facts and replies in the order they came, which a `Client` hides.
/// A reader thread hands frames over a channel, so a reply is waited for with `recv_timeout`
/// and no option is set on a socket whose Keeper may have gone.
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

#[derive(Debug, PartialEq, Eq)]
struct Reads {
    git: Vec<Bytes>,
    objects: Vec<(PathBuf, u64)>,
    around: BTreeSet<(PathBuf, char)>,
}

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

/// Installs repository hooks that write a file once git checks anything out or moves a ref,
/// and gives that file. Call it after `plant`, whose own checkout would run the hooks.
fn witness_of_any_checkout(world: &World, repo: &Repo) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let witness = world.home().join("hook.runs");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$0 $*\" >> '{}'\n",
        witness.display()
    );
    for hook in ["post-checkout", "reference-transaction"] {
        let file = repo.path().join(".git/hooks").join(hook);
        fs::write(&file, &script).expect("write the hook");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).expect("make it run");
    }
    witness
}

fn a_worktree_is_made_with_an_agent_in_one_request(dialect: Dialect) {
    let (world, repo) = world_for();
    // The worktree is made from the branch the request names, not from what is checked out.
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
    assert!(
        asked
            .facts
            .iter()
            .any(|fact| names_the_worktree(fact, &repository, &workspace, &folder)),
        "no list with the new workspace before the reply: {:?}",
        asked.facts
    );

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

    let written = folder.join(MARKER_FILE);
    assert!(
        wait_until(DEADLINE, || written.exists()),
        "the stand-in wrote no {MARKER_FILE} in {}: {:?}",
        folder.display(),
        world.agent_log("default")
    );
    assert_eq!(fs::read_to_string(&written).expect("read it"), "yes");
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
    let witness = witness_of_any_checkout(&world, &repo);
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
    let no_agent = client.request(create("main", "task", "otheragent"));
    let taken = client.request(create("main", "main", Dialect::ClaudeCode.name()));
    let no_base = client.request(create("nowhere", "task", Dialect::ClaudeCode.name()));
    assert!(
        matches!(&no_agent, Err(Refusal::Failed { reason })
            if reason == "there is no agent CLI named otheragent"),
        "{no_agent:?}"
    );
    assert!(
        matches!(&taken, Err(Refusal::Failed { reason })
            if reason == "a branch named main exists already"),
        "{taken:?}"
    );
    assert!(
        matches!(no_base, Err(Refusal::UnknownRevision { .. })),
        "{no_base:?}"
    );

    // The barrier: the Keeper answered each, then a list, so whatever it made it made by now.
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
    assert!(
        !witness.exists(),
        "git made something and took it away again: {}",
        fs::read_to_string(&witness).unwrap_or_default()
    );
}
