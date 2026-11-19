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
//!   [`TerminalInfo`], a terminal the Keeper holds; and [`Setting`], a setting the user
//!   set, as the [`FlowForm`] of a workspace;
//! - [`Age`], how the side that sent a hello stands to this one in version;
//! - [`socket_path`], [`socket_address`] and [`data_dir`], the places both programs find
//!   by the same rule, and [`connect`] with [`check_private_folder`], by which whoever
//!   reaches the Keeper makes sure it is the user's own.
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
mod request;
mod review;
mod setting;
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
    AgentReport, CreditedFile, History, HistoryEntry, Session, SessionState, Step, TerminalInfo,
    Turn, TurnEnd, WorkspaceAgents,
};
pub use request::{Fact, Origin, Refusal, Reply, Request, RequestId, RequestKind};
pub use review::{
    Anchor, Attachment, Author, Comment, CommentId, DraftOn, Lines, Review, ReviewId, Round,
    SentBy, SessionId, Side, Thread, ThreadId, TurnId, WorkspaceReviews,
};
pub use setting::{FlowForm, Setting};
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
/// step.
pub const PROTOCOL_VERSION: u16 = 6;

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
        ];
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
            ]),
            Reply::Settings(Vec::new()),
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
                        },
                        Thread {
                            id: ThreadId::new("e".repeat(32)),
                            anchor: anchor(Some(COMMIT), Side::Old),
                            resolved: true,
                            addressed: false,
                            comments: vec![comment('f', Author::Reviewer, None)],
                        },
                    ],
                    rounds: vec![
                        Round {
                            number: 1,
                            state: COMMIT.to_owned(),
                            sent_by: SentBy::Copied,
                        },
                        Round {
                            number: 2,
                            state: COMMIT.to_owned(),
                            sent_by: SentBy::Delivered,
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
        // One of each, in the order of the table; nine kinds are tried a second time.
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
            ]
        );
        assert_eq!(RequestKind::GetChanges.origin(), Origin::Window);
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
}
