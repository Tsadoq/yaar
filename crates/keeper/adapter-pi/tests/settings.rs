//! The pi Adapter's entry in pi's settings after the user's yes, and the answer it keeps. Each
//! test has its own home, so none reaches pi's real settings; files are typed as bytes.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use yaar_adapter_pi::{
    AGENT_FOLDER, CONSENT_FILE, EXTENSION, EXTENSION_FILE, Pi, SETTINGS_FILE, SETTINGS_KEY,
};
use yaar_agent::{Adapter, Consent, ConsentAnswer};

struct World {
    root: PathBuf,
    home: PathBuf,
    folder: PathBuf,
}

impl World {
    fn without_pi() -> World {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-pi-settings-{}-{n}", std::process::id()));
        let home = root.join("home");
        // The Adapter folder's name holds a quote and a backslash, which the JSON entry must
        // escape.
        let folder = root.join("data").join("agents").join("pi \"q\" \\b");
        fs::create_dir_all(&home).expect("make the home");
        fs::create_dir_all(&folder).expect("make the folder of the Adapter");
        World { root, home, folder }
    }

    fn with_pi() -> World {
        let world = World::without_pi();
        fs::create_dir_all(world.agent_folder()).expect("make pi's folder");
        world
    }

    fn agent_folder(&self) -> PathBuf {
        self.home.join(AGENT_FOLDER)
    }

    fn settings(&self) -> PathBuf {
        self.agent_folder().join(SETTINGS_FILE)
    }

    fn extension(&self) -> String {
        self.folder
            .join(EXTENSION)
            .join(EXTENSION_FILE)
            .to_str()
            .expect("the scratch path is text")
            .to_owned()
    }

    fn quoted(&self) -> String {
        serde_json::to_string(&self.extension()).expect("a string is JSON")
    }

    fn fill(&self, template: &str) -> String {
        template.replace('@', &self.quoted())
    }

    fn put(&self, bytes: &[u8]) {
        fs::write(self.settings(), bytes).expect("write the settings");
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(self.settings()).expect("read the settings")
    }

    fn pi(&self) -> Pi {
        Pi::in_home(&self.home)
    }

    fn consent(&self) -> Consent {
        self.pi()
            .consent(&self.folder)
            .expect("pi is installed, so a question exists")
    }

    fn answer(&self, answer: ConsentAnswer) -> Consent {
        self.pi()
            .answer_consent(&self.folder, answer)
            .expect("pi is installed, so an answer is taken")
    }

    fn kept(&self) -> Option<String> {
        fs::read_to_string(self.folder.join(CONSENT_FILE)).ok()
    }

    fn modified(&self) -> SystemTime {
        fs::metadata(self.settings())
            .and_then(|meta| meta.modified())
            .expect("the modification time")
    }

    /// A modification time long ago, so that a write shows without waiting for the clock.
    fn age(&self) -> SystemTime {
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(self.settings())
            .and_then(|file| file.set_modified(long_ago))
            .expect("set the modification time");
        long_ago
    }

    fn tree(&self) -> BTreeSet<String> {
        fn walk(folder: &Path, root: &Path, found: &mut BTreeSet<String>) {
            for entry in fs::read_dir(folder).expect("read the folder") {
                let path = entry.expect("an entry").path();
                let kind = fs::symlink_metadata(&path).expect("metadata").file_type();
                let name = path.strip_prefix(root).expect("under the root");
                let tag = if kind.is_symlink() {
                    "link"
                } else if kind.is_dir() {
                    "dir"
                } else {
                    "file"
                };
                found.insert(format!("{tag} {}", name.display()));
                if kind.is_dir() {
                    walk(&path, root, found);
                }
            }
        }
        let mut found = BTreeSet::new();
        walk(&self.root, &self.root, &mut found);
        found
    }

    fn agent_names(&self) -> BTreeSet<String> {
        fs::read_dir(self.agent_folder())
            .expect("read pi's folder")
            .map(|entry| entry.expect("an entry").file_name().into_string().unwrap())
            .collect()
    }

    fn only_the_settings_are_in_pi_folder(&self) {
        assert_eq!(
            self.agent_names(),
            BTreeSet::from([SETTINGS_FILE.to_owned()]),
            "no temporary file and no lock is left beside the settings"
        );
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn parse(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("text");
    serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text)).expect("JSON")
}

fn assert_means_the_same_and_the_entry(before: &[u8], after: &[u8], entry: &str) {
    let mut expected = parse(before);
    let list = expected
        .as_object_mut()
        .expect("an object")
        .entry(SETTINGS_KEY)
        .or_insert_with(|| json!([]));
    list.as_array_mut().expect("an array").push(json!(entry));
    assert_eq!(
        parse(after),
        expected,
        "the file means `before` and the entry"
    );
}

struct Layout {
    name: &'static str,
    before: &'static str,
    yes: &'static str,
    no: Option<&'static str>,
}

