//! Link (`S-PROTO`): what the Keeper and the Window say to each other.
//!
//! Owns: the messages between the two programs and the exchange of versions.
//!
//! Surface:
//! - [`Message`], with [`write_message`] and [`read_message`], which carry it as a frame;
//! - [`Hello`], [`handshake`] and [`PROTOCOL_VERSION`], the exchange of versions;
//! - [`Request`] with its [`RequestKind`] and [`Origin`], [`Reply`], [`Refusal`] and
//!   [`Fact`]: what a Window asks, what the Keeper answers, and what it tells unasked;
//! - [`Message::Input`] and [`Message::Output`], the bytes of a terminal in both
//!   directions, which are a stream and have no reply;
//! - what those carry: [`Repository`], [`Workspace`], [`Changes`], [`FileSides`] and their
//!   parts; [`TerminalId`], [`TerminalSize`], [`Program`] and [`Ending`];
//!   [`WorkspaceReviews`] with its [`Review`]s, [`Thread`]s, [`Comment`]s and [`Round`]s,
//!   the [`Anchor`] a thread sits on, and [`DraftOn`], what a draft that is saved belongs
//!   to; [`AgentReport`], what a piece of the app inside an agent CLI reports;
//!   [`WorkspaceAgents`] with its [`Session`]s and their [`SessionState`]; [`Turn`]s with
//!   their [`Step`]s; [`History`] with its [`HistoryEntry`]s and [`CreditedFile`]s;
//!   [`TerminalInfo`], a terminal the Keeper holds; [`Setting`], a setting the user
//!   set, as the [`FlowForm`] of a workspace; and what slice 6 adds to reviews: the
//!   [`ThreadState`] of a thread, where it sits now ([`Placed`]) and the [`Notice`] of
//!   code that changed under it, and the [`AgentCli`]s offered when a review is attached;
//!   and the [`Consent`] of an Adapter that asks the user's leave before it writes into
//!   the user's own settings of its CLI, with the [`ConsentAnswer`] the user gave; and
//!   what slice 9 adds: the settings of the whole app ([`TurnEndSelects`],
//!   [`ThemeChoice`], [`TerminalPlace`], [`OptionKey`]), and the room the recorded states
//!   take ([`StateSizes`] with each workspace's [`StateSize`]);
//! - [`Age`], how the side that sent a hello stands to this one in version;
//! - [`socket_path`], [`socket_address`], [`data_dir`] and [`cache_dir`], the places both
//!   programs find by the same rule, and [`connect`] with [`check_private_folder`], by which whoever
//!   reaches the Keeper makes sure it is the user's own;
//! - [`user_shell`], the user's login shell, which both programs find by the same rule: the
//!   Keeper runs it in each terminal, and the Window takes the environment it prints for the
//!   language servers.
//!
//! A message travels as one frame: a length of four bytes, little endian, that counts what
//! follows; one byte that names the kind; the payload.
//!
//! Two frames never change shape, whatever the protocol version: [`Message::Hello`] and
//! [`Message::Shutdown`]. A Window of one version must be able to tell a Keeper of another
//! who it is, and to ask it to stop. Every other frame is spoken only between two sides of
//! the same protocol version, and is refused when it holds anything but its fields.
//!
//! A new kind of request is one line in the table of `request.rs` and one variant of
//! [`Request`]. The table makes it say who starts it, and that puts it in
//! [`RequestKind::ALL`]: a test that walks the requests a user starts cannot miss it.
//!
//! Two kinds of program connect to the Keeper's socket: a Window, which stays and is told
//! every fact, and a piece of the app inside an agent CLI, which says hello, sends one
//! request whose origin is [`Origin::Agent`], takes its reply and leaves. Both say the
//! same hello.

mod agent;
pub mod login;
mod request;
mod review;
mod setting;
mod states;
mod terminal;
mod wire;
mod workspace;

use std::fmt;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

pub use agent::{
    AgentCli, AgentReport, Consent, ConsentAnswer, CreditedFile, History, HistoryEntry, Session,
    SessionState, Step, TerminalInfo, Turn, TurnEnd, WorkspaceAgents,
};
pub use request::{Fact, Origin, Refusal, Reply, Request, RequestId, RequestKind};
pub use review::{
    Anchor, Attachment, Author, Comment, CommentId, DraftOn, Lines, Notice, Placed, Review,
    ReviewId, Round, SentBy, SessionId, Side, Thread, ThreadId, ThreadState, TurnId,
    WorkspaceReviews,
};
pub use setting::{FlowForm, OptionKey, Setting, TerminalPlace, ThemeChoice, TurnEndSelects};
pub use states::{StateSize, StateSizes};
pub use terminal::{Ending, Program, TerminalId, TerminalSize};
pub use workspace::{
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, FileSides, FileStatus,
    Head, MAX_CONTENT, NoComparison, Repository, RepositoryId, Workspace, WorkspaceId,
};

use wire::{In, Out};

/// The version of the protocol this crate speaks. Raised whenever a frame other than
/// the two fixed ones changes, and never lowered: of two sides that speak different
/// versions, the one with the lower number is the older. [`Hello::age_against`] rests on
/// that.
///
/// 2: requests, replies and facts about workspaces. 3: terminals, and the two frames of
/// their bytes. 4: reviews, and a workspace whose worktree is gone. 5: agents, their
/// sessions, turns and steps, the history of a worktree, a terminal's launcher of an agent,
/// and attaching to a terminal that still runs. 6: the user's settings, the form of Flow of
/// a workspace among them, the state a turn's change counts from, and the workspace of a
/// step. 7: attaching a review and what a piece of an agent CLI is answered with, the
/// agent's replies, the states of a thread and where it sits now, rounds whose state is
/// recorded after the send, the launcher of a CLI for a review and the resumption of a
/// session. 8: the agent CLIs the Keeper has an Adapter for, what a review shows of a
/// change with who is credited with each file, and the turn a round was sent in. 9: a
/// version of a workspace written into a folder of the app's cache as the old side of a
/// diff, and the user's answer on the language servers inside a repository, a setting. 10:
/// the settings of the whole app, a worktree made with an agent in one step, the room the
/// recorded states take and their clean-up, and a turn whose state expired.
pub const PROTOCOL_VERSION: u16 = 10;

/// The largest frame a reader accepts.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// The folder name of the app in the user's data and runtime folders.
pub const APP_FOLDER: &str = "yaar-review";

/// The variable that names the Keeper's socket outright: see [`socket_path`]. Every shell
/// of the app has it, so that a piece of the app inside an agent CLI finds the Keeper
/// that started that shell.
pub const SOCKET_VARIABLE: &str = "YAAR_SOCKET";

/// The variable that names, in every shell of the app, the id of its terminal: a piece of
/// the app inside an agent CLI passes it on, so that the Keeper knows which terminal the
/// CLI runs in.
pub const TERMINAL_VARIABLE: &str = "YAAR_TERMINAL";

const MAGIC: [u8; 4] = *b"YAAR";
const KIND_HELLO: u8 = 1;
const KIND_SHUTDOWN: u8 = 2;
const KIND_REQUEST: u8 = 3;
const KIND_REPLY: u8 = 4;
const KIND_FACT: u8 = 5;
const KIND_INPUT: u8 = 6;
const KIND_OUTPUT: u8 = 7;

/// The first frame each side sends: who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// The protocol version the sender speaks.
    pub protocol: u16,
    /// The process id of the sender.
    pub pid: u32,
    /// The version of the program that sends, as shown to the user.
    pub app_version: String,
}

impl Hello {
    /// A `Hello` for this process, speaking [`PROTOCOL_VERSION`].
    pub fn for_this_process(app_version: &str) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            app_version: app_version.to_owned(),
        }
    }

    /// Whether two sides can go on talking after the exchange.
    pub fn speaks_with(&self, other: &Hello) -> bool {
        self.protocol == other.protocol
    }

    /// How the side that sent this hello stands to the side that sent `mine`, in version.
    ///
    /// The protocol decides first: it only ever rises, so a lower one is older whatever
    /// the versions of the app say. With the same protocol the two versions of the app
    /// are compared, read as numbers parted by dots, `0.2.10` after `0.2.9`; whatever
    /// follows the numbers, as `-beta` or `+abc`, is not compared. The answer is
    /// [`Age::Unknown`] when the protocols are the same and either version does not start
    /// with a number, or when the numbers are the same and the texts differ.
    pub fn age_against(&self, mine: &Hello) -> Age {
        use std::cmp::Ordering;
        match self.protocol.cmp(&mine.protocol) {
            Ordering::Less => return Age::Older,
            Ordering::Greater => return Age::Newer,
            Ordering::Equal => {}
        }
        if self.app_version == mine.app_version {
            return Age::Same;
        }
        match (
            version_numbers(&self.app_version),
            version_numbers(&mine.app_version),
        ) {
            (Some(theirs), Some(ours)) => match compare_numbers(&theirs, &ours) {
                Ordering::Less => Age::Older,
                Ordering::Greater => Age::Newer,
                Ordering::Equal => Age::Unknown,
            },
            _ => Age::Unknown,
        }
    }
}

/// How one side of the Link stands to the other in version: see [`Hello::age_against`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Age {
    /// An older version than this side.
    Older,
    /// The same version.
    Same,
    /// A newer version than this side.
    Newer,
    /// Another version, and which one is newer cannot be told.
    Unknown,
}

/// The numbers a version of the app starts with, parted by dots: `[0, 2, 10]` for
/// `0.2.10-beta`. None when it does not start with a number.
fn version_numbers(version: &str) -> Option<Vec<u64>> {
    let numeric = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map_while(|part| part.parse::<u64>().ok())
        .collect::<Vec<_>>();
    (!numeric.is_empty()).then_some(numeric)
}

