//! Shared helpers of the Navigator component tests.

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

pub const DEADLINE: Duration = Duration::from_secs(10);

/// Real servers on a loaded machine at background priority take many seconds to load a project.
pub const SERVER_DEADLINE: Duration = Duration::from_secs(120);

pub const REAL_SERVERS: &str = "YAAR_LANGUAGE_SERVERS";

const REPOSITORY_VARIABLES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// Settings that reach git from the environment; removing the count voids the numbered keys.
const CONFIGURATION_VARIABLES: [&str; 3] =
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    TypeScript,
    Python,
    Rust,
}

pub struct Repo {
    pub root: PathBuf,
    pub base: String,
    home: PathBuf,
}

impl Repo {
    pub fn git(&self, args: &[&str]) -> String {
        String::from_utf8(git_bytes(&self.home, &self.root, args))
            .expect("git printed text")
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    }

    pub fn status(&self) -> String {
        self.git(&[
            "status",
            "--porcelain=v2",
            "--ignored",
            "--untracked-files=all",
        ])
    }

    pub fn files_under(&self, relative: &str) -> Vec<(PathBuf, u64)> {
        let mut found = Vec::new();
        collect_files(&self.root.join(relative), &mut found);
        found.sort();
        found
    }

    pub fn line(&self, relative: &str, line: u32) -> String {
        let file = self.root.join(relative);
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        line_of(&text, line, &file.display().to_string())
    }

    fn commit(&mut self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.base = self.git(&["rev-parse", "HEAD"]);
    }
}

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

fn line_of(text: &str, line: u32, what: &str) -> String {
    text.lines()
        .nth(line.checked_sub(1).expect("lines count from 1") as usize)
        .unwrap_or_else(|| panic!("{what} has no line {line}"))
        .to_owned()
}

/// `dot-gitignore` becomes `.gitignore` at the top: git does not keep a fixture's `.gitignore`.
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

pub struct Scratch {
    pub root: PathBuf,
    proxy: Loopback,
}

impl Scratch {
    /// A new scratch folder with a recorder `npm` in `bin`. Sets git's variables and `HOME` in this
    /// process, so it must be a test's first statement; nextest runs each test in its own process.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-navigator-{}-{n}", std::process::id()));
        // A folder of this name was left by an earlier process with the same process id.
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

        // SAFETY: nextest gives each test a process of its own and this is the first statement of a
        // test, so no other thread exists yet to read the environment.
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

    /// `PATH` is `bin`, the test's cargo folder, `/usr/bin`, `/bin`; of this process's environment
    /// only `RUSTUP_HOME` and `RUSTUP_TOOLCHAIN` pass. With no `NO_PROXY`, a server that honours
    /// the proxy variables reaches only the scratch's [`Loopback`].
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

    pub fn config(&self, builtin: &str) -> NavigatorConfig {
        NavigatorConfig {
            builtin: builtin.to_owned(),
            user_file: self.root.join("data").join(USER_FILE),
            servers: self.root.join("servers"),
            cache: self.root.join("navigator"),
            search_path: vec![self.root.join("bin")],
            environment: self.environment(),
            login_shell: None,
            platform: Platform::current(),
            stop_after: STOP_AFTER,
            clock: Arc::new(SystemClock),
            auto_download: false,
        }
    }

    pub fn npm_ran(&self) -> Vec<String> {
        lines_of(&self.root.join("npm.log"))
    }

    pub fn proxy_heard(&self) -> Vec<Heard> {
        self.proxy.heard()
    }

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

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn write_script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folder");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script executable");
}

fn lines_of(file: &Path) -> Vec<String> {
    match std::fs::read_to_string(file) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("read {}: {error}", file.display()),
    }
}

/// Panics, never skips, when [`REAL_SERVERS`] is unset or names a folder without every built-in
/// row's pinned version for this machine.
pub fn real_servers() -> PathBuf {
    match real_servers_in(std::env::var_os(REAL_SERVERS)) {
        Ok(folder) => folder,
        Err(found) => panic!(
            "This test needs the real language servers: run `cargo xtask language-servers \
             <folder>` once, and set {REAL_SERVERS} to that folder. Found: {found}."
        ),
    }
}

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

pub fn standin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_standin-lsp"))
}

/// `ready` is `answer`, `progress` or `notification`; `extra` is appended to the server's table as
/// written.
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
    pub fn put_standin(&self, folder: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(folder).expect("make the folder");
        let program = folder.join(name);
        std::fs::copy(standin(), &program).expect("copy the stand-in");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in executable");
        program
    }

    pub fn put_failing(&self, folder: &Path, name: &str, code: i32) -> PathBuf {
        let program = folder.join(name);
        write_script(&program, &format!("{}exit {code}\n", records_to(&program)));
        program
    }

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