/// Where `no` is `None` the bytes come back as they were; otherwise the yes made the key, which
/// stays with an empty array.
const LAYOUTS: &[Layout] = &[
    Layout {
        name: "pretty, two paths, other keys before and after",
        before: "{\n  \"theme\": \"dark\",\n  \"extensions\": [\n    \"/a\",\n    \"/b\"\n  ],\n  \"last\": 1\n}\n",
        yes: "{\n  \"theme\": \"dark\",\n  \"extensions\": [\n    \"/a\",\n    \"/b\", @\n  ],\n  \"last\": 1\n}\n",
        no: None,
    },
    Layout {
        name: "compact, one path",
        before: "{\"extensions\":[\"/a\"]}",
        yes: "{\"extensions\":[\"/a\", @]}",
        no: None,
    },
    Layout {
        name: "compact, empty array",
        before: "{\"extensions\":[]}",
        yes: "{\"extensions\":[@]}",
        no: None,
    },
    Layout {
        name: "empty array with a blank inside",
        before: "{ \"extensions\": [ ] }",
        yes: "{ \"extensions\": [@ ] }",
        no: None,
    },
    Layout {
        name: "empty array across lines",
        before: "{\n  \"extensions\": [\n  ]\n}\n",
        yes: "{\n  \"extensions\": [@\n  ]\n}\n",
        no: None,
    },
    Layout {
        name: "tabs, decoys in strings and nested values, no final line feed",
        before: "{\n\t\"packages\": [\"npm:x\", {\"source\": \"y\", \"extensions\": [\"nested\"]}],\n\t\"note\": \"a ] } \\\" \\\"extensions\\\": [\",\n\t\"extensions\": [\n\t\t\"/a\"\n\t],\n\t\"z\": null\n}",
        yes: "{\n\t\"packages\": [\"npm:x\", {\"source\": \"y\", \"extensions\": [\"nested\"]}],\n\t\"note\": \"a ] } \\\" \\\"extensions\\\": [\",\n\t\"extensions\": [\n\t\t\"/a\", @\n\t],\n\t\"z\": null\n}",
        no: None,
    },
    Layout {
        name: "byte-order mark and carriage returns",
        before: "\u{feff}{\r\n  \"extensions\": [\"/a\"]\r\n}\r\n",
        yes: "\u{feff}{\r\n  \"extensions\": [\"/a\", @]\r\n}\r\n",
        no: None,
    },
    Layout {
        name: "paths written with escapes",
        before: "{\"extensions\": [\"\\/opt\\/x\", \"\\u0041\"]}",
        yes: "{\"extensions\": [\"\\/opt\\/x\", \"\\u0041\", @]}",
        no: None,
    },
    Layout {
        name: "the key written with an escape",
        before: "{\"ext\\u0065nsions\": [\"/a\"]}",
        yes: "{\"ext\\u0065nsions\": [\"/a\", @]}",
        no: None,
    },
    Layout {
        name: "no array, pretty, a nested value last",
        before: "{\n  \"theme\": \"dark\",\n  \"models\": {\"a\": [1, 2]}\n}\n",
        yes: "{\n  \"theme\": \"dark\",\n  \"models\": {\"a\": [1, 2]},\n  \"extensions\": [@]\n}\n",
        no: Some(
            "{\n  \"theme\": \"dark\",\n  \"models\": {\"a\": [1, 2]},\n  \"extensions\": []\n}\n",
        ),
    },
    Layout {
        name: "no array, tabs, no final line feed",
        before: "{\n\t\"a\": 1\n}",
        yes: "{\n\t\"a\": 1,\n\t\"extensions\": [@]\n}",
        no: Some("{\n\t\"a\": 1,\n\t\"extensions\": []\n}"),
    },
    Layout {
        name: "no array, compact",
        before: "{\"a\":1}",
        yes: "{\"a\":1,\"extensions\":[@]}",
        no: Some("{\"a\":1,\"extensions\":[]}"),
    },
    Layout {
        name: "no array, members on one line with blanks",
        before: "{\"a\": 1, \"b\": 2}",
        yes: "{\"a\": 1, \"b\": 2, \"extensions\": [@]}",
        no: Some("{\"a\": 1, \"b\": 2, \"extensions\": []}"),
    },
    Layout {
        name: "no array, only a nested extensions, laid out as its one member is",
        before: "{\"x\": {\"extensions\": [\"n\"]}}",
        yes: "{\"x\": {\"extensions\": [\"n\"]},\"extensions\": [@]}",
        no: Some("{\"x\": {\"extensions\": [\"n\"]},\"extensions\": []}"),
    },
    Layout {
        name: "no array, byte-order mark and carriage returns",
        before: "\u{feff}{\r\n  \"a\": 1\r\n}\r\n",
        yes: "\u{feff}{\r\n  \"a\": 1,\r\n  \"extensions\": [@]\r\n}\r\n",
        no: Some("\u{feff}{\r\n  \"a\": 1,\r\n  \"extensions\": []\r\n}\r\n"),
    },
    Layout {
        name: "an empty object",
        before: "{}\n",
        yes: "{\n  \"extensions\": [\n    @\n  ]\n}\n",
        no: Some("{\n  \"extensions\": []\n}\n"),
    },
];

