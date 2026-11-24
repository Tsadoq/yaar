mod support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_adapter_pi::{INSTRUCTION, Pi, REPLY_EVENT, REPLY_TOOL, ToPi};
use yaar_agent::{
    Adapter, AgentReply, Answer, Asked, Asks, CliSession, Delivery, Event, Rendered, Report,
    ReportError,
};
use yaar_system_tests::recorded::{self, PiRecord};

use support::{Scratch, feed_asked};

/// Prompt answered with a review through `input`: line 3 is the `input`, line 4 the transform
/// of it, line 11 the message the model got.
const REPLY_TOOL_RUN: &str = "pi/reply-tool.jsonl";

/// Line 31 is a message pi's own extension queued while a run was open.
const QUEUED_MESSAGE: &str = "pi/queued-message.jsonl";

const TYPED_WHILE_WORKING: &str = "pi-typed-while-working.jsonl";

const REVIEW: &str = "R";

fn rendered(text: &str) -> Rendered<'_> {
    Rendered {
        text,
        review: REVIEW,
        round: 1,
        threads: 3,
    }
}

/// The instruction, a blank line and `characters` characters of threads holding characters of
/// two and three bytes and outside the basic plane (JSON escapes them as UTF-16 pairs),
/// quotes, backslashes, line breaks and controls.
fn review(characters: usize) -> String {
    let filler = "## Thread t1\n\nComment: \"naïve\" `x` \\ → 𝒳 😀 日本語\u{1}\t\r\n";
    let mut text = String::from(INSTRUCTION);
    text.push_str("\n\n");
    text.extend(filler.chars().cycle().take(characters));
    text
}

fn object(bytes: &[u8]) -> serde_json::Map<String, Value> {
    match serde_json::from_slice(bytes).expect("the answer is JSON") {
        Value::Object(fields) => fields,
        other => panic!("the answer is an object, not {other}"),
    }
}

