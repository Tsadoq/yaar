mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::{Heard, Scratch, commit_all, git, init, program, workspaces, write};
use yaar_workspaces::{Created, RepositoryId, WorkspaceError, Workspaces};

struct World {
    scratch: Scratch,
    repo: PathBuf,
    repository: RepositoryId,
    workspaces: Workspaces,
}

fn world() -> World {
    let scratch = Scratch::new();
    let repo = scratch.join("lair").join("repo");
    init(&repo);
    git(&repo, &["config", "user.name", "A Test"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let workspaces = workspaces(&scratch);
    let repository = workspaces
        .open_folder(&repo)
        .expect("open the repository")
        .repository;
    World {
        scratch,
        repo,
        repository,
        workspaces,
    }
}

/// A `post-checkout` hook that locks the worktree it runs in: git then refuses to remove it, as for
/// a user's own lock.
fn lock_what_is_made(world: &World) {
    program(
        &world.repo.join(".git/hooks/post-checkout"),
        "git worktree lock --reason held \"$(pwd -P)\"",
    );
}

impl World {
    fn create(&self, branch: &str) -> Created {
        let planned = self
            .workspaces
            .plan_worktree(&self.repository, "main", branch, None)
            .expect("plan");
        let added = planned.add().expect("add");
        self.workspaces.open_added(&planned, added).expect("open")
    }

    fn unlock(&self, created: &Created) {
        git(
            &self.repo,
            &[
                "worktree",
                "unlock",
                created.folder.to_str().expect("a path of text"),
            ],
        );
    }

    fn snapshot(&self) -> Snapshot {
        snapshot(&self.scratch, &self.repo)
    }
}

/// What taking a worktree away must leave as it found it: the seven reads of the repository, its
/// objects with their sizes, and every name below the scratch folder but the database and the home.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    reads: Vec<Vec<u8>>,
    objects: Vec<(PathBuf, u64)>,
    listing: Vec<(PathBuf, char)>,
}

/// / What a read of git prints, with its exit code. No optional lock: a read never rewrites the
/// / index of a repository committed to in the same second.
fn read_git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .expect("start git");
    let mut all = output.stdout;
    all.extend_from_slice(b"|");
    all.extend_from_slice(&output.status.code().unwrap_or(-1).to_le_bytes());
    all
}

/// / Every name below `folder`, relative to `below`, with a letter for its kind (`l` link,
/// / `d` folder, `f` file); top-level names in `skipping` are left out.
fn walk(folder: &Path, below: &Path, skipping: &[&str], found: &mut Vec<(PathBuf, char)>) {
    for entry in std::fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let relative = path.strip_prefix(below).expect("below").to_owned();
        if relative.components().count() == 1
            && skipping
                .iter()
                .any(|name| relative.as_path() == Path::new(name))
        {
            continue;
        }
        let kind = std::fs::symlink_metadata(&path).expect("kind").file_type();
        let letter = if kind.is_symlink() {
            'l'
        } else if kind.is_dir() {
            'd'
        } else {
            'f'
        };
        found.push((relative, letter));
        if letter == 'd' {
            walk(&path, below, skipping, found);
        }
    }
}

