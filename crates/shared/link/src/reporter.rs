use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::{
    AgentReport, Hello, LinkError, Message, Refusal, Reply, Request, RequestId, TERMINAL_VARIABLE,
    TerminalId, connect, handshake, read_message, socket_path, write_message,
};

const REQUEST: RequestId = RequestId(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlace {
    pub socket: PathBuf,
    /// From `TERMINAL_VARIABLE`; none outside the app's terminals or when it is empty.
    pub terminal: Option<String>,
    /// The program's parent process: the agent CLI, with no shell between.
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

pub struct Reporter<'a> {
    pub socket: &'a Path,
    pub who: &'a str,
    pub agent: &'a str,
    pub terminal: Option<&'a str>,
    pub parent: u32,
}

#[derive(Debug)]
pub enum Unanswered {
    /// No Keeper was reached, or the one reached speaks another protocol: nothing was sent.
    NotRunning,
    TooLate,
    Failed,
}

pub type Answered = Result<Reply, Refusal>;

/// Gives the reply as soon as it is read without waiting for the Keeper to close its end.
pub fn report_to_keeper(
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
    // A Keeper of another version would refuse the report, so it counts as no Keeper.
    if !hello.speaks_with(&keeper) {
        return Err(Unanswered::NotRunning);
    }
    let report = AgentReport {
        agent: from.agent.to_owned(),
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
