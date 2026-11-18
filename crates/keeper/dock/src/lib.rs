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
//!   [`Start::program`] for any other program, and [`user_shell`];
//! - [`Size`], [`TerminalId`], [`Event`], [`Ending`] and [`DockError`], in the Dock's own
//!   words: the Keeper program puts them into those of the Link.
//!
//! The Dock knows nothing of the Link, of Windows or of agents. It runs programs on
//! pseudo-terminals and passes bytes both ways. Of a program's environment it decides
//! only what says which terminal the program runs in ([`TERMINAL_VARIABLES`],
//! [`LEFT_OUT_VARIABLES`]); the rest comes from the caller through [`Start::env`]. It
//! keeps no copy of what a program wrote:
//! the ring of recent output, for a Window that attaches later, and the watch for the turn
//! marks come with the slices that need them.
//!
//! No interface code belongs here.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// How long each step of [`Dock::stop_all`] waits for the programs before the next,
/// harder one: hang-up, then terminate, then kill.
pub const STOP_STEP: Duration = Duration::from_secs(1);

/// The signals of [`Dock::stop_all`], in the order they are sent.
const STOP_SIGNALS: [libc::c_int; 3] = [libc::SIGHUP, libc::SIGTERM, libc::SIGKILL];

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
/// names another terminal, or a multiplexer, that the Keeper was started from. A program
/// that found them beside [`TERMINAL_VARIABLES`] would take the app's terminal for that
/// one, and send it what only that one understands.
pub const LEFT_OUT_VARIABLES: [&str; 22] = [
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
        todo!()
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
    /// The program has ended and nothing more is told of this terminal. It comes once,
    /// last, after every [`Event::Output`] of the terminal.
    Ended(Ending),
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
}

impl Start {
    /// The user's shell, [`user_shell`], as a login shell, its one argument `-l`, in
    /// `folder`.
    pub fn shell(folder: PathBuf, size: Size) -> Start {
        let _ = (folder, size);
        todo!()
    }

    /// Any program, with these arguments, in `folder`.
    pub fn program(program: PathBuf, args: Vec<OsString>, folder: PathBuf, size: Size) -> Start {
        Start {
            program,
            args,
            folder,
            size,
            env: Vec::new(),
        }
    }
}

/// The user's shell: `SHELL` from the Keeper's environment when it names, by an absolute
/// path, a file that can be run; else the shell of the user's entry in the system's list
/// of users; else `/bin/sh`.
pub fn user_shell() -> PathBuf {
    todo!()
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
    /// The Dock holds no terminal by this id: it never opened one, or its program has
    /// ended.
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
    // What the Dock holds; its builder gives it its shape.
    _held: (),
}

impl Dock {
    /// A Dock that holds no terminal.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Dock {
        todo!()
    }

    /// Opens a terminal of `start.size` under the name `id`, which the caller made with
    /// [`TerminalId::random`], and starts `start.program` in it. From then on `events` is
    /// told what the program writes, and at last how it ended.
    ///
    /// The program gets the Keeper's environment without [`LEFT_OUT_VARIABLES`], with
    /// [`TERMINAL_VARIABLES`] and then `start.env` set on top; its standard input, output
    /// and error are the terminal; and it holds no other descriptor of the Keeper: no
    /// socket, no pipe, no other terminal. An `id` the Dock holds already is refused with
    /// [`DockError::TakenId`], and nothing is started.
    pub fn open(&self, id: TerminalId, start: Start, events: Events) -> Result<(), DockError> {
        let _ = (id, start, events);
        todo!()
    }

    /// Hands `bytes` to the program of `terminal`, after everything handed to it before,
    /// and returns at once: the bytes wait until the program reads them. While
    /// [`MAX_WAITING_INPUT`] bytes wait, what more is handed is dropped, and nothing
    /// reports it: the call still returns `Ok`.
    pub fn write(&self, terminal: &TerminalId, bytes: &[u8]) -> Result<(), DockError> {
        let _ = (terminal, bytes);
        todo!()
    }

    /// Gives `terminal` a new size. The system tells its program with the signal for a
    /// changed window size.
    pub fn resize(&self, terminal: &TerminalId, size: Size) -> Result<(), DockError> {
        let _ = (terminal, size);
        todo!()
    }

    /// How many terminals hold a program that has not ended. The Keeper stays while this
    /// is not 0.
    pub fn running(&self) -> usize {
        todo!()
    }

    /// Stops every program and what it started, and returns when none is left: the
    /// hang-up signal to every process of each terminal's session, then to those still
    /// there after [`STOP_STEP`] the terminate signal, and after one more step the kill
    /// signal. A terminal's session is the one its program leads: the program, its
    /// children, and the jobs a shell put in process groups of their own. A process that
    /// left the session, as a daemon does, is not reached. Each terminal's [`Events`] is
    /// told its end, as when a program ends by itself.
    pub fn stop_all(&self) {
        let _ = STOP_SIGNALS;
        todo!()
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        // Nothing the Dock started outlives it.
        todo!()
    }
}
