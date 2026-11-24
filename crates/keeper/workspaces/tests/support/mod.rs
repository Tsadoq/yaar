#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const REPOSITORY_VARIABLES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// Shell-given git settings no file holds: a count with numbered keys and values (removing
/// the count voids them), the settings of an outer git, and a configuration file.
const CONFIGURATION_VARIABLES: [&str; 3] =
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"];

pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
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
        // SAFETY: runs first in a test, and `cargo nextest` gives each test its own process:
        // no other thread exists yet to read the environment.
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

pub fn init(folder: &Path) {
    std::fs::create_dir_all(folder).expect("make the folder");
    git(folder, &["init", "--quiet", "-b", "main"]);
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

/// Changes the time of last change and leaves the content alone, so that git has to read the file
/// to learn nothing changed.
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

/// A zombie process counts as alive.
pub fn is_alive(pid: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .expect("start the shell")
        .success()
}

pub fn workspaces(scratch: &Scratch) -> yaar_workspaces::Workspaces {
    workspaces_through(
        scratch,
        yaar_recorder::Git::find().expect("git on the search path"),
    )
}

pub fn workspaces_through(
    scratch: &Scratch,
    git: yaar_recorder::Git,
) -> yaar_workspaces::Workspaces {
    let file = scratch.join("app-data").join(yaar_store::FILE_NAME);
    let store = yaar_store::Store::open(&file).expect("open the database");
    yaar_workspaces::Workspaces::new(git, store)
}

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

pub const DEADLINE: Duration = Duration::from_secs(10);

pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "gave up waiting until {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Clone, Default)]
pub struct Heard {
    files: Arc<AtomicUsize>,
    heads: Arc<AtomicUsize>,
}

impl Heard {
    pub fn told(&self) -> Box<dyn Fn(yaar_workspaces::Seen) + Send + Sync> {
        let heard = self.clone();
        Box::new(move |seen| {
            let count = match seen {
                yaar_workspaces::Seen::Files => &heard.files,
                yaar_workspaces::Seen::Head => &heard.heads,
            };
            count.fetch_add(1, Ordering::SeqCst);
        })
    }

    pub fn files(&self) -> usize {
        self.files.load(Ordering::SeqCst)
    }

    pub fn heads(&self) -> usize {
        self.heads.load(Ordering::SeqCst)
    }

    pub fn holders(&self) -> usize {
        Arc::strong_count(&self.files)
    }
}

pub fn ago(seconds: u64) -> Instant {
    Instant::now()
        .checked_sub(Duration::from_secs(seconds))
        .expect("the machine has run for longer than the test")
}
