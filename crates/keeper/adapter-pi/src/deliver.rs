//! pi takes the review inside the user's message: the extension's `input` handler is answered
//! `{"review": "<text>"}`, whole at any length, so nothing is written to a file.

use std::io;
use std::path::Path;

use serde_json::json;
use yaar_agent::{CliSession, Delivery, Rendered};

use crate::COMMAND;

/// Goes first in every review: one `reply_to_thread` call per thread, never a resolve.
pub const INSTRUCTION: &str = "This is a code review of your changes, attached to the \
user's message. Address each thread below. For each thread, call the tool \
reply_to_thread once, with `thread` set to the id written after \"Thread\" in its \
heading, `body` set to your reply to the reviewer, and `addressed` set to true when \
your change addresses the comment, or false when you disagree or need the reviewer's \
answer. Only the reviewer resolves a thread.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToPi;

impl Delivery for ToPi {
    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    /// The JSON object `{"review": <rendered.text>}`. Never fails.
    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&json!({ "review": rendered.text })).unwrap_or_default())
    }
}

/// Shell command that resumes `session`: `pi --session-dir '<dir of cli_record>' --session '<id>'`,
/// without the dir option when `cli_record` names no folder.
pub(crate) fn resume(session: &CliSession, cli_record: Option<&Path>) -> String {
    let id = quoted(&session.0);
    match cli_record
        .and_then(Path::parent)
        .filter(|dir| !dir.as_os_str().is_empty())
    {
        Some(dir) => format!(
            "{COMMAND} --session-dir {} --session {id}",
            quoted(&dir.to_string_lossy())
        ),
        None => format!("{COMMAND} --session {id}"),
    }
}

fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}
