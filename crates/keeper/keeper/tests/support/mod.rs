#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_link::TerminalSize;

pub const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// Which variables a test's process environment is set to; the three differ in what they leave
/// as the machine has it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Git's configuration, `HOME` and `TMPDIR`.
    Git,
    /// As `Git`, and `SHELL` set to `/bin/sh`.
    GitAndShell,
    /// As `GitAndShell`, and the shell's `ENV` and `PS1` removed.
    Full,
}

pub struct Scratch(pub PathBuf);

impl Scratch {
    /// Sets the process environment; has to be the first statement of a test.
    pub fn in_tmp(name: &str, environment: Environment) -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-{name}-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
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
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            if environment != Environment::Git {
                std::env::set_var("SHELL", "/bin/sh");
            }
            std::env::set_var("HOME", root.join("home"));
            // Where the Keeper's copies of git's file lists and a terminal's programs go.
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
            ] {
                std::env::remove_var(name);
            }
            if environment == Environment::Full {
                for name in ["ENV", "PS1"] {
                    std::env::remove_var(name);
                }
            }
        }
        Scratch(root)
    }

    /// For what a test sets on top of its environment; call it before anything starts a thread.
    pub fn set_env(&self, name: &str, value: impl AsRef<OsStr>) {
        // SAFETY: called only while a test sets itself up, before any thread starts.
        unsafe { std::env::set_var(name, value) }
    }

    pub fn unset_env(&self, name: &str) {
        // SAFETY: as in `set_env`.
        unsafe { std::env::remove_var(name) }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Process(pub std::process::Child);

impl Process {
    pub fn end(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.end();
    }
}

pub fn git(folder: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn contains(bytes: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(bytes).contains(text)
}

pub fn is_state(state: yaar_link::SessionState) -> impl Fn(&yaar_link::WorkspaceAgents) -> bool {
    move |agents| agents.sessions.iter().any(|session| session.state == state)
}
