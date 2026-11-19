//! Component tests: who is credited with a change, through the calls the Keeper makes, in
//! real repositories the tests make, with the real watcher on them and a real database.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::{Heard, Scratch, ago, commit_all, init, wait_until, workspaces, write};
use yaar_workspaces::{Change, CreditedPath, Refusal, WorkspaceError, WorkspaceId, Workspaces};

/// A repository with `edited.txt` and `kept.txt` committed, opened and watched. The
/// scratch folder stays where the test can see it.
struct Watched {
    workspaces: Workspaces,
    workspace: WorkspaceId,
    root: PathBuf,
    heard: Heard,
}

fn watched(scratch: &Scratch) -> Watched {
    let root = scratch.join("repo");
    init(&root);
    write(&root, "edited.txt", "one\n");
    write(&root, "kept.txt", "kept\n");
    commit_all(&root, "first");
    let workspaces = workspaces(scratch);
    let workspace = workspaces.open_folder(&root).expect("open").workspace;
    let heard = Heard::default();
    workspaces
        .watch(&workspace, heard.told())
        .expect("watch the worktree");
    Watched {
        workspaces,
        workspace,
        root,
        heard,
    }
}

impl Watched {
    fn credits(&self) -> Vec<CreditedPath> {
        self.workspaces.credits(&self.workspace).expect("credits")
    }

    /// The sessions the change of `path` is credited to, none where it is not listed.
    fn by(&self, path: &str) -> Option<Vec<String>> {
        self.credits()
            .into_iter()
            .find(|one| one.path == Path::new(path))
            .map(|one| one.by)
    }

    /// Waits until the watcher has told of a change of files after this call, the way a
    /// test learns that the ledger has noted a write it made before.
    fn wait_for_a_notice_of_files(&self, since: usize) {
        wait_until("the watcher told of the files", || {
            self.heard.files() > since
        });
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_file_changed_by_an_editor_outside_the_app_shows_as_a_change_no_agent_made() {
    let scratch = Scratch::new();
    let w = watched(&scratch);

    // A file an agent's tool named, in the same worktree, to show that the rule tells the
    // two apart.
    write(&w.root, "agent.txt", "from the agent\n");
    w.workspaces
        .agent_wrote(&w.workspace, "agent-session", &w.root.join("agent.txt"));

    // An editor: a program of the user that is no supported agent CLI. First one that
    // writes the file itself, then a child process that writes through its shell.
    let before = w.heard.files();
    std::fs::write(w.root.join("edited.txt"), "two, by hand\n").expect("edit the file");
    w.wait_for_a_notice_of_files(before);
    let before = w.heard.files();
    let status = Command::new("sh")
        .args(["-c", "echo x > other.txt"])
        .current_dir(&w.root)
        .status()
        .expect("start the shell");
    assert!(status.success());
    w.wait_for_a_notice_of_files(before);

    wait_until("credits lists the three files", || {
        ["agent.txt", "edited.txt", "other.txt"]
            .iter()
            .all(|path| w.by(path).is_some())
    });
    let credits = w.credits();
    let edited = credits
        .iter()
        .find(|one| one.path == Path::new("edited.txt"))
        .expect("edited.txt is listed");
    assert_eq!(edited.change, Change::Modified);
    assert!(edited.by.is_empty(), "an editor's change is no agent's");
    let other = credits
        .iter()
        .find(|one| one.path == Path::new("other.txt"))
        .expect("other.txt is listed");
    assert_eq!(other.change, Change::Added);
    assert!(other.by.is_empty(), "a shell's change is no agent's");
    assert_eq!(w.by("agent.txt"), Some(strings(&["agent-session"])));
}

#[test]
fn a_write_an_agents_tool_named_is_credited_whether_the_path_is_absolute_or_relative() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    for (file, named) in [
        ("absolute.txt", w.root.join("absolute.txt")),
        ("relative.txt", PathBuf::from("relative.txt")),
        ("dir/inner.txt", PathBuf::from("./dir/inner.txt")),
    ] {
        write(&w.root, file, "written\n");
        w.workspaces.agent_wrote(&w.workspace, "a", &named);
    }
    assert_eq!(w.by("absolute.txt"), Some(strings(&["a"])));
    assert_eq!(w.by("relative.txt"), Some(strings(&["a"])));
    assert_eq!(w.by("dir/inner.txt"), Some(strings(&["a"])));
}

#[test]
fn a_write_named_outside_the_worktree_is_not_noted() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    write(scratch.path(), "elsewhere/x.txt", "not ours\n");
    for outside in [
        scratch.join("elsewhere/x.txt"),
        PathBuf::from("../elsewhere/x.txt"),
        // A sibling whose name starts like the worktree's own folder.
        scratch.join("repo-two/x.txt"),
    ] {
        w.workspaces.agent_wrote(&w.workspace, "a", &outside);
    }
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn two_sessions_that_named_one_path_are_both_credited_in_the_order_they_wrote() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    for session in ["b", "a", "b"] {
        w.workspaces
            .agent_wrote(&w.workspace, session, Path::new("x.txt"));
    }
    assert_eq!(w.by("x.txt"), Some(strings(&["b", "a"])));
}

