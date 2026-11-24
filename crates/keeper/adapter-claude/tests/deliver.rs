mod support;

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_adapter_claude::{ClaudeCode, INLINE_LIMIT, INSTRUCTION, REVIEWS};
use yaar_agent::{Adapter, Answer, Asked, Asks, CliSession, Rendered};
use yaar_system_tests::recorded::{self, HookRecord};

use support::{Scratch, feed_asked};

/// Lines 0 to 3: the start, the instructions loaded, the user's message, the `PreToolUse` read.
const READ_FILE: &str = "claude/review-file-read.jsonl";

/// Messages at lines 2, 10 and 15, a self-started one at 23.
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

/// A message at line 2, a self-started one at line 33.
const TURNS: &str = "claude/turn-tools.jsonl";

const REVIEW: &str = "R";

struct Place {
    _scratch: Scratch,
    folder: PathBuf,
    project: PathBuf,
}

impl Place {
    fn new() -> Place {
        let scratch = Scratch::new();
        let folder = scratch.path().join("agents").join("claude");
        let project = scratch.path().join("project");
        std::fs::create_dir_all(&project).expect("make the project");
        Place {
            _scratch: scratch,
            folder,
            project,
        }
    }

    fn reviews(&self) -> PathBuf {
        self.folder.join(REVIEWS)
    }

    fn record(&self, name: &str, line: usize) -> HookRecord {
        recorded::hook_events(name)[line].in_project(&self.project)
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

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn shape(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), shape(value)))
                .collect(),
        ),
        Value::String(_) => json!("string"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::Array(_) => json!("array"),
        Value::Null => json!("null"),
    }
}

fn answer_shape(hook: &str) -> Value {
    let shapes = recorded::hand_made_answers("claude-hook-answers.jsonl");
    let one = shapes
        .iter()
        .find(|one| one.answers == hook)
        .expect("the file gives an answer of this hook");
    shape(&one.stdout)
}

fn review(characters: usize, filler: &str) -> String {
    let mut text = String::from(INSTRUCTION);
    text.push_str("\n\n## Thread t1\n\n");
    let have = text.chars().count();
    text.extend(filler.chars().cycle().take(characters - have));
    text
}

fn rendered(text: &str) -> Rendered<'_> {
    Rendered {
        text,
        review: REVIEW,
        round: 1,
        threads: 2,
    }
}

fn hand_out(adapter: &ClaudeCode, place: &Place, asked: &Asked, text: &str) -> io::Result<Vec<u8>> {
    adapter
        .delivery(&place.folder, asked)
        .hand_out(&rendered(text))
}

fn heard_the_session(place: &Place) -> (ClaudeCode, Asked) {
    let mut adapter = ClaudeCode::new();
    let mut message = None;
    for line in 0..=3 {
        let asked = feed_asked(&mut adapter, &place.folder, &place.record(READ_FILE, line));
        if line == 2 {
            message = asked;
        }
    }
    (adapter, message.expect("line 2 is the user's message"))
}

fn read_asks(place: &Place, changes: impl FnOnce(&mut HookRecord)) -> Option<Asked> {
    read_asks_of(&mut ClaudeCode::new(), place, changes)
}

fn read_asks_of(
    adapter: &mut ClaudeCode,
    place: &Place,
    changes: impl FnOnce(&mut HookRecord),
) -> Option<Asked> {
    let mut record = place.record(READ_FILE, 3);
    changes(&mut record);
    feed_asked(adapter, &place.folder, &record)
}

fn pointer_of(context: &str) -> (PathBuf, &str) {
    let after = context
        .strip_prefix(INSTRUCTION)
        .expect("the pointer starts with the instruction");
    let lines: Vec<&str> = after.lines().collect();
    let at: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at].starts_with('/'))
        .collect();
    assert_eq!(at.len(), 1, "one path in {after:?}");
    let request = lines.get(at[0] + 1).expect("a request after the path");
    (PathBuf::from(lines[at[0]]), request)
}

fn reading(path: &Path, session: &str) -> impl FnOnce(&mut HookRecord) {
    let path = path.to_string_lossy().into_owned();
    let session = session.to_owned();
    move |record| {
        record.input["tool_input"]["file_path"] = json!(path);
        record.input["session_id"] = json!(session);
    }
}

fn parsed(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("the answer is JSON")
}

