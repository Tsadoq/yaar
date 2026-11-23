//! Component tests: a worktree made from a branch for an agent, through the calls the
//! Keeper makes, in real repositories the tests make and with a real database.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::{Scratch, commit_all, git, init, program, workspaces, write};
use yaar_workspaces::{
    Base, BaseSource, Comparison, Created, Planned, Refusal, RepositoryId, WorkspaceError,
    Workspaces,
};

/// A repository `lair/repo` with `main` at one commit, opened, in a scratch folder whose
/// database is the Keeper's.
struct World {
    scratch: Scratch,
    lair: PathBuf,
    repo: PathBuf,
    repository: RepositoryId,
    workspaces: Workspaces,
}

fn world() -> World {
    let scratch = Scratch::new();
    let lair = scratch.join("lair");
    let repo = lair.join("repo");
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
        lair,
        repo,
        repository,
        workspaces,
    }
}

impl World {
    /// The folder the app puts worktrees in, beside the repository.
    fn worktrees(&self) -> PathBuf {
        self.lair.join("repo.worktrees")
    }

    /// `plan_worktree`, `add` and `open_added`, as the Keeper calls them.
    fn create(&self, from: &str, branch: &str, folder: Option<&Path>) -> Created {
        let planned = self
            .workspaces
            .plan_worktree(&self.repository, from, branch, folder)
            .expect("plan");
        let added = planned.add().expect("add");
        self.workspaces.open_added(&planned, added).expect("open")
    }

    fn main_commit(&self) -> String {
        git(&self.repo, &["rev-parse", "refs/heads/main"])
    }

    fn planned_folder(&self, branch: &str, folder: Option<&Path>) -> PathBuf {
        self.workspaces
            .plan_worktree(&self.repository, "main", branch, folder)
            .expect("plan")
            .folder()
            .to_owned()
    }

    fn snapshot(&self) -> Snapshot {
        snapshot(&self.scratch, &self.repo)
    }
}

/// Everything a refusal must leave as it was: the seven reads of the repository, the names
/// below the scratch folder (but for the database and the home), and what the Store holds.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    reads: Vec<Vec<u8>>,
    objects: Vec<(PathBuf, u64)>,
    listing: Vec<(PathBuf, char)>,
    stored: String,
}

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
    let file = scratch.join("app-data").join(yaar_store::FILE_NAME);
    let store = yaar_store::Store::open(&file).expect("open the database");
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
        stored: format!(
            "{:?} {:?}",
            store.repositories().expect("repositories"),
            store.workspaces().expect("workspaces")
        ),
    }
}

/// Two objects no ref reaches, planted in the repository before the first read.
fn plant(world: &World) -> Vec<String> {
    write(
        world.scratch.path(),
        "blob.txt",
        "a blob nothing refers to\n",
    );
    let blob = git(
        &world.repo,
        &[
            "hash-object",
            "-w",
            world.scratch.join("blob.txt").to_str().expect("text"),
        ],
    );
    write(&world.repo, "a.txt", "edited\n");
    let dangling = git(&world.repo, &["stash", "create"]);
    git(&world.repo, &["checkout", "--quiet", "--", "a.txt"]);
    assert!(!dangling.is_empty());
    vec![blob, dangling]
}

fn refused(result: Result<impl std::fmt::Debug, WorkspaceError>) -> Refusal {
    match result {
        Err(WorkspaceError::Refused(refusal)) => refusal,
        other => panic!("not refused: {other:?}"),
    }
}

fn base(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::User,
    }
}