#[test]
fn a_write_named_in_a_workspace_that_is_gone_or_unknown_changes_nothing() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    let unknown = WorkspaceId::new("0".repeat(32));
    w.workspaces.agent_wrote(&unknown, "a", Path::new("x.txt"));
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn credits_lists_every_uncommitted_change_ordered_by_path_and_nothing_else() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "kept.txt", "changed\n");
    write(&w.root, "b.txt", "new\n");
    write(&w.root, "a/inner.txt", "new\n");
    let listed: Vec<(PathBuf, Change)> = w
        .credits()
        .into_iter()
        .map(|one| (one.path, one.change))
        .collect();
    assert_eq!(
        listed,
        [
            (PathBuf::from("a/inner.txt"), Change::Added),
            (PathBuf::from("b.txt"), Change::Added),
            (PathBuf::from("kept.txt"), Change::Modified),
        ]
    );
}

#[test]
fn a_path_leaves_the_ledger_when_it_is_committed_and_a_later_change_of_it_is_new() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "by the agent\n");
    w.workspaces
        .agent_wrote(&w.workspace, "a", Path::new("x.txt"));
    assert_eq!(w.by("x.txt"), Some(strings(&["a"])));

    commit_all(&w.root, "the agent's change, committed");
    assert_eq!(w.by("x.txt"), None, "committed: no change to list");

    // The same path edited again, by hand: nothing of the first credit is left.
    write(&w.root, "x.txt", "by hand\n");
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn a_change_seen_while_one_session_runs_a_shell_command_is_credited_to_that_session() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    w.workspaces.command_began(&w.workspace, "a", ago(1));
    let before = w.heard.files();
    write(&w.root, "made-by-the-command.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is credited", || {
        w.by("made-by-the-command.txt") == Some(strings(&["a"]))
    });
}

#[test]
fn a_change_seen_after_a_shell_command_ended_is_no_agents() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    // The command ran, and its end was reported, long before the change.
    w.workspaces.command_began(&w.workspace, "a", ago(2));
    w.workspaces.command_ended(&w.workspace, "a", ago(1));
    let before = w.heard.files();
    write(&w.root, "by-hand.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is listed", || w.by("by-hand.txt").is_some());
    assert_eq!(w.by("by-hand.txt"), Some(Vec::new()));
}

#[test]
fn a_change_seen_while_two_sessions_run_shell_commands_is_no_agents() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    w.workspaces.command_began(&w.workspace, "a", ago(2));
    w.workspaces.command_began(&w.workspace, "b", ago(1));
    let before = w.heard.files();
    write(&w.root, "x.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is listed", || w.by("x.txt").is_some());
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn a_workspace_that_is_not_watched_credits_only_what_a_tool_named() {
    let scratch = Scratch::new();
    let root = scratch.join("repo");
    init(&root);
    write(&root, "kept.txt", "kept\n");
    commit_all(&root, "first");
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&root).expect("open").workspace;
    assert!(workspaces.watched().is_empty());

    write(&root, "named.txt", "x\n");
    write(&root, "by-hand.txt", "x\n");
    workspaces.agent_wrote(&workspace, "a", Path::new("named.txt"));
    workspaces.command_began(&workspace, "a", ago(1));
    let by: Vec<(PathBuf, Vec<String>)> = workspaces
        .credits(&workspace)
        .expect("credits")
        .into_iter()
        .map(|one| (one.path, one.by))
        .collect();
    assert_eq!(
        by,
        [
            (PathBuf::from("by-hand.txt"), Vec::new()),
            (PathBuf::from("named.txt"), strings(&["a"])),
        ]
    );
}

#[test]
fn the_credits_of_a_workspace_the_store_does_not_hold_are_refused() {
    let scratch = Scratch::new();
    let workspaces = workspaces(&scratch);
    let unknown = WorkspaceId::new("0".repeat(32));
    match workspaces.credits(&unknown) {
        Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(id))) => assert_eq!(id, unknown),
        other => panic!("not refused: {other:?}"),
    }
}
