//! The reply tool's server: the program `yaar-claude-tools`, which Claude Code starts from
//! the app's plugin and speaks MCP with over its standard input and output.
//!
//! It serves one tool, `reply_to_thread`, with three arguments: `thread`, the thread's id
//! as the review names it; `body`, the reply; `addressed`, whether the agent's change
//! addresses the thread. It has no argument and no other tool that resolves a thread:
//! only the reviewer resolves.
//!
//! Each message is one line of JSON-RPC 2.0, as Claude Code writes them. The server
//! answers `initialize` with the protocol version Claude Code asked for, the capability of
//! tools and its name, [`SERVER_NAME`]; `tools/list` with the one tool; `ping` with an
//! empty result; `tools/call` of the reply tool with the result the Keeper hands back; a
//! request of any other method with the error `-32601`, "method not found"; and nothing to
//! a notification, which carries no `id`. A line that is no JSON is skipped.
//!
//! A call is passed to the Keeper as one `yaar_link::Request::Report` of the Adapter
//! `claude`, as the hook program passes a hook: with the terminal its environment names
//! and its parent, Claude Code, and as payload the JSON object `{"tool_server": "yaar",
//! "session_id": <CLAUDE_CODE_SESSION_ID>, "cwd": <its folder>, "name": <the tool>,
//! "arguments": <the call's arguments, as Claude Code sent them>}`, so that the reply's
//! body reaches the Keeper byte for byte. The Keeper's `yaar_link::Reply::Answer` is the
//! call's `result`, which the server writes back as it came. Any other answer, and none,
//! is a result that says the reply was not kept, an error the agent reads,
//! `{"isError": true, "content": [{"type": "text", "text": <why>}]}`, `why` one of: "the
//! app did not keep the reply" for `yaar_link::Reply::Done`, the Keeper's reason for a
//! refusal, "the app did not answer in time" past [`TOOL_DEADLINE`], and "the app is not
//! running" with no Keeper or a Keeper of another protocol. The Keeper answers a call
//! with `Reply::Done` only when it could not work it out: a report it could not read, or
//! one that came while it stopped.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

/// The longest the server waits for the Keeper's answer to one call. Claude Code waits for
/// the tool; a reply is kept by the Keeper with one write to the drive.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

/// The server's name, as it gives it in its answer to `initialize`.
pub const SERVER_NAME: &str = "yaar";

/// The one tool the server lists.
pub const TOOL: &str = "reply_to_thread";

/// Where the server finds the Keeper, and what it says of the CLI that started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPlace {
    /// The socket the Keeper is asked to listen at, as for the hook program.
    pub socket: PathBuf,
    /// The terminal of the app Claude Code runs in, from `yaar_link::TERMINAL_VARIABLE`;
    /// none outside the app's terminals.
    pub terminal: Option<String>,
    /// The server's parent process: Claude Code.
    pub parent: u32,
    /// Claude Code's own id of the session that started the server, from its variable
    /// `CLAUDE_CODE_SESSION_ID`; none where it is not set.
    pub session: Option<String>,
    /// The folder the server was started in: the folder Claude Code works in.
    pub folder: PathBuf,
}

impl ToolPlace {
    /// What the environment of this process says, its parent and its folder.
    pub fn of_this_process() -> ToolPlace {
        todo!()
    }
}

/// The tool as `tools/list` gives it: its name, a description that says what a call does,
/// and the schema of its three arguments, `thread` and `body` required strings and
/// `addressed` a boolean that defaults to false.
pub fn tool() -> serde_json::Value {
    todo!()
}

/// Serves the messages of `input`, one JSON-RPC message a line, until it ends, writing
/// each answer to `output` as one line and flushing it at once. Errors are only those of
/// `output`: everything the Keeper does or fails to do is an answer to the call.
pub fn serve(input: impl BufRead, output: impl Write, place: &ToolPlace) -> io::Result<()> {
    let _ = (input, output, place);
    todo!()
}
