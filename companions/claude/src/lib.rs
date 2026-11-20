//! Claude Code companion (`X-CC-PLUGIN`).
//!
//! Owns: the hook program, the reply tool server and the two allow hooks that Claude Code
//! loads from the app's plugin folder.
//!
//! Surface: [`run_hook`], what the hook program `yaar-claude-hook` does, with
//! [`HookPlace`], where it finds the Keeper and what it says of its CLI, [`HookAnswer`],
//! what it gives Claude Code back, and [`DEADLINE`]; and [`serve`], what the reply tool's
//! server `yaar-claude-tools` does, with [`ToolPlace`], [`tool`], [`TOOL`],
//! [`SERVER_NAME`] and [`TOOL_DEADLINE`] (`tools.rs`).
//!
//! Claude Code runs the hook program for every hook the app's plugin asks for, with the
//! hook's input on its standard input. The program passes that input on, unread, to the
//! Keeper as one `yaar_link::Request::Report`, with the terminal id its environment names
//! and its parent process, which is Claude Code, and prints what the Keeper answers: the
//! bytes of `yaar_link::Reply::Answer`, as the Keeper's Adapter of Claude Code wrote them,
//! and nothing for `yaar_link::Reply::Done`. It decides nothing itself, a permission
//! request least of all: the Keeper never answers one, so Claude Code shows the user its
//! own dialog and the user answers it there. What the Keeper answers is a review that
//! rides with the user's message, and the allowing of the read of a review file the app
//! wrote and of a call of the app's own reply tool. A Keeper that is not there, or that
//! does not answer within [`DEADLINE`], changes nothing of that: the program prints
//! nothing and succeeds, and Claude Code goes on as without the app.

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

/// The Adapter the reports are for.
const AGENT: &str = "claude";

/// The only request the program sends, and so the id its reply carries.
const REQUEST: RequestId = RequestId(0);

/// The longest the hook program waits for the Keeper, from connecting to the reply. Claude
/// Code waits for the program, and so does the user at a permission prompt.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// Where the hook program finds the Keeper, and what it says of the CLI that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlace {
    /// The socket the Keeper is asked to listen at: `yaar_link::socket_path` of the
    /// program's environment, which the app's shells set.
    pub socket: PathBuf,
    /// The terminal of the app Claude Code runs in, from `yaar_link::TERMINAL_VARIABLE`;
    /// none outside the app's terminals.
    pub terminal: Option<String>,
    /// The program's parent process: Claude Code.
    pub parent: u32,
}

impl HookPlace {
    /// What the environment of this process says, and its parent.
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

/// What the hook program gives Claude Code back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAnswer {
    /// What it prints on its standard output: the bytes of the Keeper's
    /// `yaar_link::Reply::Answer`, and nothing for any other reply or none.
    pub printed: Vec<u8>,
    /// Its exit status: 0, whatever became of the report.
    pub status: i32,
}

/// Passes `input`, the hook's input as Claude Code gave it, to the Keeper at `place` as
/// one report of the Adapter `claude`, and gives the answer for Claude Code: what the
/// Keeper's `yaar_link::Reply::Answer` holds, byte for byte, and nothing for any other
/// reply. Waits at most [`DEADLINE`] for the Keeper; an answer that came in time and was
/// not whole is not printed.
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
    // Whatever went wrong, Claude Code is better served by a hook that said nothing than
    // by one that held the user up or failed: the report is a courtesy to the app.
    let printed = match sent {
        Ok(Ok(Reply::Answer { bytes })) => bytes,
        _ => Vec::new(),
    };
    HookAnswer { printed, status: 0 }
}

/// Who a report comes from and where it goes: what the hook program and the tool server
/// each say of themselves.
struct Reporter<'a> {
    socket: &'a Path,
    /// What the program calls itself in its hello.
    who: &'a str,
    terminal: Option<&'a str>,
    parent: u32,
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

/// Sends `payload` as one report of the Adapter `claude` and reads until the Keeper's reply
/// to it, all before `deadline`.
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
        // A Keeper of another version of the app: it would refuse the request, and the
        // report means nothing to it.
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