#[test]
fn a_yes_with_no_file_writes_one_as_pi_would() {
    let world = World::with_pi();

    let consent = world.answer(ConsentAnswer::Yes);

    assert_eq!(
        String::from_utf8(world.bytes()).unwrap(),
        format!("{{\n  \"extensions\": [\n    {}\n  ]\n}}\n", world.quoted()),
        "pi's own form: two spaces, the one key, a final line feed"
    );
    assert_eq!(consent.answer, ConsentAnswer::Yes);
    assert_eq!(consent.problem, None);
    assert_eq!(world.kept().as_deref(), Some("yes\n"));
    world.only_the_settings_are_in_pi_folder();
}

#[test]
fn a_yes_changes_no_other_byte_of_a_file_of_any_layout() {
    for layout in LAYOUTS {
        let world = World::with_pi();
        world.put(layout.before.as_bytes());

        let consent = world.answer(ConsentAnswer::Yes);

        assert_eq!(
            String::from_utf8(world.bytes()).unwrap(),
            world.fill(layout.yes),
            "{}: the bytes are the file's and the one string at its place",
            layout.name
        );
        assert_means_the_same_and_the_entry(
            layout.before.as_bytes(),
            &world.bytes(),
            &world.extension(),
        );
        assert_eq!(consent.answer, ConsentAnswer::Yes, "{}", layout.name);
        assert_eq!(consent.problem, None, "{}", layout.name);
        world.only_the_settings_are_in_pi_folder();
    }
}

#[test]
fn a_yes_leaves_an_array_of_two_with_the_entry_third() {
    let world = World::with_pi();
    world.put(br#"{"extensions": ["/one.ts", "/two.ts"]}"#);

    world.answer(ConsentAnswer::Yes);

    let settings = parse(&world.bytes());
    assert_eq!(
        settings[SETTINGS_KEY],
        json!(["/one.ts", "/two.ts", world.extension()]),
        "the entry is at the end, after the user's two"
    );
}

#[test]
fn a_yes_with_the_entry_already_there_writes_nothing() {
    let spellings: [fn(&str) -> String; 3] = [
        |path| serde_json::to_string(path).unwrap(),
        |path| serde_json::to_string(path).unwrap().replace('/', "\\/"),
        |path| {
            let quoted = serde_json::to_string(path).unwrap();
            quoted.replacen("yaar", "\\u0079aar", 1)
        },
    ];
    for spell in spellings {
        let world = World::with_pi();
        let text = format!(
            "{{\n  \"extensions\": [\n    \"/a\",\n    {}\n  ]\n}}\n",
            spell(&world.extension())
        );
        world.put(text.as_bytes());
        let before = world.age();
        let tree = world.tree();

        let consent = world.answer(ConsentAnswer::Yes);

        assert_eq!(world.bytes(), text.as_bytes(), "the bytes are as they were");
        assert_eq!(world.modified(), before, "the file was not written");
        assert_eq!(consent.answer, ConsentAnswer::Yes);
        assert_eq!(consent.problem, None);
        assert_eq!(world.kept().as_deref(), Some("yes\n"));
        let mut expected = tree;
        expected.insert(format!(
            "file data/agents/{}/{CONSENT_FILE}",
            "pi \"q\" \\b"
        ));
        assert_eq!(
            world.tree(),
            expected,
            "only the answer is kept, no lock was made"
        );
    }
}

#[test]
fn a_second_yes_writes_nothing() {
    let world = World::with_pi();
    world.put(b"{\"a\": 1}\n");
    world.answer(ConsentAnswer::Yes);
    let written = world.bytes();
    let before = world.age();

    world.answer(ConsentAnswer::Yes);

    assert_eq!(world.bytes(), written);
    assert_eq!(world.modified(), before);
}

#[test]
fn the_entry_is_the_extensions_absolute_path_as_a_json_string() {
    let world = World::with_pi();
    world.put(b"{\"extensions\": []}");

    world.answer(ConsentAnswer::Yes);

    let settings = parse(&world.bytes());
    let entry = settings[SETTINGS_KEY][0].as_str().expect("a string");
    assert!(
        world.extension().contains("\"q\" \\b"),
        "the path is a hard one"
    );
    assert_eq!(entry, world.extension());
    assert!(Path::new(entry).is_absolute());
    assert!(entry.ends_with(&format!("{EXTENSION}/{EXTENSION_FILE}")));
}

#[test]
fn the_question_names_the_extension_and_the_settings_and_says_the_answer_can_change() {
    let world = World::with_pi();

    let consent = world.consent();

    assert!(
        consent.question.contains(&world.extension()),
        "{}",
        consent.question
    );
    assert!(
        consent
            .question
            .contains(world.settings().to_str().unwrap()),
        "{}",
        consent.question
    );
    assert!(
        consent.question.contains(SETTINGS_KEY),
        "{}",
        consent.question
    );
    assert!(
        consent.question.contains("change your answer"),
        "{}",
        consent.question
    );
    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert_eq!(consent.problem, None);
}

#[test]
fn taking_the_yes_back_takes_out_the_one_string_and_changes_nothing_else() {
    for layout in LAYOUTS {
        for taking_back in [ConsentAnswer::No, ConsentAnswer::NotAsked] {
            let world = World::with_pi();
            world.put(layout.before.as_bytes());
            world.answer(ConsentAnswer::Yes);

            let consent = world.answer(taking_back);

            let expected = match layout.no {
                Some(no) => no.to_owned(),
                None => layout.before.to_owned(),
            };
            assert_eq!(
                String::from_utf8(world.bytes()).unwrap(),
                expected,
                "{} ({taking_back:?}): where the array was there the bytes are as before the \
                 yes, where the yes made the key it stays with an empty array",
                layout.name
            );
            assert_eq!(consent.answer, taking_back, "{}", layout.name);
            assert_eq!(consent.problem, None, "{}", layout.name);
            world.only_the_settings_are_in_pi_folder();
        }
    }
}

#[test]
fn a_no_after_a_yes_on_no_file_leaves_the_file_the_app_made_with_an_empty_array() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);

    world.answer(ConsentAnswer::No);

    assert_eq!(
        String::from_utf8(world.bytes()).unwrap(),
        "{\n  \"extensions\": []\n}\n"
    );
}