#[test]
fn a_message_of_the_user_is_what_a_delivery_waits_for_and_a_short_review_rides_inline() {
    let place = Place::new();
    let (adapter, asked) = heard_the_session(&place);
    assert_eq!(asked.asks, Asks::Message);
    let session = asked.session.clone();
    assert_eq!(session.0, place.record(READ_FILE, 2).session());

    let text = review(INLINE_LIMIT, "x");
    assert_eq!(units(&text), INLINE_LIMIT);
    let answer = parsed(&hand_out(&adapter, &place, &asked, &text).expect("a delivery"));

    assert_eq!(
        shape(&answer),
        answer_shape("UserPromptSubmit"),
        "the keys and the kinds Claude Code reads"
    );
    assert_eq!(
        answer["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert_eq!(
        answer["hookSpecificOutput"]["additionalContext"], text,
        "the review rides whole in the answer"
    );
    assert_eq!(
        answer["systemMessage"],
        "yaar: review round 1 attached (2 threads)"
    );
    assert_eq!(files_under(&place.reviews()), Vec::<PathBuf>::new());
}

fn assert_delivered_as_a_file(place: &Place, adapter: &ClaudeCode, asked: &Asked, text: &str) {
    let answer = parsed(&hand_out(adapter, place, asked, text).expect("a delivery"));

    assert_eq!(
        shape(&answer),
        answer_shape("UserPromptSubmit"),
        "the same keys and kinds as an inline answer"
    );
    assert_eq!(
        answer["systemMessage"],
        "yaar: review round 1 attached (2 threads)"
    );
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("a text");
    assert!(
        units(context) <= INLINE_LIMIT,
        "the pointer is {} units",
        units(context)
    );
    let (path, request) = pointer_of(context);
    let own = place.reviews().join(&asked.session.0);
    assert!(path.starts_with(&own), "{path:?} lies under {own:?}");
    assert!(!path.starts_with(&place.project), "outside the repository");
    assert!(
        request.starts_with("Read the whole file with your Read tool"),
        "it asks to read the file whole: {request:?}"
    );

    assert_eq!(
        std::fs::read(&path).expect("the file is there"),
        text.as_bytes(),
        "the file holds the review, byte for byte"
    );
    assert_eq!(
        files_under(&place.reviews()),
        [path],
        "the file and no temporary one"
    );
}

#[test]
fn a_review_over_the_limit_is_a_file_outside_the_repository_with_a_pointer_to_read_it_whole() {
    let ascii = review(INLINE_LIMIT + 1, "x");
    // One character outside the basic plane is two units: 10,000 characters, 10,001 units.
    let mut astral = review(INLINE_LIMIT - 1, "x");
    astral.push('\u{1F600}');
    assert_eq!(astral.chars().count(), INLINE_LIMIT);
    let long = review(200_000, "é日本ü\u{1F600}x");

    for (what, text) in [
        ("one unit more", ascii),
        ("10,000 characters, 10,001 units", astral),
        ("200,000 characters", long),
    ] {
        assert!(units(&text) > INLINE_LIMIT, "{what}");
        let place = Place::new();
        let (adapter, asked) = heard_the_session(&place);

        assert_delivered_as_a_file(&place, &adapter, &asked, &text);
    }
}

#[test]
fn a_second_round_is_a_file_of_its_own_and_the_first_stays() {
    let place = Place::new();
    let (adapter, asked) = heard_the_session(&place);
    let text = review(INLINE_LIMIT + 500, "x");
    let again = review(INLINE_LIMIT + 700, "y");

    hand_out(&adapter, &place, &asked, &text).expect("the first round");
    adapter
        .delivery(&place.folder, &asked)
        .hand_out(&Rendered {
            round: 2,
            ..rendered(&again)
        })
        .expect("the second round");

    let files: Vec<_> = files_under(&place.reviews())
        .iter()
        .map(|file| std::fs::read_to_string(file).expect("read"))
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.contains(&text) && files.contains(&again));
}

