//! Link (`S-PROTO`): what the Keeper and the Window say to each other.
//!
//! Owns: the messages between the two programs and the exchange of versions.
//!
//! Surface: [`Message`], [`Hello`], [`write_message`], [`read_message`], [`handshake`],
//! [`socket_path`] and [`PROTOCOL_VERSION`].
//!
//! A message travels as one frame: a length of four bytes, little endian, that counts what
//! follows; one byte that names the kind; the payload.
//!
//! Two frames never change shape, whatever the protocol version: [`Message::Hello`] and
//! [`Message::Shutdown`]. A Window of one version must be able to tell a Keeper of another
//! who it is, and to ask it to stop.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// The version of the protocol this crate speaks. Raised whenever a frame other than
/// the two fixed ones changes.
pub const PROTOCOL_VERSION: u16 = 1;

/// The largest frame a reader accepts.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// The folder name of the app in the user's data and runtime folders.
pub const APP_FOLDER: &str = "yaar-review";

const MAGIC: [u8; 4] = *b"YAAR";
const KIND_HELLO: u8 = 1;
const KIND_SHUTDOWN: u8 = 2;

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
    }
    let len = (frame.len() - 4) as u32;
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

/// Where the Keeper listens.
///
/// `YAAR_SOCKET` names the socket outright. Otherwise it is `keeper.sock` in the app's
/// runtime folder: under `XDG_RUNTIME_DIR` on Linux when that is set, else in the app's
/// data folder (`~/Library/Application Support/yaar-review` on macOS,
/// `$XDG_DATA_HOME/yaar-review` or `~/.local/share/yaar-review` elsewhere).
pub fn socket_path() -> PathBuf {
    socket_path_from(|name| std::env::var_os(name).map(PathBuf::from))
}

fn socket_path_from(var: impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.as_os_str().is_empty());
    if let Some(path) = set("YAAR_SOCKET") {
        return path;
    }
    let home = set("HOME").unwrap_or_else(|| PathBuf::from("/"));
    let folder = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if let Some(runtime) = set("XDG_RUNTIME_DIR") {
        runtime
    } else if let Some(data) = set("XDG_DATA_HOME") {
        data
    } else {
        home.join(".local/share")
    };
    folder.join(APP_FOLDER).join("keeper.sock")
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
}
