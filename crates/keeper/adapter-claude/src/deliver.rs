//! A review rides in the `UserPromptSubmit` answer as `additionalContext`, or as a file under
//! `<folder>/`[`REVIEWS`]`/<session>/` when longer than [`INLINE_LIMIT`]; both with a
//! `systemMessage`. `allow` goes only to a `Read` inside the session's own folder and to
//! [`LISTED_REPLY_TOOL`]; nothing is remembered. A reply-tool call arrives as a report
//! `{"tool_server", "session_id", "cwd", "name", "arguments"}`, answered with an MCP `result`.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_agent::{
    AgentReply, Answer, Asked, Asks, CliSession, Delivery, Rendered, Report, ReportError,
};

use crate::COMMAND;
use crate::report::Remembered;

/// Claude Code's limit for one string of a hook's output, in UTF-16 code units: it passes
/// 10,000 whole and cuts 10,001 to a path and a preview, so a longer review goes to a file.
pub const INLINE_LIMIT: usize = 10_000;

pub const REVIEWS: &str = "reviews";

pub const TOOL_SERVER_NAME: &str = "yaar";

pub const TOOL_SERVER: &str = "yaar-claude-tools";

pub const REPLY_TOOL: &str = "reply_to_thread";

/// As Claude Code lists it to the agent: `mcp__plugin_<plugin>_<server>__<tool>`.
pub const LISTED_REPLY_TOOL: &str = "mcp__plugin_yaar-review_yaar__reply_to_thread";

pub const INSTRUCTION: &str = "This is a code review of your changes, attached to the \
user's message. Address each thread below. For each thread, call the tool \
mcp__plugin_yaar-review_yaar__reply_to_thread once, with `thread` set to the id written \
after \"Thread\" in its heading, `body` set to your reply to the reviewer, and \
`addressed` set to true when your change addresses the comment, or false when you \
disagree or need the reviewer's answer. Only the reviewer resolves a thread.";

/// What a message Claude Code starts itself opens with, such as a finished background task's
/// report: the user did not type it.
const SELF_STARTED: &str = "<task-notification>";

impl Remembered {
    pub(crate) fn asked(
        &self,
        folder: &Path,
        report: &Report<'_>,
    ) -> Result<Option<Asked>, ReportError> {
        let input: Value = serde_json::from_slice(report.payload)
            .map_err(|why| ReportError::Unreadable(why.to_string()))?;
        let asked = |session: &str, asks| {
            Some(Asked {
                session: CliSession(session.to_owned()),
                terminal: report.terminal.map(str::to_owned),
                folder: text(&input, "cwd").map(PathBuf::from),
                asks,
            })
        };

        if input.get("tool_server").is_some() {
            // A call that names no session is asked for all the same: the Keeper then
            // finds the session by the process that called.
            let session = text(&input, "session_id").unwrap_or_default();
            return Ok(asked(session, Asks::Reply(reply_of(&input)?)));
        }

        let hook = text(&input, "hook_event_name")
            .ok_or_else(|| ReportError::Unreadable("it names no hook_event_name".to_owned()))?;
        let session = text(&input, "session_id")
            .ok_or_else(|| ReportError::Unreadable("it names no session_id".to_owned()))?;
        let asks = match hook {
            "UserPromptSubmit" => {
                let prompt = text(&input, "prompt").unwrap_or_default();
                (!prompt.trim_start().starts_with(SELF_STARTED)).then_some(Asks::Message)
            }
            "PreToolUse" => match text(&input, "tool_name") {
                Some("Read") => {
                    let file = text(&input["tool_input"], "file_path");
                    file.is_some_and(|file| is_own_review(folder, session, Path::new(file)))
                        .then_some(Asks::ReadOwnFile)
                }
                Some(LISTED_REPLY_TOOL) => Some(Asks::OwnTool),
                _ => None,
            },
            _ => None,
        };
        Ok(asks.and_then(|asks| asked(session, asks)))
    }

    /// An answer not meant for what `asked` waits for gives no bytes: an allow never goes to a
    /// message.
    pub(crate) fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        let answered = match (&asked.asks, answer) {
            (Asks::ReadOwnFile, Answer::Allow) => allow("a file of the review the app delivered"),
            (Asks::OwnTool, Answer::Allow) => allow("the app's own tool, which the user installed"),
            (
                Asks::Reply(_),
                Answer::Replied {
                    thread,
                    addressed,
                    characters,
                },
            ) => result(
                false,
                &format!(
                    "recorded reply to {thread} ({}, {characters} characters)",
                    if *addressed { "addressed" } else { "open" }
                ),
            ),
            (Asks::Reply(_), Answer::Refused { why }) => result(true, why),
            _ => return Vec::new(),
        };
        serde_json::to_vec(&answered).expect("JSON is written")
    }
}

