//! System tests (`B-BUILD`): the whole app through what a user or an agent touches.
//!
//! The tests in `tests/` start the built programs. This library holds what they share:
//! - [`World`], the folders, repositories and Keeper of one test, with [`Repo`] to drive
//!   git in it and [`Tap`] to read the requests a real Window sends;
//! - [`Client`], the scripted client that speaks the Link in place of the Window;
//! - [`descriptors_of`] and [`descendants_of`], which read from outside what a process
//!   holds open and what it started;
//! - [`program`], [`is_alive`] and [`wait_until`], and the bare [`attach`] and [`Place`]
//!   of the tests that need no world.
//!
//! The stand-in agent CLI joins it when the Switchboard exists.

mod client;
mod processes;
mod world;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use yaar_link::{Hello, handshake};

pub use client::{Client, DEADLINE};
pub use processes::{Descriptor, descendants_of, descriptors_of};
pub use world::{Repo, Tap, World};

/// The path of a built program of the workspace, building both programs first. Cargo
/// puts them in the profile folder, `target/debug` or the like; a test program runs from
/// `build/` or `deps/` somewhere beneath it, depending on the Cargo version.
///
/// They are built every time, once in each process that asks, and not only when one is
/// missing: a run of the tests builds a program only for the tests of its own package,
/// so one left in the folder by an earlier build can be older than the sources, and a
/// test would then meet a Window or a Keeper that is no longer the one in the tree.
/// Where both are up to date, the build does nothing.
pub fn program(name: &str) -> PathBuf {
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
    let path = profile_dir.join(name);
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut build = Command::new(cargo);
        build.args([
            "build",
            "--locked",
            "-p",
            "yaar-keeper",
            "-p",
            "yaar-window",
        ]);
        if profile_dir.file_name().is_some_and(|p| p == "release") {
            build.arg("--release");
        }
        let status = build.status().expect("run cargo");
        assert!(status.success(), "building the programs failed");
    });
    assert!(path.is_file(), "{} was not built", path.display());
    path
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
        for folder in ["home", "run"] {
            std::fs::create_dir_all(path.join(folder)).expect("create the test's folder");
        }
        Self(path)
    }

    /// Where the Keeper of this test listens.
    pub fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// A command that runs `program` as a user of this folder alone: with the environment
    /// of this process cleared but for the search path, and the home, data and runtime
    /// folders of the user inside the folder. A program that keeps files in the user's
    /// folders, as the Keeper does with its database, then keeps them here, and no test
    /// touches the folders of whoever runs it.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        if let Some(search) = std::env::var_os("PATH") {
            command.env("PATH", search);
        }
        command
            .env("HOME", self.0.join("home"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_RUNTIME_DIR", self.0.join("run"));
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
