//! What the component tests of the Navigator share: a scratch folder with a home of its
//! own, the fixed projects made into repositories, the folder of real servers, the stand-in
//! language server, a Navigator started on a port the test plays the Frame's end of, and a
//! listener on the loopback address that serves downloads.
//!
//! A test plays the Frame's model: it sends [`Ask`]s and reads [`Told`]s, and when the
//! Navigator wants an old side it plays the Keeper's answer too
//! ([`Running::export_when_wanted`]), writing the version with git as plain files, as the
//! tests of the panes answer the Window's requests with data. It never reaches into the
//! Navigator.
//!
//! Every server a test starts runs with [`Scratch::environment`]: a home, cache, config,
//! data and temporary folder of the scratch's own, a cargo home of its own, a search path
//! whose first folder holds a recorder named `npm` ([`Scratch::npm_ran`]), and the proxy
//! variables naming a listener of the scratch's own ([`Scratch::proxy_heard`]). Nothing a
//! server writes reaches the user's folders, and a server that honours the proxy
//! variables reaches nothing beyond this machine. A test that ends its Navigator with
//! [`Running::end`] has every server's process proved gone.
//!
//! [`Scratch::new`] sets variables of this process, as the tests of the Keeper's crates do:
//! it is the first statement of a test, and the tests are run by `cargo nextest`, a process
//! each.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{
    Answered, Ask, AskId, BUILTIN, Location, Navigator, NavigatorConfig, Place, Platform, Port,
    Registry, STOP_AFTER, STOP_DEADLINE, ServerState, SystemClock, Told, Tree, USER_FILE,
    WorkspaceRoot, download,
};

/// How long a test waits for something of the Navigator or the stand-in that has to
/// happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a test waits for a real language server to be ready or to answer: rust-analyzer
/// on a loaded 3-core machine running at background priority (`taskpolicy -b`) takes many
/// seconds to load even a small project, and its first answer waits for that. Reaching it
/// fails the test.
pub const SERVER_DEADLINE: Duration = Duration::from_secs(120);

/// The variable that names the folder of the real servers, filled once on a machine by
/// `cargo xtask language-servers <folder>`.
pub const REAL_SERVERS: &str = "YAAR_LANGUAGE_SERVERS";

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

/// The variables with which a shell hands git settings that no file holds: a count of
/// settings with their numbered keys and values (removing the count voids them), the
/// settings of an outer git, and one file of configuration.
const CONFIGURATION_VARIABLES: [&str; 3] =
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"];

/// What git is told in place of the user's configuration: none of theirs or the system's, a
/// fixed author and committer, and a fixed date.
const GIT_SETTINGS: [(&str, &str); 8] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "A Test"),
    ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
    ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
    ("GIT_COMMITTER_NAME", "A Test"),
    ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
    ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
];

/// One of the fixed projects of `tests/fixtures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// `typescript`, with JavaScript files.
    TypeScript,
    /// `python`.
    Python,
    /// `rust`.
    Rust,
}

/// A repository a test made: its worktree, and the commit of its base.
pub struct Repo {
    /// The folder of the worktree, its links resolved.
    pub root: PathBuf,
    /// The full id of the base commit, the old side of the change; for a repository made by
    /// [`Scratch::repository`], its one commit.
    pub base: String,
    /// The scratch's `home`, which git is told is its home.
    home: PathBuf,
}

impl Repo {
    /// Runs git in the worktree with these arguments, as the tests of the Recorder run it:
    /// `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`, a fixed author, committer and
    /// date, `HOME` in the scratch folder, and every variable that names a repository
    /// removed. Gives what it printed, with no line end; panics with what git wrote when
    /// git fails.
    pub fn git(&self, args: &[&str]) -> String {
        String::from_utf8(git_bytes(&self.home, &self.root, args))
            .expect("git printed text")
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    }

    /// `git status --porcelain=v2 --ignored --untracked-files=all`.
    pub fn status(&self) -> String {
        self.git(&[
            "status",
            "--porcelain=v2",
            "--ignored",
            "--untracked-files=all",
        ])
    }

    /// Every file under `relative` in the worktree, with its size, by path (absolute, as
    /// the worktree is): what a build folder holds. A link counts as a file of the size of
    /// the link; a folder that is not there holds none.
    pub fn files_under(&self, relative: &str) -> Vec<(PathBuf, u64)> {
        let mut found = Vec::new();
        collect_files(&self.root.join(relative), &mut found);
        found.sort();
        found
    }

    /// The text of the line `line`, from 1, of the file `relative` in the worktree.
    pub fn line(&self, relative: &str, line: u32) -> String {
        let file = self.root.join(relative);
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        line_of(&text, line, &file.display().to_string())
    }

    /// Adds every file and commits them as the repository's base.
    fn commit(&mut self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.base = self.git(&["rev-parse", "HEAD"]);
    }
}

/// Runs git in `folder` with the environment of a test, and gives what it printed.
fn git_bytes(home: &Path, folder: &Path, args: &[&str]) -> Vec<u8> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null());
    for (name, value) in GIT_SETTINGS {
        command.env(name, value);
    }
    command.env("HOME", home);
    for name in REPOSITORY_VARIABLES
        .into_iter()
        .chain(CONFIGURATION_VARIABLES)
    {
        command.env_remove(name);
    }
    let output = command.output().expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} in {} failed: {}",
        folder.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Every file under `folder` with its size, links not followed.