fn reply_of(input: &Value) -> Result<AgentReply, ReportError> {
    let unreadable = |why: &str| ReportError::Unreadable(format!("the tool call {why}"));
    if text(input, "name") != Some(REPLY_TOOL) {
        return Err(unreadable("names a tool the server does not list"));
    }
    let arguments = &input["arguments"];
    let thread = text(arguments, "thread").ok_or_else(|| unreadable("names no thread"))?;
    let body = text(arguments, "body").ok_or_else(|| unreadable("holds no body"))?;
    let addressed = match arguments.get("addressed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(addressed)) => *addressed,
        Some(_) => return Err(unreadable("holds an `addressed` that is no boolean")),
    };
    Ok(AgentReply {
        thread: thread.to_owned(),
        text: body.to_owned(),
        addressed,
    })
}

fn is_own_review(folder: &Path, session: &str, file: &Path) -> bool {
    if !is_folder_name(session) || !file.is_absolute() {
        return false;
    }
    let Ok(own) = folder.join(REVIEWS).join(session).canonicalize() else {
        return false;
    };
    file.canonicalize()
        .is_ok_and(|file| file != own && file.starts_with(&own))
}

fn allow(why: &str) -> Value {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": why,
    }})
}

fn result(is_error: bool, message: &str) -> Value {
    let mut result = json!({"content": [{"type": "text", "text": message}]});
    if is_error {
        result["isError"] = json!(true);
    }
    result
}

fn text<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

pub(crate) fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) struct ToClaudeCode<'a> {
    pub(crate) folder: &'a Path,
    pub(crate) session: &'a CliSession,
}

impl Delivery for ToClaudeCode<'_> {
    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        // Refused on every delivery, however short: a session whose id is no folder name has no
        // folder of reviews, so the allow of its reads could name none.
        if !is_folder_name(&self.session.0) {
            return Err(invalid(format!(
                "the session id {:?} is not made of ASCII letters, digits, `-` and `_`",
                self.session.0
            )));
        }
        let context = if rendered.text.encode_utf16().count() <= INLINE_LIMIT {
            rendered.text.to_owned()
        } else {
            let file = self.write(rendered)?;
            format!(
                "{INSTRUCTION}\n\nThe review is too long to show here. It is the whole of this \
                 file:\n{}\nRead the whole file with your Read tool before you reply to the \
                 threads.",
                file.display()
            )
        };
        let line = format!(
            "yaar: review round {} attached ({} threads)",
            rendered.round, rendered.threads
        );
        let answer = json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": context,
            },
            "systemMessage": line,
        });
        serde_json::to_vec(&answer).map_err(io::Error::other)
    }
}

impl ToClaudeCode<'_> {
    /// Writes `rendered` whole as `<review>-<round>.md` in the session's folder through a temporary
    /// file renamed into place, so the agent never reads half of it; gives the absolute path.
    fn write(&self, rendered: &Rendered<'_>) -> io::Result<PathBuf> {
        if !is_folder_name(rendered.review) {
            return Err(invalid(format!(
                "the review id {:?} is no file name",
                rendered.review
            )));
        }
        let folder = std::path::absolute(self.folder)?
            .join(REVIEWS)
            .join(&self.session.0);
        std::fs::create_dir_all(&folder)?;
        let name = format!("{}-{}.md", rendered.review, rendered.round);
        let file = folder.join(&name);
        let temporary = folder.join(format!(".{name}.{}.tmp", std::process::id()));
        let written = std::fs::write(&temporary, rendered.text)
            .and_then(|()| std::fs::rename(&temporary, &file));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.map(|()| file)
    }
}

fn invalid(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why)
}

