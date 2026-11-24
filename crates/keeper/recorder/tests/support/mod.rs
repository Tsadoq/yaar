//! Helpers shared by the test programs: a scratch folder, an environment in which git reads nothing
//! of the user's, and git calls.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const REPOSITORY_VARIABLES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// Removing `GIT_CONFIG_COUNT` also voids the numbered `GIT_CONFIG_KEY_n` and
/// `GIT_CONFIG_VALUE_n`, so they are not listed.
const CONFIGURATION_VARIABLES: [&str; 3] =
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"];

pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Sets environment variables, so it must be the first statement of a test.
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-recorder-{}-{n}", std::process::id()));
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
        // SAFETY: changing the environment is sound only while no other thread reads it; this is
        // the first statement of a test, and nextest gives each test a process of its own.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            for name in REPOSITORY_VARIABLES
                .into_iter()
                .chain(CONFIGURATION_VARIABLES)
            {
                std::env::remove_var(name);
            }
        }
        Scratch { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

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

pub fn git(folder: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(folder, args))
        .expect("git printed text")
        .trim()
        .to_owned()
}

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

/// Git on Linux guesses no identity, and a commit made here must not depend on this
/// process's variables, so the repository gets an identity of its own.
pub fn init(folder: &Path) {
    std::fs::create_dir_all(folder).expect("make the folder");
    git(folder, &["init", "--quiet", "-b", "main"]);
    git(folder, &["config", "user.name", "A Test"]);
    git(folder, &["config", "user.email", "test@example.invalid"]);
}

pub fn write(folder: &Path, path: &str, content: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().expect("a folder above")).expect("make the folders");
    std::fs::write(file, content).expect("write the file");
}

pub fn commit_all(folder: &Path, message: &str) -> String {
    git(folder, &["add", "--all"]);
    git(
        folder,
        &["commit", "--quiet", "--allow-empty", "-m", message],
    );
    git(folder, &["rev-parse", "HEAD"])
}

pub fn program(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folders");
    std::fs::write(path, format!("#!/bin/sh\n{script}\n")).expect("write the program");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("make it run");
}

/// Changes the time of the last change and leaves the content alone, so git has to read the file.
pub fn touch(folder: &Path, path: &str) {
    let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
    std::fs::File::options()
        .write(true)
        .open(folder.join(path))
        .expect("open the file")
        .set_modified(long_ago)
        .expect("set the time");
}

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

/// Stand-in for git 2.34 that refuses `worktree list -z`, `cat-file -Z` and the `ahead-behind`
/// atom; the rest goes to the real git. Must come right after `Scratch::new`.
pub fn an_older_git(scratch: &Scratch) {
    let search = std::env::var_os("PATH").expect("a search path");
    let real = std::env::split_paths(&search)
        .map(|folder| folder.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git on the search path");
    let folder = scratch.join("older-git");
    program(
        &folder.join("git"),
        &format!(
            "worktree=no\ncat_file=no\nz=no\nbig_z=no\nlast=\n\
             for argument in \"$@\"; do\n  last=$argument\n  case $argument in\n    \
             worktree) worktree=yes ;;\n    cat-file) cat_file=yes ;;\n    -z) z=yes ;;\n    \
             -Z) big_z=yes ;;\n    *'%(ahead-behind'*)\n      \
             echo 'fatal: unknown field name: ahead-behind:HEAD' >&2\n      exit 128 ;;\n  \
             esac\ndone\n\
             if [ \"$last\" = version ]; then\n  echo 'git version 2.34.1'\n  exit 0\nfi\n\
             if [ $worktree = yes ] && [ $z = yes ]; then\n  \
             echo \"error: unknown switch \\`z'\" >&2\n  exit 129\nfi\n\
             if [ $cat_file = yes ] && [ $big_z = yes ]; then\n  \
             echo \"error: unknown switch \\`Z'\" >&2\n  exit 129\nfi\n\
             exec '{}' \"$@\"",
            real.display()
        ),
    );
    let mut folders = vec![folder];
    folders.extend(std::env::split_paths(&search));
    // SAFETY: as in `Scratch::new`.
    unsafe {
        std::env::set_var(
            "PATH",
            std::env::join_paths(folders).expect("a search path"),
        );
    }
}

/// True also for a process that has ended but not been waited for.
pub fn is_alive(pid: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .expect("start the shell")
        .success()
}