fn collect_files(folder: &Path, into: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_files(&path, into);
        } else {
            into.push((path, meta.len()));
        }
    }
}

/// The line `line`, from 1, of `text`, with no line end; `what` names the text when the line
/// is not there.
fn line_of(text: &str, line: u32, what: &str) -> String {
    text.lines()
        .nth(line.checked_sub(1).expect("lines count from 1") as usize)
        .unwrap_or_else(|| panic!("{what} has no line {line}"))
        .to_owned()
}

/// Copies the files under `from` into `into`, making the folders; at the top, a file named
/// `dot-gitignore` is written as `.gitignore` (git does not keep a `.gitignore` of a
/// fixture as a fixture's file).
fn copy_tree(from: &Path, into: &Path, top: bool) {
    std::fs::create_dir_all(into).expect("make the folder");
    for entry in std::fs::read_dir(from).expect("read the fixture") {
        let entry = entry.expect("read the fixture");
        let name = entry.file_name();
        let target = if top && name == "dot-gitignore" {
            into.join(".gitignore")
        } else {
            into.join(&name)
        };
        if entry.file_type().expect("the fixture's file type").is_dir() {
            copy_tree(&entry.path(), &target, false);
        } else {
            std::fs::copy(entry.path(), target).expect("copy the fixture's file");
        }
    }
}

/// One test's scratch folder, under the system's temporary folder, its links resolved:
/// `home`, `cache`, `config`, `data`, `tmp`, `cargo`, `bin` (the recorder `npm`), `repos`,
/// `servers` (the app's downloads of this test) and `navigator` (its cache); and a
/// [`Loopback`] serving nothing, which the servers' proxy variables name. Removed when
/// dropped.
pub struct Scratch {
    /// The folder.
    pub root: PathBuf,
    proxy: Loopback,
}

impl Scratch {
    /// A new scratch folder, with its folders made and the recorder `npm` in `bin`: a
    /// script that appends its arguments as one line to `<root>/npm.log` and ends with 1.
    /// The variables of this process that tell git what to read are set for git as
    /// [`Repo::git`] sets them, and `HOME` is the scratch's `home`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-navigator-{}-{n}", std::process::id()));
        // A folder of this name is what an earlier process of the same number left.
        let _ = std::fs::remove_dir_all(&root);
        for folder in [
            "home",
            "cache",
            "config",
            "data",
            "tmp",
            "cargo",
            "bin",
            "repos",
            "servers",
            "navigator",
        ] {
            std::fs::create_dir_all(root.join(folder)).expect("make the scratch folder");
        }

