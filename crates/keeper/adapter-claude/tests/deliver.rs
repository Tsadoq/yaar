//! How a review is delivered to Claude Code: what a hook's input asks of the app, what the
//! app answers a message with, and which read of a file the app allows. The recorded hooks
//! go in through the Adapter's calls, as the Keeper makes them (the report, then the question
//! of what it waits for), and the answers come out as bytes.

mod support;

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_adapter_claude::{ClaudeCode, INLINE_LIMIT, INSTRUCTION, REVIEWS};
use yaar_agent::{Adapter, Answer, Asked, Asks, CliSession, Rendered};
use yaar_system_tests::recorded::{self, HookRecord};

use support::{Scratch, feed_asked};

/// The recording of a session that reads a file of the reviews' folder: lines 0 to 3 are the
/// start, the instructions loaded, the user's message and the `PreToolUse` of the read.
const READ_FILE: &str = "claude/review-file-read.jsonl";

/// The recording of the dialogs: messages at lines 2, 10 and 15, a self-started one at 23.
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

/// The recording of a turn with every kind of tool: a message at line 2, a self-started
/// one at line 33.
const TURNS: &str = "claude/turn-tools.jsonl";

/// The review of the delivery tests, and the round it starts.
const REVIEW: &str = "R";

/// A folder of the Adapter and a project folder, inside one scratch folder: the Adapter's
/// folder is not made, as it is not before the app first writes there.
struct Place {
    /// Removes everything when the test ends.
    _scratch: Scratch,
    /// The Adapter's own folder of the app.
    folder: PathBuf,
    /// The folder the recorded session works in, moved here from the recording's.
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

    /// The folder of reviews the Adapter's delivery writes into.
    fn reviews(&self) -> PathBuf {
        self.folder.join(REVIEWS)
    }

    /// The record `line` of the recording `name`, as if it had been made in the project.
    fn record(&self, name: &str, line: usize) -> HookRecord {
        recorded::hook_events(name)[line].in_project(&self.project)
    }
}

/// Every file under `folder`, none when it is not there.
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

/// Claude Code's count of a string: UTF-16 code units.
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// What a JSON value is made of: the keys of every object and the kind of every other
/// value, which is what Claude Code reads a hook's answer by.
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

/// The shape of what a hook of the event `hook` prints, as the hand-made file gives it.
fn answer_shape(hook: &str) -> Value {
    let shapes = recorded::hand_made_answers("claude-hook-answers.jsonl");
    let one = shapes
        .iter()
        .find(|one| one.answers == hook)
        .expect("the file gives an answer of this hook");
    shape(&one.stdout)
}

/// A review text of `characters` characters that starts with the instruction, as the
/// Review engine renders it, and goes on with the cycle of `filler`'s characters.
fn review(characters: usize, filler: &str) -> String {
    let mut text = String::from(INSTRUCTION);
    text.push_str("\n\n## Thread t1\n\n");
    let have = text.chars().count();
    text.extend(filler.chars().cycle().take(characters - have));
    text
}

/// What the engine hands the delivery for `text`: the second round of review `R`, of two
/// threads.
fn rendered(text: &str) -> Rendered<'_> {
    Rendered {
        text,
        review: REVIEW,
        round: 1,
        threads: 2,
    }
}

/// The answer to the message `asked` stands for, as the hook's bytes.
fn hand_out(adapter: &ClaudeCode, place: &Place, asked: &Asked, text: &str) -> io::Result<Vec<u8>> {
    adapter
        .delivery(&place.folder, asked)
        .hand_out(&rendered(text))
}

/// The session `review-file-read.jsonl` records, the Adapter that has heard its lines 0 to
/// 3 (its folder under `place`), and the question line 2, the user's message, asked.
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

/// What a fresh Adapter says line 3 of the recording waits for, with `changes` made on its
/// input (each named by the caller).
fn read_asks(place: &Place, changes: impl FnOnce(&mut HookRecord)) -> Option<Asked> {
    let mut record = place.record(READ_FILE, 3);
    changes(&mut record);
    feed_asked(&mut ClaudeCode::new(), &place.folder, &record)
}

/// The record of the read of `path`, in a session of its own id.
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

    // Exactly the limit, in UTF-16 units.
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

