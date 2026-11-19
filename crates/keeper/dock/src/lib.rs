//! Dock (`H-TERM`).
//!
//! Owns: pseudo-terminals and their children, the ring of recent output, the environment of each shell, plain quit and stop-everything quit, the watch for the turn marks in the terminal bytes.
//!
//! Surface:
//! - [`Dock`], which opens terminals, hands each the bytes typed for it, tells it its new
//!   size, tells what its program writes and how the program ended, and stops them all;
//! - [`TerminalId::random`], the id a caller gives a terminal before it is opened, so
//!   that the caller can name it in the program's environment;
//! - [`Start`], what a terminal runs, with [`Start::shell`] for the user's shell and
//!   [`Start::program`] for any other program, and [`user_shell`]; [`Start::with`], the
//!   [`Entry`]s of the app a shell of the app gets on top of the user's own environment;
//! - [`Size`], [`TerminalId`], [`Event`], [`Ending`] and [`DockError`], in the Dock's own
//!   words: the Keeper program puts them into those of the Link;
//! - [`Dock::attach`], the newest output of a terminal, kept in a ring of [`RING_BYTES`],
//!   handed to whoever attaches with nothing lost or told twice after it, and
//!   [`Dock::redraw`], which asks a terminal's program to draw its screen again.
//!
//! The Dock knows nothing of the Link, of Windows or of agents. It runs programs on
//! pseudo-terminals and passes bytes both ways. Of a program's environment it decides
//! only what says which terminal the program runs in ([`TERMINAL_VARIABLES`],
//! [`LEFT_OUT_VARIABLES`]); the rest comes from the caller through [`Start::env`] and
//! [`Start::with`]. Of what a program writes it keeps the newest [`RING_BYTES`], and it
//! looks for the runs of bytes the caller asked it to watch for, [`Start::marks`],
//! telling each as [`Event::Mark`]: what they mean is the caller's to say.
//!
//! No interface code belongs here.

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
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use terminal::Terminal;

/// How long each step of [`Dock::stop_all`] waits for the programs before the next,
/// harder one: hang-up, then terminate, then kill.
pub const STOP_STEP: Duration = Duration::from_secs(1);

/// The signals of [`Dock::stop_all`], in the order they are sent.
const STOP_SIGNALS: [libc::c_int; 3] = [libc::SIGHUP, libc::SIGTERM, libc::SIGKILL];

/// The signals a program starts with their default action, whatever the Keeper made of
/// them.
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

/// The variables every program of a terminal finds set, whatever the Keeper's own
/// environment holds: what kind of terminal it runs in. `TERM` names a description every
/// system has, and the app's emulator answers as the terminal that name stands for.
/// `TERM_PROGRAM_VERSION` is the app's version, which every crate of the workspace has.
///
/// They live here, not with the caller, because they describe the terminal the Dock
/// makes, the same for every program in it.
pub const TERMINAL_VARIABLES: [(&str, &str); 4] = [
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "yaar"),
    ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
];

/// Variables of the Keeper's own environment that no program of a terminal gets: each
/// names another terminal, a multiplexer or an editor's connection that the Keeper was
/// started from. A program that found them beside [`TERMINAL_VARIABLES`] would take the
/// app's terminal for that one, and send it what only that one understands.
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
    // An editor's connection, or another editor's terminal: a Keeper started from an
    // editor must not hand it on, and the app uses no editor connection.
    "CLAUDE_CODE_SSE_PORT",
    "ENABLE_IDE_INTEGRATION",
    "VSCODE_INJECTION",
    "VSCODE_IPC_HOOK_CLI",
    "VSCODE_GIT_IPC_HANDLE",
    "ZED_TERM",
];

/// The most bytes that wait for one program to read them; see [`Dock::write`].
pub const MAX_WAITING_INPUT: usize = 16 * 1024 * 1024;