fn snapshot(scratch: &Scratch, repo: &Path) -> Snapshot {
    let objects_folder = repo.join(".git/objects");
    let mut files = Vec::new();
    walk(&objects_folder, &objects_folder, &[], &mut files);
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
    walk(
        scratch.path(),
        scratch.path(),
        &["app-data", "home"],
        &mut listing,
    );
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

fn listed_folders(world: &World) -> Vec<PathBuf> {
    world
        .workspaces
        .repositories()
        .expect("repositories")
        .into_iter()
        .flat_map(|repository| repository.workspaces)
        .map(|workspace| workspace.path)
        .collect()
}

#[test]
fn a_worktree_that_is_taken_away_is_no_longer_watched_listed_or_there_and_the_repository_is_as_it_was()
 {
    let world = world();
    let before = world.snapshot();
    let created = world.create("task");
    world
        .workspaces
        .watch(&created.workspace, Heard::default().told())
        .expect("watch");
    assert_eq!(
        world.workspaces.watched(),
        std::slice::from_ref(&created.workspace)
    );

    world.workspaces.remove_created(&created).expect("remove");

    assert!(
        world.workspaces.watched().is_empty(),
        "a worktree that is gone is not watched"
    );
    assert!(!created.folder.exists());
    assert_eq!(
        listed_folders(&world),
        [std::fs::canonicalize(&world.repo).expect("the repository")],
        "the repository lists the one worktree it has"
    );
    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_worktree_that_cannot_be_taken_away_stays_watched_and_listed_until_it_is_taken_away() {
    let world = world();
    lock_what_is_made(&world);
    let before = world.snapshot();
    let created = world.create("task");
    let heard = Heard::default();
    world
        .workspaces
        .watch(&created.workspace, heard.told())
        .expect("watch");

    let kept = world
        .workspaces
        .worktree(&created.workspace)
        .expect("read the worktree")
        .expect("a worktree that is there");

    let refused = world.workspaces.remove_created(&created);
    assert!(
        matches!(refused, Err(WorkspaceError::Git(_))),
        "git was to refuse to remove a locked worktree: {refused:?}"
    );
    let still_kept = world
        .workspaces
        .worktree(&created.workspace)
        .expect("read the worktree")
        .expect("a worktree that is there");
    assert!(
        std::sync::Arc::ptr_eq(&kept, &still_kept),
        "the workspace was forgotten before the worktree was taken away: the next read opened \
         the worktree anew"
    );
    assert_eq!(
        world.workspaces.watched(),
        std::slice::from_ref(&created.workspace),
        "the worktree is there, so it is still watched"
    );
    assert!(created.folder.is_dir());
    assert!(
        listed_folders(&world).contains(&created.folder),
        "and still listed: {:?}",
        listed_folders(&world)
    );
    assert!(
        world.workspaces.changes(&created.workspace).is_ok(),
        "and its changes can still be read"
    );

    world.unlock(&created);
    world
        .workspaces
        .remove_created(&created)
        .expect("remove again");
    assert!(world.workspaces.watched().is_empty());
    assert!(!created.folder.exists());
    assert_eq!(world.snapshot(), before);
}

const CHILD: &str = "YAAR_TAKING_AWAY_IN_THE_CHILD";

fn opening_fails_and_so_does_the_take_away() {
    let world = world();
    lock_what_is_made(&world);
    // git 2.43 leaves a `packed-refs` with only its header when a never-packed branch is deleted,
    // newer git does not. Making and deleting a branch here puts the file there before the
    // snapshot, whichever git runs.
    git(&world.repo, &["branch", "spare"]);
    git(&world.repo, &["branch", "-D", "spare"]);
    git(&world.repo, &["branch", "side"]);
    let before = world.snapshot();
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "side", "task", None)
        .expect("plan");
    let added = planned.add().expect("add");
    git(&world.repo, &["branch", "-D", "side"]);

    let outcome = world.workspaces.open_added(&planned, added);
    assert!(
        matches!(
            outcome,
            Err(WorkspaceError::Refused(yaar_workspaces::Refusal::UnknownRevision(ref name)))
                if name == "side"
        ),
        "{outcome:?}"
    );
    assert!(planned.folder().is_dir());
    assert!(
        listed_folders(&world).contains(&planned.folder().to_owned()),
        "{:?}",
        listed_folders(&world)
    );

    let folder = planned.folder().to_str().expect("a path of text");
    git(&world.repo, &["worktree", "unlock", folder]);
    git(&world.repo, &["worktree", "remove", "--force", folder]);
    git(&world.repo, &["branch", "-D", "task"]);
    git(&world.repo, &["branch", "side"]);
    std::fs::remove_dir(planned.folder().parent().expect("the folder above"))
        .expect("the folder the add made");
    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_take_away_that_fails_after_a_failed_opening_is_noted_on_the_error_output() {
    if std::env::var_os(CHILD).is_some() {
        opening_fails_and_so_does_the_take_away();
        return;
    }
    // The error output of a program is what the note is written to, so this test is run again as a
    // program of its own and its error output is read.
    let ran = Command::new(std::env::current_exe().expect("this program"))
        .args([
            "--exact",
            "a_take_away_that_fails_after_a_failed_opening_is_noted_on_the_error_output",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .expect("start the test again");
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(
        ran.status.success(),
        "the child failed: {said} {}",
        String::from_utf8_lossy(&ran.stdout)
    );
    assert!(
        said.contains("repo.worktrees/task could not be taken away"),
        "the worktree that stays is named: {said}"
    );
}