/// A text that is over the limit: delivered as a file under the session's folder of reviews,
/// with a pointer in the answer; the answer and the file put together equal the text.
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
    let after = context
        .strip_prefix(INSTRUCTION)
        .expect("the pointer starts with the instruction");
    let paths: Vec<_> = after
        .split_whitespace()
        .filter(|word| word.starts_with('/'))
        .collect();
    assert_eq!(paths.len(), 1, "one path in {after:?}");
    let path = Path::new(paths[0]);
    let own = place.reviews().join(&asked.session.0);
    assert!(path.starts_with(&own), "{path:?} lies under {own:?}");
    assert!(!path.starts_with(&place.project), "outside the repository");
    let lower = after.to_lowercase();
    assert!(
        lower.contains("read") && lower.contains("whole"),
        "it asks to read the file whole: {after:?}"
    );

    assert_eq!(
        std::fs::read(path).expect("the file is there"),
        text.as_bytes(),
        "the file holds the review, byte for byte"
    );
    assert_eq!(
        files_under(&place.reviews()),
        [path.to_owned()],
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
    let delivered = PathBuf::from(
        context
            .split_whitespace()
            .find(|word| word.starts_with('/'))
            .expect("the pointer names the file"),
    );
    let session = asked.session.0.clone();

    // The one field of line 3 that changes is the file it reads (and, where named, the
    // session).
    let allowed = read_asks(&place, reading(&delivered, &session)).expect("the read is asked");
    assert_eq!(allowed.asks, Asks::ReadOwnFile);
    assert_eq!(allowed.session, CliSession(session.clone()));
    let allow = parsed(&adapter.answer(&allowed, &Answer::Allow));
    assert_eq!(shape(&allow), answer_shape("PreToolUse"));
    assert_eq!(allow["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(allow["hookSpecificOutput"]["permissionDecision"], "allow");

    // The Keeper started again: the rule needs nothing the first Adapter heard.
    let after_restart = read_asks(&place, reading(&delivered, &session));
    assert_eq!(
        after_restart.map(|asked| asked.asks),
        Some(Asks::ReadOwnFile)
    );

    // A path that goes up and comes back down inside the folder is that file.
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

    // Line 3 as recorded: a file outside the project and the folder.
    assert_eq!(read_asks(&place, |_| {}), None);

    // The file of this session, asked by another session, which has a folder of its own.
    std::fs::create_dir_all(place.reviews().join("another-session")).expect("make the folder");
    assert_eq!(
        read_asks(&place, reading(&delivered, "another-session")),
        None,
        "session id changed along with the file"
    );

    // A path that leaves the folder, to a file that is there.
    let outside = place.reviews().join("other.md");
    std::fs::write(&outside, "not a delivery").expect("write");
    let leaving = place.reviews().join(&session).join("..").join("other.md");
    assert!(leaving.exists());
    assert_eq!(read_asks(&place, reading(&leaving, &session)), None);

    // A link inside the folder to a file outside it.
    let link = place.reviews().join(&session).join("link.md");
    std::os::unix::fs::symlink(&outside, &link).expect("make the link");
    assert_eq!(read_asks(&place, reading(&link, &session)), None);

    // A session id that names the folder's own parent is no folder of reviews.
    let secret = place.folder.join("secret.txt");
    std::fs::write(&secret, "not a review").expect("write");
    assert_eq!(read_asks(&place, reading(&secret, "..")), None);

    // Only the read tool is allowed to read it.
    for tool in ["Write", "Edit", "Bash"] {
        let asks = read_asks(&place, |record| {
            reading(&delivered, &session)(record);
            record.input["tool_name"] = json!(tool);
        });
        assert_eq!(asks, None, "{tool}");
    }
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
    // A file where the folder of reviews should be.
    std::fs::write(&place.folder, "not a folder").expect("write");

    let failed = hand_out(&adapter, &place, &asked, &review(INLINE_LIMIT + 1, "x"));

    assert!(
        failed.is_err(),
        "nothing is sent when the file is not written"
    );
}

/// The answer of each prompt a session heard, in order, read as what it waits for.
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
    // The prompt itself (4) and the answer to it (6) ask for no review.
    assert_eq!(asks[4], None);
    assert_eq!(asks[6], None);
}

#[test]
fn a_message_claude_code_starts_itself_asks_nothing_and_the_next_one_the_user_types_asks() {
    let place = Place::new();

    // A turn of tools, then the message of a background task that finished.
    let turns = asks_of(TURNS, &place, 0..=33);
    assert_eq!(turns[2], Some(Asks::Message));
    assert_eq!(turns[33], None, "line 33 opens with <task-notification>");

    // The same after a permission prompt: line 23 is the self-started message.
    let dialogs = asks_of(DIALOGS, &place, 0..=23);
    assert_eq!(dialogs[15], Some(Asks::Message));
    assert_eq!(dialogs[23], None);

    // A message the user types after one Claude Code started.
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
