#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use std::sync::atomic::{AtomicU32, Ordering};
use yaar_recorder::{Git, Worktree};
use yaar_review_engine::{
    CommentId, DraftOn, DraftSave, NewAnchor, Reviews, SessionId, SessionRecord, Side, ThreadId,
};
use yaar_store::{Store, WorkspaceId};

const REPOSITORY_VARIABLES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// Removing GIT_CONFIG_COUNT also voids every GIT_CONFIG_KEY_<n> and GIT_CONFIG_VALUE_<n>.
const CONFIGURATION_VARIABLES: [&str; 3] =
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"];

pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Must be the first statement of a test: cleans the process environment for git.
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-review-engine-{}-{n}", std::process::id()));
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
    // Git on Linux guesses no identity, so the repository holds its own.
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

pub fn files_of(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, folder: &Path, found: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, found);
            } else {
                let relative = path.strip_prefix(root).expect("below the root").to_owned();
                found.insert(relative, std::fs::read(&path).expect("read the file"));
            }
        }
    }
    let mut found = std::collections::BTreeMap::new();
    walk(root, root, &mut found);
    found
}

pub fn id(n: u128) -> String {
    format!("{n:032x}")
}

pub fn workspace() -> WorkspaceId {
    WorkspaceId::new(id(0xaa))
}

pub fn store(scratch: &Scratch) -> Store {
    Store::open(&scratch.join("app/store.db")).expect("open the store")
}

pub fn engine(scratch: &Scratch, store: Store) -> Reviews {
    Reviews::new(
        store,
        Git::find().expect("git on the search path"),
        scratch.join("app/states"),
    )
}

pub fn open(folder: &Path) -> Worktree {
    Worktree::open(&Git::find().expect("git on the search path"), folder).expect("open")
}

pub fn new_thread(n: u128, path: &str, line: u32, code: &str, text: &str) -> DraftSave {
    DraftSave {
        thread: ThreadId::new(id(n)),
        comment: CommentId::new(id(n + 1000)),
        on: DraftOn::NewThread(NewAnchor {
            tree: None,
            path: PathBuf::from(path),
            side: Side::New,
            first: line,
            last: line,
            text: code.to_string(),
        }),
        text: text.to_string(),
    }
}

pub fn session(store: &Store, n: u128) -> SessionRecord {
    session_in(store, n, workspace())
}

pub fn session_in(store: &Store, n: u128, workspace: WorkspaceId) -> SessionRecord {
    let record = SessionRecord {
        id: SessionId::new(id(n)),
        workspace,
        agent: "claude".to_owned(),
        cli_session: format!("cli-{n}"),
        terminal: Some(format!("t{n}")),
        number: 1,
        running: true,
        ended: None,
        ended_by: None,
    };
    store.put_session(&record).expect("keep the session");
    record
}
