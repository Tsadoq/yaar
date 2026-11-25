//! Pseudo-terminals and their programs. The Dock knows nothing of the Link, Windows or agents.

mod marks;
mod pty;
mod queue;
mod ring;
mod session;
mod terminal;

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use terminal::Terminal;
use yaar_link::user_shell;

pub const STOP_STEP: Duration = Duration::from_secs(1);

const STOP_SIGNALS: [libc::c_int; 3] = [libc::SIGHUP, libc::SIGTERM, libc::SIGKILL];

const RESET_SIGNALS: [libc::c_int; 11] = [
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGPIPE,
    libc::SIGCHLD,
    libc::SIGCONT,
    libc::SIGTSTP,
    libc::SIGTTIN,
    libc::SIGTTOU,
    libc::SIGWINCH,
];

/// `TERM` names a description every system has, which the app's emulator answers.
pub const TERMINAL_VARIABLES: [(&str, &str); 4] = [
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "yaar"),
    ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
];

pub const LEFT_OUT_VARIABLES: [&str; 28] = [
    "TERM_SESSION_ID",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "KITTY_PUBLIC_KEY",
    "KITTY_LISTEN_ON",
    "GHOSTTY_RESOURCES_DIR",
    "GHOSTTY_BIN_DIR",
    "GHOSTTY_SHELL_FEATURES",
    "WEZTERM_PANE",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_UNIX_SOCKET",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "VTE_VERSION",
    "WT_SESSION",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "CLAUDE_CODE_SSE_PORT",
    "ENABLE_IDE_INTEGRATION",
    "VSCODE_INJECTION",
    "VSCODE_IPC_HOOK_CLI",
    "VSCODE_GIT_IPC_HANDLE",
    "ZED_TERM",
];

pub const MAX_WAITING_INPUT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn random() -> io::Result<TerminalId> {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(TerminalId(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A value under 1 is taken as 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Exited(i32),
    Signalled(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Never empty.
    Output(Vec<u8>),
    /// The run `Start::marks[n]` was found, also across two reads. Told after the
    /// [`Event::Output`] holding its last byte, once per find.
    Mark(usize),
    Ended(Ending),
}

pub const RING_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Variable {
        name: OsString,
        value: OsString,
    },
    /// `path` is added after the Keeper's value with `:`, or is the whole value where it holds
    /// none.
    PathList {
        name: OsString,
        path: PathBuf,
    },
}

/// Runs on a thread of the Dock, one event at a time; the output waits while it runs.
pub type Events = Box<dyn FnMut(Event) + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub folder: PathBuf,
    pub size: Size,
    pub env: Vec<(OsString, OsString)>,
    pub marks: Vec<Vec<u8>>,
}

impl Start {
    pub fn shell(folder: PathBuf, size: Size) -> Start {
        Start::program(user_shell(), vec![OsString::from("-l")], folder, size)
    }

    pub fn launcher(folder: PathBuf, size: Size, command: &str) -> Start {
        let args = ["-l", "-i", "-c", command].map(OsString::from).to_vec();
        Start::program(user_shell(), args, folder, size)
    }

    pub fn program(program: PathBuf, args: Vec<OsString>, folder: PathBuf, size: Size) -> Start {
        Start {
            program,
            args,
            folder,
            size,
            env: Vec::new(),
            marks: Vec::new(),
        }
    }

    pub fn with(self, entries: &[Entry]) -> Start {
        self.with_environment(entries, |name| std::env::var_os(name))
    }

    fn with_environment(
        mut self,
        entries: &[Entry],
        keeper: impl Fn(&OsStr) -> Option<OsString>,
    ) -> Start {
        let mut added: HashMap<OsString, OsString> = HashMap::new();
        for entry in entries {
            let (name, value) = match entry {
                Entry::Variable { name, value } => (name.clone(), value.clone()),
                Entry::PathList { name, path } => {
                    let held = held_value(&added, &keeper, name);
                    let mut value = held;
                    if !value.is_empty() {
                        value.push(":");
                    }
                    value.push(path);
                    (name.clone(), value)
                }
            };
            added.insert(name.clone(), value.clone());
            self.env.push((name, value));
        }
        self
    }
}

fn held_value(
    added: &HashMap<OsString, OsString>,
    keeper: &impl Fn(&OsStr) -> Option<OsString>,
    name: &OsStr,
) -> OsString {
    added
        .get(name)
        .cloned()
        .or_else(|| keeper(name))
        .unwrap_or_default()
}

#[derive(Debug)]
pub enum DockError {
    Start {
        program: PathBuf,
        reason: io::Error,
    },
    /// No terminal by this id: never opened, its program ended, or [`Dock::stop_all`] is under way.
    UnknownTerminal(TerminalId),
    TakenId(TerminalId),
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start { program, reason } => {
                write!(f, "{} could not be started: {reason}", program.display())
            }
            Self::UnknownTerminal(terminal) => write!(f, "there is no terminal {terminal}"),
            Self::TakenId(terminal) => write!(f, "the terminal {terminal} exists already"),
        }
    }
}

