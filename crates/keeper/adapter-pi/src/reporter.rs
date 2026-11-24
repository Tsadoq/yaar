//! The reporting program `yaar-pi-report [--tool] < <payload>`: sends the payload unread to the
//! Keeper as one `Request::Report` and prints the bytes of a `Reply::Answer`, else nothing. With
//! `--tool` it prints one JSON object in every case: the answer, else `{"error": <why>}`.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use yaar_link::{
    AgentReport, Hello, LinkError, Message, Refusal, Reply, Request, RequestId, TERMINAL_VARIABLE,
    TerminalId, connect, handshake, read_message, socket_path, write_message,
};

use crate::NAME;

const REQUEST: RequestId = RequestId(0);

const NOT_KEPT: &str = "the app did not keep the reply";

const TOO_LATE: &str = "the app did not answer in time";

const NOT_RUNNING: &str = "the app is not running";

/// Longest wait for the Keeper, connect to reply: pi waits for the handler, the user for pi.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// Longest wait for the answer to a reply-tool call: the agent waits for the tool's result.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlace {
    pub socket: PathBuf,
    /// From `yaar_link::TERMINAL_VARIABLE`; none outside the app's terminals or when it is empty.
    pub terminal: Option<String>,
    /// pi, which runs the program with no shell between.
    pub parent: u32,
}

impl ReportPlace {
    pub fn of_this_process() -> ReportPlace {
        ReportPlace {
            socket: socket_path(),
            terminal: std::env::var(TERMINAL_VARIABLE)
                .ok()
                .filter(|terminal| !terminal.is_empty()),
            parent: std::os::unix::process::parent_id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportAnswer {
    pub printed: Vec<u8>,
}

pub fn pass_on(payload: &[u8], place: &ReportPlace, tool: bool) -> ReportAnswer {
    let waits = if tool { TOOL_DEADLINE } else { DEADLINE };
    let sent = report_to_keeper(place, payload, Instant::now() + waits);
    let printed = match (sent, tool) {
        (Ok(Ok(Reply::Answer { bytes })), _) => bytes,
        (_, false) => Vec::new(),
        (Ok(Ok(_)), true) | (Err(Unanswered::Failed), true) => error_object(NOT_KEPT),
        (Ok(Err(refusal)), true) => error_object(&refusal.to_string()),
        (Err(Unanswered::TooLate), true) => error_object(TOO_LATE),
        (Err(Unanswered::NotRunning), true) => error_object(NOT_RUNNING),
    };
    ReportAnswer { printed }
}

fn error_object(why: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "error": why })).unwrap_or_default()
}

#[derive(Debug)]
enum Unanswered {
    NotRunning,
    TooLate,
    Failed,
}

type Answered = Result<Reply, Refusal>;

/// Gives the reply as soon as it is read without waiting for the Keeper to close its end.
fn report_to_keeper(
    place: &ReportPlace,
    payload: &[u8],
    deadline: Instant,
) -> Result<Answered, Unanswered> {
    let mut stream = connect(&place.socket).map_err(|_| Unanswered::NotRunning)?;
    let hello = Hello::for_this_process(concat!("yaar-pi-report ", env!("CARGO_PKG_VERSION")));
    limit(&stream, deadline).map_err(late)?;
    let keeper = handshake(&mut stream, &hello).map_err(|e| match e {
        LinkError::Io(e) if is_late(&e) => Unanswered::TooLate,
        _ => Unanswered::NotRunning,
    })?;
    // A Keeper of another version would refuse the report, so it counts as no Keeper.
    if !hello.speaks_with(&keeper) {
        return Err(Unanswered::NotRunning);
    }
    let report = AgentReport {
        agent: NAME.to_owned(),
        terminal: place.terminal.as_deref().map(TerminalId::new),
        pid: place.parent,
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
        // Set per frame: a Keeper that keeps telling facts must not stretch the deadline.
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
        // macOS refuses a timeout on a socket whose peer has closed. Nothing can then block: a read
        // gives what is buffered, then the end; the frames the Keeper left must still be read.
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => Ok(()),
        done => done,
    }
}