// WS-05.
#[test]
fn a_worktree_made_from_a_branch_lies_beside_the_repository_or_in_the_folder_named_and_has_that_branch_as_its_base()
 {
    let world = world();
    let main = world.main_commit();

    // "A worktree created on a new branch from a branch name": the three steps of the
    // Keeper, with no folder named.
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "task/one", None)
        .expect("plan");
    let one = world.worktrees().join("task-one");
    assert_eq!(planned.folder(), one);
    let added = planned.add().expect("add");
    let created = world.workspaces.open_added(&planned, added).expect("open");

    // "Lies beside the repository", where it keeps its worktrees: a slash in the branch's
    // name makes a dash in the folder's.
    assert_eq!(created.folder, one);
    let listed = git(&world.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!(
            "worktree {}\nHEAD {main}\nbranch refs/heads/task/one",
            one.display()
        )),
        "{listed}"
    );
    assert_eq!(
        std::fs::read_to_string(one.join("a.txt")).expect("a file"),
        "one\n"
    );
    assert_eq!(created.repository, world.repository);

    // "And has the branch it was made from as its base": as the user would have set it.
    assert_eq!(created.base, base("main"));
    drop(world.workspaces);
    let again = workspaces(&world.scratch);
    assert_eq!(
        again.base(&created.workspace).expect("base"),
        Some(base("main"))
    );
    let changes = again.changes(&created.workspace).expect("changes");
    assert_eq!(
        changes.comparison,
        Comparison::SinceBase { base: base("main") }
    );
    assert_eq!(
        changes.counted_from.as_ref().map(ToString::to_string),
        Some(main.clone())
    );
    assert!(changes.commits.is_empty() && changes.files.is_empty());
    // The base is `main`, not the branch: a commit on the new branch is what changed.
    write(&one, "b.txt", "new\n");
    let on_branch = commit_all(&one, "work on the task");
    let changes = again.changes(&created.workspace).expect("changes");
    assert_eq!(
        changes.counted_from.as_ref().map(ToString::to_string),
        Some(main.clone())
    );
    assert_eq!(
        changes
            .commits
            .iter()
            .map(|commit| commit.id.to_string())
            .collect::<Vec<_>>(),
        vec![on_branch]
    );

    // A second one lies in the folder the first one made: the folder the linked worktrees
    // share.
    let world = World {
        workspaces: again,
        ..world
    };
    let two = world.create("main", "two", None);
    assert_eq!(two.folder, world.worktrees().join("two"));
    // The base is the branch it was made from, whichever that is.
    git(&world.repo, &["branch", "side"]);
    let from_side = world.create("side", "from-side", None);
    assert_eq!(from_side.base, base("side"));
    assert_eq!(
        world.workspaces.base(&from_side.workspace).expect("base"),
        Some(base("side"))
    );

    // A repository whose linked worktree lies elsewhere puts the next one there.
    let other = world.scratch.join("other");
    init(&other.join("repo"));
    git(&other.join("repo"), &["config", "user.name", "A Test"]);
    git(
        &other.join("repo"),
        &["config", "user.email", "t@example.invalid"],
    );
    write(&other.join("repo"), "a.txt", "one\n");
    commit_all(&other.join("repo"), "first");
    let elsewhere = world.scratch.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a folder");
    git(
        &other.join("repo"),
        &[
            "worktree",
            "add",
            "-b",
            "x",
            elsewhere.join("x").to_str().expect("text"),
        ],
    );
    let other_id = world
        .workspaces
        .open_folder(&other.join("repo"))
        .expect("open")
        .repository;
    let next = world
        .workspaces
        .plan_worktree(&other_id, "main", "y", None)
        .expect("plan");
    assert_eq!(next.folder(), elsewhere.join("y"));
    // Linked worktrees in two folders share none: rule 2, the folder beside the main
    // worktree, and not the folder of the first, the last or the most of them.
    let elsewhere_too = world.scratch.join("elsewhere-too");
    std::fs::create_dir_all(&elsewhere_too).expect("a folder");
    git(
        &other.join("repo"),
        &[
            "worktree",
            "add",
            "-b",
            "z",
            elsewhere_too.join("z").to_str().expect("text"),
        ],
    );
    assert_eq!(
        world
            .workspaces
            .plan_worktree(&other_id, "main", "w", None)
            .expect("plan")
            .folder(),
        other.join("repo.worktrees").join("w")
    );

    // "Or in the folder the user named for the repository": a folder not there yet.
    let places = world.scratch.join("places");
    world
        .workspaces
        .check_worktrees_folder(&world.repository, &places)
        .expect("the folder may be named");
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "three", Some(&places))
        .expect("plan");
    assert_eq!(planned.folder(), places.join("three"));
    assert!(!places.exists(), "planning makes nothing");
    let added = planned.add().expect("add");
    let three = world.workspaces.open_added(&planned, added).expect("open");
    assert_eq!(three.folder, places.join("three"));
    assert!(places.is_dir(), "the folder named was made");
    let listed = git(&world.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!(
            "worktree {}\nHEAD {main}\nbranch refs/heads/three",
            places.join("three").display()
        )),
        "{listed}"
    );
    assert_eq!(three.base, base("main"));

    // The folder named comes before the folder the linked worktrees share, which it now is
    // for none: `places` holds one, and `repo.worktrees` two.
    assert_eq!(
        world.planned_folder("four", Some(&places)),
        places.join("four")
    );
    // With no folder named the rule is back: the linked worktrees lie in two folders now,
    // so the folder beside the repository.
    assert_eq!(
        world.planned_folder("five", None),
        world.worktrees().join("five")
    );
}