impl std::error::Error for DockError {}

pub(crate) struct Inner {
    state: Mutex<State>,
    told: Condvar,
    /// Separate from `state`: finding who is left looks at every process of the system.
    left_behind: Mutex<Vec<session::Remembered>>,
}

#[derive(Default)]
struct State {
    taken: HashSet<TerminalId>,
    live: HashMap<TerminalId, Arc<Terminal>>,
    untold: usize,
    opening: usize,
    stopping: usize,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn forget(&self, id: &TerminalId) {
        self.state().live.remove(id);
    }

    pub(crate) fn told(&self) {
        let mut state = self.state();
        state.untold -= 1;
        drop(state);
        self.told.notify_all();
    }

    pub(crate) fn leave_behind(&self, session: libc::pid_t) {
        let mut left = self
            .left_behind
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *left = left
            .iter()
            .filter_map(session::Remembered::renewed)
            .collect();
        left.extend(session::Remembered::of(session));
    }

    /// A session whose last process is gone is let go here, before the system reuses its id for a
    /// new session that a stop would signal.
    fn left_behind_sessions(&self) -> Vec<libc::pid_t> {
        let mut left = self
            .left_behind
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *left = left
            .iter()
            .filter_map(session::Remembered::renewed)
            .collect();
        left.iter().map(session::Remembered::session).collect()
    }

    fn live(&self, id: &TerminalId) -> Result<Arc<Terminal>, DockError> {
        self.state()
            .live
            .get(id)
            .cloned()
            .ok_or_else(|| DockError::UnknownTerminal(id.clone()))
    }
}

