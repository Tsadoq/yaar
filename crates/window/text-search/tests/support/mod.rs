// Each test program uses only some helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

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

/// A folder of the test's own, removed when the test ends. Only git runs with a home in it:
/// the search reads the user's real global excludes file, so the names the tests search for
/// are none such a file commonly leaves out.
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
        let root = temp.join(format!("yaar-text-search-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).expect("make the scratch folder");
        Scratch { root }
    }

    pub fn repository(&self) -> PathBuf {
        let repository = self.root.join("repo");
        std::fs::create_dir_all(&repository).expect("make the repository folder");
        repository
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Runs git in `folder` with no user or system configuration, a fixed author and date, and
    /// a home inside the scratch folder; fails the test when git fails.
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

    pub fn init(&self, folder: &Path) {
        self.git(folder, &["init", "--quiet", "--initial-branch=main"]);
        self.git(folder, &["config", "user.name", "A Test"]);
        self.git(folder, &["config", "user.email", "test@example.invalid"]);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Removing thousands of files on a machine at background priority can return "not empty"
        // from the first pass on macOS; the second finds the rest.
        let mut last = Ok(());
        for _ in 0..3 {
            last = std::fs::remove_dir_all(&self.root);
            if last.is_ok() {
                break;
            }
        }
        if let Err(error) = last {
            eprintln!(
                "the scratch folder {:?} was not removed: {error}",
                self.root
            );
        }
    }
}

pub fn write(folder: &Path, path: &str, content: &[u8]) {
    let file = folder.join(path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("make the folder of the file");
    }
    std::fs::write(&file, content).expect("write the file");
}