// A repository whose worktrees lie in the folder it names `<repository>.worktrees` first
// gets it made by the creation, and the next creation finds it there.
#[test]
fn the_folder_beside_the_repository_is_made_by_the_creation_and_not_by_the_plan() {
    let world = world();
    assert!(!world.worktrees().exists());
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "task", None)
        .expect("plan");
    assert!(!world.worktrees().exists(), "planning makes nothing");
    planned.add().expect("add");
    assert!(world.worktrees().is_dir());
}

// A bare repository keeps its worktrees where they all lie, and otherwise beside its git
// folder.
#[test]
fn a_bare_repository_puts_its_worktrees_where_they_lie_and_otherwise_beside_its_git_folder() {
    let scratch = Scratch::new();
    let source = scratch.join("source");
    init(&source);
    git(&source, &["config", "user.name", "A Test"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    write(&source, "a.txt", "one\n");
    commit_all(&source, "first");
    let lair = scratch.join("lair");
    std::fs::create_dir_all(&lair).expect("a folder");
    git(
        &lair,
        &[
            "clone",
            "--quiet",
            "--bare",
            source.to_str().expect("text"),
            "app.git",
        ],
    );
    let bare = lair.join("app.git");
    git(&bare, &["config", "user.name", "A Test"]);
    git(&bare, &["config", "user.email", "test@example.invalid"]);
    let trees = scratch.join("trees");
    std::fs::create_dir_all(&trees).expect("a folder");
    git(
        &bare,
        &[
            "worktree",
            "add",
            "-b",
            "w1",
            trees.join("w1").to_str().expect("text"),
            "main",
        ],
    );
    let workspaces = workspaces(&scratch);
    let repository = workspaces
        .open_folder(&trees.join("w1"))
        .expect("open")
        .repository;

    // Its one linked worktree lies in `trees`: so does the next.
    let planned = workspaces
        .plan_worktree(&repository, "main", "w2", None)
        .expect("plan");
    assert_eq!(planned.folder(), trees.join("w2"));
    let added = planned.add().expect("add");
    workspaces.open_added(&planned, added).expect("open");

    // The git folder of a bare repository is no place for them, though no worktree lies
    // there.
    assert_eq!(
        refused(workspaces.check_worktrees_folder(&repository, &bare.join("inside"))),
        Refusal::FolderInsideRepository(bare.join("inside"))
    );

    // Two folders now: beside the git folder.
    let elsewhere = scratch.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a folder");
    git(
        &bare,
        &[
            "worktree",
            "add",
            "-b",
            "w3",
            elsewhere.join("w3").to_str().expect("text"),
            "main",
        ],
    );
    let planned = workspaces
        .plan_worktree(&repository, "main", "w4", None)
        .expect("plan");
    assert_eq!(planned.folder(), lair.join("app.git.worktrees").join("w4"));
}

// Every refusal of the table, one by one, with the reads of the repository, the names below
// the scratch folder and the Store as they were; the barrier is that the call returned.
#[test]
fn each_refusal_makes_nothing_and_leaves_the_repository_the_folder_beside_it_and_the_store_as_they_were()
 {
    let world = world();
    git(&world.repo, &["branch", "elsewhere"]);
    git(&world.repo, &["tag", "release"]);
    let planted = plant(&world);
    // A second repository's worktrees must not be taken for this one's.
    std::fs::create_dir_all(world.worktrees()).expect("the folder of worktrees");
    write(&world.worktrees(), "in-the-way", "a file\n");
    let before = world.snapshot();

    let attempt = |from: &str, branch: &str| -> Result<(), WorkspaceError> {
        let planned = world
            .workspaces
            .plan_worktree(&world.repository, from, branch, None)?;
        planned.add().map(|_| ())
    };
    assert_eq!(
        refused(attempt("main", "has space")),
        Refusal::NotABranchName("has space".to_owned())
    );
    assert_eq!(
        refused(attempt("main", "-dash")),
        Refusal::NotABranchName("-dash".to_owned())
    );
    assert_eq!(
        refused(attempt("main", "main")),
        Refusal::BranchExists("main".to_owned())
    );
    assert_eq!(
        refused(attempt("main", "elsewhere")),
        Refusal::BranchExists("elsewhere".to_owned())
    );
    for from in ["nowhere", "release", "main^"] {
        assert_eq!(
            refused(attempt(from, "task")),
            Refusal::UnknownRevision(from.to_owned())
        );
    }
    // Something at the folder: `in-the-way` is a branch's name here.
    assert_eq!(
        refused(attempt("main", "in-the-way")),
        Refusal::FolderExists(world.worktrees().join("in-the-way"))
    );
    assert_eq!(
        refused(world.workspaces.plan_worktree(
            &RepositoryId::new("no-such-repository"),
            "main",
            "x",
            None
        )),
        Refusal::UnknownRepository(RepositoryId::new("no-such-repository"))
    );

    assert_eq!(world.snapshot(), before);
    for id in planted {
        git(&world.repo, &["cat-file", "-e", &id]);
    }
}

// `feature/x` and `feature-x` lie in one folder: the second is refused for it.
#[test]
fn two_branches_that_give_one_folder_meet_the_refusal_of_a_folder_that_is_there() {
    let world = world();
    world.create("main", "feature/x", None);
    let before = world.snapshot();

    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "feature-x", None)
        .expect("plan");
    let outcome = planned.add();

    assert_eq!(
        refused(outcome),
        Refusal::FolderExists(world.worktrees().join("feature-x"))
    );
    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_repository_whose_every_worktree_is_gone_is_unknown() {
    let world = world();
    // Moves the repository's folder away without git's knowing.
    let moved = world.scratch.join("moved");
    std::fs::rename(&world.repo, &moved).expect("move");
    let outcome = world
        .workspaces
        .plan_worktree(&world.repository, "main", "next", None);
    // The repository's only worktree is the folder the Store knows, and nothing lies
    // there now: no worktree of it can be read.
    match outcome {
        Err(WorkspaceError::Refused(Refusal::UnknownRepository(id))) => {
            assert_eq!(id, world.repository)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_worktree_taken_away_before_it_is_opened_leaves_nothing() {
    let world = world();
    let before = world.snapshot();
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "task", None)
        .expect("plan");
    let added = planned.add().expect("add");
    assert!(planned.folder().is_dir());

    planned.take_away(&added).expect("take away");

    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_worktree_that_was_opened_and_is_taken_away_leaves_no_folder_and_no_branch() {
    let world = world();
    let before = world.snapshot();
    let created = world.create("main", "task", None);
    assert!(created.folder.is_dir());

    world.workspaces.remove_created(&created).expect("remove");

    assert!(!created.folder.exists());
    assert!(!world.worktrees().exists());
    assert_eq!(
        git(&world.repo, &["for-each-ref", "refs/heads"])
            .lines()
            .count(),
        1
    );
    let after = world.snapshot();
    assert_eq!(after.reads, before.reads);
    assert_eq!(after.listing, before.listing);
    // The Store keeps the record of the workspace, as for any worktree that is gone: it is
    // not listed, and the repository lists the one worktree it has.
    let listed = world.workspaces.repositories().expect("repositories");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workspaces.len(), 1);
    assert_eq!(
        listed[0].workspaces[0].path,
        std::fs::canonicalize(&world.repo).unwrap()
    );
}

// The base the worktree was made from is gone between the add and the opening: the
// opening fails, and the worktree and its branch go.
#[test]
fn an_opening_that_fails_takes_the_worktree_and_its_branch_away() {
    let world = world();
    git(&world.repo, &["branch", "side"]);
    let planned = world
        .workspaces
        .plan_worktree(&world.repository, "side", "task", None)
        .expect("plan");
    let added = planned.add().expect("add");
    git(&world.repo, &["branch", "-D", "side"]);

    let outcome = world.workspaces.open_added(&planned, added);

    assert_eq!(
        refused(outcome),
        Refusal::UnknownRevision("side".to_owned())
    );
    assert!(!planned.folder().exists());
    assert!(!world.worktrees().exists());
    let refs = git(&world.repo, &["for-each-ref", "refs/heads"]);
    assert!(!refs.contains("task"), "{refs}");
    let listed = git(&world.repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(listed.matches("worktree ").count(), 1, "{listed}");
}

// The user's hook runs: the worktree is made as the user's git makes it.
#[test]
fn the_user_s_hook_runs_in_the_new_worktree() {
    let world = world();
    program(
        &world.repo.join(".git/hooks/post-checkout"),
        &format!(
            "echo \"$PWD\" > '{}'",
            world.scratch.join("hook-ran").display()
        ),
    );

    let created = world.create("main", "task", None);

    let ran = std::fs::read_to_string(world.scratch.join("hook-ran")).expect("the hook ran");
    assert_eq!(Path::new(ran.trim()), created.folder);
}

// A remote branch as the base is kept by its short name, as every branch base is.
#[test]
fn a_branch_of_a_remote_is_a_base_by_its_short_name() {
    let scratch = Scratch::new();
    let clone = support::a_clone(&scratch);
    let workspaces = workspaces(&scratch);
    let repository = workspaces.open_folder(&clone).expect("open").repository;
    let planned = workspaces
        .plan_worktree(&repository, "origin/main", "task", None)
        .expect("plan");
    let added = planned.add().expect("add");

    let created = workspaces.open_added(&planned, added).expect("open");

    assert_eq!(created.base, base("origin/main"));
}

// check_worktrees_folder
fn linked_worktree(world: &World, name: &str) -> PathBuf {
    let folder = world.scratch.join(name);
    git(
        &world.repo,
        &[
            "worktree",
            "add",
            "-b",
            name,
            folder.to_str().expect("text"),
        ],
    );
    std::fs::canonicalize(folder).expect("the folder")
}

#[test]
fn a_folder_beside_the_repository_its_parent_or_one_not_there_may_hold_the_worktrees() {
    let world = world();
    let before = world.snapshot();
    for folder in [
        world.scratch.join("places"),
        world.scratch.path().to_owned(),
        world.lair.join("beside"),
        world.worktrees(),
    ] {
        world
            .workspaces
            .check_worktrees_folder(&world.repository, &folder)
            .unwrap_or_else(|e| panic!("{folder:?}: {e}"));
    }
    assert_eq!(world.snapshot(), before, "a check writes nothing");
}

#[test]
fn a_folder_that_is_not_absolute_or_holds_a_dot_dot_is_refused() {
    let world = world();
    for folder in ["places", "/a/../b", "..", "./x"] {
        let folder = Path::new(folder);
        assert_eq!(
            refused(
                world
                    .workspaces
                    .check_worktrees_folder(&world.repository, folder)
            ),
            Refusal::FolderNotAbsolute(folder.to_owned()),
            "{folder:?}"
        );
    }
}

#[test]
fn a_folder_that_is_or_lies_inside_a_worktree_or_the_git_folder_is_refused() {
    let world = world();
    let linked = linked_worktree(&world, "linked");
    std::os::unix::fs::symlink(&world.repo, world.scratch.join("link")).expect("a link");
    let inside = [
        world.repo.clone(),
        world.repo.join("sub"),
        world.repo.join(".git"),
        world.repo.join(".git/x"),
        linked.clone(),
        linked.join("below/it"),
        world.scratch.join("link/x"),
        world.scratch.join("link"),
    ];
    for folder in inside {
        let outcome = world
            .workspaces
            .check_worktrees_folder(&world.repository, &folder);
        assert_eq!(
            refused(outcome),
            Refusal::FolderInsideRepository(folder.clone()),
            "{folder:?}"
        );
    }
    // `/src/app2` does not lie inside `/src/app`: whole parts.
    let neighbour = world.lair.join("repo2");
    world
        .workspaces
        .check_worktrees_folder(&world.repository, &neighbour)
        .expect("a folder that only starts with the same letters");
}

#[test]
fn a_repository_the_store_does_not_hold_is_unknown_to_the_check() {
    let world = world();
    let unknown = RepositoryId::new("no-such-repository");
    assert_eq!(
        refused(
            world
                .workspaces
                .check_worktrees_folder(&unknown, &world.scratch.join("places"))
        ),
        Refusal::UnknownRepository(unknown)
    );
}

// At creation the folder is asked again, and what is at it.
#[test]
fn a_folder_named_that_is_a_file_or_has_no_parent_either_is_refused_at_creation_and_nothing_is_made()
 {
    let world = world();
    let planted = plant(&world);
    let file = world.scratch.join("a-file");
    write(world.scratch.path(), "a-file", "not a folder\n");
    std::os::unix::fs::symlink("nowhere", world.scratch.join("a-link")).expect("a link");
    let before = world.snapshot();

    let plan = |folder: &Path| {
        world
            .workspaces
            .plan_worktree(&world.repository, "main", "task", Some(folder))
    };
    assert_eq!(refused(plan(&file)), Refusal::NoSuchFolder(file.clone()));
    let link = world.scratch.join("a-link");
    assert_eq!(refused(plan(&link)), Refusal::NoSuchFolder(link.clone()));
    let parent = world.scratch.join("not-there");
    assert_eq!(
        refused(plan(&parent.join("places"))),
        Refusal::NoSuchFolder(parent.clone())
    );
    // A parent that is a file is no place for a folder either.
    assert_eq!(
        refused(plan(&file.join("places"))),
        Refusal::NoSuchFolder(file.clone())
    );

    assert_eq!(world.snapshot(), before);
    for id in planted {
        git(&world.repo, &["cat-file", "-e", &id]);
    }
}

#[test]
fn a_folder_named_under_a_worktree_that_came_to_exist_after_the_setting_is_refused_at_creation() {
    let world = world();
    let planted = plant(&world);
    // The folder is under a folder where no worktree lies yet, so setting it is fine.
    let named = world.scratch.join("around").join("places");
    world
        .workspaces
        .check_worktrees_folder(&world.repository, &named)
        .expect("the folder is fine when it is set");
    // A worktree then comes to lie around it.
    let around = linked_worktree(&world, "around");
    assert_eq!(named, around.join("places"));
    assert!(!named.exists());
    let before = world.snapshot();

    let outcome = world
        .workspaces
        .plan_worktree(&world.repository, "main", "four", Some(&named));

    assert_eq!(refused(outcome), Refusal::FolderInsideRepository(named));
    assert_eq!(world.snapshot(), before);
    for id in planted {
        git(&world.repo, &["cat-file", "-e", &id]);
    }
}

#[test]
fn a_hook_that_fails_in_a_folder_named_that_was_not_there_leaves_no_folder() {
    let world = world();
    let planted = plant(&world);
    program(
        &world.repo.join(".git/hooks/post-checkout"),
        &format!(
            "echo \"$PWD\" > '{}'\nexit 1",
            world.scratch.join("hook-ran").display()
        ),
    );
    let places = world.scratch.join("places");
    let before = world.snapshot();
    let planned: Planned = world
        .workspaces
        .plan_worktree(&world.repository, "main", "task", Some(&places))
        .expect("plan");
    assert_eq!(planned.folder(), places.join("task"));

    let outcome = planned.add();

    assert!(
        matches!(outcome, Err(WorkspaceError::Git(_))),
        "{outcome:?}"
    );
    // Barrier: the hook ran, and in the folder named, so the add made `places` and the
    // worktree in it before the failure took them away.
    let ran = std::fs::read_to_string(world.scratch.join("hook-ran")).expect("the hook ran");
    assert_eq!(Path::new(ran.trim()), planned.folder());
    assert!(!places.exists(), "the folder the add made is gone");
    // The hook's own file is the one thing the scratch holds that it did not.
    let after = world.snapshot();
    assert_eq!(after.reads, before.reads);
    assert_eq!(after.objects, before.objects);
    assert_eq!(after.stored, before.stored);
    let names = |snapshot: &Snapshot| -> Vec<PathBuf> {
        snapshot
            .listing
            .iter()
            .map(|(path, _)| path.clone())
            .filter(|path| path != Path::new("hook-ran"))
            .collect()
    };
    assert_eq!(names(&after), names(&before));
    for id in planted {
        git(&world.repo, &["cat-file", "-e", &id]);
    }
}