        // SAFETY: changing the environment is sound only while no other thread reads it,
        // as a thread that starts a process does. This runs as the first statement of a
        // test, and the tests of this crate are run by `cargo nextest`, which gives each
        // test a process of its own: no other thread exists yet.
        unsafe {
            for (name, value) in GIT_SETTINGS {
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

        write_script(
            &root.join("bin").join("npm"),
            &format!(
                "printf '%s\\n' \"$*\" >> {}\nexit 1\n",
                quoted(&root.join("npm.log"))
            ),
        );
        Scratch {
            root,
            proxy: Loopback::serve(Vec::new()),
        }
    }

    /// The whole environment of the servers of this test: `HOME`, `XDG_CACHE_HOME`,
    /// `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `TMPDIR` and `CARGO_HOME` in the scratch folder;
    /// `PATH` the scratch's `bin`, then the folder that holds the `cargo` the test was built
    /// by (from `CARGO`, which Cargo sets for a test), then `/usr/bin` and `/bin`;
    /// `RUSTUP_HOME` and `RUSTUP_TOOLCHAIN` as this process has them, where set, so that a
    /// rustup proxy finds the toolchain the repository pins and installs none
    /// (`RUSTUP_AUTO_INSTALL=0`); `CARGO_NET_OFFLINE=true`; `LANG=C.UTF-8`; `HTTP_PROXY`,
    /// `HTTPS_PROXY`, `ALL_PROXY`, `http_proxy`, `https_proxy` and `all_proxy` the address
    /// `http://127.0.0.1:<port>` of the scratch's own [`Loopback`], and no `NO_PROXY`.
    /// Nothing else of this process's environment.
    pub fn environment(&self) -> Vec<(OsString, OsString)> {
        let folder = |name: &str| self.root.join(name).into_os_string();
        let mut search = vec![self.root.join("bin")];
        let cargo = std::env::var_os("CARGO").or_else(|| option_env!("CARGO").map(OsString::from));
        if let Some(cargo_folder) = cargo.as_deref().and_then(|cargo| Path::new(cargo).parent()) {
            search.push(cargo_folder.to_path_buf());
        }
        search.push(PathBuf::from("/usr/bin"));
        search.push(PathBuf::from("/bin"));

        let mut environment: Vec<(OsString, OsString)> = vec![
            ("HOME".into(), folder("home")),
            ("XDG_CACHE_HOME".into(), folder("cache")),
            ("XDG_CONFIG_HOME".into(), folder("config")),
            ("XDG_DATA_HOME".into(), folder("data")),
            ("TMPDIR".into(), folder("tmp")),
            ("CARGO_HOME".into(), folder("cargo")),
            (
                "PATH".into(),
                std::env::join_paths(search).expect("a search path"),
            ),
        ];
        for name in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
            if let Some(value) = std::env::var_os(name) {
                environment.push((name.into(), value));
            }
        }
        environment.push(("RUSTUP_AUTO_INSTALL".into(), "0".into()));
        environment.push(("CARGO_NET_OFFLINE".into(), "true".into()));
        environment.push(("LANG".into(), "C.UTF-8".into()));
        let proxy = format!("http://127.0.0.1:{}", self.proxy.port);
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            environment.push((name.into(), proxy.clone().into()));
        }
        environment
    }

    /// The fixed project `which` made into a repository under `repos`, as
    /// `tests/fixtures/README.md` says, with an identity of its own in its `.git/config`.
    pub fn fixture(&self, which: Fixture) -> Repo {
        let name = match which {
            Fixture::TypeScript => "typescript",
            Fixture::Python => "python",
            Fixture::Rust => "rust",
        };
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        let mut repo = self.init(name);
        copy_tree(&source.join("base"), &repo.root, true);
        repo.commit("base");

        if source.join("ignored").is_dir() {
            copy_tree(&source.join("ignored"), &repo.root, false);
        }
        let gone = std::fs::read_to_string(source.join("gone")).expect("the fixture's gone list");
        for path in gone.lines().filter(|path| !path.trim().is_empty()) {
            std::fs::remove_file(repo.root.join(path.trim()))
                .unwrap_or_else(|error| panic!("remove {path}: {error}"));
        }
        copy_tree(&source.join("change"), &repo.root, false);
        repo
    }

    /// A repository `name` under `repos` holding `files` (path, text), committed once,
    /// with an identity of its own in its `.git/config`.
    pub fn repository(&self, name: &str, files: &[(&str, &str)]) -> Repo {
        let mut repo = self.init(name);
        for (path, text) in files {
            let file = repo.root.join(path);
            std::fs::create_dir_all(file.parent().expect("a folder above"))
                .expect("make the folders");
            std::fs::write(file, text).expect("write the file");
        }
        repo.commit("first");
        repo
    }

    /// A new, empty repository `name` under `repos`, whose branch is `main`.
    fn init(&self, name: &str) -> Repo {
        let root = self.root.join("repos").join(name);
        std::fs::create_dir(&root)
            .unwrap_or_else(|error| panic!("make the repository {name}: {error}"));
        let repo = Repo {
            root,
            base: String::new(),
            home: self.root.join("home"),
        };
        repo.git(&["init", "--quiet", "-b", "main"]);
        // Git on Linux guesses no identity, so the repository holds its own.
        repo.git(&["config", "user.name", "A Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo
    }

    /// A configuration of the Navigator for this test: the built-in table `builtin`, the
    /// user's file `<root>/data/language-servers.toml` (there only when the test writes it),
    /// the downloads in `servers`, the cache in `navigator`, the search path the scratch's
    /// `bin` alone, [`Scratch::environment`], `Platform::current()`,
    /// `yaar_navigator::STOP_AFTER` and the system's clock. A test changes a field where it
    /// needs another.
    pub fn config(&self, builtin: &str) -> NavigatorConfig {
        NavigatorConfig {
            builtin: builtin.to_owned(),
            user_file: self.root.join("data").join(USER_FILE),
            servers: self.root.join("servers"),
            cache: self.root.join("navigator"),
            search_path: vec![self.root.join("bin")],
            environment: self.environment(),
            platform: Platform::current(),
            stop_after: STOP_AFTER,
            clock: Arc::new(SystemClock),
        }
    }

    /// The lines the recorder `npm` wrote, one a run: what a server ran of the network's
    /// package manager.
    pub fn npm_ran(&self) -> Vec<String> {
        lines_of(&self.root.join("npm.log"))
    }

    /// Every request the scratch's own [`Loopback`], which the servers' proxy variables
    /// name, heard: a connection a server opened through a proxy variable.
    pub fn proxy_heard(&self) -> Vec<Heard> {
        self.proxy.heard()
    }

    /// Every file under the scratch's `home`, `cache`, `config`, `data` and `tmp`, and
    /// under `cargo` but the files cargo itself keeps there (`.package-cache`,
    /// `.package-cache-mutate`, `.global-cache`, `registry/CACHEDIR.TAG`): what a server
    /// left outside the app's folders.
    pub fn left_outside(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for folder in ["home", "cache", "config", "data", "tmp", "cargo"] {
            let mut files = Vec::new();
            collect_files(&self.root.join(folder), &mut files);
            found.extend(files.into_iter().map(|(path, _)| path));
        }
        let cargo = self.root.join("cargo");
        let kept = [
            cargo.join(".package-cache"),
            cargo.join(".package-cache-mutate"),
            cargo.join(".global-cache"),
            cargo.join("registry").join("CACHEDIR.TAG"),
        ];
        found.retain(|path| !kept.contains(path));
        found.sort();
        found
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `path` in single quotes, for a `/bin/sh` script.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Writes a `/bin/sh` script, made executable.
fn write_script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folder");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script executable");
}

/// The lines of a file; none when it is not there.
fn lines_of(file: &Path) -> Vec<String> {
    match std::fs::read_to_string(file) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("read {}: {error}", file.display()),
    }
}