pub struct Dock {
    inner: Arc<Inner>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Dock {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Dock {
        Dock {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                told: Condvar::new(),
                left_behind: Mutex::new(Vec::new()),
            }),
            threads: Mutex::new(Vec::new()),
        }
    }

    pub fn open(&self, id: TerminalId, start: Start, events: Events) -> Result<(), DockError> {
        {
            let mut state = self.inner.state();
            if state.stopping > 0 {
                return Err(DockError::Start {
                    program: start.program,
                    reason: io::Error::other("the Dock is stopping every terminal"),
                });
            }
            if !state.taken.insert(id.clone()) {
                return Err(DockError::TakenId(id));
            }
            state.opening += 1;
        }
        let made = self.start(&id, &start, events);
        {
            let mut state = self.inner.state();
            state.opening -= 1;
            if made.is_err() {
                state.taken.remove(&id);
            }
        }
        self.inner.told.notify_all();
        made.map_err(|reason| DockError::Start {
            program: start.program,
            reason,
        })
    }

    fn start(&self, id: &TerminalId, start: &Start, events: Events) -> io::Result<()> {
        let (master, slave) = pty::open_pair(start.size)?;
        pty::set_nonblocking(master.as_raw_fd())?;
        let wake = pty::pipe()?;

        let mut command = Command::new(&start.program);
        command.args(&start.args).current_dir(&start.folder);
        for name in LEFT_OUT_VARIABLES {
            command.env_remove(name);
        }
        command.envs(TERMINAL_VARIABLES);
        command.envs(start.env.iter().map(|(name, value)| (name, value)));
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave.try_clone()?));
        #[cfg(target_os = "macos")]
        let descriptors = pty::descriptor_limit();
        // SAFETY: the closure runs between fork and exec and calls only `setsid`, `ioctl`, `signal`
        // and `fcntl`: no allocation, no lock.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // A daemonised Keeper may ignore signals and a program inherits that: it would not
                // take the hang-up of a closing terminal.
                for signal in RESET_SIGNALS {
                    libc::signal(signal, libc::SIG_DFL);
                }
                // std on macOS makes some descriptors and only then marks them close-on-exec, so
                // one made by another thread at that moment reaches here open. Marked, not closed:
                // std reports a failed `exec` through a pipe of its own.
                #[cfg(target_os = "macos")]
                for descriptor in 3..descriptors {
                    libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        // The command holds the slave: while it or ours is open here, the program's end would not
        // show on the master.
        drop(command);
        drop(slave);
        let child = spawned?;

        let terminal = Arc::new(Terminal::new(
            master,
            child.id() as libc::pid_t,
            wake,
            start.marks.clone(),
        ));
        let mut started = Vec::with_capacity(3);
        let served = self.serve(id, &terminal, child, events, &mut started);
        if served.is_err() && !started.is_empty() {
            // SAFETY: `kill` only sends. The waiter thread, which did start, ends the others.
            unsafe { libc::kill(terminal.pid(), libc::SIGKILL) };
        }
        let mut threads = self.threads();
        threads.retain(|thread| !thread.is_finished());
        threads.extend(started);
        served
    }

    fn serve(
        &self,
        id: &TerminalId,
        terminal: &Arc<Terminal>,
        child: std::process::Child,
        events: Events,
        started: &mut Vec<JoinHandle<()>>,
    ) -> io::Result<()> {
        let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(format!("dock-{name}"))
                .spawn(work)
        };
        let waiting = {
            let terminal = Arc::clone(terminal);
            spawn("waiter", Box::new(move || terminal.wait_for_program(child)))
        };
        match waiting {
            Ok(thread) => started.push(thread),
            Err(error) => {
                // SAFETY: `kill` only sends, to our own program, not yet collected.
                unsafe {
                    libc::kill(terminal.pid(), libc::SIGKILL);
                    libc::waitpid(terminal.pid(), std::ptr::null_mut(), 0);
                }
                return Err(error);
            }
        }
        let writing = {
            let terminal = Arc::clone(terminal);
            spawn("writer", Box::new(move || terminal.write_input()))?
        };
        started.push(writing);
        {
            let mut state = self.inner.state();
            state.live.insert(id.clone(), Arc::clone(terminal));
            state.untold += 1;
        }
        let reading = {
            let (terminal, inner, id) = (Arc::clone(terminal), Arc::clone(&self.inner), id.clone());
            spawn(
                "reader",
                Box::new(move || terminal.tell_events(&inner, &id, events)),
            )
        };
        match reading {
            Ok(thread) => {
                started.push(thread);
                Ok(())
            }
            Err(error) => {
                let mut state = self.inner.state();
                state.live.remove(id);
                state.untold -= 1;
                Err(error)
            }
        }
    }

    fn threads(&self) -> MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.threads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Returns at once. Past [`MAX_WAITING_INPUT`] waiting bytes, more is dropped silently.
    pub fn write(&self, terminal: &TerminalId, bytes: &[u8]) -> Result<(), DockError> {
        if self.inner.live(terminal)?.type_in(bytes) {
            Ok(())
        } else {
            Err(DockError::UnknownTerminal(terminal.clone()))
        }
    }

    pub fn resize(&self, terminal: &TerminalId, size: Size) -> Result<(), DockError> {
        pty::set_size(self.inner.live(terminal)?.master(), size)
            .map_err(|_| DockError::UnknownTerminal(terminal.clone()))
    }

    pub fn running(&self) -> usize {
        self.inner.state().live.len()
    }

    /// Calls `attach` with the kept output and whether older output was dropped, while no event is
    /// told: later output reaches what `attach` sets up, none twice. It must return at once and not
    /// call the Dock, and the caller must hold no lock that the [`Events`] take.
    pub fn attach(
        &self,
        terminal: &TerminalId,
        attach: impl FnOnce(&[u8], bool),
    ) -> Result<(), DockError> {
        self.inner.live(terminal)?.attach(attach);
        Ok(())
    }

    pub fn redraw(&self, id: &TerminalId) -> Result<(), DockError> {
        let terminal = self.inner.live(id)?;
        // SAFETY: `tcgetpgrp` only reads, from a descriptor the terminal keeps open.
        let group = unsafe { libc::tcgetpgrp(terminal.master()) };
        if group <= 0 {
            return Err(DockError::UnknownTerminal(id.clone()));
        }
        // SAFETY: `kill` only sends; `ESRCH` from a vanished group is fine.
        unsafe { libc::kill(-group, libc::SIGWINCH) };
        Ok(())
    }

    /// Hangs up, then after [`STOP_STEP`] terminates, then kills every process of one terminal's
    /// session, on a background thread so the caller does not wait. Returns `false` if no live
    /// terminal has `id`.
    pub fn stop_one(&self, id: &TerminalId) -> bool {
        let Some(terminal) = self.inner.state().live.get(id).cloned() else {
            return false;
        };
        terminal.stop_typing();
        let session = terminal.pid();
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("dock-stopper".to_owned())
            .spawn(move || {
                stop_sessions(&[session]);
                inner.left_behind_sessions();
            });
        if let Ok(thread) = spawned {
            let mut threads = self.threads();
            threads.retain(|thread| !thread.is_finished());
            threads.push(thread);
        } else {
            for pid in session::members(&[session]) {
                // SAFETY: `kill` only sends; `ESRCH` from a vanished process is fine.
                unsafe { libc::kill(pid, libc::SIGHUP) };
            }
        }
        true
    }

    /// Hangs up, then after [`STOP_STEP`] terminates, then kills every process of each terminal's
    /// session. A process that left its session, as a daemon does, is not reached.
    pub fn stop_all(&self) {
        let sessions = self.begin_stopping();
        let _stopping = Stopping(&self.inner);
        stop_sessions(&sessions);
        {
            let mut state = self.inner.state();
            while state.untold > 0 {
                state = self
                    .inner
                    .told
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        // Called for its side effect: lets go now of the sessions the stop emptied, not at the next
        // stop.
        self.inner.left_behind_sessions();
    }

    fn begin_stopping(&self) -> Vec<libc::pid_t> {
        let terminals: Vec<Arc<Terminal>> = {
            let mut state = self.inner.state();
            state.stopping += 1;
            while state.opening > 0 {
                state = self
                    .inner
                    .told
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            state.live.values().cloned().collect()
        };
        for terminal in &terminals {
            terminal.stop_typing();
        }
        let mut sessions: Vec<libc::pid_t> =
            terminals.iter().map(|terminal| terminal.pid()).collect();
        // Read after the live ones: a terminal enters this list before it is forgotten, so none is
        // missed.
        sessions.extend(self.inner.left_behind_sessions());
        sessions.sort_unstable();
        sessions.dedup();
        sessions
    }
}

fn stop_sessions(sessions: &[libc::pid_t]) {
    for signal in STOP_SIGNALS {
        let members = session::members(sessions);
        if members.is_empty() {
            break;
        }
        for pid in members {
            // SAFETY: `kill` only sends; `ESRCH` from a vanished process is fine.
            unsafe { libc::kill(pid, signal) };
        }
        let deadline = Instant::now() + STOP_STEP;
        while !session::members(sessions).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

struct Stopping<'a>(&'a Inner);

impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        self.0.state().stopping -= 1;
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        self.stop_all();
        for thread in std::mem::take(&mut *self.threads()) {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_32_lowercase_hexadecimal_digits_and_two_are_not_the_same() {
        let (one, other) = (
            TerminalId::random().expect("an id"),
            TerminalId::random().expect("an id"),
        );
        for id in [&one, &other] {
            assert_eq!(id.as_str().len(), 32, "{id}");
            assert!(
                id.as_str()
                    .bytes()
                    .all(|digit| digit.is_ascii_digit() || (b'a'..=b'f').contains(&digit)),
                "{id}"
            );
        }
        assert_ne!(one, other);
        assert_eq!(one.to_string(), one.as_str());
    }

    struct Leader(std::process::Child);

    impl Drop for Leader {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn session_leader() -> Leader {
        let mut command = Command::new("sleep");
        command.arg("300");
        // SAFETY: `setsid` is safe between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Leader(command.spawn().expect("start a session leader"))
    }

    #[test]
    fn a_session_that_a_stop_has_emptied_is_no_longer_kept_to_be_signalled() {
        let dock = Dock::new();
        let mut leader = session_leader();
        let session = leader.0.id() as libc::pid_t;
        dock.inner.leave_behind(session);
        assert_eq!(dock.inner.left_behind_sessions(), vec![session]);

        dock.stop_all();
        leader.0.wait().expect("collect the leader");
        // Read the list directly: `left_behind_sessions` would itself let the emptied session go.
        assert!(
            dock.inner.left_behind.lock().expect("the list").is_empty(),
            "the emptied session is still kept"
        );
    }

    #[test]
    fn a_stop_renews_the_sessions_left_behind_before_it_signals_them() {
        let dock = Dock::new();
        let leader = session_leader();
        let session = leader.0.id() as libc::pid_t;
        let forged = session::Remembered::of(session)
            .expect("the leader is in its session")
            .begun_at_another_time();
        dock.inner
            .left_behind
            .lock()
            .expect("the list")
            .push(forged);

        let stopping = Instant::now();
        dock.stop_all();
        let mut leader = leader;
        assert!(
            leader.0.try_wait().expect("ask the leader").is_none(),
            "the stop signalled a session that was not the one left behind"
        );
        assert!(
            stopping.elapsed() < STOP_STEP,
            "took {:?}",
            stopping.elapsed()
        );
    }

    #[test]
    fn a_session_that_is_not_empty_stays_kept_through_a_stop_that_does_not_reach_it() {
        let dock = Dock::new();
        let mut leader = session_leader();
        let session = leader.0.id() as libc::pid_t;
        dock.inner.leave_behind(session);
        assert_eq!(dock.inner.left_behind_sessions(), vec![session]);
        assert_eq!(dock.inner.left_behind_sessions(), vec![session]);
        leader.0.kill().expect("kill the leader");
        leader.0.wait().expect("collect the leader");
        assert!(dock.inner.left_behind_sessions().is_empty());
    }

    fn start() -> Start {
        Start::program(
            PathBuf::from("/bin/true"),
            Vec::new(),
            PathBuf::from("/"),
            Size {
                columns: 80,
                rows: 24,
            },
        )
    }

    fn env_of(start: &Start) -> Vec<(String, String)> {
        start
            .env
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn a_list_variable_gets_the_path_after_the_keepers_value_or_alone_where_it_holds_none() {
        let entry = [Entry::PathList {
            name: "DIRS".into(),
            path: PathBuf::from("/app/plugin"),
        }];
        for (held, value) in [
            (Some("/user/own"), "/user/own:/app/plugin"),
            (Some("/a:/b"), "/a:/b:/app/plugin"),
            // An empty value would put the current folder first on the list.
            (Some(""), "/app/plugin"),
            (None, "/app/plugin"),
        ] {
            let start = start().with_environment(&entry, |name| {
                assert_eq!(name, "DIRS");
                held.map(OsString::from)
            });
            assert_eq!(env_of(&start), [pair("DIRS", value)], "{held:?}");
        }
    }

    #[test]
    fn entries_come_after_what_the_start_holds_and_two_for_one_variable_build_on_each_other() {
        let mut before = start();
        before.env.push(("KEPT".into(), "1".into()));
        let with = before.with_environment(
            &[
                Entry::Variable {
                    name: "YAAR_TERMINAL".into(),
                    value: "abc".into(),
                },
                Entry::PathList {
                    name: "DIRS".into(),
                    path: PathBuf::from("/x"),
                },
                Entry::PathList {
                    name: "DIRS".into(),
                    path: PathBuf::from("/y"),
                },
            ],
            |name| (name == "DIRS").then(OsString::new),
        );
        assert_eq!(
            env_of(&with),
            [
                pair("KEPT", "1"),
                pair("YAAR_TERMINAL", "abc"),
                pair("DIRS", "/x"),
                pair("DIRS", "/x:/y"),
            ]
        );
    }

    #[test]
    fn a_launcher_runs_the_users_shell_as_an_interactive_login_shell_told_one_command() {
        let launcher = Start::launcher(
            PathBuf::from("/work"),
            Size {
                columns: 100,
                rows: 30,
            },
            "claude --flag 'a b'",
        );
        assert_eq!(launcher.program, user_shell());
        assert_eq!(
            launcher.args,
            ["-l", "-i", "-c", "claude --flag 'a b'"].map(OsString::from)
        );
        assert_eq!(launcher.folder, PathBuf::from("/work"));
        assert_eq!((launcher.size.columns, launcher.size.rows), (100, 30));
    }

    #[test]
    fn stop_one_terminates_one_running_terminal_and_emits_ended_without_blocking_or_touching_others()
     {
        use std::sync::mpsc;

        let dock = Dock::new();
        let unknown = TerminalId::random().expect("an id");
        assert!(!dock.stop_one(&unknown));

        let sleeper = || {
            Start::program(
                PathBuf::from("sleep"),
                vec![OsString::from("300")],
                PathBuf::from("/"),
                Size {
                    columns: 80,
                    rows: 24,
                },
            )
        };
        let (first_tx, first_rx) = mpsc::channel();
        let (second_tx, second_rx) = mpsc::channel();
        let first = TerminalId::random().expect("first id");
        let second = TerminalId::random().expect("second id");
        dock.open(
            first.clone(),
            sleeper(),
            Box::new(move |event| {
                let _ = first_tx.send(event);
            }),
        )
        .expect("open first");
        dock.open(
            second.clone(),
            sleeper(),
            Box::new(move |event| {
                let _ = second_tx.send(event);
            }),
        )
        .expect("open second");
        assert_eq!(dock.running(), 2);

        let started = Instant::now();
        assert!(dock.stop_one(&first));
        assert!(
            started.elapsed() < STOP_STEP,
            "stop_one blocked for {:?}",
            started.elapsed()
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let ended = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for Event::Ended");
            if let Ok(Event::Ended(how)) = first_rx.recv_timeout(left) {
                break how;
            }
        };
        assert!(matches!(ended, Ending::Signalled(_) | Ending::Exited(_)));
        assert_eq!(dock.running(), 1);
        assert!(
            second_rx.try_recv().is_err(),
            "stopping one terminal must not end another"
        );
    }
}