#[test]
fn the_read_of_the_file_that_was_delivered_is_allowed_and_no_other_read_is() {
    let place = Place::new();
    let (adapter, asked) = heard_the_session(&place);
    let text = review(INLINE_LIMIT + 1, "x");
    let answer = parsed(&hand_out(&adapter, &place, &asked, &text).expect("a delivery"));
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("a text");
    let (delivered, _) = pointer_of(context);
    let session = asked.session.0.clone();

    let mut heard = adapter;
    let allowed =
        read_asks_of(&mut heard, &place, reading(&delivered, &session)).expect("the read is asked");
    assert_eq!(allowed.asks, Asks::ReadOwnFile);
    assert_eq!(allowed.session, CliSession(session.clone()));
    let allow = parsed(&heard.answer(&allowed, &Answer::Allow));
    assert_eq!(shape(&allow), answer_shape("PreToolUse"));
    assert_eq!(allow["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(allow["hookSpecificOutput"]["permissionDecision"], "allow");

    let after_restart = read_asks(&place, reading(&delivered, &session));
    assert_eq!(
        after_restart.map(|asked| asked.asks),
        Some(Asks::ReadOwnFile)
    );

    let roundabout = place
        .reviews()
        .join(&session)
        .join("..")
        .join(&session)
        .join(delivered.file_name().expect("a file name"));
    assert_eq!(
        read_asks(&place, reading(&roundabout, &session)).map(|asked| asked.asks),
        Some(Asks::ReadOwnFile)
    );

    assert_eq!(read_asks(&place, |_| {}), None);

    std::fs::create_dir_all(place.reviews().join("another-session")).expect("make the folder");
    assert_eq!(
        read_asks(&place, reading(&delivered, "another-session")),
        None,
        "session id changed along with the file"
    );

    let outside = place.reviews().join("other.md");
    std::fs::write(&outside, "not a delivery").expect("write");
    let leaving = place.reviews().join(&session).join("..").join("other.md");
    assert!(leaving.exists());
    assert_eq!(read_asks(&place, reading(&leaving, &session)), None);

    let link = place.reviews().join(&session).join("link.md");
    std::os::unix::fs::symlink(&outside, &link).expect("make the link");
    assert_eq!(read_asks(&place, reading(&link, &session)), None);

    let secret = place.folder.join("secret.txt");
    std::fs::write(&secret, "not a review").expect("write");
    assert_eq!(read_asks(&place, reading(&secret, "..")), None);

    for tool in ["Write", "Edit", "Bash"] {
        let asks = read_asks(&place, |record| {
            reading(&delivered, &session)(record);
            record.input["tool_name"] = json!(tool);
        });
        assert_eq!(asks, None, "{tool}");
    }
}

#[test]
fn the_read_of_the_delivered_file_is_allowed_when_the_path_of_the_adapters_folder_holds_a_link() {
    let mut place = Place::new();
    // The folder is given through a link, as a data folder under /var is on macOS.
    let link = place._scratch.path().join("linked");
    std::os::unix::fs::symlink(place._scratch.path(), &link).expect("make the link");
    place.folder = link.join("agents").join("claude");

    let (adapter, asked) = heard_the_session(&place);
    let text = review(INLINE_LIMIT + 1, "x");
    let answer = parsed(&hand_out(&adapter, &place, &asked, &text).expect("a delivery"));
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("a text");
    let (delivered, _) = pointer_of(context);
    let canonical = delivered.canonicalize().expect("the file is there");
    assert!(
        delivered.starts_with(&link),
        "the pointer names the path with the link: {delivered:?}"
    );
    assert_ne!(delivered, canonical, "the link is in the way of the path");
    let session = asked.session.0.clone();

    let mut heard = adapter;
    for path in [&delivered, &canonical] {
        let allowed =
            read_asks_of(&mut heard, &place, reading(path, &session)).expect("the read is asked");
        assert_eq!(
            allowed.asks,
            Asks::ReadOwnFile,
            "{path:?}, as the Adapter that heard it"
        );
        assert_eq!(
            read_asks(&place, reading(path, &session)).map(|asked| asked.asks),
            Some(Asks::ReadOwnFile),
            "{path:?}, as a fresh Adapter"
        );
    }

    std::fs::create_dir_all(place.reviews().join("another-session")).expect("make the folder");
    assert_eq!(
        read_asks(&place, reading(&delivered, "another-session")),
        None
    );
}

#[test]
fn a_read_asks_nothing_while_the_folder_of_reviews_is_not_there() {
    let place = Place::new();
    let session = place.record(READ_FILE, 3).session().to_owned();
    let file = place.reviews().join(&session).join("R-1.md");

    assert_eq!(read_asks(&place, reading(&file, &session)), None);
    assert!(!place.reviews().exists(), "asking made nothing");
}

#[test]
fn a_session_id_that_is_no_folder_name_gets_no_file_and_no_delivery() {
    let place = Place::new();
    let adapter = ClaudeCode::new();
    for id in ["a/b", "..", "a.b", "", "é", "a b"] {
        let asked = Asked {
            session: CliSession(id.to_owned()),
            terminal: None,
            folder: None,
            asks: Asks::Message,
        };
        for text in [review(INLINE_LIMIT + 1, "x"), review(2_000, "x")] {
            let error = hand_out(&adapter, &place, &asked, &text).expect_err(id);

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{id:?}");
        }
    }

    assert!(!place.folder.exists(), "nothing was made");
}

#[test]
fn a_review_id_that_is_no_file_name_is_no_file_either() {
    let place = Place::new();
    let (adapter, asked) = heard_the_session(&place);
    let text = review(INLINE_LIMIT + 1, "x");

    let error = adapter
        .delivery(&place.folder, &asked)
        .hand_out(&Rendered {
            review: "../x",
            ..rendered(&text)
        })
        .expect_err("a review id with a slash");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(files_under(&place.folder), Vec::<PathBuf>::new());
}

#[test]
fn a_folder_that_cannot_be_written_fails_the_delivery_of_a_long_review() {
    let place = Place::new();
    let (adapter, asked) = heard_the_session(&place);
    std::fs::create_dir_all(place.folder.parent().expect("a parent")).expect("make the parent");
    std::fs::write(&place.folder, "not a folder").expect("write");

    let failed = hand_out(&adapter, &place, &asked, &review(INLINE_LIMIT + 1, "x"));

    assert!(
        failed.is_err(),
        "nothing is sent when the file is not written"
    );
}

fn asks_of(name: &str, place: &Place, lines: std::ops::RangeInclusive<usize>) -> Vec<Option<Asks>> {
    let mut adapter = ClaudeCode::new();
    let records = recorded::hook_events(name);
    lines
        .map(|line| {
            let record = records[line].in_project(&place.project);
            feed_asked(&mut adapter, &place.folder, &record).map(|asked| asked.asks)
        })
        .collect()
}

#[test]
fn the_answer_to_a_permission_prompt_asks_nothing_and_the_users_next_message_asks() {
    let place = Place::new();
    let records = recorded::hook_events(DIALOGS);
    let kinds: Vec<_> = records[..=10].iter().map(|r| r.event.as_str()).collect();
    assert_eq!(
        kinds[2..=6],
        [
            "UserPromptSubmit",
            "PreToolUse",
            "PermissionRequest",
            "Notification",
            "PostToolUse"
        ],
        "lines 3 to 6: the Write, the prompt, its notice and the tool after the yes"
    );
    assert_eq!(kinds[10], "UserPromptSubmit");

    let asks = asks_of(DIALOGS, &place, 0..=10);

    let asking: Vec<_> = asks
        .iter()
        .enumerate()
        .filter(|(_, asks)| asks.is_some())
        .map(|(line, _)| line)
        .collect();
    assert_eq!(asking, [2, 10], "only the user's two messages ask");
    assert_eq!(asks[2], Some(Asks::Message));
    assert_eq!(asks[10], Some(Asks::Message));
    assert_eq!(asks[4], None);
    assert_eq!(asks[6], None);
}

#[test]
fn a_message_claude_code_starts_itself_asks_nothing_and_the_next_one_the_user_types_asks() {
    let place = Place::new();

    let turns = asks_of(TURNS, &place, 0..=33);
    assert_eq!(turns[2], Some(Asks::Message));
    assert_eq!(turns[33], None, "line 33 opens with <task-notification>");

    let dialogs = asks_of(DIALOGS, &place, 0..=23);
    assert_eq!(dialogs[15], Some(Asks::Message));
    assert_eq!(dialogs[23], None);

    let mut adapter = ClaudeCode::new();
    let asks: Vec<_> = recorded::hand_made_hook_events("claude-typed-after-self-started.jsonl")
        .iter()
        .map(|record| feed_asked(&mut adapter, &place.folder, record).map(|asked| asked.asks))
        .collect();
    assert_eq!(
        asks,
        [
            None,
            Some(Asks::Message),
            None,
            None,
            None,
            Some(Asks::Message),
            None,
            None
        ],
        "records 1 and 5 are typed, 3 is the self-started one"
    );
}