/// Compares two lists of version numbers; a missing number counts as 0, so `1.2` and
/// `1.2.0` are the same.
fn compare_numbers(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    let at = |numbers: &[u64], i: usize| numbers.get(i).copied().unwrap_or(0);
    (0..len)
        .map(|i| at(a, i).cmp(&at(b, i)))
        .find(|order| order.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// One message between the Window and the Keeper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Who the sender is. The first frame in each direction.
    Hello(Hello),
    /// The Window asks the Keeper to stop everything and exit.
    Shutdown,
    /// A Window asks the Keeper for something.
    Request {
        /// Pairs the reply with this request.
        id: RequestId,
        /// What is asked.
        request: Request,
    },
    /// The Keeper answers one request.
    Reply {
        /// The id of the request this answers.
        to: RequestId,
        /// What was asked for, or why it was refused.
        reply: Result<Reply, Refusal>,
    },
    /// The Keeper tells a Window what changed.
    Fact(Fact),
    /// A Window sends bytes to the program of a terminal: what the user typed or pasted,
    /// encoded as the terminal's state asks, and the terminal's answers to the program's
    /// questions. Nothing answers it. Bytes for a terminal the Keeper does not hold, as
    /// one whose program has ended, are dropped.
    Input {
        /// The terminal.
        terminal: TerminalId,
        /// The bytes, in the order the program is to read them.
        bytes: Vec<u8>,
    },
    /// The Keeper sends a Window bytes the program of a terminal wrote, in the order it
    /// wrote them. Sent to the Window that opened the terminal, for as long as it is
    /// attached. Nothing answers it.
    Output {
        /// The terminal.
        terminal: TerminalId,
        /// The bytes, as the program wrote them.
        bytes: Vec<u8>,
    },
}

/// Why a frame could not be read.
#[derive(Debug)]
pub enum LinkError {
    /// The other side closed the connection between two frames.
    Closed,
    /// The connection failed.
    Io(io::Error),
    /// The frame is longer than [`MAX_FRAME`], or empty.
    BadLength(u32),
    /// The frame names a kind this version does not know.
    UnknownKind(u8),
    /// The payload does not have the shape its kind requires.
    Malformed(&'static str),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "the other side closed the connection"),
            Self::Io(e) => write!(f, "the connection failed: {e}"),
            Self::BadLength(n) => write!(f, "a frame of {n} bytes is not acceptable"),
            Self::UnknownKind(k) => write!(f, "unknown kind of frame: {k}"),
            Self::Malformed(what) => write!(f, "malformed frame: {what}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Writes one message as one frame and flushes.
pub fn write_message(out: &mut impl Write, message: &Message) -> io::Result<()> {
    let mut frame = vec![0u8; 4];
    match message {
        Message::Hello(hello) => {
            let version = hello.app_version.as_bytes();
            let len = u16::try_from(version.len())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "version too long"))?;
            frame.push(KIND_HELLO);
            frame.extend_from_slice(&MAGIC);
            frame.extend_from_slice(&hello.protocol.to_le_bytes());
            frame.extend_from_slice(&hello.pid.to_le_bytes());
            frame.extend_from_slice(&len.to_le_bytes());
            frame.extend_from_slice(version);
        }
        Message::Shutdown => frame.push(KIND_SHUTDOWN),
        Message::Request { id, request } => {
            frame.push(KIND_REQUEST);
            let mut body = Out(frame);
            request::write_request(&mut body, *id, request);
            frame = body.0;
        }
        Message::Reply { to, reply } => {
            frame.push(KIND_REPLY);
            let mut body = Out(frame);
            request::write_reply(&mut body, *to, reply);
            frame = body.0;
        }
        Message::Fact(fact) => {
            frame.push(KIND_FACT);
            let mut body = Out(frame);
            request::write_fact(&mut body, fact);
            frame = body.0;
        }
        Message::Input { terminal, bytes } => {
            frame.push(KIND_INPUT);
            let mut body = Out(frame);
            body.text(terminal.as_str());
            body.bytes(bytes);
            frame = body.0;
        }
        Message::Output { terminal, bytes } => {
            frame.push(KIND_OUTPUT);
            let mut body = Out(frame);
            body.text(terminal.as_str());
            body.bytes(bytes);
            frame = body.0;
        }
    }
    // What the reader would refuse is not sent.
    let len = u32::try_from(frame.len() - 4)
        .ok()
        .filter(|len| *len <= MAX_FRAME)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
    frame[..4].copy_from_slice(&len.to_le_bytes());
    out.write_all(&frame)?;
    out.flush()
}

/// Reads one frame and returns its message.
pub fn read_message(input: &mut impl Read) -> Result<Message, LinkError> {
    let mut len = [0u8; 4];
    // A read that a signal interrupts before it gives a byte is tried again, as
    // `read_exact` does for the rest of the frame.
    let first = loop {
        match input.read(&mut len[..1]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            read => break read?,
        }
    };
    match first {
        0 => return Err(LinkError::Closed),
        _ => input.read_exact(&mut len[1..])?,
    }
    let len = u32::from_le_bytes(len);
    if len == 0 || len > MAX_FRAME {
        return Err(LinkError::BadLength(len));
    }
    let mut body = vec![0u8; len as usize];
    input.read_exact(&mut body)?;
    let (kind, payload) = (body[0], &body[1..]);
    match kind {
        KIND_HELLO => parse_hello(payload).map(Message::Hello),
        KIND_SHUTDOWN => Ok(Message::Shutdown),
        KIND_REQUEST => {
            let mut fields = In::new(payload);
            let (id, request) = request::read_request(&mut fields)?;
            fields.end()?;
            Ok(Message::Request { id, request })
        }
        KIND_REPLY => {
            let mut fields = In::new(payload);
            let (to, reply) = request::read_reply(&mut fields)?;
            fields.end()?;
            Ok(Message::Reply { to, reply })
        }
        KIND_FACT => {
            let mut fields = In::new(payload);
            let fact = request::read_fact(&mut fields)?;
            fields.end()?;
            Ok(Message::Fact(fact))
        }
        KIND_INPUT | KIND_OUTPUT => {
            let mut fields = In::new(payload);
            let terminal = TerminalId::new(fields.text()?);
            let bytes = fields.bytes()?;
            fields.end()?;
            Ok(if kind == KIND_INPUT {
                Message::Input { terminal, bytes }
            } else {
                Message::Output { terminal, bytes }
            })
        }
        other => Err(LinkError::UnknownKind(other)),
    }
}

fn parse_hello(payload: &[u8]) -> Result<Hello, LinkError> {
    // A later version may append fields, so a longer payload is not an error.
    if payload.len() < 12 {
        return Err(LinkError::Malformed("hello is too short"));
    }
    if payload[..4] != MAGIC {
        return Err(LinkError::Malformed("hello does not start with YAAR"));
    }
    let protocol = u16::from_le_bytes([payload[4], payload[5]]);
    let pid = u32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]);
    let len = u16::from_le_bytes([payload[10], payload[11]]) as usize;
    let version = payload
        .get(12..12 + len)
        .ok_or(LinkError::Malformed("hello ends inside its version"))?;
    let app_version = std::str::from_utf8(version)
        .map_err(|_| LinkError::Malformed("the version is not UTF-8"))?
        .to_owned();
    Ok(Hello {
        protocol,
        pid,
        app_version,
    })
}

/// Sends `mine` and reads the other side's [`Hello`].
pub fn handshake(stream: &mut (impl Read + Write), mine: &Hello) -> Result<Hello, LinkError> {
    write_message(stream, &Message::Hello(mine.clone()))?;
    match read_message(stream)? {
        Message::Hello(theirs) => Ok(theirs),
        _ => Err(LinkError::Malformed("the first frame was not a hello")),
    }
}

/// Where the Keeper is asked to listen.
///
/// `YAAR_SOCKET` names the socket outright. Otherwise it is `keeper.sock` in the app's
/// runtime folder: under `XDG_RUNTIME_DIR` on Linux when that is set, else in
/// [`data_dir`].
///
/// A socket is bound and reached at [`socket_address`] of this path, which differs from it
/// only when the path is longer than the system allows.
pub fn socket_path() -> PathBuf {
    socket_path_from(|name| std::env::var_os(name).map(PathBuf::from))
}

fn socket_path_from(var: impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.as_os_str().is_empty());
    if let Some(path) = set(SOCKET_VARIABLE) {
        return path;
    }
    let folder = match set("XDG_RUNTIME_DIR") {
        Some(runtime) if !cfg!(target_os = "macos") => runtime.join(APP_FOLDER),
        _ => data_dir_from(&var),
    };
    folder.join("keeper.sock")
}

/// The app's folder in the user's data folder, where what must outlive a restart of the
/// machine is kept: `~/Library/Application Support/yaar-review` on macOS,
/// `$XDG_DATA_HOME/yaar-review` or `~/.local/share/yaar-review` elsewhere.
pub fn data_dir() -> PathBuf {
    data_dir_from(|name| std::env::var_os(name).map(PathBuf::from))
}

fn data_dir_from(var: impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.as_os_str().is_empty());
    let home = set("HOME").unwrap_or_else(|| PathBuf::from("/"));
    let folder = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if let Some(data) = set("XDG_DATA_HOME") {
        data
    } else {
        home.join(".local/share")
    };
    folder.join(APP_FOLDER)
}

/// The app's folder in the user's cache folder, where what can be made again is kept: the
/// old side of a diff written for a language server, and the build folders of those
/// servers. `~/Library/Caches/yaar-review` on macOS, `$XDG_CACHE_HOME/yaar-review` or
/// `~/.cache/yaar-review` elsewhere. Both programs find it by this rule: the Window names
/// a folder in it, and the Keeper writes there only.
pub fn cache_dir() -> PathBuf {
    cache_dir_from(|name| std::env::var_os(name).map(PathBuf::from))
}

/// The user's shell: `SHELL` from this process's environment when it names, by an absolute
/// path, a file that can be run; else the shell of the user's entry in the system's list
/// of users; else `/bin/sh`. The Keeper runs it as the login shell of each terminal, and
/// the Window runs it once to take the environment the language servers run in.
pub fn user_shell() -> PathBuf {
    shell_from(std::env::var_os("SHELL"), passwd_shell)
}