/// The folder of the real servers, which `cargo xtask language-servers` filled. Panics, and
/// fails the test, when [`REAL_SERVERS`] is not set, or names a folder that does not hold
/// every built-in row's pinned version for this machine (`download::missing`), with the
/// sentence: "This test needs the real language servers: run `cargo xtask
/// language-servers <folder>` once, and set YAAR_LANGUAGE_SERVERS to that folder. Found:
/// <what was found>." It never skips the test.
pub fn real_servers() -> PathBuf {
    match real_servers_in(std::env::var_os(REAL_SERVERS)) {
        Ok(folder) => folder,
        Err(found) => panic!(
            "This test needs the real language servers: run `cargo xtask language-servers \
             <folder>` once, and set {REAL_SERVERS} to that folder. Found: {found}."
        ),
    }
}

/// [`real_servers`] for a value of the variable: the folder, or what was found instead.
pub fn real_servers_in(variable: Option<OsString>) -> Result<PathBuf, String> {
    let Some(variable) = variable.filter(|value| !value.is_empty()) else {
        return Err(format!("{REAL_SERVERS} is not set"));
    };
    let folder = PathBuf::from(variable);
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let platform = Platform::current().ok_or("no real server is pinned for this machine")?;
    let (registry, _) = Registry::read(BUILTIN, None);
    let missing = download::missing(&registry, platform, &folder);
    if missing.is_empty() {
        Ok(folder)
    } else {
        Err(format!(
            "{} lacks the pinned version of {}",
            folder.display(),
            missing.join(", ")
        ))
    }
}

/// The stand-in language server: the binary of this package.
pub fn standin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_standin-lsp"))
}

/// A registry text of one row `name` for files ending in `.sil` (`extensions = { sil =
/// "standin" }`, `markers = ["main.sil"]`), with one server `standin`: the stand-in, found by
/// `machine = ["standin-lsp"]` in a folder the test puts on the search path
/// ([`Scratch::put_standin`]), started with `args`, `trial = ["--version"]`, and `ready` of
/// the kind `ready` (`answer`, `progress`, or `notification` with the method
/// `standin/status` and the match `{ready = true}`), and no download. `extra` is appended to
/// the server's table as written: more keys of it, or a `[[<name>.server.download]]`.
pub fn standin_row(name: &str, args: &[&str], ready: &str, extra: &str) -> String {
    let ready = match ready {
        "answer" => r#"{ kind = "answer" }"#,
        "progress" => r#"{ kind = "progress" }"#,
        "notification" => {
            r#"{ kind = "notification", method = "standin/status", match = { ready = true } }"#
        }
        other => {
            panic!("the stand-in's readiness is answer, progress or notification, not {other}")
        }
    };
    let args: Vec<String> = args.iter().map(|arg| toml_string(arg)).collect();
    format!(
        "[{name}]\n\
         extensions = {{ sil = \"standin\" }}\n\
         markers = [\"main.sil\"]\n\
         \n\
         [[{name}.server]]\n\
         name = \"standin\"\n\
         machine = [\"standin-lsp\"]\n\
         args = [{}]\n\
         trial = [\"--version\"]\n\
         ready = {ready}\n\
         {extra}\n",
        args.join(", ")
    )
}

