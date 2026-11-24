//! Shows that the check at the end of a test with the stand-in agent finds a file of the app.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use yaar_system_tests::standin::Script;
use yaar_system_tests::{DEADLINE, Repo, World, is_alive, wait_until};

const REVIEW_FILE: &str = "0123456789abcdef0123456789abcdef-1.md";

const CHILD: &str = "YAAR_APP_FILES_CHILD";

fn world_with_standins() -> (World, Repo) {
    let mut world = World::new();
    world.agents_on_path();
    let repo = world.repository("app");
    (world, repo)
}

fn listed(world: &World) -> Vec<(PathBuf, String)> {
    let mut listed = world.app_files_listed();
    listed.sort();
    listed
}

fn untracked(worktree: &Path, files: &[&str]) -> Vec<(PathBuf, String)> {
    let at = |line: String| (worktree.to_owned(), line);
    files
        .iter()
        .flat_map(|file| [at(format!("? {file}")), at((*file).to_owned())])
        .collect()
}

fn sorted(mut listed: Vec<(PathBuf, String)>) -> Vec<(PathBuf, String)> {
    listed.sort();
    listed
}

#[test]
fn a_file_of_the_app_left_in_a_worktree_is_named_by_the_check_at_the_end_of_the_test() {
    let (world, app) = world_with_standins();
    let other = world.repository("other");
    let linked = world.home().join("linked");
    app.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    app.write(".gitignore", "yaar-ignored.txt\n");
    app.commit("Ignore a file");

    for decoy in [
        "notes-1.md",
        "0123456789abcdef0123456789abcde-1.md",
        "0123456789abcdef0123456789abcdef-x.md",
        "sub/the-yaar-way.txt",
        "ade-ids.txt",
    ] {
        app.write(decoy, "not the app's\n");
    }
    assert_eq!(listed(&world), [], "nothing of the app's, nothing named");

    for (file, text) in [
        (".mcp.json", "{}"),
        ("yaar-note.txt", "a note"),
        (REVIEW_FILE, "a review"),
        ("sub/ade-id", "an id"),
        ("yaar-ignored.txt", "ignored by the repository"),
    ] {
        app.write(file, text);
    }
    other.write(".claude-plugin/plugin.json", "{}");
    fs::create_dir_all(linked.join("src")).expect("make a folder of the linked worktree");
    fs::write(linked.join("src/yaar.ts"), "export {}").expect("write in the linked worktree");

    let mut wanted = untracked(
        app.path(),
        &[".mcp.json", "yaar-note.txt", REVIEW_FILE, "sub/ade-id"],
    );
    // An ignored file is listed by `git status --ignored` and the third command, not the second.
    wanted.push((app.path().to_owned(), "! yaar-ignored.txt".to_owned()));
    wanted.push((app.path().to_owned(), "yaar-ignored.txt".to_owned()));
    wanted.extend(untracked(other.path(), &[".claude-plugin/plugin.json"]));
    wanted.extend(untracked(&linked, &["src/yaar.ts"]));
    assert_eq!(
        listed(&world),
        sorted(wanted),
        "each file, by each command that lists it, in the repository or worktree it lies in"
    );

    fs::remove_dir_all(&linked).expect("remove the linked worktree");
    assert!(
        listed(&world).iter().all(|(folder, _)| folder != &linked),
        "a worktree that is gone is not listed"
    );

    for file in [
        ".mcp.json",
        "yaar-note.txt",
        REVIEW_FILE,
        "sub/ade-id",
        "yaar-ignored.txt",
    ] {
        fs::remove_file(app.path().join(file)).expect("remove a file of the app");
    }
    fs::remove_dir_all(other.path().join(".claude-plugin")).expect("remove a folder of the app");
    assert_eq!(listed(&world), [], "removed, nothing is named");
}