#[test]
fn taking_the_yes_back_leaves_the_users_own_entries_wherever_ours_stands() {
    let arrays: [(&str, &str); 4] = [
        ("[@, \"/b\", \"/c\"]", "[\"/b\", \"/c\"]"),
        ("[\"/a\", @, \"/c\"]", "[\"/a\", \"/c\"]"),
        ("[\"/a\", \"/b\", @]", "[\"/a\", \"/b\"]"),
        (
            "[\n    \"/a\",\n    @,\n    \"/c\"\n  ]",
            "[\n    \"/a\",\n    \"/c\"\n  ]",
        ),
    ];
    for (typed, left) in arrays {
        let world = World::with_pi();
        let text = format!(
            "{{\n  \"x\": 1,\n  \"extensions\": {}\n}}\n",
            world.fill(typed)
        );
        world.put(text.as_bytes());
        world.answer(ConsentAnswer::Yes);
        assert_eq!(
            world.bytes(),
            text.as_bytes(),
            "{typed}: the entry was there already"
        );

        world.answer(ConsentAnswer::No);

        assert_eq!(
            String::from_utf8(world.bytes()).unwrap(),
            format!("{{\n  \"x\": 1,\n  \"extensions\": {left}\n}}\n"),
            "{typed}"
        );
    }
}

#[test]
fn a_no_and_a_question_not_yet_answered_write_nothing_into_pis_settings() {
    for layout in LAYOUTS {
        let world = World::with_pi();
        world.put(layout.before.as_bytes());
        let before = world.age();
        let agent_tree = world.agent_names();

        let asked = world.consent();
        let no = world.answer(ConsentAnswer::No);
        let again = world.answer(ConsentAnswer::NotAsked);

        assert_eq!(asked.answer, ConsentAnswer::NotAsked, "{}", layout.name);
        assert_eq!(no.answer, ConsentAnswer::No, "{}", layout.name);
        assert_eq!(again.answer, ConsentAnswer::NotAsked, "{}", layout.name);
        assert_eq!(world.bytes(), layout.before.as_bytes(), "{}", layout.name);
        assert_eq!(world.modified(), before, "{}: not rewritten", layout.name);
        assert_eq!(
            world.agent_names(),
            agent_tree,
            "{}: nothing made beside it",
            layout.name
        );
    }
}

#[test]
fn a_no_and_a_question_not_yet_answered_make_no_file_where_there_is_none() {
    let world = World::with_pi();

    world.consent();
    let no = world.answer(ConsentAnswer::No);
    let again = world.answer(ConsentAnswer::NotAsked);

    assert!(
        world.agent_names().is_empty(),
        "pi's folder is as it was: empty"
    );
    assert_eq!(no.answer, ConsentAnswer::No);
    assert_eq!(again.answer, ConsentAnswer::NotAsked);
}

#[test]
fn the_kept_answer_follows_the_last_one_given() {
    let world = World::with_pi();
    world.put(b"{}");

    world.answer(ConsentAnswer::Yes);
    assert_eq!(world.kept().as_deref(), Some("yes\n"));
    world.answer(ConsentAnswer::No);
    assert_eq!(world.kept().as_deref(), Some("no\n"));
    world.answer(ConsentAnswer::Yes);
    assert_eq!(world.kept().as_deref(), Some("yes\n"));
    world.answer(ConsentAnswer::NotAsked);
    assert_eq!(world.kept(), None, "ask again removes what was kept");
    assert!(!world.folder.join(CONSENT_FILE).exists());
}

