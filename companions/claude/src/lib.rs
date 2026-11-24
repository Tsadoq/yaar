//! Claude Code companion: the hook program and the reply tool server of the app's plugin.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yaar_link::{
    AgentReport, Hello, LinkError, Message, Refusal, Reply, Request, RequestId, TERMINAL_VARIABLE,
    TerminalId, connect, handshake, read_message, socket_path, write_message,
};

mod tools;

pub use tools::{SERVER_NAME, TOOL, TOOL_DEADLINE, ToolPlace, serve, tool};

const AGENT: &str = "claude";

const REQUEST: RequestId = RequestId(0);

/// The longest the hook program waits for the Keeper, from connecting to the reply.
/// Claude Code waits for the program, and so does the user at a permission prompt.
pub const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlace {
    pub socket: PathBuf,
    pub terminal: Option<String>,
    /// The program's parent process: Claude Code.
    pub parent: u32,
}

impl HookPlace {
    pub fn of_this_process() -> HookPlace {
        HookPlace {
            socket: socket_path(),
            // An empty variable is a shell's leftover, not a terminal.
            terminal: std::env::var(TERMINAL_VARIABLE)
                .ok()
                .filter(|terminal| !terminal.is_empty()),
            parent: std::os::unix::process::parent_id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAnswer {
    pub printed: Vec<u8>,
    pub status: i32,
}

/// A late or partial answer is not printed.
pub fn run_hook(input: &[u8], place: &HookPlace) -> HookAnswer {
    let sent = report_to_keeper(
        &Reporter {
            socket: &place.socket,
            who: concat!("yaar-claude-hook ", env!("CARGO_PKG_VERSION")),
            terminal: place.terminal.as_deref(),
            parent: place.parent,
        },
        input,
        Instant::now() + DEADLINE,
    );
    // A hook that said nothing serves Claude Code better than one that held the user up or failed.
    let printed = match sent {
        Ok(Ok(Reply::Answer { bytes })) => bytes,
        _ => Vec::new(),
    };
    HookAnswer { printed, status: 0 }
}

struct Reporter<'a> {
    socket: &'a Path,
    who: &'a str,
    terminal: Option<&'a str>,
    parent: u32,
}

#[derive(Debug)]
enum Unanswered {
    /// No Keeper was reached, or the one reached speaks another protocol: nothing was sent.
    NotRunning,
    TooLate,
    Failed,
}

type Answered = Result<Reply, Refusal>;

fn report_to_keeper(
    from: &Reporter<'_>,
    payload: &[u8],
    deadline: Instant,
) -> Result<Answered, Unanswered> {
    let mut stream = connect(from.socket).map_err(|_| Unanswered::NotRunning)?;
    let hello = Hello::for_this_process(from.who);
    limit(&stream, deadline).map_err(late)?;
    let keeper = handshake(&mut stream, &hello).map_err(|e| match e {
        LinkError::Io(e) if is_late(&e) => Unanswered::TooLate,
        _ => Unanswered::NotRunning,
    })?;
    if !hello.speaks_with(&keeper) {
        return Err(Unanswered::NotRunning);
    }
    let report = AgentReport {
        agent: AGENT.to_owned(),
        terminal: from.terminal.map(TerminalId::new),
        pid: from.parent,
        payload: payload.to_vec(),
    };
    limit(&stream, deadline).map_err(late)?;
    write_message(
        &mut stream,
        &Message::Request {
            id: REQUEST,
            request: Request::Report { report },
        },
    )
    .map_err(late)?;
    loop {
        // Set again for each frame: a Keeper that keeps telling facts must not stretch the
        // deadline.
        limit(&stream, deadline).map_err(late)?;
        if let Message::Reply { to: REQUEST, reply } =
            read_message(&mut stream).map_err(link_failure)?
        {
            return Ok(reply);
        }
    }
}

fn is_late(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn late(error: io::Error) -> Unanswered {
    if is_late(&error) {
        Unanswered::TooLate
    } else {
        Unanswered::Failed
    }
}

fn link_failure(error: LinkError) -> Unanswered {
    match error {
        LinkError::Io(e) => late(e),
        _ => Unanswered::Failed,
    }
}

fn limit(stream: &UnixStream, deadline: Instant) -> io::Result<()> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "the deadline is over"))?;
    match stream
        .set_read_timeout(Some(left))
        .and_then(|()| stream.set_write_timeout(Some(left)))
    {
        // macOS refuses a timeout on a socket whose peer has closed. Nothing can then block a read,
        // and the frames the Keeper left must still be read.
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => Ok(()),
        done => done,
    }
}