#[test]
fn a_file_of_the_app_that_was_committed_is_named_by_the_listing_of_tracked_files() {
    let (world, app) = world_with_standins();
    app.write(".mcp.json", "{}");
    app.write("docs/Yaar-Notes.md", "a name, case aside");
    app.commit("Commit files of the app");
    assert_eq!(
        listed(&world),
        [
            (app.path().to_owned(), ".mcp.json".to_owned()),
            (app.path().to_owned(), "docs/Yaar-Notes.md".to_owned()),
        ],
        "tracked and clean: only the listing of tracked files names them"
    );
    app.git(&["rm", "--quiet", ".mcp.json", "docs/Yaar-Notes.md"]);
    app.commit("Take them away again");
    assert_eq!(listed(&world), [], "removed, nothing is named");
}

#[test]
fn a_path_git_quotes_is_named_all_the_same() {
    let (world, app) = world_with_standins();
    // Git quotes a name with a non-ASCII letter, in octal: the opening quote sits on the path's
    // first part and the closing one after its last.
    app.write("\u{e9}/.mcp.json", "{}");
    app.write("yaar-\u{e9}.txt", "x");
    let listed = listed(&world);
    assert_eq!(listed.len(), 4, "two files, two commands each: {listed:?}");
    fs::remove_file(app.path().join("yaar-\u{e9}.txt")).expect("remove a file of the app");
    fs::remove_dir_all(app.path().join("\u{e9}")).expect("remove a folder of the app");
    assert!(
        listed.iter().any(|(_, line)| line.contains(".mcp.json")),
        "{listed:?}"
    );
    assert!(
        listed.iter().any(|(_, line)| line.contains("yaar-")),
        "{listed:?}"
    );
}

/// A repository whose tracked file looks changed to git by its time alone, so `git status`
/// rewrites the index unless told not to take the lock.
fn repository_with_a_stale_index(world: &World) -> Repo {
    let app = world.repository("app");
    app.write("tracked.txt", "same bytes\n");
    app.commit("Track a file");
    fs::File::options()
        .write(true)
        .open(app.path().join("tracked.txt"))
        .and_then(|file| file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000)))
        .expect("give the tracked file another time");
    app
}

#[test]
fn the_check_leaves_the_index_of_the_repository_it_reads_as_it_was() {
    let mut world = World::new();
    world.agents_on_path();
    let app = repository_with_a_stale_index(&world);
    let index = app.path().join(".git/index");
    let before = fs::read(&index).expect("read the index");
    assert_eq!(listed(&world), [], "nothing of the app's");
    assert!(
        fs::read(&index).expect("read the index again") == before,
        "a check that only reads writes no index"
    );
}

#[test]
fn a_git_command_of_a_test_still_refreshes_the_index_as_git_does_by_itself() {
    let world = World::new();
    let app = repository_with_a_stale_index(&world);
    let index = app.path().join(".git/index");
    let before = fs::read(&index).expect("read the index");
    app.git(&["status", "--porcelain"]);
    assert!(
        fs::read(&index).expect("read the index again") != before,
        "the no-lock rule belongs to the check: a test's own git runs as git does"
    );
}

#[test]
fn a_world_without_the_stand_in_is_not_checked_when_it_is_dropped() {
    let world = World::new();
    let app = world.repository("app");
    app.write(".mcp.json", "{}");
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(world)));
    assert!(dropped.is_ok(), "a world that put no stand-in on its path");
}

struct Ended {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Output goes to files, so a child that writes a lot cannot fill a pipe nobody reads.
fn run_as_child(name: &str) -> Ended {
    let scratch = World::new();
    let (out, err) = (
        scratch.home().join("child.out"),
        scratch.home().join("child.err"),
    );
    let mut child = Command::new(std::env::current_exe().expect("the test's own path"))
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out).expect("a file for the child's output"))
        .stderr(fs::File::create(&err).expect("a file for the child's errors"))
        .spawn()
        .expect("start the test as a child");
    let mut status = None;
    let ended = wait_until(DEADLINE * 3 + Duration::from_secs(30), || {
        status = child.try_wait().expect("look at the child");
        status.is_some()
    });
    if !ended {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the child test did not end");
    }
    Ended {
        code: status.and_then(|status| status.code()),
        stdout: fs::read_to_string(out).expect("the child's output"),
        stderr: fs::read_to_string(err).expect("the child's errors"),
    }
}

