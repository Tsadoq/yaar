//! System tests (`B-BUILD`): the whole app through what a user or an agent touches.
//!
//! The tests in `tests/` start the built programs. This library holds what they share:
//! - [`World`], the folders, repositories and Keeper of one test, with [`Repo`] to drive
//!   git in it and [`Tap`] to read the requests a real Window sends;
//! - [`Client`], the scripted client that speaks the Link in place of the Window;
//! - [`descriptors_of`] and [`descendants_of`], which read from outside what a process
//!   holds open and what it started;
//! - [`program`], [`is_alive`] and [`wait_until`], and the bare [`attach`] and [`Place`]
//!   of the tests that need no world;
//! - [`standin`], the stand-in agent CLI, `standin-agent`, and the script a test hands
//!   it; [`World::agents_on_path`] puts it in a world's shells;
//! - [`recorded`], the recorded events of the real agent CLIs, which the tests of the
//!   Adapters, the Companion and the Workspaces replay, also from crates of their own.

mod client;
mod processes;
pub mod recorded;
pub mod standin;
mod world;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, Once, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use yaar_link::{Hello, handshake};

pub use client::{Client, DEADLINE};
pub use processes::{Descriptor, descendants_of, descriptors_of};
pub use world::{Repo, Tap, World, without_agent_clis};

/// The programs the Keeper finds beside itself: its companions, and the stand-in agent.
const KEEPER_SET: [&str; 3] = ["yaar-keeper", "yaar-claude-hook", "standin-agent"];

/// The Window, kept apart: it is large, and nothing looks for it beside the others.
const WINDOW_SET: [&str; 1] = ["yaar-review"];

/// The path of a built program of the workspace, building the programs first: the Keeper,
/// the Window, the hook program of the Claude Code companion and the stand-in agent.
///
/// They are built every time, once in each process that asks, and not only when one is
/// missing: a run of the tests builds a program only for the tests of its own package,
/// so one left in the profile folder by an earlier build can be older than the sources,
/// and a test would then meet a Window or a Keeper that is no longer the one in the tree.
/// Every program is named in the build, since a build that names one binary builds that
/// one alone. Where all are up to date, the build does nothing.
///
/// What is returned is a copy, in a folder of its own under the profile folder, and not
/// the file Cargo built. Cargo puts a program in its place again at every build, even one
/// that has nothing to do, and test programs run side by side each run one: a path that
/// leads to Cargo's file leads to nothing for the moment between its removal and its
/// return, which a Keeper that is started or a shell that finds a stand-in would
/// meet as a program that is not there. A copy is never touched again. The Keeper, its
/// companions and the stand-in lie in one folder, as the Keeper expects its companions
/// beside itself.
pub fn program(name: &str) -> PathBuf {
    static STAGED: Mutex<Vec<(&'static [&'static str], PathBuf)>> = Mutex::new(Vec::new());
    static BUILD: Once = Once::new();
    let set: &'static [&'static str] = [&KEEPER_SET[..], &WINDOW_SET[..]]
        .into_iter()
        .find(|set| set.contains(&name))
        .unwrap_or_else(|| panic!("{name} is not a program the system tests build"));
    let test_exe = std::env::current_exe().expect("the test's own path");
    let profile_dir = test_exe
        .ancestors()
        .find(|folder| {
            folder
                .file_name()
                .is_some_and(|name| name == "build" || name == "deps")
        })
        .and_then(Path::parent)
        .expect("a test program runs from beneath build/ or deps/")
        .to_owned();
    BUILD.call_once(|| build_programs(&profile_dir));
    let mut staged = STAGED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, folder)) = staged.iter().find(|(held, _)| *held == set) {
        return folder.join(name);
    }
    let folder = stage(&profile_dir, set);
    let path = folder.join(name);
    staged.push((set, folder));
    path
}