fn unreadable_files() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let not_json = "not JSON";
    let not_strings = "not a list of strings";
    let twice = "more than once";
    vec![
        (
            "not UTF-8",
            b"{\"extensions\": [\"\xff\xfe\"]}".to_vec(),
            "not text",
        ),
        ("not JSON", b"extensions: yes\n".to_vec(), not_json),
        ("empty", Vec::new(), not_json),
        (
            "a JSON array",
            b"[\"extensions\"]\n".to_vec(),
            "not a JSON object",
        ),
        ("a JSON string", b"\"x\"".to_vec(), "not a JSON object"),
        (
            "extensions that is a string",
            br#"{"extensions": "x"}"#.to_vec(),
            not_strings,
        ),
        (
            "extensions that is an object",
            br#"{"extensions": {}}"#.to_vec(),
            not_strings,
        ),
        (
            "extensions that is null",
            br#"{"extensions": null}"#.to_vec(),
            not_strings,
        ),
        (
            "extensions holding a number",
            br#"{"extensions": [1]}"#.to_vec(),
            not_strings,
        ),
        (
            "extensions holding an object",
            br#"{"extensions": [{"path": "/a"}]}"#.to_vec(),
            not_strings,
        ),
        (
            "two top-level extensions",
            b"{\n  \"extensions\": [\"/a\"],\n  \"extensions\": []\n}\n".to_vec(),
            twice,
        ),
        (
            "two top-level extensions, one spelled with an escape",
            br#"{"extensions": [], "ext\u0065nsions": []}"#.to_vec(),
            twice,
        ),
        (
            "a comment",
            b"{\n  // my extensions\n  \"extensions\": []\n}\n".to_vec(),
            not_json,
        ),
        (
            "a trailing comma",
            br#"{"extensions": [],}"#.to_vec(),
            not_json,
        ),
        ("cut off", b"{\"extensions\": [\"/a\"".to_vec(), not_json),
        (
            "text after the object",
            b"{\"extensions\": []} {\"a\": 1}".to_vec(),
            not_json,
        ),
    ]
}

#[test]
fn a_file_pi_would_not_read_is_left_byte_for_byte_and_the_answer_says_why() {
    for (why, bytes, words) in unreadable_files() {
        let world = World::with_pi();
        world.put(&bytes);
        let before = world.age();
        let named = world.settings().to_str().unwrap().to_owned();

        let consent = world.answer(ConsentAnswer::Yes);

        assert_eq!(world.bytes(), bytes, "{why}: left byte for byte");
        assert_eq!(world.modified(), before, "{why}: not rewritten");
        let problem = consent
            .problem
            .unwrap_or_else(|| panic!("{why}: a problem is told"));
        assert!(
            problem.contains(&named),
            "{why}: the problem names the file: {problem}"
        );
        assert!(
            problem.contains(words),
            "{why}: the problem says what is wrong: {problem}"
        );
        assert_eq!(
            consent.answer,
            ConsentAnswer::NotAsked,
            "{why}: a yes that was not written is not kept"
        );
        assert_eq!(world.kept(), None, "{why}");
        world.only_the_settings_are_in_pi_folder();
    }
}

#[test]
fn the_question_shows_the_problem_before_any_answer() {
    let world = World::with_pi();
    world.put(b"{ not json");
    let before = world.bytes();

    let consent = world.consent();

    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert!(
        consent
            .problem
            .unwrap()
            .contains(world.settings().to_str().unwrap())
    );
    assert_eq!(world.bytes(), before);
}

#[test]
fn a_no_and_ask_again_are_kept_where_the_file_cannot_be_read_and_it_is_left_alone() {
    let world = World::with_pi();
    world.put(b"{\"extensions\": [1]}");
    let before = world.age();
    let named = world.settings().to_str().unwrap().to_owned();

    let no = world.answer(ConsentAnswer::No);

    assert_eq!(no.answer, ConsentAnswer::No, "the user's no stands");
    assert!(
        no.problem.is_some_and(|problem| problem.contains(&named)),
        "and the app says it could not look in the file"
    );
    assert_eq!(world.kept().as_deref(), Some("no\n"));
    assert_eq!(world.bytes(), b"{\"extensions\": [1]}");
    assert_eq!(world.modified(), before);
    // A restart: the file cannot be read and nothing is written after a no, so a kept no says
    // nothing of it.
    assert_eq!(world.consent().answer, ConsentAnswer::No);
    assert_eq!(world.consent().problem, None);

    let again = world.answer(ConsentAnswer::NotAsked);

    assert_eq!(
        again.answer,
        ConsentAnswer::NotAsked,
        "the question comes back"
    );
    assert!(
        again
            .problem
            .is_some_and(|problem| problem.contains(&named)),
        "and the app says it could not look in the file"
    );
    assert_eq!(world.kept(), None, "the kept no is forgotten");
    assert_eq!(world.bytes(), b"{\"extensions\": [1]}");
    assert_eq!(world.modified(), before);
    world.only_the_settings_are_in_pi_folder();
}