/// `text` as a TOML basic string.
fn toml_string(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

impl Scratch {
    /// A copy of the stand-in as `<folder>/<name>`, made executable, `folder` made first:
    /// in the scratch's `bin` for a server installed on the machine, in a repository for a
    /// project's own, anywhere for a registered one. Gives its path.
    pub fn put_standin(&self, folder: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(folder).expect("make the folder");
        let program = folder.join(name);
        std::fs::copy(standin(), &program).expect("copy the stand-in");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in executable");
        program
    }

    /// A script `<folder>/<name>`, `folder` made first, made executable, that appends its
    /// arguments (`"$@"`, joined by one space) as one line to `<folder>/<name>.runs` and
    /// then ends with `code` whatever it is asked: a candidate that fails its trial run, or
    /// a registered command that fails. [`runs`] reads what it recorded. Gives its path.
    pub fn put_failing(&self, folder: &Path, name: &str, code: i32) -> PathBuf {
        let program = folder.join(name);
        write_script(&program, &format!("{}exit {code}\n", records_to(&program)));
        program
    }

    /// A script `<folder>/<name>`, `folder` made first, made executable, that appends its
    /// arguments (`"$@"`, joined by one space) as one line to `<folder>/<name>.runs` and
    /// then runs the stand-in with them (`exec`): a candidate whose every run, the trial
    /// run included, leaves a line ([`runs`]). Gives its path.
    pub fn put_recording(&self, folder: &Path, name: &str) -> PathBuf {
        let program = folder.join(name);
        write_script(
            &program,
            &format!(
                "{}exec {} \"$@\"\n",
                records_to(&program),
                quoted(&standin())
            ),
        );
        program
    }
}

/// The path of the `.runs` file of the script at `program`.
fn runs_file(program: &Path) -> PathBuf {
    let mut runs = program.as_os_str().to_owned();
    runs.push(".runs");
    PathBuf::from(runs)
}

/// The line of a script that records its arguments in the `.runs` file of `program`.
fn records_to(program: &Path) -> String {
    format!("printf '%s\\n' \"$*\" >> {}\n", quoted(&runs_file(program)))
}

/// The lines a script of [`Scratch::put_failing`] or [`Scratch::put_recording`] at
/// `program` recorded, one a run, in order; empty when it never ran (no `.runs` file).
pub fn runs(program: &Path) -> Vec<String> {
    lines_of(&runs_file(program))
}

/// The ids a test names its workspaces and repositories by: `digit` 32 times.
pub fn workspace_id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

/// See [`workspace_id`].
pub fn repository_id(digit: char) -> RepositoryId {
    RepositoryId::new(digit.to_string().repeat(32))
}

/// The place of the `nth` whole-word occurrence, from 0, of `name` in the line `line`, from
/// 1, of `path` in `repo`'s worktree (or, for a [`Tree::Version`], in the base as git holds
/// it), in `tree`; `shown` empty. Panics when the line does not hold it.
pub fn place(
    repo: &Repo,
    workspace: &WorkspaceId,
    tree: Tree,
    path: &str,
    line: u32,
    name: &str,
    nth: usize,
) -> Place {
    let text = match &tree {
        Tree::Worktree => repo.line(path, line),
        Tree::Version(version) => line_of(
            &repo.git(&["show", &format!("{version}:{path}")]),
            line,
            &format!("{path} at {version}"),
        ),
        Tree::Past { text, .. } => line_of(text, line, &format!("{path} of the past version")),
    };
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let column = text
        .match_indices(name)
        .filter(|(at, _)| {
            !text[..*at].chars().next_back().is_some_and(is_word)
                && !text[at + name.len()..].chars().next().is_some_and(is_word)
        })
        .nth(nth)
        .map(|(at, _)| at)
        .unwrap_or_else(|| {
            panic!("line {line} of {path} holds no occurrence {nth} of {name}: {text:?}")
        });
    Place {
        workspace: workspace.clone(),
        tree,
        path: PathBuf::from(path),
        line,
        column: column as u32,
        name: name.to_owned(),
        shown: Vec::new(),
    }
}

/// Wakes a thread that parks: the waker of [`receive`].
struct Unparker(std::thread::Thread);

impl Wake for Unparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// What [`receive`] found.
enum Received<T> {
    Item(T),
    TimedOut,
    Closed,
}

/// Waits on the channel itself for an item, up to `within`: the thread parks and the channel
/// wakes it, so that nothing is looked at on a timer.
fn receive<T>(receiver: &async_channel::Receiver<T>, within: Duration) -> Received<T> {
    let deadline = Instant::now() + within;
    let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut next = std::pin::pin!(receiver.recv());
    loop {
        match next.as_mut().poll(&mut context) {
            Poll::Ready(Ok(item)) => return Received::Item(item),
            Poll::Ready(Err(_)) => return Received::Closed,
            Poll::Pending => {}
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Received::TimedOut;
        }
        std::thread::park_timeout(left);
    }
}

/// What ends the Navigator a [`Running`] stands for: waits up to the time given for it to
/// end, and gives whether it did.
type Ending = Box<dyn FnOnce(Duration) -> bool + Send>;

/// A Navigator started for a test, and the Frame's end of its port, which the test plays.
pub struct Running {
    navigator: Option<Ending>,
    port: Option<Port>,
    /// Everything told so far, in order.
    pub told: Vec<Told>,
    next: u64,
}

impl Running {
    /// Starts a Navigator with `config`, and reads [`Told::Languages`], which it tells
    /// first.
    pub fn start(config: NavigatorConfig) -> Running {
        let (port, end) = yaar_navigator::port();
        let navigator = Navigator::start(config, end);
        let mut running = Running {
            navigator: Some(Box::new(move |within| navigator.wait(within))),
            port: Some(port),
            told: Vec::new(),
            next: 1,
        };
        running.wait_told("the languages of the registry", DEADLINE, |told| {
            matches!(told, Told::Languages(_))
        });
        running
    }

    /// The Frame's end of a port whose other end the test plays itself: no Navigator runs,
    /// and what a test of this support asks, it answers on the [`yaar_navigator::PortEnd`]
    /// it made the port with.
    pub fn over(port: Port) -> Running {
        Running {
            navigator: None,
            port: Some(port),
            told: Vec::new(),
            next: 1,
        }
    }

    /// As [`Running::over`], for a test of [`Running::end`] itself: `ending` is what the
    /// test's Navigator does while `end` waits for it. It is handed the time `end` waits
    /// for, and gives whether the Navigator ended; what it tells meanwhile, it tells on
    /// the [`yaar_navigator::PortEnd`] of the port.
    pub fn over_ending(
        port: Port,
        ending: impl FnOnce(Duration) -> bool + Send + 'static,
    ) -> Running {
        Running {
            navigator: Some(Box::new(ending)),
            port: Some(port),
            told: Vec::new(),
            next: 1,
        }
    }

    fn port(&self) -> &Port {
        self.port.as_ref().expect("the Navigator was ended")
    }

    /// A new id for an ask that wants an answer.
    fn fresh(&mut self) -> AskId {
        let id = AskId(self.next);
        self.next += 1;
        id
    }

    /// Hands the Navigator `ask`.
    pub fn ask(&self, ask: Ask) {
        self.port()
            .asks
            .send_blocking(ask)
            .expect("the Navigator's end of the port is gone: it ended, or its thread failed");
    }

    /// Tells the Navigator that `repo` is the workspace `workspace` of the repository
    /// `repository`, the only one, and that it is on screen: [`Ask::Workspaces`], then
    /// [`Ask::OnScreen`].
    pub fn show(&self, repo: &Repo, workspace: &WorkspaceId, repository: &RepositoryId) {
        self.ask(Ask::Workspaces(vec![WorkspaceRoot {
            id: workspace.clone(),
            repository: repository.clone(),
            folder: Some(repo.root.clone()),
        }]));
        self.ask(Ask::OnScreen(Some(workspace.clone())));
    }

    /// Reads what the Navigator tells, waiting on the channel itself (`recv` with the time
    /// left, never a sleep between looks), until one told thing satisfies `wanted` or
    /// `deadline` passes; gives it. Panics with `what` and
    /// everything told meanwhile when the deadline passes. A thing told before the call
    /// counts only if it was not read yet.
    pub fn wait_told(
        &mut self,
        what: &str,
        deadline: Duration,
        wanted: impl Fn(&Told) -> bool,
    ) -> Told {
        let until = Instant::now() + deadline;
        let first = self.told.len();
        let receiver = self.port().told.clone();
        loop {
            match receive(&receiver, until.saturating_duration_since(Instant::now())) {
                Received::Item(told) => {
                    self.told.push(told.clone());
                    if wanted(&told) {
                        return told;
                    }
                }
                Received::TimedOut => panic!(
                    "waited {deadline:?} for {what}, and was told only: {:#?}",
                    &self.told[first..]
                ),
                Received::Closed => panic!(
                    "the Navigator ended while the test waited for {what}; it had told: {:#?}",
                    &self.told[first..]
                ),
            }
        }
    }

    /// The newest state told of the server of `language` in `workspace`, of the files on
    /// disk (`old` none) or of the old side of `old`; waits, as [`Running::wait_told`],
    /// until a [`Told::Servers`] holds one that satisfies `wanted`.
    pub fn wait_state(
        &mut self,
        workspace: &WorkspaceId,
        language: &str,
        old: Option<&str>,
        deadline: Duration,
        wanted: impl Fn(&ServerState) -> bool,
    ) -> ServerState {
        let of_the_server = |state: &&ServerState| {
            state.workspace == *workspace
                && state.language == language
                && state.old_side.as_deref() == old
        };
        let what = format!("a state of the {language} server of {workspace:?} (old side {old:?})");
        match self.wait_told(&what, deadline, |told| match told {
            Told::Servers(states) => states.iter().filter(of_the_server).any(&wanted),
            _ => false,
        }) {
            Told::Servers(states) => states
                .iter()
                .filter(of_the_server)
                .find(|state| wanted(state))
                .expect("the state that satisfied the wait")
                .clone(),
            _ => unreachable!("only a list of servers satisfies the wait"),
        }
    }

    /// Asks [`Ask::Hover`] at `place` and waits for its answer until `deadline`.
    pub fn hover(&mut self, place: Place, deadline: Duration) -> Answered<Option<String>> {
        let id = self.fresh();
        self.ask(Ask::Hover { id, place });
        match self.wait_told(
            "the answer to a hover",
            deadline,
            |told| matches!(told, Told::Hover { id: got, .. } if *got == id),
        ) {
            Told::Hover { answer, .. } => answer,
            _ => unreachable!("only the answer to the ask satisfies the wait"),
        }
    }

    /// Asks [`Ask::Definition`] at `place` and waits for its answer until `deadline`.
    pub fn definition(&mut self, place: Place, deadline: Duration) -> Answered<Vec<Location>> {
        let id = self.fresh();
        self.ask(Ask::Definition { id, place });
        match self.wait_told(
            "the answer to a definition",
            deadline,
            |told| matches!(told, Told::Definition { id: got, .. } if *got == id),
        ) {
            Told::Definition { answer, .. } => answer,
            _ => unreachable!("only the answer to the ask satisfies the wait"),
        }
    }

    /// Asks [`Ask::References`] at `place` and waits for its answer until `deadline`.
    pub fn references(&mut self, place: Place, deadline: Duration) -> Answered<Vec<Location>> {
        let id = self.fresh();
        self.ask(Ask::References { id, place });
        match self.wait_told(
            "the answer to a references ask",
            deadline,
            |told| matches!(told, Told::References { id: got, .. } if *got == id),
        ) {
            Told::References { answer, .. } => answer,
            _ => unreachable!("only the answer to the ask satisfies the wait"),
        }
    }

    /// Plays the Keeper's answer to the next [`Told::ExportWanted`]: waits for it until
    /// `deadline`, writes the asked version of `repo` into the asked folder with
    /// `git archive` unpacked by the `tar` crate, and hands back [`Ask::Exported`]. Gives
    /// what was asked. A folder that exists, or a version that cannot be written, is answered
    /// as the Keeper answers a failure, with its reason (and the reason is printed), and
    /// leaves nothing behind.
    pub fn export_when_wanted(
        &mut self,
        repo: &Repo,
        deadline: Duration,
    ) -> yaar_navigator::ExportAsk {
        let asked = match self.wait_told("the Navigator to want an export", deadline, |told| {
            matches!(told, Told::ExportWanted(_))
        }) {
            Told::ExportWanted(asked) => asked,
            _ => unreachable!("only an export wanted satisfies the wait"),
        };
        let failed = write_version(repo, &asked.version, &asked.into).err();
        if let Some(reason) = &failed {
            eprintln!("the export of {} failed: {reason}", asked.version);
        }
        self.ask(Ask::Exported {
            asked: asked.clone(),
            failed,
        });
        asked
    }

    /// Ends the Navigator: drops the Frame's end of the port, waits until the Navigator's
    /// thread ended ([`DEADLINE`] plus `yaar_navigator::STOP_DEADLINE` for each server
    /// told), then asserts that every process any [`Told::Servers`] named is gone.
    pub fn end(self) {
        self.end_and_read();
    }

    /// [`Running::end`], giving everything that was told, up to the closing of the port.
    pub fn end_and_read(mut self) -> Vec<Told> {
        let told = self.port.take().expect("the Navigator was ended").told;
        self.drain(&told);
        if let Some(navigator) = self.navigator.take() {
            let within = DEADLINE + STOP_DEADLINE * self.pids().len() as u32;
            assert!(
                navigator(within),
                "the Navigator did not end within {within:?} of its port being dropped"
            );
        }
        // It has ended: whatever it told last, the stops of its servers, is in the channel.
        self.drain(&told);
        let alive: Vec<u32> = self
            .pids()
            .into_iter()
            .filter(|pid| is_alive(*pid))
            .collect();
        assert!(
            alive.is_empty(),
            "the processes {alive:?} of servers the Navigator named are still running"
        );
        std::mem::take(&mut self.told)
    }

    /// Reads everything in the channel now, and no more.
    fn drain(&mut self, told: &async_channel::Receiver<Told>) {
        while let Ok(one) = told.try_recv() {
            self.told.push(one);
        }
    }

    /// Every process any list of servers told named.
    fn pids(&self) -> BTreeSet<u32> {
        self.told
            .iter()
            .filter_map(|told| match told {
                Told::Servers(states) => Some(states),
                _ => None,
            })
            .flatten()
            .filter_map(|state| state.pid)
            .collect()
    }
}

impl Drop for Running {
    /// A test that failed before its end still ends its Navigator, so that no server of it
    /// is left running.
    fn drop(&mut self) {
        let Some(navigator) = self.navigator.take() else {
            return;
        };
        let told = self.port.take().map(|port| port.told);
        if let Some(told) = &told {
            self.drain(told);
        }
        let _ = navigator(DEADLINE + STOP_DEADLINE * self.pids().len() as u32);
    }
}

/// Writes `version` of `repo` into `into`, which is made, as plain files.
///
/// As the Keeper writes an export: `version` must be a full commit id, 40 hexadecimal
/// characters, `into` must not exist, its parents are made, and a write that fails leaves no
/// folder. A Navigator that asks twice for one folder, or reuses one left over, is refused
/// here as it is by the Keeper.
///
/// What the real Keeper does that this does not, so that no test leans on the difference:
/// it writes into a cache folder it keeps, and refuses a folder outside it; it writes the
/// files with `checkout-index` from its own store, so the export attributes of a repository
/// that `git archive` applies here (`export-ignore`, `export-subst`) do not reach its export;
/// and it turns off the filter drivers it finds in the configuration it reads, which
/// `git archive` runs here. It also refuses a 64-character id of a SHA-256 repository,
/// which the real Keeper takes; the fixtures of the tests have none of these.
fn write_version(repo: &Repo, version: &str, into: &Path) -> Result<(), String> {
    if version.len() != 40 || !version.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{version:?} is no full commit id: it is not 40 hexadecimal characters"
        ));
    }
    if into.symlink_metadata().is_ok() {
        return Err(format!("{} already exists", into.display()));
    }
    let archive = git_archive(repo, version)?;
    if let Some(parent) = into.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir(into).map_err(|error| error.to_string())?;
    tar::Archive::new(&archive[..])
        .unpack(into)
        .map_err(|error| {
            let _ = std::fs::remove_dir_all(into);
            error.to_string()
        })
}