fn files_under(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn moved(records: &[PiRecord], line: usize, project: &Path) -> PiRecord {
    records[line].in_project(project)
}

fn with(record: &PiRecord, field: &str, value: Value) -> PiRecord {
    let mut changed = record.clone();
    changed.payload[field] = value;
    changed
}

fn hand_out(adapter: &Pi, folder: &Path, asked: &Asked, text: &str) -> Vec<u8> {
    adapter
        .delivery(folder, asked)
        .hand_out(&rendered(text))
        .expect("pi's delivery cannot fail")
}

#[test]
fn a_review_of_one_and_a_half_million_characters_is_delivered_whole_with_the_instruction() {
    let scratch = Scratch::new();
    let project = scratch.project();
    let folder = scratch.adapter_folder();
    let records = recorded::pi_events(REPLY_TOOL_RUN);

    assert_eq!(records[3].event, "input");
    assert_eq!(records[4].event, "inject:transform");
    assert_eq!(records[4].payload["route"], "input");
    let number = |line: usize, key: &str| {
        records[line].payload[key]
            .as_u64()
            .unwrap_or_else(|| panic!("line {line} holds a number under {key}"))
    };
    assert_eq!(
        number(11, "textLen"),
        number(3, "textLen") + 2 + number(4, "chars")
    );

    let record = moved(&records, 3, &project);
    let mut adapter = Pi::new();
    let (_, asked) = feed_asked(&mut adapter, &folder, &record);
    let asked = asked.expect("a message asks");
    assert_eq!(asked.asks, Asks::Message);
    assert_eq!(asked.session, CliSession(record.session().to_owned()));
    assert_eq!(asked.terminal.as_deref(), record.terminal());
    assert_eq!(asked.folder.as_deref(), Some(project.as_path()));

    let text = review(1_500_000);
    assert_eq!(
        text.chars().count(),
        INSTRUCTION.chars().count() + 2 + 1_500_000
    );
    let bytes = hand_out(&adapter, &folder, &asked, &text);

    let fields = object(&bytes);
    assert_eq!(fields.keys().collect::<Vec<_>>(), ["review"]);
    assert_eq!(fields["review"].as_str(), Some(text.as_str()));
    assert!(
        bytes.len() < yaar_link::MAX_FRAME as usize,
        "the answer is {} bytes",
        bytes.len()
    );
    assert_eq!(files_under(&folder), Vec::<PathBuf>::new());
    assert!(!folder.exists(), "the Adapter's folder is not made");

    assert!(text.starts_with(INSTRUCTION));
    assert!(fields["review"].as_str().unwrap().starts_with(INSTRUCTION));
    assert_eq!(REPLY_TOOL, "reply_to_thread");
    for named in [
        REPLY_TOOL,
        "`thread`",
        "`body`",
        "`addressed`",
        "Only the reviewer resolves",
    ] {
        assert!(INSTRUCTION.contains(named), "the instruction names {named}");
    }
    // pi's bare tool name is part of Claude Code's prefixed one: the instruction must not carry
    // the prefix.
    assert!(
        !INSTRUCTION.contains("mcp__plugin_yaar-review_yaar__"),
        "the instruction names the tool of Claude Code by its prefix"
    );
    for text_typed in ["ok", "", "do not answer the threads"] {
        let changed = with(&record, "text", json!(text_typed));
        let (_, asked) = feed_asked(&mut Pi::new(), &folder, &changed);
        let asked = asked.expect("a message asks");
        assert_eq!(asked.asks, Asks::Message, "{text_typed:?}");
        assert_eq!(adapter.instruction(), INSTRUCTION);
        assert_eq!(adapter.delivery(&folder, &asked).instruction(), INSTRUCTION);
        assert_eq!(ToPi.instruction(), INSTRUCTION);
        assert_eq!(
            object(&hand_out(&adapter, &folder, &asked, &text))["review"].as_str(),
            Some(text.as_str()),
            "{text_typed:?}"
        );
    }
}

#[test]
fn text_that_json_escapes_comes_back_byte_for_byte() {
    let mut delivery = ToPi;
    for text in [
        "",
        "plain",
        "\"quoted\" `tick` $(touch PWNED) \\ end\\",
        "line\nbreak\r\nand\ttab \u{0} \u{1f} \u{7f} \u{2028}\u{2029}",
        "𝒳 😀 \u{10ffff} ï 日本語",
    ] {
        let bytes = delivery.hand_out(&rendered(text)).expect("it cannot fail");
        assert_eq!(object(&bytes)["review"].as_str(), Some(text), "{text:?}");
    }
}

type Heard = Vec<(Vec<yaar_agent::Reported>, Option<Asked>)>;

fn typed_while_working(
    streaming: Option<&str>,
    project: &Path,
    folder: &Path,
) -> (Vec<PiRecord>, Heard, Pi) {
    let mut records: Vec<PiRecord> = recorded::hand_made_pi_events(TYPED_WHILE_WORKING)
        .iter()
        .map(|record| record.in_project(project))
        .collect();
    if let Some(streaming) = streaming {
        records[4] = with(&records[4], "streamingBehavior", json!(streaming));
    }
    let mut adapter = Pi::new();
    let heard = records
        .iter()
        .map(|record| feed_asked(&mut adapter, folder, record))
        .collect();
    (records, heard, adapter)
}

fn a_review_attached_while_pi_works_goes_with_the_message_it_queued(streaming: Option<&str>) {
    let scratch = Scratch::new();
    let project = scratch.project();
    let folder = scratch.adapter_folder();
    let (records, heard, adapter) = typed_while_working(streaming, &project, &folder);

    // Record 1 opens the run, 3 is the shell command, 4 the message typed meanwhile, 5 the
    // command's result, 6 to 9 the queued message's own turn.
    let named: Vec<&str> = records.iter().map(|record| record.event.as_str()).collect();
    assert_eq!(
        &named[..13],
        [
            "session_start",
            "input",
            "agent_start",
            "tool_call",
            "input",
            "tool_result",
            "turn_end",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
            "agent_settled",
        ]
    );
    assert_eq!(
        records[4].payload["streamingBehavior"],
        streaming.map_or(Value::from("steer"), Value::from),
    );

    let session = CliSession(records[4].session().to_owned());
    let asked = heard[4].1.as_ref().expect("the queued message asks");
    assert_eq!(asked.asks, Asks::Message);
    assert_eq!(asked.session, session);
    assert_eq!(asked.terminal.as_deref(), records[4].terminal());
    assert_eq!(asked.folder.as_deref(), Some(project.as_path()));
    let text = review(2_000);
    let bytes = hand_out(&adapter, &folder, asked, &text);
    assert_eq!(object(&bytes)["review"].as_str(), Some(text.as_str()));
    assert_eq!(files_under(&folder), Vec::<PathBuf>::new());

    assert_eq!(heard[4].0, []);

    let events: Vec<(usize, &Event)> = heard
        .iter()
        .enumerate()
        .flat_map(|(line, (events, _))| events.iter().map(move |one| (line, &one.event)))
        .collect();
    assert_eq!(
        events,
        [
            (
                0,
                &Event::SessionStarted {
                    how: yaar_agent::Start::New
                }
            ),
            (1, &Event::TurnStarted),
            (
                3,
                &Event::Command {
                    phase: yaar_agent::Phase::Began,
                    command: "sleep 8".to_owned()
                }
            ),
            (
                5,
                &Event::Command {
                    phase: yaar_agent::Phase::Ended,
                    command: "sleep 8".to_owned()
                }
            ),
            (
                12,
                &Event::TurnEnded {
                    how: yaar_agent::TurnEnd::Finished
                }
            ),
            (13, &Event::SessionEnded),
        ]
    );
    for (line, (events, asked)) in heard.iter().enumerate().take(10).skip(6) {
        assert_eq!(events, &[], "record {line}");
        assert_eq!(asked, &None, "record {line}");
    }

    let asking: Vec<usize> = heard
        .iter()
        .enumerate()
        .filter(|(_, (_, asked))| asked.is_some())
        .map(|(line, _)| line)
        .collect();
    assert_eq!(asking, [1, 4]);
}

#[test]
fn a_review_attached_while_pi_works_goes_with_the_message_pi_queued_with_enter() {
    a_review_attached_while_pi_works_goes_with_the_message_it_queued(None);
}

#[test]
fn a_review_attached_while_pi_works_goes_with_the_message_pi_queued_with_alt_enter() {
    a_review_attached_while_pi_works_goes_with_the_message_it_queued(Some("followUp"));
}

#[test]
fn a_message_pis_own_extension_queued_asks_nothing_and_every_other_asks_for_the_review() {
    let scratch = Scratch::new();
    let folder = scratch.adapter_folder();
    let records = recorded::pi_events(QUEUED_MESSAGE);
    // Line 31's `input` is an extension's message while the run line 19 opened is open; line 4
    // is another, with pi idle.
    assert_eq!(records[31].payload["source"], "extension");
    assert_eq!(records[31].payload["streamingBehavior"], "followUp");
    assert_eq!(records[19].payload["source"], "interactive");
    assert_eq!(records[4].payload["source"], "extension");

    let mut adapter = Pi::new();
    let mut asking = Vec::new();
    let mut inputs = Vec::new();
    for (line, record) in records.iter().enumerate() {
        let (_, asked) = feed_asked(&mut adapter, &folder, record);
        if record.event == "input" {
            inputs.push(line);
        }
        if let Some(asked) = asked {
            assert_eq!(asked.asks, Asks::Message, "line {line}");
            asking.push(line);
        }
    }
    assert_eq!(inputs, [4, 19, 31]);
    assert_eq!(asking, [19]);
}

#[test]
fn an_input_asks_for_the_review_whatever_its_source_but_an_extensions() {
    let scratch = Scratch::new();
    let folder = scratch.adapter_folder();
    let adapter = Pi::new();
    let asked = |source: Value| {
        let payload = serde_json::to_vec(&json!({
            "event": "input", "sessionId": "s1", "cwd": "/w", "source": source,
            "streamingBehavior": null,
        }))
        .expect("a payload");
        adapter
            .asked(
                &folder,
                &Report {
                    terminal: Some("t1"),
                    pid: 3,
                    payload: &payload,
                },
            )
            .expect("a report")
            .map(|asked| asked.asks)
    };
    assert_eq!(asked(json!("interactive")), Some(Asks::Message));
    assert_eq!(asked(json!("rpc")), Some(Asks::Message));
    assert_eq!(asked(json!("extension")), None);
}

fn reply_payload(arguments: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "event": REPLY_EVENT,
        "sessionId": "s1",
        "sessionFile": "/sessions/s1.jsonl",
        "cwd": "/work",
        "toolCallId": "call_1",
        "arguments": arguments,
    }))
    .expect("a payload")
}

