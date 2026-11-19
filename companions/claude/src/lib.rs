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

use std::error::Error;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use yaar_link::{
    AgentReport, Hello, Message, Request, RequestId, TERMINAL_VARIABLE, TerminalId, connect,
    handshake, read_message, socket_path, write_message,
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
    // Whatever went wrong, Claude Code is better served by a hook that said nothing than
    // by one that held the user up or failed: the report is a courtesy to the app.
    let _ = pass_on(input, place, Instant::now() + DEADLINE);
    HookAnswer {
        printed: Vec::new(),
        status: 0,
    }
}

/// Sends the report and reads until the Keeper's reply to it, all before `deadline`.
fn pass_on(input: &[u8], place: &HookPlace, deadline: Instant) -> Result<(), Box<dyn Error>> {
    let mut stream = connect(&place.socket)?;
    let hello = Hello::for_this_process(concat!("yaar-claude-hook ", env!("CARGO_PKG_VERSION")));
    limit(&stream, deadline)?;
    let keeper = handshake(&mut stream, &hello)?;
    if !hello.speaks_with(&keeper) {
        // A Keeper of another version of the app: it would refuse the request, and the
        // report means nothing to it.
        return Err("the Keeper speaks another protocol".into());
    }
    let report = AgentReport {
        agent: AGENT.to_owned(),
        terminal: place.terminal.clone().map(TerminalId::new),
        pid: place.parent,
        payload: input.to_vec(),
    };
    limit(&stream, deadline)?;
    write_message(
        &mut stream,
        &Message::Request {
            id: REQUEST,
            request: Request::Report { report },
        },
    )?;
    loop {
        // Set again for each frame: a Keeper that keeps telling facts must not stretch
        // the deadline a read at a time.
        limit(&stream, deadline)?;
        if let Message::Reply { to: REQUEST, .. } = read_message(&mut stream)? {
            return Ok(());
        }
    }
}

/// Makes reading and writing `stream` give up when `deadline` comes.
fn limit(stream: &UnixStream, deadline: Instant) -> io::Result<()> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "the deadline is over"))?;
    stream.set_read_timeout(Some(left))?;
    stream.set_write_timeout(Some(left))
}