/// `git archive` of `version`, as tar bytes.
fn git_archive(repo: &Repo, version: &str) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&repo.root)
        .args(["archive", "--format=tar", version])
        .stdin(Stdio::null());
    for (name, value) in GIT_SETTINGS {
        command.env(name, value);
    }
    command.env("HOME", &repo.home);
    for name in REPOSITORY_VARIABLES
        .into_iter()
        .chain(CONFIGURATION_VARIABLES)
    {
        command.env_remove(name);
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Whether a process of this id runs and is not a zombie.
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists; it sends nothing.
    let signalled = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if signalled != 0 {
        // A process of another user exists, and answers that it may not be signalled.
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    !is_zombie(pid)
}

/// Whether a process that exists has ended and waits for its parent to read how.
#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            // `pid (name) state ...`, the name any text, so the last `)` ends it.
            let after_name = stat.rsplit_once(')')?.1;
            after_name.trim_start().chars().next()
        })
        .is_some_and(|state| state == 'Z' || state == 'X')
}

/// Whether a process that exists has ended and waits for its parent to read how.
#[cfg(not(target_os = "linux"))]
fn is_zombie(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('Z')
        })
}

/// A request a [`Loopback`] heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    /// The request line's method and path: `GET /stand-in.tar.gz`.
    pub line: String,
    /// Each header, its name lowercased, in the order sent.
    pub headers: Vec<(String, String)>,
}

