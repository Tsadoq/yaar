//! Component tests: who is credited with a change, through the calls the Keeper makes, in
//! real repositories the tests make, with the real watcher on them and a real database.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use support::{Heard, Scratch, commit_all, init, wait_until, workspaces, write};
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