fn assert_nothing_is_asked_or_written(prepare: &dyn Fn(&World)) {
    for answer in [
        ConsentAnswer::Yes,
        ConsentAnswer::No,
        ConsentAnswer::NotAsked,
    ] {
        let world = World::without_pi();
        prepare(&world);
        let before = world.tree();

        assert_eq!(world.pi().consent(&world.folder), None, "{answer:?}");
        assert_eq!(
            world.tree(),
            before,
            "{answer:?}: a question writes nothing"
        );
        assert_eq!(
            world.pi().answer_consent(&world.folder, answer),
            None,
            "{answer:?}"
        );
        assert_eq!(
            world.tree(),
            before,
            "{answer:?}: the home and the folder hold what they held"
        );
        assert_eq!(world.kept(), None, "{answer:?}: no answer is kept");
    }
}

#[test]
fn with_pi_not_installed_nothing_is_asked_and_nothing_is_written() {
    assert_nothing_is_asked_or_written(&|_| {});
}

#[test]
fn a_dot_pi_with_no_agent_folder_is_pi_not_installed_and_so_is_a_file_in_its_place() {
    assert_nothing_is_asked_or_written(&|world| {
        fs::create_dir_all(world.home.join(".pi")).unwrap();
    });
    assert_nothing_is_asked_or_written(&|world| {
        fs::create_dir_all(world.home.join(".pi")).unwrap();
        fs::write(world.agent_folder(), b"not a folder").unwrap();
    });
}

#[test]
fn an_adapter_that_knows_no_home_asks_nothing_and_writes_nothing() {
    let world = World::with_pi();
    let before = world.tree();
    let pi = Pi::new();

    assert_eq!(pi.consent(&world.folder), None);
    assert_eq!(pi.answer_consent(&world.folder, ConsentAnswer::Yes), None);

    assert_eq!(world.tree(), before);
}

#[test]
fn a_settings_file_that_cannot_be_read_as_a_file_is_a_problem_and_stays() {
    let world = World::with_pi();
    fs::create_dir(world.settings()).unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert!(
        consent
            .problem
            .unwrap()
            .contains(world.settings().to_str().unwrap())
    );
    assert!(world.settings().is_dir());
    assert_eq!(world.kept(), None);
}

#[test]
fn a_yes_a_no_and_a_question_not_yet_answered_are_each_kept_across_a_restart() {
    let cases = [
        (Some(ConsentAnswer::Yes), ConsentAnswer::Yes),
        (Some(ConsentAnswer::No), ConsentAnswer::No),
        (Some(ConsentAnswer::NotAsked), ConsentAnswer::NotAsked),
        (None, ConsentAnswer::NotAsked),
    ];
    for (given, kept) in cases {
        let world = World::with_pi();
        world.put(b"{\"theme\": \"dark\"}\n");
        if let Some(given) = given {
            world.answer(given);
        }

        let again = world.consent();

        assert_eq!(again.answer, kept, "{given:?}");
        assert_eq!(again.problem, None, "{given:?}");
    }
}

#[test]
fn a_kept_yes_whose_entry_the_user_took_out_is_asked_again_and_nothing_is_written_back() {
    let world = World::with_pi();
    let original = "{\n  \"theme\": \"dark\",\n  \"extensions\": [\"/mine\"]\n}\n";
    world.put(original.as_bytes());
    world.answer(ConsentAnswer::Yes);
    assert_eq!(world.consent().answer, ConsentAnswer::Yes);
    world.put(original.as_bytes());
    let before = world.age();
    let tree = world.tree();

    let first = world.consent();
    let second = world.pi().consent(&world.folder).unwrap();

    assert_eq!(
        first.answer,
        ConsentAnswer::NotAsked,
        "the question comes back"
    );
    assert_eq!(second, first);
    assert_eq!(first.problem, None);
    assert_eq!(
        world.bytes(),
        original.as_bytes(),
        "nothing is written back"
    );
    assert_eq!(world.modified(), before);
    assert_eq!(
        world.tree(),
        tree,
        "nothing is made, the kept yes is not touched"
    );
    assert_eq!(world.kept().as_deref(), Some("yes\n"));
}

#[test]
fn a_kept_yes_whose_file_the_user_removed_is_asked_again() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);
    fs::remove_file(world.settings()).unwrap();

    let consent = world.consent();

    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert_eq!(consent.problem, None);
    assert!(!world.settings().exists(), "no file is written back");
}

#[test]
fn a_kept_yes_stands_again_when_the_user_puts_the_entry_back() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);
    let with_entry = world.bytes();
    world.put(b"{}");
    assert_eq!(world.consent().answer, ConsentAnswer::NotAsked);

    world.put(&with_entry);

    assert_eq!(world.consent().answer, ConsentAnswer::Yes);
}

#[test]
fn a_kept_yes_with_settings_that_cannot_be_read_is_asked_again_with_the_problem() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);
    world.put(b"{ broken");

    let consent = world.consent();

    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert!(
        consent
            .problem
            .unwrap()
            .contains(world.settings().to_str().unwrap())
    );
    assert_eq!(world.bytes(), b"{ broken");
}

#[test]
fn a_kept_answer_that_is_neither_yes_nor_no_is_a_question_not_yet_answered() {
    let world = World::with_pi();
    fs::write(world.folder.join(CONSENT_FILE), b"maybe\n").unwrap();

    assert_eq!(world.consent().answer, ConsentAnswer::NotAsked);
}