pub(crate) fn resume(session: &CliSession) -> String {
    format!("{COMMAND} --resume '{}'", session.0.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use yaar_agent::Adapter;

    use super::*;
    use crate::ClaudeCode;

    // The tests' folder lies under /dev/null, where no file can be made: a regression that
    // writes one fails instead of leaving a file behind.
    fn asked_of_prompt(prompt: &str) -> Option<Asked> {
        let input = json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "s",
            "cwd": "/w",
            "prompt": prompt,
        });
        let payload = serde_json::to_vec(&input).expect("JSON is written");
        let report = Report {
            terminal: Some("t"),
            pid: 7,
            payload: &payload,
        };
        let mut adapter = ClaudeCode::new();
        adapter.report(&report).expect("a report");
        adapter
            .asked(Path::new("/dev/null/yaar"), &report)
            .expect("a question")
    }

    #[test]
    fn every_message_of_the_user_is_one_a_review_rides_with_whatever_it_says() {
        let long = "a".repeat(5_000);
        for prompt in [
            "ok",
            "",
            "   ",
            long.as_str(),
            "Do not answer the threads, just fix the code.",
            "  please <task-notification> later",
        ] {
            let asked = asked_of_prompt(prompt).expect("a message asks");

            assert_eq!(asked.asks, Asks::Message, "{prompt:?}");
            assert_eq!(asked.session, CliSession("s".to_owned()));
            assert_eq!(asked.terminal.as_deref(), Some("t"));
            assert_eq!(asked.folder, Some(PathBuf::from("/w")));
        }
    }

    #[test]
    fn only_a_prompt_that_opens_with_the_tag_after_white_space_is_one_claude_code_started() {
        for prompt in [
            "<task-notification>\n<task-id>b1</task-id>\n</task-notification>",
            "  \n\t<task-notification>x",
        ] {
            assert_eq!(asked_of_prompt(prompt), None, "{prompt:?}");
        }
    }

    #[test]
    fn a_prompt_with_no_text_is_still_a_message() {
        let input = json!({"hook_event_name": "UserPromptSubmit", "session_id": "s"});
        let payload = serde_json::to_vec(&input).expect("JSON is written");
        let report = Report {
            terminal: None,
            pid: 1,
            payload: &payload,
        };

        let asked = ClaudeCode::new().asked(Path::new("/x"), &report);

        assert_eq!(
            asked.expect("read").map(|asked| asked.asks),
            Some(Asks::Message)
        );
    }

    #[test]
    fn the_instruction_names_the_reply_tool_its_three_arguments_and_each_thread() {
        for word in [
            LISTED_REPLY_TOOL,
            "`thread`",
            "`body`",
            "`addressed`",
            "each thread",
        ] {
            assert!(INSTRUCTION.contains(word), "{word}");
        }
        assert!(
            INSTRUCTION.contains("Only the reviewer resolves a thread"),
            "it says who resolves, and asks for nothing that does"
        );
        assert_eq!(ClaudeCode::new().instruction(), INSTRUCTION);
        let session = CliSession("s".to_owned());
        let delivery = ToClaudeCode {
            folder: Path::new("/dev/null/yaar"),
            session: &session,
        };
        assert_eq!(delivery.instruction(), INSTRUCTION);
    }

    #[test]
    fn the_review_that_starts_with_the_instruction_goes_out_unchanged_and_the_users_words_are_not_in_it()
     {
        let prompt = "Do not answer the threads, just fix the code.";
        let asked = asked_of_prompt(prompt).expect("a message asks");
        let text = format!("{INSTRUCTION}\n\n## Thread t1\n\nComment:\nwhy `this`?");
        let session = asked.session.clone();
        let mut delivery = ToClaudeCode {
            folder: Path::new("/dev/null/yaar"),
            session: &session,
        };

        let bytes = delivery
            .hand_out(&Rendered {
                text: &text,
                review: "R",
                round: 3,
                threads: 1,
            })
            .expect("a delivery");

        let answer: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(answer["hookSpecificOutput"]["additionalContext"], text);
        assert!(
            answer["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .is_some_and(|context| context.starts_with(INSTRUCTION)),
            "the instruction comes first, whatever the user wrote"
        );
        assert!(
            !String::from_utf8_lossy(&bytes).contains(prompt),
            "the answer holds the review and not the user's message"
        );
    }

    #[test]
    fn the_resume_command_quotes_the_id_for_sh() {
        assert_eq!(
            resume(&CliSession("0f1e-2d".to_owned())),
            "claude --resume '0f1e-2d'"
        );
        assert_eq!(
            resume(&CliSession("a'b; rm -rf ~".to_owned())),
            r"claude --resume 'a'\''b; rm -rf ~'"
        );
    }
}