fn asked_of(payload: &[u8]) -> Result<Option<Asked>, ReportError> {
    Pi::new().asked(
        Path::new("/unused"),
        &Report {
            terminal: Some("t1"),
            pid: 3,
            payload,
        },
    )
}

#[test]
fn a_reply_of_the_agent_asks_with_its_thread_and_its_text_byte_for_byte() {
    let hostile = "\"quoted\" `tick` $(touch PWNED) \\n\nsecond line\\";
    let asked = asked_of(&reply_payload(
        json!({"thread": "T1", "body": hostile, "addressed": true}),
    ))
    .expect("a report")
    .expect("a reply asks");
    assert_eq!(
        asked.asks,
        Asks::Reply(AgentReply {
            thread: "T1".to_owned(),
            text: hostile.to_owned(),
            addressed: true,
        })
    );
    assert_eq!(asked.session, CliSession("s1".to_owned()));
    assert_eq!(asked.terminal.as_deref(), Some("t1"));
    assert_eq!(asked.folder.as_deref(), Some(Path::new("/work")));

    for arguments in [
        json!({"thread": "T2", "body": "b"}),
        json!({"thread": "T2", "body": "b", "addressed": false}),
        json!({"thread": "T2", "body": "b", "addressed": null}),
    ] {
        let asked = asked_of(&reply_payload(arguments))
            .expect("a report")
            .expect("a reply asks");
        assert_eq!(
            asked.asks,
            Asks::Reply(AgentReply {
                thread: "T2".to_owned(),
                text: "b".to_owned(),
                addressed: false,
            })
        );
    }
}

