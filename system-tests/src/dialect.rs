//! The CLI a stand-in speaks as: what one test of the driver `agent` names to run for Claude
//! Code and for pi, through the same script and the same assertions.
//!
//! The stand-in writes down every piece of the app it ran and what it answered as
//! `standin::Did::Hook`, under the name of the moment in its CLI's own words: Claude Code's
//! hook (`UserPromptSubmit`), pi's event (`input`). A test that waits for "the user's
//! message was taken" asks the dialect for that name, and reads the review the answer
//! carried through [`Dialect::review_in`]: the same test then holds for both.

use serde_json::Value;

/// One of the two CLIs the app supports, as the stand-in speaks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Claude Code: the stand-in runs the hooks of the app's plugin.
    ClaudeCode,
    /// pi: the stand-in, started by its name, finds the app's extension in pi's own
    /// settings in its home, as pi does once the user said yes, and runs the reporting
    /// program a shell of the app names, as pi's extension does.
    Pi,
}

impl Dialect {
    /// Both, in the order the Keeper lists its Adapters.
    pub const BOTH: [Dialect; 2] = [Dialect::ClaudeCode, Dialect::Pi];

    /// The name of the CLI: what is typed in a shell, the name of its launcher's Adapter
    /// (`yaar_link::Program::Agent`), and the stand-in's name on a world's search path.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "claude",
            Dialect::Pi => "pi",
        }
    }

    /// The other CLI.
    pub fn other(self) -> Dialect {
        match self {
            Dialect::ClaudeCode => Dialect::Pi,
            Dialect::Pi => Dialect::ClaudeCode,
        }
    }

    /// The moment the user's message is taken, whose answer carries an attached review:
    /// Claude Code's `UserPromptSubmit`, pi's `input`.
    pub fn message_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "UserPromptSubmit",
            Dialect::Pi => "input",
        }
    }

    /// The moment a turn ends: Claude Code's `Stop`, pi's `agent_settled`.
    pub fn turn_end_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "Stop",
            Dialect::Pi => "agent_settled",
        }
    }

    /// The moment a tool of the agent has run, a write among them: Claude Code's
    /// `PostToolUse`, pi's `tool_result`.
    pub fn tool_done_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "PostToolUse",
            Dialect::Pi => "tool_result",
        }
    }

    /// The moment the session ends: Claude Code's `SessionEnd`, pi's `session_shutdown`.
    pub fn session_end_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "SessionEnd",
            Dialect::Pi => "session_shutdown",
        }
    }

    /// Whether the CLI asks the user's permission at a prompt of its own: Claude Code does,
    /// pi never does.
    pub fn asks_permission(self) -> bool {
        matches!(self, Dialect::ClaudeCode)
    }

    /// The review an answer to [`Dialect::message_event`] carried, as the CLI hands it to
    /// the model: for Claude Code the hook's `hookSpecificOutput.additionalContext`, for pi
    /// the `review` of the extension's answer. None for an empty answer, or one that carries
    /// none.
    pub fn review_in(self, answer: &[u8]) -> Option<String> {
        let value: Value = serde_json::from_slice(answer).ok()?;
        let review = match self {
            Dialect::ClaudeCode => &value["hookSpecificOutput"]["additionalContext"],
            Dialect::Pi => &value["review"],
        };
        review.as_str().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_is_read_from_the_answer_of_either_cli() {
        let claude = br#"{"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "R"}, "systemMessage": "x"}"#;
        assert_eq!(Dialect::ClaudeCode.review_in(claude), Some("R".to_owned()));
        assert_eq!(Dialect::Pi.review_in(claude), None);
        let pi = br#"{"review": "R2"}"#;
        assert_eq!(Dialect::Pi.review_in(pi), Some("R2".to_owned()));
        assert_eq!(Dialect::ClaudeCode.review_in(pi), None);
        assert_eq!(Dialect::Pi.review_in(b""), None);
        for dialect in Dialect::BOTH {
            assert_eq!(dialect.other().other(), dialect);
            assert_ne!(dialect.other(), dialect);
        }
    }
}