/// The choice of [`user_shell`], with the value of `SHELL` and the look at the system's
/// list of users handed in.
fn shell_from(
    variable: Option<std::ffi::OsString>,
    passwd: impl FnOnce() -> Option<PathBuf>,
) -> PathBuf {
    variable
        .map(PathBuf::from)
        .filter(|shell| can_run(shell))
        .or_else(|| passwd().filter(|shell| can_run(shell)))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// An absolute path to a file with an execute bit.
fn can_run(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The shell in the entry of the user this process runs as, in the system's list of
/// users.
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
            return (!shell.is_empty()).then(|| {
                PathBuf::from(std::ffi::OsString::from(std::ffi::OsStr::from_bytes(shell)))
            });
        }
    }
}

fn cache_dir_from(var: impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.as_os_str().is_empty());
    let home = set("HOME").unwrap_or_else(|| PathBuf::from("/"));
    let folder = if cfg!(target_os = "macos") {
        home.join("Library/Caches")
    } else if let Some(cache) = set("XDG_CACHE_HOME") {
        cache
    } else {
        home.join(".cache")
    };
    folder.join(APP_FOLDER)
}

/// The longest path a socket can be bound at, in bytes: the system's limit less the byte
/// that ends the path.
pub const SOCKET_PATH_MAX: usize = if cfg!(target_os = "macos") { 103 } else { 107 };

/// Where a socket wanted at `wanted` is really bound and reached.
///
/// It is `wanted` itself when that is at most [`SOCKET_PATH_MAX`] bytes long. A longer
/// path, as a long user name or a deep runtime folder makes it, cannot be bound at all,
/// so both programs then use a short one computed from the long one alone:
/// `/tmp/yaar-review-<hash>/keeper.sock`, where `<hash>` is the 64-bit FNV-1a hash of the
/// bytes of `wanted`, written as 16 lowercase hexadecimal digits.
///
/// The Keeper applies this to the path it is told to listen at, and whoever connects
/// applies it to the same path, so the two meet without having to agree on anything else.
/// The rule is as fixed as the two frames that never change: a Window of one version has
/// to find the Keeper of another.
///
/// The short path lies where any user can make folders, under a name anyone can compute.
/// It is therefore used only while its folder is the user's own: the Keeper makes the
/// folder for the user alone, and both the Keeper before it binds and whoever connects
/// refuse it otherwise, by [`check_private_folder`]. Connect through [`connect`], which
/// does this.
pub fn socket_address(wanted: &Path) -> PathBuf {
    short_place(wanted).unwrap_or_else(|| wanted.to_owned())
}

/// The short path that stands in for `wanted`; none when `wanted` can be bound itself.
fn short_place(wanted: &Path) -> Option<PathBuf> {
    let wanted = wanted.as_os_str().as_bytes();
    if wanted.len() <= SOCKET_PATH_MAX {
        return None;
    }
    // FNV-1a, 64 bits, written out: the hasher of the standard library may change from
    // one compiler to the next, and this value may not.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in wanted {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(PathBuf::from(format!(
        "/tmp/{APP_FOLDER}-{hash:016x}/keeper.sock"
    )))
}

/// Connects to the socket wanted at `wanted`, where [`socket_address`] puts it, and makes
/// sure that the other end is the user's own.
///
/// Two things are refused, both with [`io::ErrorKind::PermissionDenied`]:
/// - a short path whose folder is not the user's own, by [`check_private_folder`], before
///   anything is connected;
/// - on any path, a process at the other end that runs as another user.
///
/// Where nothing listens the error is the system's own, as from connecting directly.
pub fn connect(wanted: &Path) -> io::Result<UnixStream> {
    let short = short_place(wanted);
    if let Some(folder) = short.as_deref().and_then(Path::parent) {
        check_private_folder(folder)?;
    }
    let address = short.as_deref().unwrap_or(wanted);
    let stream = UnixStream::connect(address)?;
    if user_at_the_other_end(&stream)? != this_user() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the process that listens at {} runs as another user",
                address.display()
            ),
        ));
    }
    Ok(stream)
}

/// Passes when `folder` is one that only the user of this process can use: a folder and
/// not a link to one, owned by that user, and closed to everybody else. Anything else is
/// refused with [`io::ErrorKind::PermissionDenied`]; a folder that is not there is the
/// system's own error.
///
/// The Keeper calls this on the folder of a short socket path after making it, and
/// [`connect`] calls it before connecting there.
pub fn check_private_folder(folder: &Path) -> io::Result<()> {
    // Not followed: a link is judged as a link, whatever it leads to.
    let found = std::fs::symlink_metadata(folder)?;
    let wrong = if found.file_type().is_symlink() {
        "is a link"
    } else if !found.is_dir() {
        "is not a folder"
    } else if found.uid() != this_user() {
        "belongs to another user"
    } else if found.mode() & 0o077 != 0 {
        "is open to other users"
    } else {
        return Ok(());
    };
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} {wrong}", folder.display()),
    ))
}

/// The user this process runs as.
fn this_user() -> u32 {
    // SAFETY: `geteuid` only reads and cannot fail.
    unsafe { libc::geteuid() }
}

/// The user of the process at the other end of a connected socket, as the system knows
/// it. Nothing the other end sends can change the answer.
#[cfg(target_os = "linux")]
fn user_at_the_other_end(stream: &UnixStream) -> io::Result<u32> {
    let mut found = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the descriptor is open for as long as `stream` is borrowed, and the two
    // pointers lead to a value of the type this option fills and to its size.
    let failed = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::from_mut(&mut found).cast(),
            &mut size,
        )
    } != 0;
    if failed {
        return Err(io::Error::last_os_error());
    }
    Ok(found.uid)
}