#[test]
fn a_reply_that_names_no_thread_or_no_text_is_unreadable() {
    for arguments in [
        json!({"body": "b"}),
        json!({"thread": "T1"}),
        json!({"thread": 1, "body": "b"}),
        json!({"thread": "T1", "body": ["b"]}),
        json!({"thread": "T1", "body": "b", "addressed": "yes"}),
        json!("T1"),
        json!(null),
    ] {
        assert!(
            matches!(
                asked_of(&reply_payload(arguments.clone())),
                Err(ReportError::Unreadable(_))
            ),
            "{arguments}"
        );
    }
}

#[test]
fn an_event_that_waits_for_nothing_asks_nothing_and_a_payload_that_is_no_json_is_unreadable() {
    for event in [
        "session_start",
        "agent_start",
        "tool_call",
        "tool_result",
        "agent_settled",
    ] {
        let payload = serde_json::to_vec(&json!({"event": event, "sessionId": "s1"})).unwrap();
        assert_eq!(asked_of(&payload), Ok(None), "{event}");
    }
    for payload in [&b"not json"[..], b"", b"[]", b"{}"] {
        assert!(
            matches!(asked_of(payload), Err(ReportError::Unreadable(_))),
            "{}",
            String::from_utf8_lossy(payload)
        );
    }
    assert!(matches!(
        asked_of(br#"{"event": "input", "source": "interactive"}"#),
        Err(ReportError::Unreadable(_))
    ));
}

fn answered(answer: &Answer<'_>) -> Vec<u8> {
    let asked = asked_of(&reply_payload(json!({"thread": "T1", "body": "b"})))
        .expect("a report")
        .expect("a reply asks");
    Pi::new().answer(&asked, answer)
}

#[test]
fn the_answers_to_a_reply_are_the_json_the_extension_reads() {
    let kept = answered(&Answer::Replied {
        thread: "T1",
        addressed: true,
        characters: 34,
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&kept).expect("JSON"),
        json!({"text": "recorded reply to T1 (addressed, 34 characters)"})
    );
    let open = answered(&Answer::Replied {
        thread: "T2",
        addressed: false,
        characters: 1,
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&open).expect("JSON"),
        json!({"text": "recorded reply to T2 (open, 1 characters)"})
    );
    let odd = answered(&Answer::Replied {
        thread: "T\"1\\\n",
        addressed: true,
        characters: 0,
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&odd).expect("JSON"),
        json!({"text": "recorded reply to T\"1\\\n (addressed, 0 characters)"})
    );

    let refused = answered(&Answer::Refused {
        why: "no thread \"T9\" in this review\nthe app is running",
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&refused).expect("JSON"),
        json!({"error": "no thread \"T9\" in this review\nthe app is running"})
    );

    assert_eq!(answered(&Answer::Allow), Vec::<u8>::new());
}

#[test]
fn the_command_that_brings_a_session_back_names_the_folder_of_its_file_when_it_is_known() {
    let adapter = Pi::new();
    let session = CliSession("ab'c".to_owned());
    assert_eq!(
        adapter.resume(&session, Some(Path::new("/x y/s/f.jsonl"))),
        "pi --session-dir '/x y/s' --session 'ab'\\''c'"
    );
    assert_eq!(adapter.resume(&session, None), "pi --session 'ab'\\''c'");
    assert_eq!(
        adapter.resume(&session, Some(Path::new("f.jsonl"))),
        "pi --session 'ab'\\''c'"
    );
    assert_eq!(
        adapter.resume(
            &CliSession("s".to_owned()),
            Some(Path::new("/it's $HOME/`x`/f.jsonl"))
        ),
        "pi --session-dir '/it'\\''s $HOME/`x`' --session 's'"
    );
}