#[test]
fn an_answer_is_kept_where_the_adapters_folder_does_not_exist_yet() {
    let world = World::with_pi();
    fs::remove_dir_all(&world.folder).unwrap();

    world.answer(ConsentAnswer::No);

    assert_eq!(world.kept().as_deref(), Some("no\n"));
    assert_eq!(world.consent().answer, ConsentAnswer::No);
}

#[test]
fn a_settings_file_that_is_a_symbolic_link_stays_a_link_and_its_target_holds_the_entry() {
    let world = World::with_pi();
    let dotfiles = world.root.join("dotfiles");
    fs::create_dir(&dotfiles).unwrap();
    let target = dotfiles.join("pi-settings.json");
    let original = "{\n  \"theme\": \"dark\",\n  \"extensions\": []\n}\n";
    fs::write(&target, original).unwrap();
    symlink(&target, world.settings()).unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert_eq!(consent.problem, None);
    assert!(fs::symlink_metadata(world.settings()).unwrap().is_symlink());
    assert_eq!(fs::read_link(world.settings()).unwrap(), target);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        format!(
            "{{\n  \"theme\": \"dark\",\n  \"extensions\": [{}]\n}}\n",
            world.quoted()
        )
    );
    assert_eq!(
        fs::read_dir(&dotfiles).unwrap().count(),
        1,
        "no temporary file lies beside the target"
    );

    world.answer(ConsentAnswer::No);

    assert!(fs::symlink_metadata(world.settings()).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    world.only_the_settings_are_in_pi_folder();
}

#[test]
fn a_link_to_a_relative_path_and_a_chain_of_links_are_followed() {
    let world = World::with_pi();
    let dotfiles = world.root.join("dotfiles");
    fs::create_dir(&dotfiles).unwrap();
    fs::write(dotfiles.join("real.json"), "{}").unwrap();
    symlink("real.json", dotfiles.join("middle.json")).unwrap();
    symlink(dotfiles.join("middle.json"), world.settings()).unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert_eq!(consent.problem, None);
    assert!(
        fs::symlink_metadata(dotfiles.join("middle.json"))
            .unwrap()
            .is_symlink()
    );
    assert!(fs::symlink_metadata(world.settings()).unwrap().is_symlink());
    assert_eq!(
        parse(&fs::read(dotfiles.join("real.json")).unwrap())[SETTINGS_KEY],
        json!([world.extension()])
    );
}

#[test]
fn a_link_to_nothing_is_a_settings_file_the_app_leaves_alone() {
    let world = World::with_pi();
    let target = world.root.join("gone.json");
    symlink(&target, world.settings()).unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert!(
        consent
            .problem
            .unwrap()
            .contains(world.settings().to_str().unwrap())
    );
    assert!(!target.exists(), "nothing is made where the link points");
    assert_eq!(fs::read_link(world.settings()).unwrap(), target);
    assert_eq!(world.kept(), None);
}

#[test]
fn the_permissions_of_the_file_are_kept() {
    for mode in [0o600, 0o640, 0o444, 0o755] {
        let world = World::with_pi();
        world.put(b"{\"extensions\": [\"/a\"]}");
        fs::set_permissions(world.settings(), fs::Permissions::from_mode(mode)).unwrap();

        world.answer(ConsentAnswer::Yes);
        let after_yes = fs::metadata(world.settings()).unwrap().permissions().mode() & 0o777;
        world.answer(ConsentAnswer::No);
        let after_no = fs::metadata(world.settings()).unwrap().permissions().mode() & 0o777;

        assert_eq!(after_yes, mode, "after a yes");
        assert_eq!(after_no, mode, "after a no");
        assert_eq!(world.bytes(), b"{\"extensions\": [\"/a\"]}");
        world.only_the_settings_are_in_pi_folder();
    }
}

#[test]
fn a_lock_pi_holds_makes_a_yes_change_nothing_and_say_so_and_stays_pis() {
    let world = World::with_pi();
    world.put(b"{\"a\": 1}\n");
    let before = world.age();
    let lock = world.agent_folder().join(format!("{SETTINGS_FILE}.lock"));
    fs::create_dir(&lock).unwrap();
    fs::write(lock.join("pi"), b"mine").unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert_eq!(world.bytes(), b"{\"a\": 1}\n");
    assert_eq!(world.modified(), before);
    assert!(
        consent
            .problem
            .as_deref()
            .unwrap()
            .contains("pi is changing its settings: try again"),
        "{consent:?}"
    );
    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert_eq!(world.kept(), None);
    assert_eq!(
        fs::read(lock.join("pi")).unwrap(),
        b"mine",
        "the lock is still pi's"
    );
    assert_eq!(
        world.agent_names(),
        BTreeSet::from([SETTINGS_FILE.to_owned(), format!("{SETTINGS_FILE}.lock")])
    );

    fs::remove_dir_all(&lock).unwrap();
    let consent = world.answer(ConsentAnswer::Yes);
    assert_eq!(consent.answer, ConsentAnswer::Yes);
    assert_eq!(consent.problem, None);
    world.only_the_settings_are_in_pi_folder();
}

