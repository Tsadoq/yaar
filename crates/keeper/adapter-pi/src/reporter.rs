//! What the reporting program `yaar-pi-report` does: pi's extension runs it for each report
//! with the payload on its standard input, and reads what it prints.
//!
//! ```text
//! yaar-pi-report [--tool] < <the payload>
//! ```
//!
//! It passes the payload on, unread, to the Keeper at [`ReportPlace::socket`] as one
//! `yaar_link::Request::Report` of the Adapter [`crate::NAME`], with the terminal its
//! environment names and its parent, pi, as `pid`, reached with `yaar_link::connect`, which
//! makes sure the Keeper is the user's own, and says hello first as the Link asks. It reads
//! past the facts the Keeper may send until the reply to its one request.
//!
//! Without `--tool` it waits at most [`DEADLINE`] for the reply and prints the bytes of a
//! `yaar_link::Reply::Answer`, and nothing for any other reply, a refusal, no Keeper, a
//! Keeper of another protocol, or no reply in time: pi goes on as without the app.
//!
//! With `--tool`, for a call of the reply tool, it waits at most [`TOOL_DEADLINE`] and
//! prints one JSON object in every case: the bytes of the Keeper's `yaar_link::Reply::Answer`
//! as they are, else `{"error": <why>}` with `why` one of "the app did not keep the reply"
//! for `yaar_link::Reply::Done`, the Keeper's reason for a refusal, "the app did not answer
//! in time" past the deadline, and "the app is not running" with no Keeper or one of another
//! protocol. It ends with 0 in every case.
//!
//! It leaves as soon as it has what it prints: a Keeper that answered and keeps its end of
//! the socket open does not hold it until the deadline, since pi's extension runs the next
//! report only once this one has ended.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use yaar_link::{
    AgentReport, Hello, LinkError, Message, Refusal, Reply, Request, RequestId, TERMINAL_VARIABLE,
    TerminalId, connect, handshake, read_message, socket_path, write_message,
};

use crate::NAME;

/// The only request the program sends, and so the id its reply carries.
const REQUEST: RequestId = RequestId(0);

/// What a call of the reply tool is told when the Keeper answered it with no result.
const NOT_KEPT: &str = "the app did not keep the reply";

/// What a call of the reply tool is told when the Keeper did not answer in time.
const TOO_LATE: &str = "the app did not answer in time";

/// What a call of the reply tool is told when there is no Keeper to answer it.
const NOT_RUNNING: &str = "the app is not running";

/// The longest the program waits for the Keeper, from connecting to the reply, for any
/// report but a call of the reply tool: pi waits for the extension's handler, and the user
/// for pi.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// The longest it waits for the Keeper's answer to a call of the reply tool: a reply is
/// kept with one write to the drive, and the agent waits for the tool's result.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

/// Where the program finds the Keeper, and what it says of the pi that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlace {
    /// The socket the Keeper is asked to listen at: `yaar_link::socket_path` of the
    /// program's environment, which the app's shells set.
    pub socket: PathBuf,
    /// The terminal of the app pi runs in, from `yaar_link::TERMINAL_VARIABLE`; none
    /// outside the app's terminals and for an empty value.
    pub terminal: Option<String>,
    /// The program's parent process: pi, which runs it with no shell between.
    pub parent: u32,
}

impl ReportPlace {
    /// What the environment of this process says, and its parent.
    pub fn of_this_process() -> ReportPlace {
        ReportPlace {
            socket: socket_path(),
            // An empty variable is a shell's leftover, not a terminal.
            terminal: std::env::var(TERMINAL_VARIABLE)
                .ok()
                .filter(|terminal| !terminal.is_empty()),
            parent: std::os::unix::process::parent_id(),
        }
    }
}

/// What the program gives pi back. It ends with 0 whatever became of the report, so its
/// exit says nothing: the extension reads only what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportAnswer {
    /// What it prints on its standard output, as the module's doc says.
    pub printed: Vec<u8>,
}

/// Passes `payload` to the Keeper at `place` as one report of pi, and gives what the
/// program prints: as the module's doc says, `tool` being whether it was run with `--tool`.
pub fn pass_on(payload: &[u8], place: &ReportPlace, tool: bool) -> ReportAnswer {
    let waits = if tool { TOOL_DEADLINE } else { DEADLINE };
    let sent = report_to_keeper(place, payload, Instant::now() + waits);
    let printed = match (sent, tool) {
        (Ok(Ok(Reply::Answer { bytes })), _) => bytes,
        // Without the flag, pi goes on as without the app: nothing is said.
        (_, false) => Vec::new(),
        (Ok(Ok(_)), true) | (Err(Unanswered::Failed), true) => error_object(NOT_KEPT),
        (Ok(Err(refusal)), true) => error_object(&refusal.to_string()),
        (Err(Unanswered::TooLate), true) => error_object(TOO_LATE),
        (Err(Unanswered::NotRunning), true) => error_object(NOT_RUNNING),
    };
    ReportAnswer { printed }
}

/// The one JSON object a call of the reply tool is told when it got no result.
fn error_object(why: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "error": why })).unwrap_or_default()
}

/// Why a report got no reply from the Keeper.
#[derive(Debug)]
enum Unanswered {
    /// No Keeper was reached, or the one reached speaks another protocol: nothing was sent.
    NotRunning,
    /// The report was sent, or was to be, and the deadline came first.
    TooLate,
    /// The report could not be sent whole, or the connection broke before the reply.
    Failed,
}

/// What the Keeper answered one report with: its reply or its refusal.
type Answered = Result<Reply, Refusal>;

/// Sends `payload` as one report of pi and reads until the Keeper's reply to it, all
/// before `deadline`. It gives the reply as soon as it is read: the Keeper's end of the
/// socket is not waited on, so that the next report of pi's chain does not wait for it.
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
    if !hello.speaks_with(&keeper) {
        // A Keeper of another version of the app: it would refuse the request, and the
        // report means nothing to it.
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
        // Set again for each frame: a Keeper that keeps telling facts must not stretch
        // the deadline a read at a time.
        limit(&stream, deadline).map_err(late)?;
        if let Message::Reply { to: REQUEST, reply } =
            read_message(&mut stream).map_err(link_failure)?
        {
            return Ok(reply);
        }
    }
}

/// Whether `error` is a read or write that gave up at its timeout.
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

/// Makes reading and writing `stream` give up when `deadline` comes.
fn limit(stream: &UnixStream, deadline: Instant) -> io::Result<()> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "the deadline is over"))?;
    match stream
        .set_read_timeout(Some(left))
        .and_then(|()| stream.set_write_timeout(Some(left)))
    {
        // macOS refuses a timeout on a socket whose peer has closed. Nothing can then block
        // a read, which gives what is still buffered and then the end, and a write gives a
        // broken pipe: the frames the Keeper left must still be read.
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => Ok(()),
        done => done,
    }
}
