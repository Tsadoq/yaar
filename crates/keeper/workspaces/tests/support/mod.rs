//! What the tests of this crate share: a folder of their own, an environment in which git
//! reads nothing of the user's, and git itself as the test runs it to make a repository
//! and to ask what git says.

// Each test program uses the helpers it needs, and none uses all of them.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// The variables with which a process tells git which repository to use, whatever folder
/// it runs in.
const REPOSITORY_VARIABLES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// A folder of the test's own, removed when the test ends.
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Makes the folder and cleans the environment of this process for git: no
    /// configuration of the user or of the system, a fixed author and date, and an empty
    /// home. It has to be the first statement of a test.
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-workspaces-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).expect("make the scratch folder");

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
        // test, and the tests of this crate are run by `cargo nextest`, which gives each
        // test a process of its own: no other thread exists yet.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            for name in REPOSITORY_VARIABLES {
                std::env::remove_var(name);
            }
        }
        Scratch { root }
    }

    /// The folder itself, with every link in its path resolved.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// A path inside the folder.
    pub fn join(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Runs git in `folder` and gives what it printed, as bytes. Fails the test when git
/// fails.
pub fn git_bytes(folder: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} in {folder:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Runs git in `folder` and gives what it printed, without the white space around it.
pub fn git(folder: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(folder, args))
        .expect("git printed text")
        .trim()
        .to_owned()
}

/// Runs git in `folder` with `input` on its standard input, and gives what it printed.
pub fn git_with_input(folder: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start git");
    child
        .stdin
        .take()
        .expect("a pipe to git")
        .write_all(input)
        .expect("write to git");
    let output = child.wait_with_output().expect("wait for git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("git printed text")
        .trim()
        .to_owned()
}

/// Makes a repository at `folder` whose first branch is `main`.
pub fn init(folder: &Path) {
    std::fs::create_dir_all(folder).expect("make the folder");
    git(folder, &["init", "--quiet", "-b", "main"]);
}

/// Writes a file of the worktree, making the folders above it.
pub fn write(folder: &Path, path: &str, content: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().expect("a folder above")).expect("make the folders");
    std::fs::write(file, content).expect("write the file");
}

/// Commits everything in the worktree and gives the id of the commit.
pub fn commit_all(folder: &Path, message: &str) -> String {
    git(folder, &["add", "--all"]);
    git(
        folder,
        &["commit", "--quiet", "--allow-empty", "-m", message],
    );
    git(folder, &["rev-parse", "HEAD"])
}

/// Writes a program of the shell at `path`.
pub fn program(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folders");
    std::fs::write(path, format!("#!/bin/sh\n{script}\n")).expect("write the program");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("make it run");
}

/// Gives a file of the worktree another time of its last change and leaves its content
/// alone, so that git has to read the file to learn that nothing changed.
pub fn touch(folder: &Path, path: &str) {
    let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
    std::fs::File::options()
        .write(true)
        .open(folder.join(path))
        .expect("open the file")
        .set_modified(long_ago)
        .expect("set the time");
}

/// A git that writes down every start of it, one line each: its process id, then its
/// arguments. Returns the program and the file it writes to.
pub fn git_that_keeps_a_list(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let list = scratch.join("starts-of-git");
    let wrapper = scratch.join("bin/git-listed");
    program(
        &wrapper,
        &format!(
            "printf '%s\\n' \"$$ $*\" >> '{}'\nexec git \"$@\"",
            list.display()
        ),
    );
    (wrapper, list)
}

/// Whether a process with this id is there, running or waiting to be waited for.
pub fn is_alive(pid: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .expect("start the shell")
        .success()
}

/// The Workspaces of a Keeper whose database lies in the scratch folder. Dropping it
/// and making another is a Keeper that restarts: nothing but the database file and the
/// repositories is carried over.
pub fn workspaces(scratch: &Scratch) -> yaar_workspaces::Workspaces {
    workspaces_through(
        scratch,
        yaar_recorder::Git::find().expect("git on the search path"),
    )
}

/// The same, reading through the git program given.
pub fn workspaces_through(
    scratch: &Scratch,
    git: yaar_recorder::Git,
) -> yaar_workspaces::Workspaces {
    let file = scratch.join("app-data").join(yaar_store::FILE_NAME);
    let store = yaar_store::Store::open(&file).expect("open the database");
    yaar_workspaces::Workspaces::new(git, store)
}

/// A repository with two commits on `main`, a bare copy of it to serve as the remote,
/// and a clone of that in the folder `clone`, which the function returns. The clone is
/// on `main`, which follows `origin/main`, and the remote's default branch is recorded
/// as `main` whatever the git that made the clone does by itself.
pub fn a_clone(scratch: &Scratch) -> PathBuf {
    let source = scratch.join("source");
    init(&source);
    write(&source, "a.txt", "one\n");
    commit_all(&source, "first");
    write(&source, "a.txt", "two\n");
    commit_all(&source, "second");
    git(
        scratch.path(),
        &["clone", "--quiet", "--bare", "source", "remote.git"],
    );
    let remote = scratch.join("remote.git");
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            remote.to_str().expect("a path of text"),
            "clone",
        ],
    );
    let clone = scratch.join("clone");
    git(&clone, &["remote", "set-head", "origin", "main"]);
    clone
}
