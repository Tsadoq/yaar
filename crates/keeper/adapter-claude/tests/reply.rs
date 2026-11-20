//! The reply tool's end of the Adapter: a call of the tool server is a report that asks to
//! keep a reply, the agent's permission to call the tool is allowed, and what the app answers
//! comes out as the call's result.

mod support;

use std::path::Path;

use serde_json::{Value, json};
use yaar_adapter_claude::{ClaudeCode, LISTED_REPLY_TOOL, REPLY_TOOL, TOOL_SERVER_NAME};
use yaar_agent::{Adapter, AgentReply, Answer, Asked, Asks, CliSession, Report, ReportError};
use yaar_system_tests::recorded;

use support::feed_asked;

/// The recording of a session that edits a file and replies to four threads through the
/// tool: its `PreToolUse` of the tool is line 11, the `PostToolUse` after it line 12.
const REPLIES: &str = "claude/reply-tool-hooks.jsonl";

/// What the tool server writes for a call of `arguments`, as the Keeper's report.
fn call(arguments: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "tool_server": TOOL_SERVER_NAME,
        "session_id": "sess-1",
        "cwd": "/work",
        "name": REPLY_TOOL,
        "arguments": arguments,
    }))
    .expect("JSON is written")
}

/// The report of a call, from terminal `t` and process 41.
fn report(payload: &[u8]) -> Report<'_> {
    Report {
        terminal: Some("t"),
        pid: 41,
        payload,
    }
}

fn asked_by(payload: &[u8]) -> Result<Option<Asked>, ReportError> {
    let mut adapter = ClaudeCode::new();
    assert_eq!(
        adapter.report(&report(payload)).map(|events| events.len()),
        Ok(0),
        "a call of the tool tells the app no event and is no error"
    );
    adapter.asked(Path::new("/nowhere"), &report(payload))
}

#[test]
fn a_call_of_the_reply_tool_asks_to_keep_the_reply_byte_for_byte_in_its_session() {
    let body =
        "Quotes \" and ' and `ticks`,\nline two\r\n\t$(rm -rf ~); echo \"$HOME\" \\ \u{1F600} é";

    let asked = asked_by(&call(
        json!({"thread": "t-9", "body": body, "addressed": true}),
    ))
    .expect("a call is readable")
    .expect("a call asks");

    assert_eq!(
        asked,
        Asked {
            session: CliSession("sess-1".to_owned()),
            terminal: Some("t".to_owned()),
            folder: Some("/work".into()),
            asks: Asks::Reply(AgentReply {
                thread: "t-9".to_owned(),
                text: body.to_owned(),
                addressed: true,
            }),
        }
    );
}

#[test]
fn a_reply_without_the_addressed_mark_is_not_addressed() {
    for arguments in [
        json!({"thread": "t", "body": "b"}),
        json!({"thread": "t", "body": "b", "addressed": null}),
        json!({"thread": "t", "body": "b", "addressed": false}),
    ] {
        let asked = asked_by(&call(arguments)).expect("read").expect("asks");

        let Asks::Reply(reply) = asked.asks else {
            panic!("a reply");
        };
        assert!(!reply.addressed);
    }
}

#[test]
fn a_call_that_names_no_session_is_asked_for_with_no_session_id() {
    let payload = serde_json::to_vec(&json!({
        "tool_server": "yaar", "session_id": null, "cwd": "/work", "name": REPLY_TOOL,
        "arguments": {"thread": "t", "body": "b"},
    }))
    .expect("JSON");

    let asked = asked_by(&payload).expect("read").expect("asks");

    assert_eq!(asked.session, CliSession(String::new()));
    assert_eq!(asked.terminal.as_deref(), Some("t"));
}