#[test]
fn a_change_that_is_none_needs_no_lock_so_pi_holding_it_is_no_problem() {
    let world = World::with_pi();
    world.put(world.fill("{\"extensions\": [@]}").as_bytes());
    let lock = world.agent_folder().join(format!("{SETTINGS_FILE}.lock"));
    fs::create_dir(&lock).unwrap();

    let yes = world.answer(ConsentAnswer::Yes);
    world.put(b"{}");
    let no = world.answer(ConsentAnswer::No);

    assert_eq!(
        (yes.answer, yes.problem),
        (ConsentAnswer::Yes, None),
        "the entry is there already"
    );
    assert_eq!(
        (no.answer, no.problem),
        (ConsentAnswer::No, None),
        "there is no entry to take out"
    );
    assert!(lock.is_dir());
}

#[test]
fn a_no_that_finds_the_lock_held_is_kept_and_the_entry_stays_with_the_problem() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);
    let with_entry = world.bytes();
    let lock = world.agent_folder().join(format!("{SETTINGS_FILE}.lock"));
    fs::create_dir(&lock).unwrap();

    let consent = world.answer(ConsentAnswer::No);

    assert_eq!(world.bytes(), with_entry);
    assert_eq!(consent.answer, ConsentAnswer::No, "the user's no is kept");
    assert!(consent.problem.unwrap().contains("try again"));
    assert!(lock.is_dir());
}

#[test]
fn a_no_whose_entry_could_not_be_taken_out_is_told_again_after_a_restart_until_it_is_gone() {
    let world = World::with_pi();
    world.answer(ConsentAnswer::Yes);
    let with_entry = world.bytes();
    let lock = world.agent_folder().join(format!("{SETTINGS_FILE}.lock"));
    fs::create_dir(&lock).unwrap();
    world.answer(ConsentAnswer::No);
    fs::remove_dir(&lock).unwrap();
    let before = world.age();
    let tree = world.tree();

    let first = world.consent();
    let second = world.consent();

    assert_eq!(first.answer, ConsentAnswer::No, "the user's no stands");
    assert_eq!(second, first);
    let problem = first.problem.expect("the entry that stays is told");
    assert!(
        problem.contains(world.settings().to_str().unwrap()),
        "it names the settings: {problem}"
    );
    assert!(
        problem.contains("answer no again"),
        "and says what to do: {problem}"
    );
    assert_eq!(world.bytes(), with_entry, "asking writes nothing");
    assert_eq!(world.modified(), before);
    assert_eq!(world.tree(), tree);

    let again = world.answer(ConsentAnswer::No);
    assert_eq!((again.answer, again.problem), (ConsentAnswer::No, None));
    assert!(
        !String::from_utf8(world.bytes())
            .unwrap()
            .contains(&world.extension())
    );
    let restarted = world.consent();
    assert_eq!(
        (restarted.answer, restarted.problem),
        (ConsentAnswer::No, None)
    );
}

#[test]
fn an_answer_that_cannot_be_renamed_into_place_leaves_no_temporary_file() {
    // The temporary file is written whole and the rename then fails, because what stands
    // where the kept answer goes is a folder that is not empty.
    let world = World::with_pi();
    let in_the_way = world.folder.join(CONSENT_FILE);
    fs::create_dir(&in_the_way).unwrap();
    fs::write(in_the_way.join("keep"), b"x").unwrap();
    let before = world.tree();

    let consent = world.answer(ConsentAnswer::No);

    assert!(
        consent
            .problem
            .is_some_and(|problem| problem.contains(in_the_way.to_str().unwrap())),
        "the app says it could not keep the answer"
    );
    assert_eq!(world.tree(), before, "no temporary file is left behind");
}

#[test]
fn a_write_that_fails_half_way_leaves_the_file_as_it_was_and_the_lock_removed() {
    // The settings are a link to a file whose name leaves no room for a temporary file
    // beside it: reading the file works, making the temporary file fails, on any user.
    let world = World::with_pi();
    let dotfiles = world.root.join("dotfiles");
    fs::create_dir(&dotfiles).unwrap();
    let target = dotfiles.join("s".repeat(250));
    let original = "{\n  \"extensions\": [\"/a\"]\n}\n";
    fs::write(&target, original).unwrap();
    symlink(&target, world.settings()).unwrap();
    let before = fs::metadata(&target).unwrap().modified().unwrap();

    let consent = world.answer(ConsentAnswer::Yes);

    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), before);
    assert!(
        consent
            .problem
            .unwrap()
            .contains(world.settings().to_str().unwrap())
    );
    assert_eq!(consent.answer, ConsentAnswer::NotAsked);
    assert_eq!(world.kept(), None, "a yes whose write failed keeps nothing");
    assert_eq!(
        fs::read_dir(&dotfiles).unwrap().count(),
        1,
        "no temporary file"
    );
    assert_eq!(
        world.agent_names(),
        BTreeSet::from([SETTINGS_FILE.to_owned()]),
        "pi's lock is removed"
    );
}
