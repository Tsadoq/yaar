//! MCP server of the reply tool `reply_to_thread` (`thread`, `body`, `addressed`). A call goes to
//! the Keeper as one `Request::Report` with payload `{"tool_server", "session_id", "cwd", "name",
//! "arguments"}`.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_link::{Reply, TERMINAL_VARIABLE, socket_path};

use crate::{Reporter, Unanswered, report_to_keeper};

/// The longest the server waits for the Keeper's answer to one call. Claude Code waits
/// for the tool; the Keeper keeps a reply with one write to the drive.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

pub const SERVER_NAME: &str = "yaar";

pub const TOOL: &str = "reply_to_thread";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPlace {
    pub socket: PathBuf,
    pub terminal: Option<String>,
    pub parent: u32,
    /// Claude Code's session id, from `CLAUDE_CODE_SESSION_ID`; none where unset.
    pub session: Option<String>,
    pub folder: PathBuf,
}

const SESSION_VARIABLE: &str = "CLAUDE_CODE_SESSION_ID";

/// Named when `initialize` asks for none: the version Claude Code's recorded trace asked for.
const PROTOCOL: &str = "2025-11-25";

impl ToolPlace {
    pub fn of_this_process() -> ToolPlace {
        let set = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        ToolPlace {
            socket: socket_path(),
            terminal: set(TERMINAL_VARIABLE),
            parent: std::os::unix::process::parent_id(),
            session: set(SESSION_VARIABLE),
            folder: std::env::current_dir().unwrap_or_default(),
        }
    }
}

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

/// Errors are only those of `output`: what the Keeper does or fails to do is an answer to the call.
pub fn serve(mut input: impl BufRead, mut output: impl Write, place: &ToolPlace) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        // Claude Code is gone when its pipe breaks, as when it closes it.
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

fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn not_kept(why: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": why}]})
}

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