/// Names one terminal of the Dock: 32 lowercase hexadecimal digits from `/dev/urandom`,
/// made by [`TerminalId::random`] before the terminal is opened, so that no two terminals
/// share one, also across restarts of the Keeper.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    /// The id with this text, as the Keeper hands it back.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// A new id: 16 bytes of `/dev/urandom` as 32 lowercase hexadecimal digits. Fails
    /// only when `/dev/urandom` cannot be read.
    pub fn random() -> io::Result<TerminalId> {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(TerminalId(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The size of a terminal in character cells. A value under 1 is taken as 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    /// Characters on a line.
    pub columns: u16,
    /// Lines on the screen.
    pub rows: u16,
}

/// How the program of a terminal ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It exited by itself, with this status.
    Exited(i32),
    /// A signal ended it: this one.
    Signalled(i32),
}

/// What the Dock tells about one terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The program wrote these bytes. Never empty.
    Output(Vec<u8>),
    /// The run of bytes `Start::marks[n]` was found in what the program wrote, also when
    /// it came in two reads. Told after the [`Event::Output`] that held its last byte,
    /// once for each time it was found.
    Mark(usize),
    /// The program has ended and nothing more is told of this terminal. It comes once,
    /// last, after every [`Event::Output`] of the terminal.
    Ended(Ending),
}

/// How many of the newest bytes a program wrote the Dock keeps for each terminal, for a
/// Window that attaches later: 1 MiB, which P4's measures found enough to give a screen
/// and its recent history back whole, and little against the memory limit.
pub const RING_BYTES: usize = 1024 * 1024;

/// One thing a shell of the app gets on top of the user's own environment, beside what
/// says which terminal it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A variable set to this value, as the address of the Keeper's socket or the id of
    /// the terminal.
    Variable {
        /// The variable.
        name: OsString,
        /// Its value.
        value: OsString,
    },
    /// A variable whose value is a list of paths parted by `:`: `path` is added at its
    /// end, after what the Keeper's own environment holds, or is its whole value where it
    /// holds none.
    PathList {
        /// The variable.
        name: OsString,
        /// The path added.
        path: PathBuf,
    },
    /// A folder put first on the search path, `PATH`, before what the Keeper's own
    /// environment holds there.
    SearchFirst(PathBuf),
}

/// Where the events of one terminal go. Called on a thread of the Dock's own, one event
/// at a time and in order. It should hand the event on and return: the program's output
/// waits while it runs.
pub type Events = Box<dyn FnMut(Event) + Send + 'static>;

/// What a terminal runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The program, by its path.
    pub program: PathBuf,
    /// Its arguments, without the program's own name.
    pub args: Vec<OsString>,
    /// The folder it starts in.
    pub folder: PathBuf,
    /// The size of its terminal at the start.
    pub size: Size,
    /// Variables set for the program on top of the Keeper's own environment and of
    /// [`TERMINAL_VARIABLES`]: what the caller adds, as the id of the terminal. Of two
    /// with one name, the later wins.
    pub env: Vec<(OsString, OsString)>,
    /// The runs of bytes to watch for in what the program writes, each told as
    /// [`Event::Mark`] by its index here. Empty: none is watched for.
    pub marks: Vec<Vec<u8>>,
}

impl Start {
    /// The user's shell, [`user_shell`], as a login shell, its one argument `-l`, in
    /// `folder`.
    pub fn shell(folder: PathBuf, size: Size) -> Start {
        Start::program(user_shell(), vec![OsString::from("-l")], folder, size)
    }

    /// The user's shell, [`user_shell`], as an interactive login shell told to run
    /// `command`, with the arguments `-l`, `-i`, `-c` and the command, in `folder`: what
    /// a launcher of an agent CLI runs, so that the CLI finds what it would find typed in
    /// a shell of the app. The terminal ends when the command does.
    pub fn launcher(folder: PathBuf, size: Size, command: &str) -> Start {
        let args = ["-l", "-i", "-c", command].map(OsString::from).to_vec();
        Start::program(user_shell(), args, folder, size)
    }

    /// Any program, with these arguments, in `folder`.
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

    /// This start with the app's `entries` added to [`Start::env`], each worked out from
    /// the Keeper's own environment: a [`Entry::Variable`] as it is, a [`Entry::PathList`]
    /// added after that variable's value, a [`Entry::SearchFirst`] put before `PATH`'s.
    /// Two entries of one call that name one variable build on each other.
    /// Nothing else of the environment changes: the program's environment differs from
    /// the Keeper's by these entries, by [`TERMINAL_VARIABLES`] and by
    /// [`LEFT_OUT_VARIABLES`] alone.
    pub fn with(self, entries: &[Entry]) -> Start {
        self.with_environment(entries, |name| std::env::var_os(name))
    }