/// The user of the process at the other end of a connected socket, as the system knows
/// it. Nothing the other end sends can change the answer.
#[cfg(not(target_os = "linux"))]
fn user_at_the_other_end(stream: &UnixStream) -> io::Result<u32> {
    let mut user: libc::uid_t = 0;
    let mut group: libc::gid_t = 0;
    // SAFETY: the descriptor is open for as long as `stream` is borrowed, and the two
    // pointers lead to values of the types this call fills.
    let failed = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) } != 0;
    if failed {
        return Err(io::Error::last_os_error());
    }
    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(message: &Message) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_message(&mut bytes, message).expect("write to a vector");
        bytes
    }

    #[test]
    fn a_hello_comes_back_as_it_was_sent() {
        let hello = Hello {
            protocol: 7,
            pid: 4242,
            app_version: "0.1.0-été".to_owned(),
        };
        let bytes = frame(&Message::Hello(hello.clone()));
        assert_eq!(
            read_message(&mut Cursor::new(bytes)).expect("read"),
            Message::Hello(hello)
        );
    }

    #[test]
    fn the_hello_frame_keeps_its_bytes() {
        // These bytes are the promise to every later version. If this test has to change,
        // a Window and a Keeper of different versions can no longer tell each other apart.
        let bytes = frame(&Message::Hello(Hello {
            protocol: 1,
            pid: 0x0102_0304,
            app_version: "0.1".to_owned(),
        }));
        assert_eq!(
            bytes,
            [
                16, 0, 0, 0, // length
                1, // kind
                b'Y', b'A', b'A', b'R', 1, 0, 4, 3, 2, 1, 3, 0, b'0', b'.', b'1',
            ]
        );
        assert_eq!(frame(&Message::Shutdown), [1, 0, 0, 0, 2]);
    }

    #[test]
    fn a_hello_with_fields_appended_by_a_later_version_is_read() {
        let mut bytes = frame(&Message::Hello(Hello::for_this_process("9.9")));
        bytes.extend_from_slice(b"later");
        let len = (bytes.len() - 4) as u32;
        bytes[..4].copy_from_slice(&len.to_le_bytes());
        match read_message(&mut Cursor::new(bytes)) {
            Ok(Message::Hello(hello)) => assert_eq!(hello.app_version, "9.9"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn two_frames_in_a_row_are_read_one_by_one_and_then_the_end_is_closed() {
        let mut bytes = frame(&Message::Shutdown);
        bytes.extend(frame(&Message::Hello(Hello::for_this_process("1"))));
        let mut input = Cursor::new(bytes);
        assert_eq!(read_message(&mut input).expect("first"), Message::Shutdown);
        assert!(matches!(read_message(&mut input), Ok(Message::Hello(_))));
        assert!(matches!(read_message(&mut input), Err(LinkError::Closed)));
    }

    /// A reader that fails once with `Interrupted`, as a read does when a signal reaches
    /// the process, before it gives its bytes.
    struct Interrupted<R> {
        inner: R,
        interrupted: bool,
    }

    impl<R> Interrupted<R> {
        fn once_before(inner: R) -> Interrupted<R> {
            Interrupted {
                inner,
                interrupted: false,
            }
        }
    }

    impl<R: Read> Read for Interrupted<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.inner.read(buf)
        }
    }

    #[test]
    fn a_read_interrupted_before_the_first_byte_of_a_frame_is_tried_again() {
        let bytes = frame(&Message::Hello(Hello::for_this_process("1")));
        let mut input = Interrupted::once_before(Cursor::new(bytes));
        assert!(matches!(read_message(&mut input), Ok(Message::Hello(_))));
    }

    #[test]
    fn a_read_interrupted_before_the_end_of_the_input_is_closed() {
        let mut input = Interrupted::once_before(Cursor::new(Vec::new()));
        assert!(matches!(read_message(&mut input), Err(LinkError::Closed)));
    }

    #[test]
    fn what_is_not_a_frame_is_refused() {
        let cut = &frame(&Message::Hello(Hello::for_this_process("1")))[..9];
        assert!(matches!(
            read_message(&mut Cursor::new(cut)),
            Err(LinkError::Io(_))
        ));
        assert!(matches!(
            read_message(&mut Cursor::new([0u8, 0, 0, 0])),
            Err(LinkError::BadLength(0))
        ));
        assert!(matches!(
            read_message(&mut Cursor::new([0xffu8, 0xff, 0xff, 0xff])),
            Err(LinkError::BadLength(_))
        ));
        assert!(matches!(
            read_message(&mut Cursor::new([1u8, 0, 0, 0, 99])),
            Err(LinkError::UnknownKind(99))
        ));
        assert!(matches!(
            read_message(&mut Cursor::new([5u8, 0, 0, 0, 1, b'N', b'O', b'P', b'E'])),
            Err(LinkError::Malformed(_))
        ));
    }

    #[test]
    fn only_the_same_protocol_speaks_on() {
        let mine = Hello::for_this_process("1");
        let mut theirs = mine.clone();
        assert!(mine.speaks_with(&theirs));
        theirs.protocol += 1;
        assert!(!mine.speaks_with(&theirs));
    }

    #[test]
    fn the_socket_is_where_the_environment_says() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| PathBuf::from(v))
            }
        };
        assert_eq!(
            socket_path_from(env(&[("YAAR_SOCKET", "/x/k.sock"), ("HOME", "/home/a")])),
            PathBuf::from("/x/k.sock")
        );
        let home_only = socket_path_from(env(&[("HOME", "/home/a"), ("YAAR_SOCKET", "")]));
        if cfg!(target_os = "macos") {
            assert_eq!(
                home_only,
                PathBuf::from("/home/a/Library/Application Support/yaar-review/keeper.sock")
            );
        } else {
            assert_eq!(
                home_only,
                PathBuf::from("/home/a/.local/share/yaar-review/keeper.sock")
            );
            assert_eq!(
                socket_path_from(env(&[("HOME", "/home/a"), ("XDG_RUNTIME_DIR", "/run/u")])),
                PathBuf::from("/run/u/yaar-review/keeper.sock")
            );
        }
    }

    #[test]
    fn the_data_folder_is_where_the_environment_says() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| PathBuf::from(v))
            }
        };
        // The runtime folder holds the socket only: it does not outlive a restart.
        let home_only = data_dir_from(env(&[("HOME", "/home/a"), ("XDG_RUNTIME_DIR", "/run/u")]));
        if cfg!(target_os = "macos") {
            assert_eq!(
                home_only,
                PathBuf::from("/home/a/Library/Application Support/yaar-review")
            );
        } else {
            assert_eq!(home_only, PathBuf::from("/home/a/.local/share/yaar-review"));
            assert_eq!(
                data_dir_from(env(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")])),
                PathBuf::from("/data/yaar-review")
            );
        }
    }

    #[test]
    fn an_answer_on_the_servers_of_a_repository_takes_the_place_of_the_last_for_that_repository_only()
     {
        let repository = |digit: char| RepositoryId::new(digit.to_string().repeat(32));
        let yes = Setting::ServersInRepository {
            repository: repository('1'),
            allowed: true,
        };
        let no = Setting::ServersInRepository {
            repository: repository('1'),
            allowed: false,
        };
        let other = Setting::ServersInRepository {
            repository: repository('2'),
            allowed: true,
        };
        let form = Setting::FlowForm {
            workspace: WorkspaceId::new("1".repeat(32)),
            form: FlowForm::Timeline,
        };
        assert!(yes.same_thing_as(&no));
        assert!(!yes.same_thing_as(&other));
        assert!(!yes.same_thing_as(&form));
        assert!(!form.same_thing_as(&yes));
    }

    #[test]
    fn a_setting_of_the_app_is_the_same_thing_as_any_other_of_its_kind_and_no_other() {
        // Two values of each kind of the app's settings.
        let pairs = [
            (
                Setting::TurnEnd(TurnEndSelects::WhatChanged),
                Setting::TurnEnd(TurnEndSelects::KeepSelection),
            ),
            (
                Setting::StopAfter { seconds: 60 },
                Setting::StopAfter { seconds: 600 },
            ),
            (
                Setting::Theme(ThemeChoice::System),
                Setting::Theme(ThemeChoice::Named("Dark".to_owned())),
            ),
            (
                Setting::TerminalPlace(TerminalPlace::Right),
                Setting::TerminalPlace(TerminalPlace::Left),
            ),
            (
                Setting::KeepStates { days: 30 },
                Setting::KeepStates { days: 1 },
            ),
            (
                Setting::OptionKey(OptionKey::ByLayout),
                Setting::OptionKey(OptionKey::Meta),
            ),
        ];
        let form = Setting::FlowForm {
            workspace: WorkspaceId::new("1".repeat(32)),
            form: FlowForm::Timeline,
        };
        for (at, (one, other)) in pairs.iter().enumerate() {
            assert!(one.same_thing_as(other), "{one:?}");
            assert!(other.same_thing_as(one), "{other:?}");
            assert!(
                !one.same_thing_as(&form) && !form.same_thing_as(one),
                "{one:?}"
            );
            for (elsewhere, (third, _)) in pairs.iter().enumerate() {
                assert_eq!(
                    one.same_thing_as(third),
                    at == elsewhere,
                    "{one:?} {third:?}"
                );
            }
        }
    }

    #[test]
    fn a_folder_for_worktrees_is_the_same_thing_as_another_of_its_repository_and_no_other() {
        let of = |repository: &str, folder: Option<&str>| Setting::WorktreesFolder {
            repository: RepositoryId::new(repository.repeat(32)),
            folder: folder.map(PathBuf::from),
        };
        assert!(of("3", Some("/a")).same_thing_as(&of("3", None)));
        assert!(of("3", None).same_thing_as(&of("3", Some("/b"))));
        assert!(!of("3", Some("/a")).same_thing_as(&of("4", Some("/a"))));
        let answer = Setting::ServersInRepository {
            repository: RepositoryId::new("3".repeat(32)),
            allowed: true,
        };
        assert!(!of("3", None).same_thing_as(&answer) && !answer.same_thing_as(&of("3", None)));
        assert!(!of("3", None).same_thing_as(&Setting::KeepStates { days: 30 }));
    }

    #[test]
    fn the_cache_folder_is_where_the_environment_says() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| PathBuf::from(v))
            }
        };
        let home_only = cache_dir_from(env(&[("HOME", "/home/a"), ("XDG_CACHE_HOME", "")]));
        if cfg!(target_os = "macos") {
            assert_eq!(
                home_only,
                PathBuf::from("/home/a/Library/Caches/yaar-review")
            );
        } else {
            assert_eq!(home_only, PathBuf::from("/home/a/.cache/yaar-review"));
            assert_eq!(
                cache_dir_from(env(&[("HOME", "/home/a"), ("XDG_CACHE_HOME", "/c")])),
                PathBuf::from("/c/yaar-review")
            );
        }
    }

    /// A path of 128 bytes, as a long user name makes it on macOS.
    const LONG: &str = "/Users/a-user-whose-name-is-longer-than-most-and-longer-than-a-socket-allows/Library/Application Support/yaar-review/keeper.sock";

    #[test]
    fn a_path_the_system_can_bind_is_the_address_as_it_is() {
        let fits = format!("/{}", "s".repeat(SOCKET_PATH_MAX - 1));
        assert_eq!(fits.len(), SOCKET_PATH_MAX);
        assert_eq!(socket_address(Path::new(&fits)), PathBuf::from(&fits));
        assert_eq!(
            socket_address(Path::new("/run/user/1000/yaar-review/keeper.sock")),
            PathBuf::from("/run/user/1000/yaar-review/keeper.sock")
        );

        // One byte more cannot be bound, and so leads to the short place.
        let too_long = format!("{fits}s");
        let short = socket_address(Path::new(&too_long));
        assert_ne!(short, PathBuf::from(&too_long));
        assert!(short.as_os_str().len() <= SOCKET_PATH_MAX);
    }

    #[test]
    fn the_address_of_a_path_too_long_keeps_its_value() {
        // Like the bytes of the hello, this value is a promise to every other version: a
        // Window that computes another address does not find the Keeper that is running.
        // The digits are the 64-bit FNV-1a hash of the path, worked out apart from this
        // crate.
        assert!(LONG.len() > SOCKET_PATH_MAX);
        assert_eq!(
            socket_address(Path::new(LONG)),
            PathBuf::from("/tmp/yaar-review-ad4c554483ec5ddb/keeper.sock")
        );
    }

    #[test]
    fn two_long_paths_lead_to_two_addresses() {
        let other = LONG.replace("keeper.sock", "keeper2.sock");
        assert_ne!(
            socket_address(Path::new(LONG)),
            socket_address(Path::new(&other))
        );
    }

    fn two_repositories() -> Vec<Repository> {
        vec![
            Repository {
                id: RepositoryId::new("0123456789abcdef0123456789abcdef"),
                name: "yaar".to_owned(),
                workspaces: vec![
                    Workspace {
                        id: WorkspaceId::new("11111111111111111111111111111111"),
                        path: PathBuf::from("/home/a/gits/yaar"),
                        head: Some(Head::Branch("main".to_owned())),
                    },
                    Workspace {
                        id: WorkspaceId::new("22222222222222222222222222222222"),
                        path: PathBuf::from("/home/a/gits/yaar.worktrees/été"),
                        head: Some(Head::Detached(
                            "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
                        )),
                    },
                    // Gone: removed by git, and listed for its reviews.
                    Workspace {
                        id: WorkspaceId::new("44444444444444444444444444444444"),
                        path: PathBuf::from("/home/a/gits/yaar.worktrees/removed"),
                        head: None,
                    },
                ],
            },
            Repository {
                id: RepositoryId::new("fedcba9876543210fedcba9876543210"),
                name: "empty".to_owned(),
                workspaces: Vec::new(),
            },
        ]
    }

    /// The full id of a commit, as the Link carries it.
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn base(name: &str, source: BaseSource) -> Base {
        Base {
            name: name.to_owned(),
            source,
        }
    }

    fn changes(comparison: Comparison, counted_from: Option<&str>) -> Changes {
        Changes {
            workspace: WorkspaceId::new("11111111111111111111111111111111"),
            comparison,
            counted_from: counted_from.map(str::to_owned),
            commits: vec![Commit {
                id: "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
                summary: "Say what the change does".to_owned(),
            }],
            files: vec![
                ChangedFile {
                    path: PathBuf::from("src/new.rs"),
                    status: FileStatus::Added,
                },
                ChangedFile {
                    path: PathBuf::from("src/lib.rs"),
                    status: FileStatus::Modified,
                },
                ChangedFile {
                    path: PathBuf::from("src/old.rs"),
                    status: FileStatus::Deleted,
                },
                ChangedFile {
                    path: PathBuf::from("src/moved.rs"),
                    status: FileStatus::Renamed {
                        from: PathBuf::from("src/was here.rs"),
                    },
                },
            ],
        }
    }

    /// One message of every shape the Link has beyond the two fixed frames.
    fn every_shape() -> Vec<Message> {
        let workspace = WorkspaceId::new("11111111111111111111111111111111");
        let requests = [
            Request::ListWorkspaces,
            Request::OpenFolder {
                path: PathBuf::from("/home/a/gits/yaar"),
            },
            Request::SetBase {
                workspace: workspace.clone(),
                base: "origin/main".to_owned(),
            },
            Request::GetChanges {
                workspace: workspace.clone(),
            },
            Request::ReadFile {
                workspace: workspace.clone(),
                since: Some(COMMIT.to_owned()),
                old_path: PathBuf::from("src/was here.rs"),
                path: PathBuf::from("src/moved.rs"),
            },
            Request::OpenTerminal {
                workspace: workspace.clone(),
                program: Program::Shell,
                size: TerminalSize {
                    columns: 80,
                    rows: 24,
                },
            },
            Request::ResizeTerminal {
                terminal: terminal(),
                size: TerminalSize {
                    columns: u16::MAX,
                    rows: 1,
                },
            },
            Request::GetReviews {
                workspace: workspace.clone(),
            },
            Request::SaveDraft {
                review: review_id(),
                thread: ThreadId::new("55555555555555555555555555555555"),
                comment: CommentId::new("66666666666666666666666666666666"),
                on: DraftOn::NewThread {
                    anchor: anchor(None, Side::Old),
                },
                text: "Why is this gone?\nIt was used.".to_owned(),
            },
            Request::DeleteDraft {
                review: review_id(),
                comment: CommentId::new("66666666666666666666666666666666"),
            },
            Request::CopyReview {
                review: review_id(),
            },
            Request::MarkReviewed {
                review: review_id(),
            },
            Request::SetViewed {
                workspace: workspace.clone(),
                path: PathBuf::from("src/été.rs"),
                viewed: true,
            },
            Request::Commit {
                workspace: workspace.clone(),
                message: "Say what the change does\n\nAnd why.".to_owned(),
            },
            Request::Report {
                report: AgentReport {
                    agent: "claude".to_owned(),
                    terminal: Some(terminal()),
                    pid: u32::MAX,
                    payload: b"{\"hook_event_name\":\"Stop\"}\n\0\xff".to_vec(),
                },
            },
            Request::ListAgents,
            Request::GetHistory {
                workspace: workspace.clone(),
            },
            Request::CompareVersions {
                workspace: workspace.clone(),
                from: COMMIT.to_owned(),
                to: Some(STATE.to_owned()),
            },
            Request::ReadVersions {
                workspace: workspace.clone(),
                old: Some(COMMIT.to_owned()),
                old_path: PathBuf::from("src/was here.rs"),
                new: Some(STATE.to_owned()),
                path: PathBuf::from("src/moved.rs"),
            },
            Request::GetTurn { turn: turn_id() },
            Request::ShowWorkspace {
                workspace: Some(workspace.clone()),
            },
            Request::ListTerminals,
            Request::AttachTerminal {
                terminal: terminal(),
                size: TerminalSize {
                    columns: 120,
                    rows: 40,
                },
            },
            Request::GetUncommitted {
                workspace: workspace.clone(),
                review: Some(review_id()),
            },
            Request::GetSettings,
            Request::SetSetting {
                setting: Setting::FlowForm {
                    workspace: workspace.clone(),
                    form: FlowForm::LiveDiff,
                },
            },
            Request::AttachReview {
                review: review_id(),
                thread: Some(ThreadId::new("55555555555555555555555555555555")),
            },
            Request::DetachReview {
                review: review_id(),
            },
            Request::ResolveThread {
                review: review_id(),
                thread: ThreadId::new("55555555555555555555555555555555"),
            },
            Request::ReopenThread {
                review: review_id(),
                thread: ThreadId::new("55555555555555555555555555555555"),
            },
            Request::ResolveAddressed {
                review: review_id(),
            },
            Request::GetAttachment {
                review: review_id(),
            },
            Request::ListClis,
            Request::CompareForReview {
                workspace: workspace.clone(),
                review: review_id(),
                from: COMMIT.to_owned(),
                to: "e".repeat(40),
            },
            Request::ListConsents,
            Request::AnswerConsent {
                agent: "pi".to_owned(),
                answer: ConsentAnswer::Yes,
            },
            Request::ExportVersion {
                workspace: workspace.clone(),
                version: COMMIT.to_owned(),
                into: PathBuf::from("/home/a/.cache/yaar-review/navigator/7/old/1"),
            },
            Request::CreateWorktree {
                repository: RepositoryId::new("0123456789abcdef0123456789abcdef"),
                from: "origin/main".to_owned(),
                branch: "feature/é-login".to_owned(),
                agent: "pi".to_owned(),
                size: TerminalSize {
                    columns: 120,
                    rows: 40,
                },
            },
            Request::GetStateSizes,
            Request::CleanUpStates,
        ];
        // The two other answers to a question for leave.
        let refused = Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        };
        let asked_again = Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::NotAsked,
        };
        // The launcher of a CLI started for a review, a session resumed, and the review
        // attached whole.
        let for_a_review = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::AgentFor {
                agent: "claude".to_owned(),
                review: review_id(),
            },
            size: TerminalSize {
                columns: 80,
                rows: 24,
            },
        };
        let resumed = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Resume {
                session: session_id(),
            },
            size: TerminalSize {
                columns: 80,
                rows: 24,
            },
        };
        let the_whole_review = Request::AttachReview {
            review: review_id(),
            thread: None,
        };
        // A launcher, a report from outside the app's terminals, the files on disk as the
        // new side, no workspace on screen, and the uncommitted list of no review named.
        let launcher = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Agent {
                agent: "claude".to_owned(),
            },
            size: TerminalSize {
                columns: 80,
                rows: 24,
            },
        };
        let from_outside = Request::Report {
            report: AgentReport {
                agent: "claude".to_owned(),
                terminal: None,
                pid: 1,
                payload: Vec::new(),
            },
        };
        let to_disk = Request::CompareVersions {
            workspace: workspace.clone(),
            from: STATE.to_owned(),
            to: None,
        };
        let disk_side = Request::ReadVersions {
            workspace: workspace.clone(),
            old: None,
            old_path: PathBuf::from("src/new.rs"),
            new: None,
            path: PathBuf::from("src/new.rs"),
        };
        let nothing_on_screen = Request::ShowWorkspace { workspace: None };
        let every_change = Request::GetUncommitted {
            workspace: workspace.clone(),
            review: None,
        };
        // A draft saved again, or a reply, names a thread that exists.
        let in_a_thread = Request::SaveDraft {
            review: review_id(),
            thread: ThreadId::new("55555555555555555555555555555555"),
            comment: CommentId::new("77777777777777777777777777777777"),
            on: DraftOn::Thread,
            text: String::new(),
        };
        let unviewed = Request::SetViewed {
            workspace: workspace.clone(),
            path: PathBuf::from("src/lib.rs"),
            viewed: false,
        };
        // The same kind of request, asking for no old version.
        let without_old = Request::ReadFile {
            workspace: workspace.clone(),
            since: None,
            old_path: PathBuf::from("src/new.rs"),
            path: PathBuf::from("src/new.rs"),
        };
        let replies = [
            Reply::Done,
            Reply::Workspaces(two_repositories()),
            Reply::Workspaces(Vec::new()),
            Reply::Opened {
                repository: RepositoryId::new("0123456789abcdef0123456789abcdef"),
                workspace: workspace.clone(),
            },
            Reply::Changes(changes(
                Comparison::SinceBase {
                    base: base("origin/main", BaseSource::Default),
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::NotOnUpstream {
                    base: base("origin/main", BaseSource::Default),
                    upstream: "origin/main".to_owned(),
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::OnNoRemoteBranch {
                    base: base("trunk", BaseSource::User),
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::NothingToCompare {
                    base: Some(base("main", BaseSource::Default)),
                    why: NoComparison::NoRemote,
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::NothingToCompare {
                    base: Some(base(COMMIT, BaseSource::User)),
                    why: NoComparison::BaseGone,
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::NothingToCompare {
                    base: Some(base("main", BaseSource::Default)),
                    why: NoComparison::NoSharedCommit,
                },
                Some(COMMIT),
            )),
            Reply::Changes(changes(
                Comparison::NothingToCompare {
                    base: None,
                    why: NoComparison::NoBase,
                },
                None,
            )),
            Reply::File(FileSides {
                old: Content::Absent,
                new: Content::Text(b"fn main() {}\n".to_vec()),
            }),
            Reply::File(FileSides {
                old: Content::Binary { size: 1 << 33 },
                new: Content::TooLarge {
                    size: MAX_CONTENT + 1,
                },
            }),
            Reply::TerminalOpened {
                terminal: terminal(),
            },
            Reply::Reviews(reviews()),
            Reply::Reviews(WorkspaceReviews {
                workspace: workspace.clone(),
                reviews: Vec::new(),
                viewed: Vec::new(),
            }),
            Reply::Copied {
                text: "## Thread 5555\n```\nfn main() {}\n```\n".to_owned(),
                round: u32::MAX,
            },
            Reply::Committed {
                commit: COMMIT.to_owned(),
            },
            Reply::Agents(vec![agents(), {
                let mut none = agents();
                none.shown = None;
                none.sessions.clear();
                none
            }]),
            Reply::Agents(Vec::new()),
            Reply::History(history()),
            Reply::History(History {
                workspace: workspace.clone(),
                start: None,
                entries: Vec::new(),
            }),
            Reply::Uncommitted(uncommitted()),
            Reply::Uncommitted(Vec::new()),
            Reply::Compared(
                changes(
                    Comparison::NothingToCompare {
                        base: None,
                        why: NoComparison::NoBase,
                    },
                    None,
                )
                .files,
            ),
            Reply::Turn(turn(Some(TurnEnd::Finished), Some(STATE))),
            Reply::Turn(turn(Some(TurnEnd::Interrupted), None)),
            Reply::Turn(turn(None, None)),
            Reply::Terminals(vec![
                TerminalInfo {
                    id: terminal(),
                    workspace: workspace.clone(),
                    program: Program::Shell,
                },
                TerminalInfo {
                    id: TerminalId::new("44444444444444444444444444444444"),
                    workspace: workspace.clone(),
                    program: Program::Agent {
                        agent: "claude".to_owned(),
                    },
                },
            ]),
            Reply::Attached { cut: true },
            Reply::Attached { cut: false },
            Reply::Settings(vec![
                Setting::FlowForm {
                    workspace: workspace.clone(),
                    form: FlowForm::Timeline,
                },
                Setting::FlowForm {
                    workspace: WorkspaceId::new("22222222222222222222222222222222"),
                    form: FlowForm::LiveDiff,
                },
                Setting::ServersInRepository {
                    repository: RepositoryId::new("33333333333333333333333333333333"),
                    allowed: true,
                },
                Setting::ServersInRepository {
                    repository: RepositoryId::new("44444444444444444444444444444444"),
                    allowed: false,
                },
                Setting::TurnEnd(TurnEndSelects::WhatChanged),
                Setting::TurnEnd(TurnEndSelects::KeepSelection),
                Setting::StopAfter { seconds: 1 },
                Setting::StopAfter { seconds: u32::MAX },
                Setting::Theme(ThemeChoice::System),
                Setting::Theme(ThemeChoice::Named("Light".to_owned())),
                Setting::Theme(ThemeChoice::Named("Sépia".to_owned())),
                Setting::TerminalPlace(TerminalPlace::Right),
                Setting::TerminalPlace(TerminalPlace::Below),
                Setting::TerminalPlace(TerminalPlace::Left),
                Setting::KeepStates { days: 30 },
                Setting::KeepStates { days: u32::MAX },
                Setting::OptionKey(OptionKey::ByLayout),
                Setting::OptionKey(OptionKey::Meta),
                Setting::OptionKey(OptionKey::Characters),
                Setting::WorktreesFolder {
                    repository: RepositoryId::new("33333333333333333333333333333333"),
                    folder: Some(PathBuf::from("/src/worktrees de l'été")),
                },
                Setting::WorktreesFolder {
                    repository: RepositoryId::new("44444444444444444444444444444444"),
                    folder: None,
                },
            ]),
            Reply::Settings(Vec::new()),
            Reply::Answer {
                bytes: b"{\"systemMessage\": \"\xc3\xa9\"}\n".to_vec(),
            },
            Reply::Answer { bytes: Vec::new() },
            Reply::AttachedReview {
                start: vec![AgentCli {
                    agent: "claude".to_owned(),
                    title: "Claude Code".to_owned(),
                }],
            },
            Reply::AttachedReview { start: Vec::new() },
            Reply::Attachment {
                text: "Answer each thread.\n\n## Thread 5555".to_owned(),
            },
            Reply::Clis(vec![
                AgentCli {
                    agent: "claude".to_owned(),
                    title: "Claude Code".to_owned(),
                },
                AgentCli {
                    agent: "pi".to_owned(),
                    title: "pi".to_owned(),
                },
            ]),
            Reply::Clis(Vec::new()),
            Reply::ComparedForReview(vec![
                CreditedFile {
                    file: ChangedFile {
                        path: PathBuf::from("src/both.rs"),
                        status: FileStatus::Modified,
                    },
                    by: vec![SessionId::new("a-session"), SessionId::new("b-session")],
                },
                CreditedFile {
                    file: ChangedFile {
                        path: PathBuf::from("by-hand.txt"),
                        status: FileStatus::Added,
                    },
                    by: Vec::new(),
                },
            ]),
            Reply::ComparedForReview(Vec::new()),
            Reply::Consents(vec![consent(ConsentAnswer::NotAsked, None)]),
            Reply::Consents(vec![
                consent(ConsentAnswer::Yes, None),
                consent(
                    ConsentAnswer::No,
                    Some("the settings at /home/u/.pi/agent/settings.json are not JSON"),
                ),
            ]),
            Reply::Consents(Vec::new()),
            Reply::WorktreeCreated {
                workspace: workspace.clone(),
                terminal: terminal(),
            },
            Reply::StateSizes(StateSizes {
                workspaces: vec![
                    StateSize {
                        workspace: workspace.clone(),
                        bytes: 36_864,
                    },
                    StateSize {
                        workspace: WorkspaceId::new("22222222222222222222222222222222"),
                        bytes: u64::MAX / 2,
                    },
                ],
                total: u64::MAX,
            }),
            Reply::StateSizes(StateSizes {
                workspaces: Vec::new(),
                total: 0,
            }),
        ];
        let refusals = [
            Refusal::NoSuchFolder {
                path: PathBuf::from("/nowhere"),
            },
            Refusal::NotARepository {
                path: PathBuf::from("/tmp"),
            },
            Refusal::BareRepository {
                path: PathBuf::from("/srv/empty.git"),
            },
            Refusal::CopiedWorktree {
                path: PathBuf::from("/home/a/gits/yaar copy"),
            },
            Refusal::UnknownWorkspace {
                workspace: workspace.clone(),
            },
            Refusal::UnknownRevision {
                revision: "no-such-branch".to_owned(),
            },
            Refusal::OutsideWorkspace {
                path: PathBuf::from("../elsewhere"),
            },
            Refusal::UnknownTerminal {
                terminal: terminal(),
            },
            Refusal::UnknownReview {
                review: review_id(),
            },
            Refusal::UnknownThread {
                thread: ThreadId::new("55555555555555555555555555555555"),
            },
            Refusal::UnknownComment {
                comment: CommentId::new("66666666666666666666666666666666"),
            },
            Refusal::NotADraft {
                comment: CommentId::new("66666666666666666666666666666666"),
            },
            Refusal::NothingToSend {
                review: review_id(),
            },
            Refusal::NothingToCommit {
                workspace: workspace.clone(),
            },
            Refusal::Failed {
                reason: "git was not found".to_owned(),
            },
            Refusal::UnknownTurn { turn: turn_id() },
        ];
        let facts = [
            Fact::Workspaces(two_repositories()),
            Fact::ChangesChanged {
                workspace: workspace.clone(),
            },
            Fact::ReviewsChanged {
                workspace: workspace.clone(),
            },
            Fact::AgentsChanged(agents()),
            Fact::TurnEnded {
                workspace: workspace.clone(),
                session: session_id(),
                turn: turn_id(),
                state: Some(STATE.to_owned()),
                since: Some(COMMIT.to_owned()),
            },
            Fact::TurnEnded {
                workspace: workspace.clone(),
                session: session_id(),
                turn: turn_id(),
                state: None,
                since: None,
            },
            Fact::SettingChanged(Setting::FlowForm {
                workspace: workspace.clone(),
                form: FlowForm::LiveDiff,
            }),
            Fact::ConsentsChanged(vec![consent(ConsentAnswer::Yes, None)]),
            Fact::ConsentsChanged(Vec::new()),
            Fact::StatesDeleted {
                workspace: workspace.clone(),
            },
            Fact::SettingChanged(Setting::Theme(ThemeChoice::Named("Dark".to_owned()))),
            Fact::SettingChanged(Setting::WorktreesFolder {
                repository: RepositoryId::new("33333333333333333333333333333333"),
                folder: None,
            }),
            Fact::Step {
                workspace,
                session: session_id(),
                turn: turn_id(),
                number: u32::MAX,
                step: Step::Command {
                    command: "echo x > made.txt".to_owned(),
                },
            },
            Fact::TerminalEnded {
                terminal: terminal(),
                ending: Ending::Exited(0),
            },
            Fact::TerminalEnded {
                terminal: terminal(),
                ending: Ending::Exited(-1),
            },
            Fact::TerminalEnded {
                terminal: terminal(),
                ending: Ending::Signalled(9),
            },
        ];

        let mut messages = Vec::new();
        for (n, request) in requests.into_iter().enumerate() {
            messages.push(Message::Request {
                id: RequestId(n as u32),
                request,
            });
        }
        messages.push(Message::Request {
            id: RequestId(u32::MAX),
            request: without_old,
        });
        for request in [in_a_thread, unviewed] {
            messages.push(Message::Request {
                id: RequestId(u32::MAX - 1),
                request,
            });
        }
        for request in [
            launcher,
            from_outside,
            to_disk,
            disk_side,
            nothing_on_screen,
            every_change,
        ] {
            messages.push(Message::Request {
                id: RequestId(u32::MAX - 2),
                request,
            });
        }
        for request in [for_a_review, resumed, the_whole_review] {
            messages.push(Message::Request {
                id: RequestId(u32::MAX - 3),
                request,
            });
        }
        for request in [refused, asked_again] {
            messages.push(Message::Request {
                id: RequestId(u32::MAX - 4),
                request,
            });
        }
        for reply in replies {
            messages.push(Message::Reply {
                to: RequestId(u32::MAX),
                reply: Ok(reply),
            });
        }
        for refusal in refusals {
            messages.push(Message::Reply {
                to: RequestId(7),
                reply: Err(refusal),
            });
        }
        messages.extend(facts.into_iter().map(Message::Fact));
        // The bytes of a terminal, every byte value among them, and none at all.
        let every_byte: Vec<u8> = (0..=u8::MAX).collect();
        messages.push(Message::Input {
            terminal: terminal(),
            bytes: every_byte.clone(),
        });
        messages.push(Message::Output {
            terminal: terminal(),
            bytes: every_byte,
        });
        messages.push(Message::Output {
            terminal: terminal(),
            bytes: Vec::new(),
        });
        messages
    }

    /// The id of a terminal, as the Keeper makes one.
    fn terminal() -> TerminalId {
        TerminalId::new("33333333333333333333333333333333")
    }

    fn review_id() -> ReviewId {
        ReviewId::new("88888888888888888888888888888888")
    }

    /// pi's question for leave, answered as given.
    fn consent(answer: ConsentAnswer, problem: Option<&str>) -> Consent {
        Consent {
            agent: "pi".to_owned(),
            question: "Install the app's extension into pi's settings?".to_owned(),
            answer,
            problem: problem.map(str::to_owned),
        }
    }

    /// The full id of a recorded state.
    const STATE: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

    fn session_id() -> SessionId {
        SessionId::new("cccccccccccccccccccccccccccccccc")
    }

    fn turn_id() -> TurnId {
        TurnId::new("dddddddddddddddddddddddddddddddd")
    }

    /// The sessions of a workspace in every state, one with no terminal and no review.
    fn agents() -> WorkspaceAgents {
        let states = [
            SessionState::WaitsForPermission,
            SessionState::Unreviewed,
            SessionState::Works,
            SessionState::Idle,
            SessionState::NotRunning,
        ];
        WorkspaceAgents {
            workspace: WorkspaceId::new("11111111111111111111111111111111"),
            shown: Some(SessionState::WaitsForPermission),
            sessions: states
                .iter()
                .enumerate()
                .map(|(n, state)| Session {
                    id: SessionId::new(n.to_string().repeat(32)),
                    agent: "claude".to_owned(),
                    title: format!("Claude Code {}", n + 1),
                    workspace: WorkspaceId::new("11111111111111111111111111111111"),
                    terminal: (*state != SessionState::NotRunning).then(terminal),
                    state: *state,
                    turn: matches!(
                        state,
                        SessionState::Works | SessionState::WaitsForPermission
                    )
                    .then(turn_id),
                    review: (n != 0).then(review_id),
                    attached: match n {
                        1 => Some(Attachment::Review),
                        2 => Some(Attachment::Thread(ThreadId::new("a".repeat(32)))),
                        _ => None,
                    },
                    turn_from_user: *state == SessionState::Works,
                    resumable: *state == SessionState::NotRunning,
                })
                .collect(),
        }
    }

    /// A turn with a step of every kind, counted from a state when it ended with one.
    fn turn(ended: Option<TurnEnd>, state: Option<&str>) -> Turn {
        Turn {
            id: turn_id(),
            session: session_id(),
            number: 3,
            ended,
            start: state.map(|_| "6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b".to_owned()),
            state: state.map(str::to_owned),
            steps: vec![
                Step::Read {
                    path: PathBuf::from("notes.txt"),
                },
                Step::Edit {
                    path: PathBuf::from("src/été.rs"),
                    lines: Some(vec![3..=3, 10..=12, u32::MAX..=u32::MAX]),
                },
                Step::Edit {
                    path: PathBuf::from("notes.txt"),
                    lines: None,
                },
                Step::Edit {
                    path: PathBuf::from("empty.txt"),
                    lines: Some(Vec::new()),
                },
                Step::Write {
                    path: PathBuf::from("/elsewhere/new.txt"),
                },
                Step::Command {
                    command: "sed -i '' 's/a/b/' notes.txt".to_owned(),
                },
            ],
        }
    }

    /// A history with a commit and a turn, and changes made by a session and by no
    /// agent.
    fn history() -> History {
        History {
            workspace: WorkspaceId::new("11111111111111111111111111111111"),
            start: Some(COMMIT.to_owned()),
            entries: vec![
                HistoryEntry::Commit(Commit {
                    id: COMMIT.to_owned(),
                    summary: "Start".to_owned(),
                }),
                HistoryEntry::Turn {
                    turn: turn_id(),
                    session: session_id(),
                    state: Some(STATE.to_owned()),
                },
                HistoryEntry::Turn {
                    turn: turn_id(),
                    session: session_id(),
                    state: None,
                },
                HistoryEntry::ExpiredTurn {
                    turn: turn_id(),
                    session: session_id(),
                },
            ],
        }
    }

    /// Uncommitted changes made by a session and by no agent.
    fn uncommitted() -> Vec<CreditedFile> {
        vec![
            CreditedFile {
                file: ChangedFile {
                    path: PathBuf::from("by-the-agent.txt"),
                    status: FileStatus::Added,
                },
                by: vec![session_id()],
            },
            CreditedFile {
                file: ChangedFile {
                    path: PathBuf::from("src/moved.rs"),
                    status: FileStatus::Renamed {
                        from: PathBuf::from("src/was here.rs"),
                    },
                },
                by: Vec::new(),
            },
        ]
    }

    fn anchor(tree: Option<&str>, side: Side) -> Anchor {
        Anchor {
            tree: tree.map(str::to_owned),
            path: PathBuf::from("src/was here.rs"),
            side,
            lines: Lines {
                first: 12,
                last: u32::MAX,
            },
            text: "    let a = 1;\n    let b = 2;".to_owned(),
        }
    }

    /// Reviews of every shape a review can have: of no agent and of a session, with a
    /// draft, a sent comment, an agent's reply, a resolved thread, an attachment of the
    /// review and of one thread, and rounds of both kinds.
    fn reviews() -> WorkspaceReviews {
        let comment = |id: char, author: Author, round: Option<u32>| Comment {
            id: CommentId::new(id.to_string().repeat(32)),
            author,
            text: format!("comment {id}"),
            round,
        };
        WorkspaceReviews {
            workspace: WorkspaceId::new("11111111111111111111111111111111"),
            reviews: vec![
                Review {
                    id: review_id(),
                    agent: None,
                    reviewed: Some(COMMIT.to_owned()),
                    attached: Some(Attachment::Review),
                    threads: vec![
                        Thread {
                            id: ThreadId::new("a".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::New),
                            resolved: false,
                            addressed: true,
                            comments: vec![
                                comment('b', Author::Reviewer, Some(1)),
                                comment('c', Author::Agent, Some(1)),
                                comment('d', Author::Reviewer, None),
                            ],
                            state: ThreadState::Draft,
                            now: Some(Placed {
                                tree: COMMIT.to_owned(),
                                path: PathBuf::from("src/renamed.rs"),
                                side: Side::New,
                                lines: Lines { first: 3, last: 4 },
                                notice: Some(Notice::Changed),
                            }),
                        },
                        Thread {
                            id: ThreadId::new("e".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::Old),
                            resolved: true,
                            addressed: false,
                            comments: vec![comment('f', Author::Reviewer, None)],
                            state: ThreadState::Resolved,
                            now: None,
                        },
                        Thread {
                            id: ThreadId::new("1".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::New),
                            resolved: false,
                            addressed: false,
                            comments: vec![
                                comment('2', Author::Reviewer, Some(2)),
                                comment('3', Author::Agent, Some(2)),
                            ],
                            state: ThreadState::WaitsForReviewer,
                            now: Some(Placed {
                                tree: COMMIT.to_owned(),
                                path: PathBuf::from("src/gone.rs"),
                                side: Side::New,
                                lines: Lines { first: 1, last: 1 },
                                notice: Some(Notice::Deleted),
                            }),
                        },
                        Thread {
                            id: ThreadId::new("4".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::New),
                            resolved: false,
                            addressed: true,
                            comments: vec![comment('5', Author::Reviewer, Some(2))],
                            state: ThreadState::Addressed,
                            now: Some(Placed {
                                tree: COMMIT.to_owned(),
                                path: PathBuf::from("src/lib.rs"),
                                side: Side::New,
                                lines: Lines { first: 9, last: 9 },
                                notice: None,
                            }),
                        },
                        Thread {
                            id: ThreadId::new("6".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::New),
                            resolved: false,
                            addressed: false,
                            comments: vec![comment('7', Author::Reviewer, Some(2))],
                            state: ThreadState::Sent,
                            now: None,
                        },
                    ],
                    rounds: vec![
                        Round {
                            number: 1,
                            state: Some(COMMIT.to_owned()),
                            sent_by: SentBy::Copied,
                            edited: false,
                            turn: None,
                        },
                        Round {
                            number: 2,
                            state: Some(COMMIT.to_owned()),
                            sent_by: SentBy::Delivered,
                            edited: true,
                            turn: Some(TurnId::new("d".repeat(32))),
                        },
                        Round {
                            number: 3,
                            state: None,
                            sent_by: SentBy::Delivered,
                            edited: false,
                            turn: Some(TurnId::new("e".repeat(32))),
                        },
                    ],
                },
                Review {
                    id: ReviewId::new("9".repeat(32)),
                    agent: Some(SessionId::new("a-session")),
                    reviewed: None,
                    attached: Some(Attachment::Thread(ThreadId::new("a".repeat(32)))),
                    threads: Vec::new(),
                    rounds: Vec::new(),
                },
            ],
            viewed: vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/été.rs")],
        }
    }

    #[test]
    fn every_request_reply_refusal_and_fact_comes_back_as_it_was_sent() {
        let messages = every_shape();
        let mut bytes = Vec::new();
        for message in &messages {
            write_message(&mut bytes, message).expect("write to a vector");
        }
        let mut input = Cursor::new(bytes);
        for message in &messages {
            assert_eq!(&read_message(&mut input).expect("read"), message);
        }
        assert!(matches!(read_message(&mut input), Err(LinkError::Closed)));
    }

    #[test]
    fn the_shapes_tried_above_leave_no_kind_of_request_out() {
        let tried: Vec<RequestKind> = every_shape()
            .iter()
            .filter_map(|message| match message {
                Message::Request { request, .. } => Some(request.kind()),
                _ => None,
            })
            .collect();
        // One of each, in the order of the table; eleven kinds are tried a second time, the
        // opening of a terminal a third and a fourth, for each kind of program, and the
        // answer to a question for leave a third, for each answer.
        assert_eq!(tried[..RequestKind::ALL.len()], *RequestKind::ALL);
        assert_eq!(
            tried[RequestKind::ALL.len()..],
            [
                RequestKind::ReadFile,
                RequestKind::SaveDraft,
                RequestKind::SetViewed,
                RequestKind::OpenTerminal,
                RequestKind::Report,
                RequestKind::CompareVersions,
                RequestKind::ReadVersions,
                RequestKind::ShowWorkspace,
                RequestKind::GetUncommitted,
                RequestKind::OpenTerminal,
                RequestKind::OpenTerminal,
                RequestKind::AttachReview,
                RequestKind::AnswerConsent,
                RequestKind::AnswerConsent,
            ]
        );
    }

    #[test]
    fn every_kind_of_request_has_its_own_number_and_says_who_starts_it() {
        for kind in RequestKind::ALL {
            assert_eq!(RequestKind::from_code(kind.code()), Some(*kind));
        }
        let mut codes: Vec<u16> = RequestKind::ALL.iter().map(|kind| kind.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), RequestKind::ALL.len());
        assert_eq!(RequestKind::from_code(0), None);

        let by_user: Vec<RequestKind> = RequestKind::started_by_user().collect();
        assert_eq!(
            by_user,
            [
                RequestKind::OpenFolder,
                RequestKind::SetBase,
                RequestKind::OpenTerminal,
                RequestKind::DeleteDraft,
                RequestKind::CopyReview,
                RequestKind::MarkReviewed,
                RequestKind::SetViewed,
                RequestKind::Commit,
                RequestKind::SetSetting,
                RequestKind::AttachReview,
                RequestKind::DetachReview,
                RequestKind::ResolveThread,
                RequestKind::ReopenThread,
                RequestKind::ResolveAddressed,
                RequestKind::AnswerConsent,
                RequestKind::CreateWorktree,
                RequestKind::CleanUpStates,
            ]
        );
        assert_eq!(RequestKind::GetChanges.origin(), Origin::Window);
        // The Window reads the sizes of the recorded states to show them, when the
        // settings screen opens and after a fact says they changed.
        assert_eq!(RequestKind::GetStateSizes.origin(), Origin::Window);
        // The Window reads the reviews to show them, and keeps what the user types in a
        // comment box by itself: no control stands for either.
        assert_eq!(RequestKind::GetReviews.origin(), Origin::Window);
        assert_eq!(RequestKind::SaveDraft.origin(), Origin::Window);
        // The Window tells the size it shows a terminal at by itself, whenever its
        // layout changes.
        assert_eq!(RequestKind::ResizeTerminal.origin(), Origin::Window);
        // The Window reads the agents, the history, the versions of a change, the
        // terminals and the settings to have something to show, and tells by itself which
        // workspace it shows; a user who selects a workspace starts nothing of the
        // Keeper's. Keeping a setting is the user's: a control of the Window sends it.
        for kind in [
            RequestKind::ListAgents,
            RequestKind::GetHistory,
            RequestKind::CompareVersions,
            RequestKind::ReadVersions,
            RequestKind::GetTurn,
            RequestKind::ShowWorkspace,
            RequestKind::ListTerminals,
            RequestKind::AttachTerminal,
            RequestKind::GetUncommitted,
            RequestKind::GetSettings,
            // What the control that views an attachment shows is read to be shown; the
            // control itself sends nothing a user starts.
            RequestKind::GetAttachment,
            // The launchers offered are read to be drawn, and what a review shows of a
            // change is read to be shown.
            RequestKind::ListClis,
            RequestKind::CompareForReview,
            // Which Adapters need the user's leave is read to know what to ask; the answer
            // is the user's, by a control.
            RequestKind::ListConsents,
        ] {
            assert_eq!(kind.origin(), Origin::Window, "{kind:?}");
        }
        // Only a piece of the app inside an agent CLI reports.
        let by_agent: Vec<RequestKind> = RequestKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.origin() == Origin::Agent)
            .collect();
        assert_eq!(by_agent, [RequestKind::Report]);
        assert_eq!(RequestKind::OpenFolder.name(), "OpenFolder");
    }

    #[test]
    fn a_request_keeps_its_bytes() {
        // Not a promise to other versions, as the hello is: a check that the layout is
        // the one wire.rs describes.
        let bytes = frame(&Message::Request {
            id: RequestId(0x0102_0304),
            request: Request::SetBase {
                workspace: WorkspaceId::new("ab"),
                base: "main".to_owned(),
            },
        });
        assert_eq!(
            bytes,
            [
                21, 0, 0, 0, // length
                3, // kind: a request
                4, 3, 2, 1, // its id
                3, 0, // SetBase
                2, 0, 0, 0, b'a', b'b', // the workspace
                4, 0, 0, 0, b'm', b'a', b'i', b'n', // the base
            ]
        );
    }

    #[test]
    fn the_bytes_of_a_terminal_keep_their_frame() {
        // Not a promise to other versions either: a check of the layout of the two frames
        // that carry a terminal's bytes, which no reply follows.
        let bytes = frame(&Message::Input {
            terminal: TerminalId::new("t1"),
            bytes: vec![0x1b, b'[', b'A'],
        });
        assert_eq!(
            bytes,
            [
                14, 0, 0, 0, // length
                6, // kind: input
                2, 0, 0, 0, b't', b'1', // the terminal
                3, 0, 0, 0, 0x1b, b'[', b'A', // the bytes
            ]
        );
        let bytes = frame(&Message::Output {
            terminal: TerminalId::new("t1"),
            bytes: vec![b'$'],
        });
        assert_eq!(bytes[4], 7, "kind: output");
    }

    #[test]
    fn the_side_with_the_lower_protocol_is_the_older_whatever_its_app_version_says() {
        let mine = Hello {
            protocol: 3,
            pid: 1,
            app_version: "0.2.0".to_owned(),
        };
        let with = |protocol: u16, app_version: &str| Hello {
            protocol,
            pid: 2,
            app_version: app_version.to_owned(),
        };
        assert_eq!(with(2, "9.9.9").age_against(&mine), Age::Older);
        assert_eq!(with(4, "0.0.1").age_against(&mine), Age::Newer);
        assert_eq!(with(3, "0.2.0").age_against(&mine), Age::Same);
    }

    #[test]
    fn with_one_protocol_the_app_versions_are_compared_as_numbers() {
        let mine = Hello {
            protocol: 3,
            pid: 1,
            app_version: "0.2.10".to_owned(),
        };
        let with = |app_version: &str| Hello {
            protocol: 3,
            pid: 2,
            app_version: app_version.to_owned(),
        };
        assert_eq!(with("0.2.9").age_against(&mine), Age::Older);
        assert_eq!(with("0.10.0").age_against(&mine), Age::Newer);
        assert_eq!(with("0.2.10-beta").age_against(&mine), Age::Unknown);
        assert_eq!(with("0.2.10.0").age_against(&mine), Age::Unknown);
        assert_eq!(with("0.3").age_against(&mine), Age::Newer);
        assert_eq!(with("scripted-window").age_against(&mine), Age::Unknown);
        assert_eq!(with("").age_against(&mine), Age::Unknown);
    }

    #[test]
    fn a_frame_with_more_or_less_than_its_fields_is_refused() {
        for message in every_shape() {
            let whole = frame(&message);

            let mut longer = whole.clone();
            longer.push(0);
            let len = (longer.len() - 4) as u32;
            longer[..4].copy_from_slice(&len.to_le_bytes());
            assert!(
                matches!(
                    read_message(&mut Cursor::new(longer)),
                    Err(LinkError::Malformed(_))
                ),
                "one byte more: {message:?}"
            );

            let mut shorter = whole[..whole.len() - 1].to_vec();
            let len = (shorter.len() - 4) as u32;
            shorter[..4].copy_from_slice(&len.to_le_bytes());
            assert!(
                matches!(
                    read_message(&mut Cursor::new(shorter)),
                    Err(LinkError::Malformed(_))
                ),
                "one byte less: {message:?}"
            );
        }
    }

    #[test]
    fn a_list_that_claims_more_items_than_its_frame_holds_is_refused() {
        // A fact with the list of repositories, whose count says four thousand million.
        let bytes = [7u8, 0, 0, 0, 5, 1, 0, 0xff, 0xff, 0xff, 0xff];
        assert!(matches!(
            read_message(&mut Cursor::new(bytes)),
            Err(LinkError::Malformed(_))
        ));
    }

    #[test]
    fn a_frame_too_long_to_be_read_is_not_written() {
        let message = Message::Reply {
            to: RequestId(1),
            reply: Ok(Reply::File(FileSides {
                old: Content::Text(vec![b'a'; MAX_FRAME as usize]),
                new: Content::Absent,
            })),
        };
        let error = write_message(&mut Vec::new(), &message).expect_err("too long");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_workspace_is_listed_by_its_branch_or_by_the_commit_it_sits_on() {
        assert_eq!(
            Head::Branch("feature/login".to_owned()).label(),
            "feature/login"
        );
        assert_eq!(
            Head::Detached("89abcdef0123456789abcdef0123456789abcdef".to_owned()).label(),
            "detached at 89abcde"
        );
    }

    fn executable(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "#!/bin/sh\n").expect("write a file");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("set its mode");
    }

    fn shell_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("yaar-link-unit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("make a folder");
        folder
    }

    #[test]
    fn shell_names_a_program_by_an_absolute_path_that_can_be_run() {
        let folder = shell_folder("shell");
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
