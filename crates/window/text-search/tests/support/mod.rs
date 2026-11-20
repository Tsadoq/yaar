//! What the tests of this crate share: a folder of their own and git, run so that it reads
//! nothing of the user's configuration.

// Each test program uses the helpers it needs, and none uses all of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// The variables with which a process tells git which repository to use, whatever folder
/// it runs in, and those with which a shell hands git settings no file holds.
const GIT_VARIABLES_REMOVED: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG",
];

/// A folder of the test's own, removed when the test ends.
///
/// The text search reads git's global excludes file of the user running the tests, as git
/// does, and a test cannot change the home of its own process safely, so this folder does
/// not touch the environment of the process: git alone is run with a home of its own, and
/// the names the tests search for are none a global excludes file leaves out by habit.
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Makes the folder.
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-text-search-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).expect("make the scratch folder");
        Scratch { root }
    }

    /// The folder holding the repository a test makes: `<scratch>/repo`.
    pub fn repository(&self) -> PathBuf {
        let repository = self.root.join("repo");
        std::fs::create_dir_all(&repository).expect("make the repository folder");
        repository
    }

    /// The folder itself, with every link in its path resolved.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Runs git in `folder`, with no configuration of the user's or the system's, a fixed
    /// author and date, and a home inside the scratch folder. Fails the test when git
    /// fails.
    pub fn git(&self, folder: &Path, args: &[&str]) -> String {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(folder)
            .args(args)
            .stdin(Stdio::null())
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "A Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
            .env("GIT_COMMITTER_NAME", "A Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z");
        for name in GIT_VARIABLES_REMOVED {
            command.env_remove(name);
        }
        let output = command.output().expect("start git");
        assert!(
            output.status.success(),
            "git {args:?} in {folder:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git printed text")
            .trim()
            .to_owned()
    }

    /// Makes a repository in `folder` on the branch `main`, with an identity of its own in
    /// its configuration.
    pub fn init(&self, folder: &Path) {
        self.git(folder, &["init", "--quiet", "--initial-branch=main"]);
        self.git(folder, &["config", "user.name", "A Test"]);
        self.git(folder, &["config", "user.email", "test@example.invalid"]);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A folder of thousands of files, removed on a machine held at background priority,
        // can come back "not empty" from the first pass on macOS; the second finds the rest.
        for _ in 0..3 {
            if std::fs::remove_dir_all(&self.root).is_ok() {
                break;
            }
        }
    }
}

/// Writes `content` to `path` below `folder`, making the folders above it.
pub fn write(folder: &Path, path: &str, content: &[u8]) {
    let file = folder.join(path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("make the folder of the file");
    }
    std::fs::write(&file, content).expect("write the file");
}
