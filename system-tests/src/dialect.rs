//! The two agent CLIs the app supports, with the name each gives to the same moment, so one test
//! holds for both.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    ClaudeCode,
    /// pi: the stand-in finds the app's extension in pi's settings in its home and runs the
    /// reporting program a shell names.
    Pi,
}

impl Dialect {
    /// In the order the Keeper lists its Adapters.
    pub const BOTH: [Dialect; 2] = [Dialect::ClaudeCode, Dialect::Pi];

    /// The name typed in a shell, of the launcher's Adapter (`yaar_link::Program::Agent`) and of
    /// the stand-in on a world's search path.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "claude",
            Dialect::Pi => "pi",
        }
    }

    pub fn other(self) -> Dialect {
        match self {
            Dialect::ClaudeCode => Dialect::Pi,
            Dialect::Pi => Dialect::ClaudeCode,
        }
    }

    /// The moment the user's message is taken; its answer carries an attached review.
    pub fn message_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "UserPromptSubmit",
            Dialect::Pi => "input",
        }
    }

    pub fn turn_end_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "Stop",
            Dialect::Pi => "agent_settled",
        }
    }

    pub fn tool_done_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "PostToolUse",
            Dialect::Pi => "tool_result",
        }
    }

    pub fn session_end_event(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "SessionEnd",
            Dialect::Pi => "session_shutdown",
        }
    }

    /// Whether the CLI asks permission at a prompt of its own.
    pub fn asks_permission(self) -> bool {
        matches!(self, Dialect::ClaudeCode)
    }

    /// The review an answer to [`Dialect::message_event`] carried; None for an empty answer or one
    /// without.
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
