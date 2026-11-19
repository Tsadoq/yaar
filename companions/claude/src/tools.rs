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
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_link::{Reply, TERMINAL_VARIABLE, socket_path};

use crate::{Reporter, Unanswered, report_to_keeper};

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

/// The variable in which Claude Code gives the tool servers it starts the id of the session
/// that started them.
const SESSION_VARIABLE: &str = "CLAUDE_CODE_SESSION_ID";

/// The protocol version the server names when `initialize` asks for none: the one the
/// recorded trace of Claude Code asked for.
const PROTOCOL: &str = "2025-11-25";

impl ToolPlace {
    /// What the environment of this process says, its parent and its folder.
    pub fn of_this_process() -> ToolPlace {
        let set = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        ToolPlace {
            socket: socket_path(),
            // An empty variable is a shell's leftover, not a terminal.
            terminal: set(TERMINAL_VARIABLE),
            parent: std::os::unix::process::parent_id(),
            session: set(SESSION_VARIABLE),
            folder: std::env::current_dir().unwrap_or_default(),
        }
    }
}

/// The tool as `tools/list` gives it: its name, a description that says what a call does,
/// and the schema of its three arguments, `thread` and `body` required strings and
/// `addressed` a boolean that defaults to false.
pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Reply to one thread of the review attached to the user's message. \
            Call it once for each thread. Set addressed to true when your change addresses \
            the comment; leave it false when you disagree or need the reviewer's answer. \
            Only the reviewer resolves a thread.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "thread": {
                    "type": "string",
                    "description": "The thread's id, as the review names it, for example T1"
                },
                "body": {
                    "type": "string",
                    "description": "Your reply to the reviewer, plain text or Markdown"
                },
                "addressed": {
                    "type": "boolean",
                    "description": "true if your change addresses the comment, false otherwise"
                }
            },
            "required": ["thread", "body"]
        }
    })
}

/// Serves the messages of `input`, one JSON-RPC message a line, until it ends, writing
/// each answer to `output` as one line and flushing it at once. Errors are only those of
/// `output`: everything the Keeper does or fails to do is an answer to the call.
pub fn serve(mut input: impl BufRead, mut output: impl Write, place: &ToolPlace) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        // Claude Code is gone when its pipe breaks, as when it closes it: no one is left to
        // serve.
        if !matches!(input.read_until(b'\n', &mut line), Ok(read) if read > 0) {
            return Ok(());
        }
        // A line that is no JSON is skipped: it may be anything a launcher wrote.
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if let Some(answer) = answer(&message, place) {
            // One line, written and flushed at once: Claude Code waits for it.
            writeln!(output, "{answer}")?;
            output.flush()?;
        }
    }
}

/// What the server writes for `message`; nothing for a notification, which has no `id`, and
/// for what is no request.
fn answer(message: &Value, place: &ToolPlace) -> Option<Value> {
    let id = message.get("id").filter(|id| !id.is_null())?;
    let method = message.get("method")?.as_str()?;
    let params = &message["params"];
    Some(match method {
        "initialize" => result(
            id,
            json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({"tools": [tool()]})),
        "tools/call" => match params["name"].as_str() {
            Some(TOOL) => result(id, call(&params["arguments"], place)),
            other => failure(
                id,
                -32602,
                &format!("unknown tool: {}", other.unwrap_or("none named")),
            ),
        },
        _ => failure(id, -32601, "method not found"),
    })
}

/// A successful JSON-RPC response.
fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// A JSON-RPC error response.
fn failure(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// A result that tells the agent the call did not work, and why.
fn not_kept(why: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": why}]})
}

/// Passes one call of the reply tool to the Keeper and gives its result: what the Keeper
/// answered, or an error result that says why it did not.
fn call(arguments: &Value, place: &ToolPlace) -> Value {
    let no_arguments = json!({});
    let payload = json!({
        "tool_server": SERVER_NAME,
        "session_id": place.session,
        "cwd": place.folder.to_string_lossy(),
        "name": TOOL,
        "arguments": if arguments.is_null() { &no_arguments } else { arguments },
    });
    let from = Reporter {
        socket: &place.socket,
        who: concat!("yaar-claude-tools ", env!("CARGO_PKG_VERSION")),
        terminal: place.terminal.as_deref(),
        parent: place.parent,
    };
    match report_to_keeper(
        &from,
        payload.to_string().as_bytes(),
        Instant::now() + TOOL_DEADLINE,
    ) {
        Ok(Ok(Reply::Answer { bytes })) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(result) if result.is_object() => result,
            _ => not_kept("the app did not keep the reply"),
        },
        Ok(Ok(_)) | Err(Unanswered::Failed) => not_kept("the app did not keep the reply"),
        Ok(Err(refusal)) => not_kept(&refusal.to_string()),
        Err(Unanswered::TooLate) => not_kept("the app did not answer in time"),
        Err(Unanswered::NotRunning) => not_kept("the app is not running"),
    }
}
