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

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{
    Answered, Ask, Location, Navigator, NavigatorConfig, Place, Port, ServerState, Told, Tree,
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
}

impl Repo {
    /// Runs git in the worktree with these arguments, as the tests of the Recorder run it:
    /// `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`, a fixed author, committer and
    /// date, `HOME` in the scratch folder, and every variable that names a repository
    /// removed. Gives what it printed, with no line end; panics with what git wrote when
    /// git fails.
    pub fn git(&self, args: &[&str]) -> String {
        let _ = args;
        todo!()
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

    /// Every file under `relative` in the worktree, with its size, by path: what a build
    /// folder holds.
    pub fn files_under(&self, relative: &str) -> Vec<(PathBuf, u64)> {
        let _ = relative;
        todo!()
    }

    /// The text of the line `line`, from 1, of the file `relative` in the worktree.
    pub fn line(&self, relative: &str, line: u32) -> String {
        let _ = (relative, line);
        todo!()
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
    #[allow(clippy::new_without_default)]
    pub fn new() -> Scratch {
        todo!()
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
        todo!()
    }

    /// The fixed project `which` made into a repository under `repos`, as
    /// `tests/fixtures/README.md` says, with an identity of its own in its `.git/config`.
    pub fn fixture(&self, which: Fixture) -> Repo {
        let _ = which;
        todo!()
    }

    /// A repository `name` under `repos` holding `files` (path, text), committed once,
    /// with an identity of its own in its `.git/config`.
    pub fn repository(&self, name: &str, files: &[(&str, &str)]) -> Repo {
        let _ = (name, files);
        todo!()
    }

    /// A configuration of the Navigator for this test: the built-in table `builtin`, the
    /// user's file `<root>/data/language-servers.toml` (there only when the test writes it),
    /// the downloads in `servers`, the cache in `navigator`, the search path the scratch's
    /// `bin` alone, [`Scratch::environment`], `Platform::current()`,
    /// `yaar_navigator::STOP_AFTER` and the system's clock. A test changes a field where it
    /// needs another.
    pub fn config(&self, builtin: &str) -> NavigatorConfig {
        let _ = builtin;
        todo!()
    }

    /// The lines the recorder `npm` wrote, one a run: what a server ran of the network's
    /// package manager.
    pub fn npm_ran(&self) -> Vec<String> {
        todo!()
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
        todo!()
    }
}

/// The folder of the real servers, which `cargo xtask language-servers` filled. Panics, and
/// fails the test, when [`REAL_SERVERS`] is not set, or names a folder that does not hold
/// every built-in row's pinned version for this machine (`download::missing`), with the
/// sentence: "This test needs the real language servers: run `cargo xtask
/// language-servers <folder>` once, and set YAAR_LANGUAGE_SERVERS to that folder. Found:
/// <what was found>." It never skips the test.
pub fn real_servers() -> PathBuf {
    todo!()
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
    let _ = (name, args, ready, extra);
    todo!()
}

impl Scratch {
    /// A copy of the stand-in as `<folder>/<name>`, made executable, `folder` made first:
    /// in the scratch's `bin` for a server installed on the machine, in a repository for a
    /// project's own, anywhere for a registered one. Gives its path.
    pub fn put_standin(&self, folder: &Path, name: &str) -> PathBuf {
        let _ = (folder, name);
        todo!()
    }

    /// A script `<folder>/<name>`, `folder` made first, made executable, that appends its
    /// arguments (`"$@"`, joined by one space) as one line to `<folder>/<name>.runs` and
    /// then ends with `code` whatever it is asked: a candidate that fails its trial run, or
    /// a registered command that fails. [`runs`] reads what it recorded. Gives its path.
    pub fn put_failing(&self, folder: &Path, name: &str, code: i32) -> PathBuf {
        let _ = (folder, name, code);
        todo!()
    }

    /// A script `<folder>/<name>`, `folder` made first, made executable, that appends its
    /// arguments (`"$@"`, joined by one space) as one line to `<folder>/<name>.runs` and
    /// then runs the stand-in with them (`exec`): a candidate whose every run, the trial
    /// run included, leaves a line ([`runs`]). Gives its path.
    pub fn put_recording(&self, folder: &Path, name: &str) -> PathBuf {
        let _ = (folder, name);
        todo!()
    }
}

/// The lines a script of [`Scratch::put_failing`] or [`Scratch::put_recording`] at
/// `program` recorded, one a run, in order; empty when it never ran (no `.runs` file).
pub fn runs(program: &Path) -> Vec<String> {
    let _ = program;
    todo!()
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
    let _ = (repo, workspace, tree, path, line, name, nth);
    todo!()
}

/// A Navigator started for a test, and the Frame's end of its port, which the test plays.
pub struct Running {
    navigator: Option<Navigator>,
    port: Option<Port>,
    /// Everything told so far, in order.
    pub told: Vec<Told>,
    next: u64,
}

impl Running {
    /// Starts a Navigator with `config`, and reads [`Told::Languages`], which it tells
    /// first.
    pub fn start(config: NavigatorConfig) -> Running {
        let _ = config;
        todo!()
    }

    /// Hands the Navigator `ask`.
    pub fn ask(&self, ask: Ask) {
        let _ = ask;
        todo!()
    }

    /// Tells the Navigator that `repo` is the workspace `workspace` of the repository
    /// `repository`, the only one, and that it is on screen: [`Ask::Workspaces`], then
    /// [`Ask::OnScreen`].
    pub fn show(&self, repo: &Repo, workspace: &WorkspaceId, repository: &RepositoryId) {
        let _ = (repo, workspace, repository);
        todo!()
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
        let _ = (what, deadline, wanted);
        todo!()
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
        let _ = (workspace, language, old, deadline, wanted);
        todo!()
    }

    /// Asks [`Ask::Hover`] at `place` and waits for its answer until `deadline`.
    pub fn hover(&mut self, place: Place, deadline: Duration) -> Answered<Option<String>> {
        let _ = (place, deadline);
        todo!()
    }

    /// Asks [`Ask::Definition`] at `place` and waits for its answer until `deadline`.
    pub fn definition(&mut self, place: Place, deadline: Duration) -> Answered<Vec<Location>> {
        let _ = (place, deadline);
        todo!()
    }

    /// Asks [`Ask::References`] at `place` and waits for its answer until `deadline`.
    pub fn references(&mut self, place: Place, deadline: Duration) -> Answered<Vec<Location>> {
        let _ = (place, deadline);
        todo!()
    }

    /// Plays the Keeper's answer to the next [`Told::ExportWanted`]: waits for it until
    /// `deadline`, writes the asked version of `repo` into the asked folder with
    /// `git archive` unpacked by the `tar` crate, and hands back [`Ask::Exported`]. Gives
    /// what was asked.
    pub fn export_when_wanted(
        &mut self,
        repo: &Repo,
        deadline: Duration,
    ) -> yaar_navigator::ExportAsk {
        let _ = (repo, deadline);
        todo!()
    }

    /// Ends the Navigator: drops the Frame's end of the port, waits until the Navigator's
    /// thread ended ([`DEADLINE`] plus `yaar_navigator::STOP_DEADLINE` for each server
    /// told), then asserts that every process any [`Told::Servers`] named is gone.
    pub fn end(self) {
        let _ = (&self.navigator, &self.port, self.next);
        todo!()
    }
}

/// Whether a process of this id runs and is not a zombie.
pub fn is_alive(pid: u32) -> bool {
    let _ = pid;
    todo!()
}

/// A request a [`Loopback`] heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    /// The request line's method and path: `GET /stand-in.tar.gz`.
    pub line: String,
    /// Each header, its name lowercased, in the order sent.
    pub headers: Vec<(String, String)>,
}

/// A listener on `127.0.0.1` of the test's own that serves downloads: what a publisher's
/// address is in every test of the downloader but LSP-01. It answers `GET /<name>` with the
/// bytes set for `name` and status 200, a name set as a redirect ([`Loopback::redirect`])
/// with 302 and its `Location`, any other request, whatever its method, with 404; and keeps
/// every request it heard. It reads one request a connection and closes it after the
/// answer. Stopped when dropped.
pub struct Loopback {
    port: u16,
}

impl Loopback {
    /// A listener on a free port of `127.0.0.1`, serving `files` (name, bytes).
    pub fn serve(files: Vec<(String, Vec<u8>)>) -> Loopback {
        let _ = files;
        todo!()
    }

    /// From now on, `GET /<name>` is answered 302 with `Location: <location>`, as written.
    pub fn redirect(&self, name: &str, location: &str) {
        let _ = (name, location);
        todo!()
    }

    /// `http://127.0.0.1:<port>/<name>`.
    pub fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

    /// Every request heard, in order.
    pub fn heard(&self) -> Vec<Heard> {
        todo!()
    }
}

/// The bytes of a `.tar.gz` holding `files` (path in the archive, bytes, mode).
pub fn tar_gz(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let _ = files;
    todo!()
}

/// The bytes of `bytes` compressed as one gzip file.
pub fn gz(bytes: &[u8]) -> Vec<u8> {
    let _ = bytes;
    todo!()
}

/// The SHA-256 digest of `bytes` in lowercase hexadecimal.
pub fn sha256(bytes: &[u8]) -> String {
    let _ = bytes;
    todo!()
}