/// What a [`Loopback`] serves and has heard.
#[derive(Default)]
struct Hearing {
    files: HashMap<String, Vec<u8>>,
    redirects: HashMap<String, String>,
    heard: Vec<Heard>,
}

/// A listener on `127.0.0.1` of the test's own that serves downloads: what a publisher's
/// address is in every test of the downloader but LSP-01. It answers `GET /<name>` with the
/// bytes set for `name` and status 200, a name set as a redirect ([`Loopback::redirect`])
/// with 302 and its `Location`, any other request, whatever its method, with 404; and keeps
/// every request it heard. It reads one request a connection and closes it after the
/// answer; a connection that never sends a request holds its thread until the process
/// ends. Stopped when dropped.
pub struct Loopback {
    port: u16,
    hearing: Arc<Mutex<Hearing>>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Loopback {
    /// A listener on a free port of `127.0.0.1`, serving `files` (name, bytes).
    pub fn serve(files: Vec<(String, Vec<u8>)>) -> Loopback {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen on the loopback address");
        let port = listener
            .local_addr()
            .expect("the listener's address")
            .port();
        let hearing = Arc::new(Mutex::new(Hearing {
            files: files.into_iter().collect(),
            ..Hearing::default()
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        let thread = {
            let hearing = Arc::clone(&hearing);
            let stopped = Arc::clone(&stopped);
            std::thread::Builder::new()
                .name("loopback".to_owned())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stopped.load(Ordering::SeqCst) {
                            return;
                        }
                        let Ok(stream) = stream else { continue };
                        let hearing = Arc::clone(&hearing);
                        // One thread a connection: a client that stalls holds no other.
                        std::thread::spawn(move || answer(stream, &hearing));
                    }
                })
                .expect("start the loopback listener")
        };
        Loopback {
            port,
            hearing,
            stopped,
            thread: Some(thread),
        }
    }

    /// From now on, `GET /<name>` is answered 302 with `Location: <location>`, as written.
    pub fn redirect(&self, name: &str, location: &str) {
        self.hearing
            .lock()
            .expect("the loopback's state")
            .redirects
            .insert(name.to_owned(), location.to_owned());
    }

    /// `http://127.0.0.1:<port>/<name>`.
    pub fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

    /// Every request heard, in order.
    pub fn heard(&self) -> Vec<Heard> {
        self.hearing
            .lock()
            .expect("the loopback's state")
            .heard
            .clone()
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        // The listener is blocked in `accept`: one connection of its own wakes it.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reads one request from `stream`, keeps it, and answers it. The request is kept before
/// the answer is written, so that whoever got the answer finds the request heard.
fn answer(stream: TcpStream, hearing: &Mutex<Hearing>) {
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    if !matches!(reader.read_line(&mut request), Ok(read) if read > 0) {
        return;
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(read) if read > 0 => {}
            _ => return,
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    let response = {
        let mut hearing = hearing.lock().expect("the loopback's state");
        hearing.heard.push(Heard {
            line: format!("{method} {target}"),
            headers,
        });
        let name = target.strip_prefix('/').unwrap_or("");
        if method != "GET" {
            None
        } else if let Some(location) = hearing.redirects.get(name) {
            Some(
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .into_bytes(),
            )
        } else {
            hearing.files.get(name).map(|bytes| {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .into_bytes();
                response.extend_from_slice(bytes);
                response
            })
        }
    };
    let response = response.unwrap_or_else(|| {
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    // A client that has gone is its own affair.
    let mut stream = &stream;
    let _ = stream.write_all(&response);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// The bytes of a `.tar.gz` holding `files` (path in the archive, bytes, mode). A path of up
/// to 100 bytes is written as given, so that an archive that holds an absolute path or one
/// that climbs out of its folder, which a downloader must refuse, can be packed too. A
/// longer path goes through the `tar` crate's long-name entry, which refuses a `..`
/// component: such a path panics here.
pub fn tar_gz(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (path, bytes, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_mtime(0);
        if path.len() <= 100 {
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, *bytes).expect("pack the file");
        } else {
            builder
                .append_data(&mut header, path, *bytes)
                .expect("pack the file");
        }
    }
    builder
        .into_inner()
        .expect("finish the archive")
        .finish()
        .expect("finish the compression")
}

/// The bytes of `bytes` compressed as one gzip file.
pub fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("compress");
    encoder.finish().expect("finish the compression")
}

/// The SHA-256 digest of `bytes` in lowercase hexadecimal.
pub fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