/// Builds every program of the system tests, and panics unless Cargo says it built each.
fn build_programs(profile_dir: &Path) {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(cargo);
    build.args([
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "-p",
        "yaar-keeper",
        "-p",
        "yaar-window",
        "-p",
        "yaar-companion-claude",
        "-p",
        "yaar-system-tests",
    ]);
    for name in KEEPER_SET.iter().chain(&WINDOW_SET) {
        build.args(["--bin", name]);
    }
    if profile_dir.file_name().is_some_and(|p| p == "release") {
        build.arg("--release");
    }
    let output = build.stderr(Stdio::inherit()).output().expect("run cargo");
    assert!(output.status.success(), "building the programs failed");
    let reported = String::from_utf8_lossy(&output.stdout);
    for name in KEEPER_SET.iter().chain(&WINDOW_SET) {
        let artifact = format!(r#""name":"{name}","src_path""#);
        assert!(
            reported
                .lines()
                .any(|line| line.contains(r#""reason":"compiler-artifact""#)
                    && line.contains(&artifact)),
            "the build did not make {name}"
        );
    }
}

/// How long a copy of the programs is kept after it was last asked for: a copy is asked for
/// by every test process that starts, and one nobody asked for in this time is of a build
/// that is no more.
const KEPT_FOR: Duration = Duration::from_secs(3600);

/// Copies the built programs `names` into a folder named by what they hold, beneath the
/// profile folder, unless a folder with that name is there, and gives that folder.
fn stage(profile_dir: &Path, names: &[&str]) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let root = profile_dir.join("yaar-st-programs");
    std::fs::create_dir_all(&root).expect("make the folder of the program copies");
    let own = root.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&own);
    std::fs::create_dir(&own).expect("make a folder for the program copies");

    let mut hasher = DefaultHasher::new();
    for name in names {
        // The file Cargo built may be out of its place for a moment: another build puts it
        // back.
        let source = profile_dir.join(name);
        let deadline = Instant::now() + DEADLINE;
        while let Err(e) = std::fs::copy(&source, own.join(name)) {
            assert!(
                Instant::now() < deadline,
                "could not copy {}: {e}",
                source.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        // Named by the copy and not the original, which may be replaced meanwhile.
        name.hash(&mut hasher);
        std::fs::read(own.join(name))
            .expect("read the copy")
            .hash(&mut hasher);
    }
    let folder = root.join(format!("{:016x}", hasher.finish()));
    if folder.exists() {
        let _ = std::fs::remove_dir_all(&own);
    } else if let Err(e) = std::fs::rename(&own, &folder) {
        // Another process made the same folder at the same moment: it holds the same.
        assert!(folder.exists(), "could not keep the program copies: {e}");
        let _ = std::fs::remove_dir_all(&own);
    }
    // Asked for now, so that no other process takes it away as unused.
    let _ = std::fs::File::open(&folder).and_then(|held| held.set_modified(SystemTime::now()));
    for old in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let unused = old
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|asked| asked.elapsed().is_ok_and(|ago| ago > KEPT_FOR));
        if unused && old.path() != folder {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    folder
}

/// A folder of the test's own, with a path short enough for a socket on every system.
/// Removed when dropped.
pub struct Place(PathBuf);

impl Place {
    /// A new, empty folder.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!("/tmp/yaar-st-{}-{n}", std::process::id()));
        for folder in ["home", "run", "tmp"] {
            std::fs::create_dir_all(path.join(folder)).expect("create the test's folder");
        }
        Self(path)
    }

    /// Where the Keeper of this test listens.
    pub fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// A command that runs `program` as a user of this folder alone: with the environment
    /// of this process cleared but for the search path, and the home, data, runtime and
    /// temporary folders of the user inside the folder. A program that keeps files in the
    /// user's folders, as the Keeper does with its database, then keeps them here, and no
    /// test touches the folders of whoever runs it.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        if let Some(search) = std::env::var_os("PATH") {
            command.env("PATH", search);
        }
        command
            .env("HOME", self.0.join("home"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_RUNTIME_DIR", self.0.join("run"))
            .env("TMPDIR", self.0.join("tmp"));
        command
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The scripted client: connects as a Window would and returns who answered.
/// Waits up to five seconds for a Keeper to listen.
pub fn attach(socket: &Path) -> (UnixStream, Hello) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(mut stream) = UnixStream::connect(socket)
            && let Ok(keeper) = handshake(&mut stream, &Hello::for_this_process("scripted-window"))
        {
            return (stream, keeper);
        }
        assert!(
            Instant::now() < deadline,
            "no Keeper at {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether a process with this id runs. One that has ended and waits to be collected does
/// not count: in a container whose first process collects nothing, a Keeper that outlived
/// its Window stays in that state for good.
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; it only asks whether the process can be signalled.
    let exists = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
    exists && !has_ended(pid)
}

/// Whether the process is a zombie, read from `/proc/<pid>/stat`: the state is the first
/// field after the name, and the name ends at the last `)`.
#[cfg(target_os = "linux")]
fn has_ended(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}

/// Elsewhere the system collects a process nobody waits for.
#[cfg(not(target_os = "linux"))]
fn has_ended(_pid: u32) -> bool {
    false
}

/// Waits until `done` holds, at most `limit`. Returns whether it came to hold.
pub fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}
