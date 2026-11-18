//! Link (`S-PROTO`): what the Keeper and the Window say to each other.
//!
//! Owns: the messages between the two programs and the exchange of versions.
//!
//! Surface:
//! - [`Message`], with [`write_message`] and [`read_message`], which carry it as a frame;
//! - [`Hello`], [`handshake`] and [`PROTOCOL_VERSION`], the exchange of versions;
//! - [`Request`] with its [`RequestKind`] and [`Origin`], [`Reply`], [`Refusal`] and
//!   [`Fact`]: what a Window asks, what the Keeper answers, and what it tells unasked;
//! - what those carry: [`Repository`], [`Workspace`], [`Changes`], [`FileSides`] and their
//!   parts;
//! - [`socket_path`], [`socket_address`] and [`data_dir`], the places both programs find
//!   by the same rule.
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

mod request;
mod wire;
mod workspace;

use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub use request::{Fact, Origin, Refusal, Reply, Request, RequestId, RequestKind};
pub use workspace::{
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, FileSides, FileStatus,
    Head, MAX_CONTENT, Repository, RepositoryId, Workspace, WorkspaceId,
};

use wire::{In, Out};

/// The version of the protocol this crate speaks. Raised whenever a frame other than
/// the two fixed ones changes.
pub const PROTOCOL_VERSION: u16 = 2;

/// The largest frame a reader accepts.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// The folder name of the app in the user's data and runtime folders.
pub const APP_FOLDER: &str = "yaar-review";

const MAGIC: [u8; 4] = *b"YAAR";
const KIND_HELLO: u8 = 1;
const KIND_SHUTDOWN: u8 = 2;
const KIND_REQUEST: u8 = 3;
const KIND_REPLY: u8 = 4;
const KIND_FACT: u8 = 5;

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
    match input.read(&mut len[..1])? {
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
    if let Some(path) = set("YAAR_SOCKET") {
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
/// to find the Keeper of another. The folder of the short path belongs to the Keeper,
/// which creates it for the user alone and refuses one that somebody else owns.
pub fn socket_address(wanted: &Path) -> PathBuf {
    let _ = wanted;
    todo!()
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

    fn two_repositories() -> Vec<Repository> {
        vec![
            Repository {
                id: RepositoryId::new("0123456789abcdef0123456789abcdef"),
                name: "yaar".to_owned(),
                git_dir: PathBuf::from("/home/a/gits/yaar/.git"),
                workspaces: vec![
                    Workspace {
                        id: WorkspaceId::new("11111111111111111111111111111111"),
                        path: PathBuf::from("/home/a/gits/yaar"),
                        head: Head::Branch("main".to_owned()),
                    },
                    Workspace {
                        id: WorkspaceId::new("22222222222222222222222222222222"),
                        path: PathBuf::from("/home/a/gits/yaar.worktrees/été"),
                        head: Head::Detached("89abcdef0123456789abcdef0123456789abcdef".to_owned()),
                    },
                ],
            },
            Repository {
                id: RepositoryId::new("fedcba9876543210fedcba9876543210"),
                name: "empty".to_owned(),
                git_dir: PathBuf::from("/srv/empty.git"),
                workspaces: Vec::new(),
            },
        ]
    }

    fn base(name: &str, source: BaseSource) -> Base {
        Base {
            name: name.to_owned(),
            source,
        }
    }

    fn changes(comparison: Comparison) -> Changes {
        Changes {
            workspace: WorkspaceId::new("11111111111111111111111111111111"),
            comparison,
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
                path: PathBuf::from("src/lib.rs"),
            },
        ];
        let replies = [
            Reply::Done,
            Reply::Workspaces(two_repositories()),
            Reply::Workspaces(Vec::new()),
            Reply::Opened {
                repository: RepositoryId::new("0123456789abcdef0123456789abcdef"),
                workspace: workspace.clone(),
            },
            Reply::Changes(changes(Comparison::SinceBase {
                base: base("origin/main", BaseSource::Default),
                fork_point: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            })),
            Reply::Changes(changes(Comparison::NotOnUpstream {
                base: base("main", BaseSource::Default),
                upstream: "origin/main".to_owned(),
            })),
            Reply::Changes(changes(Comparison::OnNoRemoteBranch {
                base: base("trunk", BaseSource::User),
            })),
            Reply::Changes(changes(Comparison::NothingToCompare {
                base: Some(base("main", BaseSource::Default)),
            })),
            Reply::Changes(changes(Comparison::NothingToCompare { base: None })),
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
            Refusal::UnknownWorkspace {
                workspace: workspace.clone(),
            },
            Refusal::UnknownRevision {
                revision: "no-such-branch".to_owned(),
            },
            Refusal::OutsideWorkspace {
                path: PathBuf::from("../elsewhere"),
            },
            Refusal::Failed {
                reason: "git was not found".to_owned(),
            },
        ];
        let facts = [
            Fact::Workspaces(two_repositories()),
            Fact::ChangesChanged { workspace },
        ];

        let mut messages = Vec::new();
        for (n, request) in requests.into_iter().enumerate() {
            messages.push(Message::Request {
                id: RequestId(n as u32),
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
        messages
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
        assert_eq!(tried, RequestKind::ALL);
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
        assert_eq!(by_user, [RequestKind::OpenFolder, RequestKind::SetBase]);
        assert_eq!(RequestKind::GetChanges.origin(), Origin::Window);
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
