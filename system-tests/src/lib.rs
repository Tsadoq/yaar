mod client;
mod dialect;
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
pub use dialect::Dialect;
pub use processes::{Descriptor, descendants_of, descriptors_of};
pub use world::{
    APP_FILE_NAMES, APP_FILE_STARTS, OutsideAgent, Repo, Tap, World, without_agent_clis,
};

const KEEPER_SET: [&str; 5] = [
    "yaar-keeper",
    "yaar-claude-hook",
    "yaar-claude-tools",
    "yaar-pi-report",
    "standin-agent",
];

const WINDOW_SET: [&str; 1] = ["yaar-review"];

const LANGUAGE_SET: [&str; 1] = ["standin-lsp"];

/// Builds every program once per process: a test run builds only its own package's programs, so
/// one left by an earlier build may be stale. The path leads to a copy, since Cargo's file is
/// briefly missing at every build while tests run side by side.
pub fn program(name: &str) -> PathBuf {
    static STAGED: Mutex<Vec<(&'static [&'static str], PathBuf)>> = Mutex::new(Vec::new());
    static BUILD: Once = Once::new();
    let set: &'static [&'static str] = [&KEEPER_SET[..], &WINDOW_SET[..], &LANGUAGE_SET[..]]
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
        "yaar-adapter-pi",
        "-p",
        "yaar-system-tests",
        "-p",
        "yaar-navigator",
    ]);
    for name in KEEPER_SET.iter().chain(&WINDOW_SET).chain(&LANGUAGE_SET) {
        build.args(["--bin", name]);
    }
    if profile_dir.file_name().is_some_and(|p| p == "release") {
        build.arg("--release");
    }
    let output = build.stderr(Stdio::inherit()).output().expect("run cargo");
    assert!(output.status.success(), "building the programs failed");
    let reported = String::from_utf8_lossy(&output.stdout);
    for name in KEEPER_SET.iter().chain(&WINDOW_SET).chain(&LANGUAGE_SET) {
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

const KEPT_FOR: Duration = Duration::from_secs(3600);

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
        // The file Cargo built may be out of its place for a moment: another build puts it back.
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
        name.hash(&mut hasher);
        std::fs::read(own.join(name))
            .expect("read the copy")
            .hash(&mut hasher);
    }
    let folder = root.join(format!("{:016x}", hasher.finish()));
    if folder.exists() {
        let _ = std::fs::remove_dir_all(&own);
    } else if let Err(e) = std::fs::rename(&own, &folder) {
        assert!(folder.exists(), "could not keep the program copies: {e}");
        let _ = std::fs::remove_dir_all(&own);
    }
    // Marked as used now, so neither the sweep below nor another test process removes it as unused.
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

pub struct Place(PathBuf);

impl Place {
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

    pub fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

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

/// A process that has ended and waits to be collected does not count: in a container whose first
/// process collects nothing, it stays that way for good.
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; it only asks whether the process can be signalled.
    let exists = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
    exists && !has_ended(pid)
}

#[cfg(target_os = "linux")]
fn has_ended(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| ended_in(&status))
}

/// A zombie with no other thread left: the first thread shows as a zombie as soon as it exits,
/// while the others may still hold a lock or a listening socket.
#[cfg(any(target_os = "linux", test))]
fn ended_in(status: &str) -> bool {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
            .map(str::trim)
    };
    let zombie = field("State").is_some_and(|state| state.starts_with('Z'));
    let alone = field("Threads").and_then(|count| count.parse::<u32>().ok()) == Some(1);
    zombie && alone
}

#[cfg(not(target_os = "linux"))]
fn has_ended(_pid: u32) -> bool {
    false
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zombie_with_a_thread_still_to_run_has_not_ended() {
        assert!(!ended_in(
            "Name:\tyaar-keeper\nState:\tZ (zombie)\nTgid:\t7\nThreads:\t2\n"
        ));
    }

    #[test]
    fn a_zombie_with_no_thread_left_has_ended() {
        assert!(ended_in(
            "Name:\tyaar-keeper\nState:\tZ (zombie)\nTgid:\t7\nThreads:\t1\n"
        ));
    }

    #[test]
    fn a_process_that_is_not_a_zombie_has_not_ended() {
        assert!(!ended_in(
            "Name:\tyaar-keeper\nState:\tS (sleeping)\nTgid:\t7\nThreads:\t1\n"
        ));
        assert!(!ended_in("Name:\tx\nState:\tR (running)\nThreads:\t3\n"));
    }
}