fn runs_file(program: &Path) -> PathBuf {
    let mut runs = program.as_os_str().to_owned();
    runs.push(".runs");
    PathBuf::from(runs)
}

fn records_to(program: &Path) -> String {
    format!("printf '%s\\n' \"$*\" >> {}\n", quoted(&runs_file(program)))
}

pub fn runs(program: &Path) -> Vec<String> {
    lines_of(&runs_file(program))
}

pub fn workspace_id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

pub fn repository_id(digit: char) -> RepositoryId {
    RepositoryId::new(digit.to_string().repeat(32))
}

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

struct Unparker(std::thread::Thread);

impl Wake for Unparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

enum Received<T> {
    Item(T),
    TimedOut,
    Closed,
}

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

type Ending = Box<dyn FnOnce(Duration) -> bool + Send>;

pub struct Running {
    navigator: Option<Ending>,
    port: Option<Port>,
    pub told: Vec<Told>,
    next: u64,
}

impl Running {
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

    pub fn over(port: Port) -> Running {
        Running {
            navigator: None,
            port: Some(port),
            told: Vec::new(),
            next: 1,
        }
    }

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

    fn fresh(&mut self) -> AskId {
        let id = AskId(self.next);
        self.next += 1;
        id
    }

    pub fn ask(&self, ask: Ask) {
        self.port()
            .asks
            .send_blocking(ask)
            .expect("the Navigator's end of the port is gone: it ended, or its thread failed");
    }

    pub fn show(&self, repo: &Repo, workspace: &WorkspaceId, repository: &RepositoryId) {
        self.ask(Ask::Workspaces(vec![WorkspaceRoot {
            id: workspace.clone(),
            repository: repository.clone(),
            folder: Some(repo.root.clone()),
        }]));
        self.ask(Ask::OnScreen(Some(workspace.clone())));
    }

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

    /// Asserts that every process any [`Told::Servers`] named is gone.
    pub fn end(self) {
        self.end_and_read();
    }

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
        // It has ended: what it told last, the stops of its servers, is in the channel.
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

    fn drain(&mut self, told: &async_channel::Receiver<Told>) {
        while let Ok(one) = told.try_recv() {
            self.told.push(one);
        }
    }

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

/// Writes `version` into `into` as the Keeper writes an export. Unlike the Keeper it uses
/// `git archive` (export attributes and filter drivers apply) and refuses SHA-256 ids; no
/// fixture depends on these.
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

pub fn is_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists; it sends nothing.
    let signalled = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if signalled != 0 {
        // A process of another user exists, and answers that it may not be signalled.
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    !is_zombie(pid)
}

/// The first thread of a process shows as a zombie as soon as it has exited, while the others may
/// not have run since the kill: the process has ended when none is left.
#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| {
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                .map(str::trim)
        };
        let ended = field("State").is_some_and(|state| state.starts_with(['Z', 'X']));
        ended && field("Threads").and_then(|count| count.parse::<u32>().ok()) == Some(1)
    })
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    pub line: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Hearing {
    files: HashMap<String, Vec<u8>>,
    redirects: HashMap<String, String>,
    statuses: HashMap<String, u16>,
    heard: Vec<Heard>,
}

/// Serves downloads on 127.0.0.1, reading one request a connection; a connection that never
/// sends one holds its thread until the process ends.
pub struct Loopback {
    port: u16,
    hearing: Arc<Mutex<Hearing>>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Loopback {
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

    pub fn redirect(&self, name: &str, location: &str) {
        self.hearing
            .lock()
            .expect("the loopback's state")
            .redirects
            .insert(name.to_owned(), location.to_owned());
    }

    pub fn status(&self, name: &str, code: u16) {
        self.hearing
            .lock()
            .expect("the loopback's state")
            .statuses
            .insert(name.to_owned(), code);
    }

    pub fn clear_status(&self, name: &str) {
        self.hearing
            .lock()
            .expect("the loopback's state")
            .statuses
            .remove(name);
    }

    pub fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

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

/// The request is kept before the answer is written, so whoever got the answer finds it heard.
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
        } else if let Some(&code) = hearing.statuses.get(name) {
            Some(
                format!("HTTP/1.1 {code} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .into_bytes(),
            )
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

/// A path of up to 100 bytes is written as given, so an absolute or climbing path can be packed;
/// a longer one goes through the `tar` crate's long-name entry, which refuses `..` (a panic here).
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

pub fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("compress");
    encoder.finish().expect("finish the compression")
}

pub fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