    /// [`Start::with`], with the look at the Keeper's environment handed in. Entries of
    /// one call that name one variable build on each other, so that two folders put
    /// first on `PATH` are both there; what the Keeper holds is the base of the first.
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
                Entry::SearchFirst(path) => {
                    let name = OsString::from("PATH");
                    let mut value = path.as_os_str().to_owned();
                    let held = held_value(&added, &keeper, &name);
                    if !held.is_empty() {
                        value.push(":");
                        value.push(held);
                    }
                    (name, value)
                }
            };
            added.insert(name.clone(), value.clone());
            self.env.push((name, value));
        }
        self
    }
}

/// What `name` holds for the entries of one call of [`Start::with`]: what an earlier
/// entry of the call gave it, else what the Keeper's environment holds, else nothing.
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

/// The user's shell: `SHELL` from the Keeper's environment when it names, by an absolute
/// path, a file that can be run; else the shell of the user's entry in the system's list
/// of users; else `/bin/sh`.
pub fn user_shell() -> PathBuf {
    shell_from(std::env::var_os("SHELL"), passwd_shell)
}

/// The choice of [`user_shell`], with the value of `SHELL` and the look at the system's
/// list of users handed in.
fn shell_from(variable: Option<OsString>, passwd: impl FnOnce() -> Option<PathBuf>) -> PathBuf {
    variable
        .map(PathBuf::from)
        .filter(|shell| can_run(shell))
        .or_else(|| passwd().filter(|shell| can_run(shell)))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// An absolute path to a file with an execute bit.
fn can_run(path: &Path) -> bool {
    path.is_absolute()
        && std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The shell in the entry of the user the Keeper runs as, in the system's list of users.
fn passwd_shell() -> Option<PathBuf> {
    // SAFETY: `getpwuid_r` fills the entry and the buffer it is given, and the entry's
    // strings point into that buffer, which outlives their use below.
    unsafe {
        let size = libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX);
        let mut buffer = vec![0 as libc::c_char; if size > 0 { size as usize } else { 1024 }];
        loop {
            let mut entry: libc::passwd = std::mem::zeroed();
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(
                libc::geteuid(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            );
            if rc == libc::ERANGE && buffer.len() < 1 << 20 {
                buffer.resize(buffer.len() * 2, 0);
                continue;
            }
            if rc != 0 || found.is_null() || entry.pw_shell.is_null() {
                return None;
            }
            let shell = std::ffi::CStr::from_ptr(entry.pw_shell).to_bytes();
            return (!shell.is_empty())
                .then(|| PathBuf::from(OsString::from(std::ffi::OsStr::from_bytes(shell))));
        }
    }
}

/// Why the Dock did not do what it was asked.
#[derive(Debug)]
pub enum DockError {
    /// The terminal could not be made, or its program could not be started: it is not
    /// there, it cannot be run, or its folder is not there.
    Start {
        /// The program that was to run.
        program: PathBuf,
        /// What the system said.
        reason: io::Error,
    },
    /// The Dock holds no terminal by this id: it never opened one, its program has
    /// ended, or [`Dock::stop_all`] is under way and the terminal no longer takes what is
    /// typed to it, although its program may still be there.
    UnknownTerminal(TerminalId),
    /// The Dock holds a terminal by this id already, running or ended: an id names one
    /// terminal only.
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

/// What the Dock and the threads of its terminals share.
pub(crate) struct Inner {
    state: Mutex<State>,
    /// Told when a terminal's events are over, and when a call of `open` is done.
    told: Condvar,
    /// The sessions of terminals whose end was told while a process of theirs was still
    /// there. Kept apart from `state` because finding who is left takes a look at every
    /// process of the system. Only sessions that are not empty are kept: see
    /// [`Inner::left_behind_sessions`].
    left_behind: Mutex<Vec<session::Remembered>>,
}

#[derive(Default)]
struct State {
    /// Every id given to `open` that made a terminal, and an `open` that is making one.
    taken: HashSet<TerminalId>,
    /// The terminals whose end has not been told.
    live: HashMap<TerminalId, Arc<Terminal>>,
    /// Terminals whose thread of events has not finished.
    untold: usize,
    /// How many calls of `open` are making a terminal.
    opening: usize,
    /// How many calls of `stop_all` are running: no terminal is opened meanwhile.
    stopping: usize,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The terminal no longer runs a program.
    pub(crate) fn forget(&self, id: &TerminalId) {
        self.state().live.remove(id);
    }

    /// A terminal's events are over.
    pub(crate) fn told(&self) {
        let mut state = self.state();
        state.untold -= 1;
        drop(state);
        self.told.notify_all();
    }

    /// Notes the session of a terminal whose program is gone when a process is left in it.
    pub(crate) fn leave_behind(&self, session: libc::pid_t) {
        let mut left = self
            .left_behind
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Sessions that have emptied since are let go, so the list stays as long as what
        // is really left behind.
        *left = left
            .iter()
            .filter_map(session::Remembered::renewed)
            .collect();
        left.extend(session::Remembered::of(session));
    }

    /// The sessions with a process left behind by a terminal that has ended, as they are
    /// now. A session whose last process is gone is let go here, and it is let go
    /// before the system can give its id to a process that starts a session of its own,
    /// which a stop would then signal as if it were ours.
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

/// The terminals of the Keeper and the programs in them.
///
/// Each terminal is a pseudo-terminal whose program runs in a session of its own, with
/// the terminal as its controlling terminal. A thread of the Dock reads what the program
/// writes and hands it to the terminal's [`Events`]; what is written to a terminal waits
/// in a queue of its own, so that no call of the Dock waits for a program.
///
/// Shared by the Keeper's threads: every call takes `&self`. Dropping it stops every
/// program, as [`Dock::stop_all`] does.
pub struct Dock {
    inner: Arc<Inner>,
    /// Three threads for each terminal opened: one reads the program's output, one writes
    /// what is typed to it, one collects it.
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Dock {
    /// A Dock that holds no terminal.
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

    /// Opens a terminal of `start.size` under the name `id`, which the caller made with
    /// [`TerminalId::random`], and starts `start.program` in it. From then on `events` is
    /// told what the program writes, and at last how it ended.
    ///
    /// The program gets the Keeper's environment without [`LEFT_OUT_VARIABLES`], with
    /// [`TERMINAL_VARIABLES`] and then `start.env` set on top; its standard input, output
    /// and error are the terminal; and it holds no other descriptor of the Keeper: no
    /// socket, no pipe, no other terminal. An `id` the Dock holds already is refused with
    /// [`DockError::TakenId`], and nothing is started. While [`Dock::stop_all`] runs, no
    /// terminal is opened: [`DockError::Start`] says so.
    pub fn open(&self, id: TerminalId, start: Start, events: Events) -> Result<(), DockError> {
        {
            let mut state = self.inner.state();
            // A terminal opened while the others are being stopped would be missed by the
            // stop, and the stop would wait for it.
            if state.stopping > 0 {
                return Err(DockError::Start {
                    program: start.program,
                    reason: io::Error::other("the Dock is stopping every terminal"),
                });
            }
            // The id is claimed before anything is made, so that two calls with one id
            // cannot both start a program.
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
        // SAFETY: the closure runs between fork and exec and calls only what is safe
        // there: `setsid`, `ioctl` and `fcntl`, with no allocation and no lock. An error
        // is made from the error number alone.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // A Keeper started as a daemon may ignore some signals, and a program
                // inherits that: it would not take the hang-up of a closing terminal.
                for signal in RESET_SIGNALS {
                    libc::signal(signal, libc::SIG_DFL);
                }
                // The standard library on macOS makes some descriptors and only then
                // marks them close-on-exec, so one made by another thread at that moment
                // reaches here open. Marked, not closed: the library reports a failed
                // `exec` through a pipe of its own.
                #[cfg(target_os = "macos")]
                for descriptor in 3..descriptors {
                    libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        // The command holds the slave. While it or ours is open here, the end of the
        // program would not show on the master.
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
            // A program nobody serves is not left running. The thread that collects it, which
            // did start, ends the others.
            //
            // SAFETY: `kill` only sends.
            unsafe { libc::kill(terminal.pid(), libc::SIGKILL) };
        }
        let mut threads = self.threads();
        threads.retain(|thread| !thread.is_finished());
        threads.extend(started);
        served
    }

    /// Starts the three threads of a terminal, the ones that start in `started`. A thread
    /// that cannot start is an error of the call, with nothing of the terminal left in the
    /// Dock's books.
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
                // The closure and the child with it are gone: kill and collect the
                // program here.
                //
                // SAFETY: `kill` only sends, and the program is ours and not yet collected.
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

    /// Hands `bytes` to the program of `terminal`, after everything handed to it before,
    /// and returns at once: the bytes wait until the program reads them. While
    /// [`MAX_WAITING_INPUT`] bytes wait, what more is handed is dropped, and nothing
    /// reports it: the call still returns `Ok`. A terminal whose program has ended, or that
    /// [`Dock::stop_all`] is stopping, takes nothing: [`DockError::UnknownTerminal`].
    pub fn write(&self, terminal: &TerminalId, bytes: &[u8]) -> Result<(), DockError> {
        if self.inner.live(terminal)?.type_in(bytes) {
            Ok(())
        } else {
            Err(DockError::UnknownTerminal(terminal.clone()))
        }
    }

    /// Gives `terminal` a new size. The system tells its program with the signal for a
    /// changed window size.
    pub fn resize(&self, terminal: &TerminalId, size: Size) -> Result<(), DockError> {
        pty::set_size(self.inner.live(terminal)?.master(), size)
            // The system refuses only when the terminal has been closed under the Dock.
            .map_err(|_| DockError::UnknownTerminal(terminal.clone()))
    }

    /// How many terminals hold a program that has not ended. The Keeper stays while this
    /// is not 0.
    pub fn running(&self) -> usize {
        self.inner.state().live.len()
    }

    /// Hands `attach` the newest output of `terminal` the Dock kept, at most
    /// [`RING_BYTES`], and whether older output was dropped to keep within that, while no
    /// [`Event`] of the terminal is told: what `attach` arranges for the output that
    /// follows, as adding a Window to those it goes to, gets every byte after the kept
    /// ones, and none twice. The kept output may begin in the middle of a character or of
    /// an escape sequence. The terminal's output waits while `attach` runs, so it should
    /// add the Window where the events go and return, and call nothing of the Dock. Nor
    /// should the caller hold, while it calls `attach`, a lock that the terminal's
    /// [`Events`] takes: the output is told under the lock `attach` waits for, so the
    /// two would wait for each other.
    /// Refused with [`DockError::UnknownTerminal`] for a terminal whose program has ended,
    /// or that the Dock never opened.
    pub fn attach(
        &self,
        terminal: &TerminalId,
        attach: impl FnOnce(&[u8], bool),
    ) -> Result<(), DockError> {
        self.inner.live(terminal)?.attach(attach);
        Ok(())
    }

    /// Asks the program of `terminal` to draw its screen again: the signal for a changed
    /// window size, sent to the terminal's foreground process group, also when the size
    /// did not change.
    pub fn redraw(&self, id: &TerminalId) -> Result<(), DockError> {
        let terminal = self.inner.live(id)?;
        // SAFETY: `tcgetpgrp` only reads, from a descriptor the terminal keeps open.
        let group = unsafe { libc::tcgetpgrp(terminal.master()) };
        if group <= 0 {
            // The system refuses only when the terminal has been closed under the Dock.
            return Err(DockError::UnknownTerminal(id.clone()));
        }
        // SAFETY: `kill` only sends; a group that went since the look answers `ESRCH`,
        // which means there is no program left to ask.
        unsafe { libc::kill(-group, libc::SIGWINCH) };
        Ok(())
    }

    /// Stops every program and what it started, and returns when none is left: the
    /// hang-up signal to every process of each terminal's session, then to those still
    /// there after [`STOP_STEP`] the terminate signal, and after one more step the kill
    /// signal. A terminal's session is the one its program leads: the program, its
    /// children, and the jobs a shell put in process groups of their own, also those still
    /// there when the program has ended. A process that left the session, as a daemon
    /// does, is not reached. Each terminal's [`Events`] is told its end, as when a program
    /// ends by itself.
    ///
    /// What waits to be typed to a program is dropped at once, and while the stop runs
    /// [`Dock::open`] refuses.
    pub fn stop_all(&self) {
        let sessions = self.begin_stopping();
        let _stopping = Stopping(&self.inner);
        for signal in STOP_SIGNALS {
            let members = session::members(&sessions);
            if members.is_empty() {
                break;
            }
            for pid in members {
                // SAFETY: `kill` only sends; a process that went since the listing
                // answers `ESRCH`, which is what is wanted of it.
                unsafe { libc::kill(pid, signal) };
            }
            let deadline = Instant::now() + STOP_STEP;
            while !session::members(&sessions).is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
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
        // What the stop has emptied is let go now, not at the next stop.
        self.inner.left_behind_sessions();
    }

    /// Refuses further terminals, waits for those being made, stops what is typed to the
    /// terminals there are, and gives the sessions to stop: the programs' ids are the
    /// sessions'. The lock is not held while waiting for the signals.
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
        // Read after the live ones: a terminal is put here before it is forgotten, so one
        // that ended between the two reads is in this list.
        sessions.extend(self.inner.left_behind_sessions());
        sessions.sort_unstable();
        sessions.dedup();
        sessions
    }
}

/// Ends the refusal of new terminals when a stop is over, however it ends.
struct Stopping<'a>(&'a Inner);

impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        self.0.state().stopping -= 1;
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        // Nothing the Dock started outlives it.
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

    fn executable(path: &Path, mode: u32) {
        std::fs::write(path, "#!/bin/sh\n").expect("write a file");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("set its mode");
    }

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("yaar-dock-unit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("make a folder");
        folder
    }

    /// A session leader that is killed and collected when the test ends, however it ends.
    struct Leader(std::process::Child);

    impl Drop for Leader {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A process that leads a session of its own, as a program of a terminal does.
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
        // Its id is free for any process to take as a session of its own: a later stop
        // would signal that one's processes if the Dock still named it.
        // Read as the stop left it: asking for the sessions would let it go by itself.
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
        // The list names a session by this id that is not the leader's: the one that was
        // noted emptied, and the id went to a process that began a session of its own.
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
        // The stop must not have signalled that process: it is not the Dock's.
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
        // Looking at the list, as every stop does, does not let go of what is there.
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
    fn a_folder_put_first_comes_before_the_keepers_search_path_and_is_alone_where_it_has_none() {
        let entry = [Entry::SearchFirst(PathBuf::from("/app/pieces"))];
        for (held, value) in [
            (Some("/usr/bin:/bin"), "/app/pieces:/usr/bin:/bin"),
            (Some(""), "/app/pieces"),
            (None, "/app/pieces"),
        ] {
            let start = start().with_environment(&entry, |name| {
                assert_eq!(name, "PATH");
                held.map(OsString::from)
            });
            assert_eq!(env_of(&start), [pair("PATH", value)], "{held:?}");
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
                Entry::SearchFirst(PathBuf::from("/one")),
                Entry::SearchFirst(PathBuf::from("/two")),
                Entry::PathList {
                    name: "DIRS".into(),
                    path: PathBuf::from("/x"),
                },
                Entry::PathList {
                    name: "DIRS".into(),
                    path: PathBuf::from("/y"),
                },
            ],
            |name| (name == "PATH").then(|| OsString::from("/usr/bin")),
        );
        assert_eq!(
            env_of(&with),
            [
                pair("KEPT", "1"),
                pair("YAAR_TERMINAL", "abc"),
                pair("PATH", "/one:/usr/bin"),
                pair("PATH", "/two:/one:/usr/bin"),
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
    fn shell_names_a_program_by_an_absolute_path_that_can_be_run() {
        let folder = folder("shell");
        let runnable = folder.join("runnable");
        let not_runnable = folder.join("not-runnable");
        executable(&runnable, 0o755);
        executable(&not_runnable, 0o644);
        // What the system's list of users names, which the test chooses.
        let listed = folder.join("listed");
        executable(&listed, 0o755);
        let from_list = || Some(listed.clone());

        // `SHELL` wins when it can be run, and the list is not asked.
        assert_eq!(
            shell_from(Some(runnable.clone().into()), || panic!("asked the list")),
            runnable
        );
        // Otherwise the shell of the list of users.
        for other in [
            Some(not_runnable.clone().into()),
            Some(folder.clone().into()),
            Some(folder.join("missing").into()),
            Some("sh".into()),
            Some("".into()),
            None,
        ] {
            assert_eq!(shell_from(other.clone(), from_list), listed, "{other:?}");
        }
        // And `/bin/sh` when that cannot be run either.
        for listed in [None, Some(not_runnable), Some(PathBuf::from("sh"))] {
            assert_eq!(
                shell_from(None, || listed.clone()),
                PathBuf::from("/bin/sh"),
                "{listed:?}"
            );
        }
        std::fs::remove_dir_all(folder).expect("remove the folder");
    }
}