#[test]
fn a_call_that_cannot_be_read_as_a_reply_is_an_error_and_not_a_reply() {
    for arguments in [
        json!({}),
        json!({"thread": "t"}),
        json!({"body": "b"}),
        json!({"thread": 3, "body": "b"}),
        json!({"thread": "t", "body": "b", "addressed": "yes"}),
        json!("not an object"),
    ] {
        let answer = asked_by(&call(arguments.clone()));

        assert!(
            matches!(answer, Err(ReportError::Unreadable(_))),
            "{arguments}: {answer:?}"
        );
    }
    // A tool the server does not list, such as one that would resolve.
    let payload = serde_json::to_vec(&json!({
        "tool_server": "yaar", "session_id": "s", "cwd": "/w", "name": "resolve_thread",
        "arguments": {"thread": "t", "body": "b"},
    }))
    .expect("JSON");
    assert!(matches!(
        asked_by(&payload),
        Err(ReportError::Unreadable(_))
    ));
}

#[test]
fn the_agents_call_of_the_reply_tool_is_allowed_and_nothing_after_it_asks() {
    let records = recorded::hook_events(REPLIES);
    let mut adapter = ClaudeCode::new();
    let folder = Path::new("/nowhere");

    let asks: Vec<_> = records[..=13]
        .iter()
        .map(|record| feed_asked(&mut adapter, folder, record).map(|asked| asked.asks))
        .collect();

    assert_eq!(records[11].event, "PreToolUse");
    assert_eq!(
        records[11].input["tool_name"], LISTED_REPLY_TOOL,
        "the recording names the tool as the app's plugin does"
    );
    assert_eq!(asks[2], Some(Asks::Message));
    assert_eq!(asks[11], Some(Asks::OwnTool));
    assert_eq!(asks[13], Some(Asks::OwnTool), "each call is allowed");
    for line in [3, 4, 6, 7, 8, 9, 12] {
        // The read of the project's own file, the search, the edit and the PostToolUse of the
        // tool.
        assert_eq!(asks[line], None, "line {line}: {}", records[line].event);
    }
    // The permission to call the tool tells the app nothing of the agent's steps.
    let events = adapter.report(&Report {
        terminal: records[11].terminal(),
        pid: records[11].parent,
        payload: &records[11].input_bytes(),
    });
    assert_eq!(events.expect("a report").len(), 0);
}

#[test]
fn the_allow_goes_to_a_read_or_a_call_and_never_to_a_message() {
    let adapter = ClaudeCode::new();
    let asked = |asks| Asked {
        session: CliSession("s".to_owned()),
        terminal: None,
        folder: None,
        asks,
    };

    for asks in [Asks::OwnTool, Asks::ReadOwnFile] {
        let bytes = adapter.answer(&asked(asks), &Answer::Allow);
        let answer: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(
            answer["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .is_some_and(|why| !why.is_empty())
        );
    }
    assert_eq!(adapter.answer(&asked(Asks::Message), &Answer::Allow), b"");
}

#[test]
fn the_result_of_a_reply_says_whether_it_was_addressed_and_how_long_it_was() {
    let adapter = ClaudeCode::new();
    let asked = Asked {
        session: CliSession("s".to_owned()),
        terminal: None,
        folder: None,
        asks: Asks::Reply(AgentReply {
            thread: "t-1".to_owned(),
            text: "x".to_owned(),
            addressed: true,
        }),
    };
    let result = |answer| -> Value {
        serde_json::from_slice(&adapter.answer(&asked, &answer)).expect("JSON")
    };

    assert_eq!(
        result(Answer::Replied {
            thread: "t-1",
            addressed: true,
            characters: 12
        }),
        json!({"content": [{"type": "text", "text": "recorded reply to t-1 (addressed, 12 characters)"}]})
    );
    assert_eq!(
        result(Answer::Replied {
            thread: "t-2",
            addressed: false,
            characters: 3
        }),
        json!({"content": [{"type": "text", "text": "recorded reply to t-2 (open, 3 characters)"}]})
    );
    assert_eq!(
        result(Answer::Refused {
            why: "no such thread in this session's review"
        }),
        json!({
            "isError": true,
            "content": [{"type": "text", "text": "no such thread in this session's review"}]
        })
    );
    assert_eq!(
        adapter.answer(&asked, &Answer::Allow),
        b"",
        "a reply is answered with a result and never an allow"
    );
}

#[test]
fn the_resume_command_of_the_adapter_is_the_one_of_claude_code() {
    assert_eq!(
        ClaudeCode::new().resume(&CliSession("abc-123".to_owned())),
        "claude --resume 'abc-123'"
    );
}