fn world_with_keeper() -> (World, Repo) {
    World::with_standin("default", &Script::parse(""))
}

fn tell_where(world: &mut World, app: &Repo) {
    let root = app
        .path()
        .parent()
        .and_then(Path::parent)
        .expect("the world");
    let keeper = world.keeper_pid().expect("the Keeper runs");
    println!("world: {}\nkeeper: {keeper}", root.display());
}

fn world_of(child: &Ended) -> PathBuf {
    // The libtest harness has written the name of the test on the line before it.
    let (_, told) = child
        .stdout
        .split_once("world: ")
        .unwrap_or_else(|| panic!("the child told no world: {}", child.stdout));
    PathBuf::from(told.lines().next().unwrap_or_default())
}

fn keeper_of(child: &Ended) -> u32 {
    let (_, told) = child
        .stdout
        .split_once("keeper: ")
        .unwrap_or_else(|| panic!("the child told no Keeper: {}", child.stdout));
    told.lines()
        .next()
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("the child's Keeper is no process id: {}", child.stdout))
}

const LEAVES_A_FILE: &str = "child_a_test_ends_with_a_file_of_the_app_left_in_its_worktree";
const FAILS_WITH_A_FILE: &str = "child_a_test_that_fails_ends_with_a_file_of_the_app_left_too";

#[test]
fn child_a_test_ends_with_a_file_of_the_app_left_in_its_worktree() {
    if std::env::var(CHILD).as_deref() != Ok(LEAVES_A_FILE) {
        return;
    }
    let (mut world, app) = world_with_keeper();
    tell_where(&mut world, &app);
    app.write("yaar-note.txt", "left behind");
    drop(world);
    println!("the drop of the world returned");
}

#[test]
fn child_a_test_that_fails_ends_with_a_file_of_the_app_left_too() {
    if std::env::var(CHILD).as_deref() != Ok(FAILS_WITH_A_FILE) {
        return;
    }
    let (mut world, app) = world_with_keeper();
    tell_where(&mut world, &app);
    app.write("yaar-note.txt", "left behind");
    let _held = world;
    panic!("the test's own failure");
}

#[test]
fn a_test_that_ends_with_a_file_of_the_app_left_fails_naming_it_and_its_world_is_still_removed() {
    let child = run_as_child(LEAVES_A_FILE);
    assert_eq!(child.code, Some(101), "the test fails: {}", child.stderr);
    assert!(
        child.stderr.contains("panicked"),
        "a panic of the drop: {}",
        child.stderr
    );
    assert!(
        child.stderr.contains("yaar-note.txt"),
        "it names the file: {}",
        child.stderr
    );
    assert!(
        !child.stdout.contains("the drop of the world returned"),
        "the drop panics: {}",
        child.stdout
    );
    assert!(
        !child.stderr.contains("panic in a destructor") && !child.stderr.contains("abort"),
        "an ordinary failure, not an abort: {}",
        child.stderr
    );
    assert!(
        !world_of(&child).exists(),
        "the world's folder is removed before the panic"
    );
    assert!(
        !is_alive(keeper_of(&child)),
        "the Keeper ends before the panic"
    );
}

#[test]
fn a_test_that_is_failing_already_does_not_have_the_check_panic_a_second_time() {
    let child = run_as_child(FAILS_WITH_A_FILE);
    assert_eq!(
        child.code,
        Some(101),
        "an ordinary failure, not an abort: {}",
        child.stderr
    );
    assert!(
        child.stderr.contains("the test's own failure"),
        "{}",
        child.stderr
    );
    assert!(
        !child.stderr.contains("yaar-note.txt"),
        "the check does not speak over the test's own failure: {}",
        child.stderr
    );
    assert!(!world_of(&child).exists(), "the world's folder is removed");
    assert!(!is_alive(keeper_of(&child)), "the Keeper is gone");
}
