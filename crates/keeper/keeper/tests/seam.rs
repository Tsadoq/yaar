//! Component test: a delivery written in a test gets the same text as the built ones. The send runs
//! on the Keeper's data folder while the Keeper is stopped. Each test sets the process environment
//! first, so nextest runs it in a process of its own.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_agent::{Asked, Asks, CliSession, CopyAsText, Delivery, Rendered};
use yaar_keeper::{Exit, Options, adapters, run};
use yaar_link::{
    Anchor, Author, CommentId, DraftOn, Hello, Lines, Message, Refusal, Reply, Request, RequestId,
    Review, SentBy, Side, ThreadId, WorkspaceId, WorkspaceReviews, handshake, read_message,
    write_message,
};
use yaar_recorder::Git;
use yaar_review_engine::{Attachment, EngineError, ReviewId, Reviews, SentBy as EngineSentBy};
use yaar_store::Store;

const DEADLINE: Duration = Duration::from_secs(20);

struct Scratch(PathBuf);

impl Scratch {
    /// Sets the process environment for git. Must be the first statement of a test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-seam-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let settings = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "A Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
            ("GIT_COMMITTER_NAME", "A Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
        ];
        // SAFETY: this runs as the first statement of a test, and nextest gives each test
        // a process of its own, so no other thread exists yet.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn keeper(&self) -> JoinHandle<io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: None,
            home: self.home(),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && handshake(&mut stream, &mine).is_ok()
            {
                stream
                    .set_read_timeout(Some(DEADLINE))
                    .expect("set the deadline of a read");
                return Window { stream, next_id: 0 };
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(folder.join("a.txt"), "one\ntwo\nthree\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(folder: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        loop {
            match read_message(&mut self.stream).expect("a frame from the Keeper") {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Fact(_) => {}
                other => panic!("not the reply to {id:?}: {other:?}"),
            }
        }
    }

    fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        }
    }

    fn draft(&mut self, review: &Review, n: u128, line: u32, text: &str) {
        let saved = self.ask(Request::SaveDraft {
            review: review.id.clone(),
            thread: ThreadId::new(format!("{n:032x}")),
            comment: CommentId::new(format!("{:032x}", n + 1)),
            on: DraftOn::NewThread {
                anchor: Anchor {
                    tree: None,
                    path: PathBuf::from("a.txt"),
                    side: Side::New,
                    lines: Lines {
                        first: line,
                        last: line,
                    },
                    text: ["one", "two", "three"][line as usize - 1].to_owned(),
                },
            },
            text: text.to_owned(),
        });
        assert_eq!(saved, Ok(Reply::Done));
    }

    fn stop(mut self, keeper: JoinHandle<io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Handed {
    text: String,
    review: String,
    round: u32,
    threads: usize,
}

struct Recorder<'a> {
    way: &'a mut dyn Delivery,
    fails: bool,
    handed: Option<Handed>,
}

impl Delivery for Recorder<'_> {
    fn instruction(&self) -> &'static str {
        self.way.instruction()
    }

    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        self.handed = Some(Handed {
            text: rendered.text.to_owned(),
            review: rendered.review.to_owned(),
            round: rendered.round,
            threads: rendered.threads,
        });
        if self.fails {
            return Err(io::Error::other("the recorder fails the hand-out"));
        }
        self.way.hand_out(rendered)
    }
}

/// Exists only in this test; its hand-out returns the length of what it was handed, which no
/// built way does.
struct ByTheTest;

const OWN_INSTRUCTION: &str = "This is a review handed out by a way of delivery of the test. \
Answer each thread, naming it by its id.";

impl Delivery for ByTheTest {
    fn instruction(&self) -> &'static str {
        OWN_INSTRUCTION
    }

    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        Ok(format!("sent {} characters", rendered.text.chars().count()).into_bytes())
    }
}

#[test]
fn a_way_of_delivery_written_in_the_test_is_handed_the_same_text_as_the_three_built_ones() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let Ok(Reply::Opened { workspace, .. }) = window.ask(Request::OpenFolder { path: repo }) else {
        panic!("the folder was not opened");
    };
    let review = window
        .reviews(&workspace)
        .reviews
        .into_iter()
        .find(|review| review.agent.is_none())
        .expect("the workspace has the review of no agent");
    window.draft(&review, 1, 1, "Why is this `one`?");
    window.draft(
        &review,
        3,
        2,
        "And this `two`, with a \"quote\" and a line\nbreak?",
    );
    let before = window.reviews(&workspace);
    let [kept] = &before.reviews[..] else {
        panic!("not one review: {:?}", before.reviews);
    };
    assert_eq!(kept.threads.len(), 2, "{:?}", kept.threads);
    assert!(
        kept.threads
            .iter()
            .all(|thread| matches!(&thread.comments[..], [comment] if comment.round.is_none())),
        "each thread holds one draft: {:?}",
        kept.threads
    );
    assert_eq!(kept.rounds, []);
    window.stop(keeper);

    let store = Store::open(&scratch.data().join(yaar_store::FILE_NAME)).expect("the Store");
    let reviews = Reviews::new(
        store,
        Git::find().expect("git"),
        scratch.data().join("states"),
    );
    let id = ReviewId::new(review.id.as_str());
    reviews
        .attach(&id, Attachment::Review, &[])
        .expect("attach the review");
    let attached = reviews.review(&id).expect("read the review");
    assert!(attached.attached().is_some());

    let adapters = adapters(&scratch.home());
    let (claude_folder, pi_folder) = (
        scratch.data().join("agents").join("claude"),
        scratch.data().join("agents").join("pi"),
    );
    let asked = Asked {
        session: CliSession("cli-1".to_owned()),
        terminal: None,
        folder: None,
        asks: Asks::Message,
    };
    let mut copy = CopyAsText;
    let named = |name: &str| {
        adapters
            .iter()
            .find(|adapter| adapter.name() == name)
            .unwrap_or_else(|| panic!("the Keeper holds no Adapter named {name}"))
    };
    let mut claude = named(yaar_adapter_claude::NAME).delivery(&claude_folder, &asked);
    let mut pi = named(yaar_adapter_pi::NAME).delivery(&pi_folder, &asked);
    let mut own = ByTheTest;
    let mut ways: Vec<(&str, &mut dyn Delivery)> = vec![
        ("copy as text", &mut copy),
        ("claude", &mut *claude),
        ("pi", &mut *pi),
        ("the test's own", &mut own),
    ];

    let mut handed = Vec::new();
    for (name, way) in &mut ways {
        let instruction = way.instruction();
        let succeeds = *name == "the test's own";
        let mut recorder = Recorder {
            way: &mut **way,
            fails: !succeeds,
            handed: None,
        };
        let sent = reviews.send_attached(&id, EngineSentBy::Delivered, &mut recorder);
        let got = recorder
            .handed
            .take()
            .unwrap_or_else(|| panic!("{name} was handed nothing"));
        if succeeds {
            let (bytes, sent) = sent
                .expect("the test's own way delivers")
                .expect("something");
            assert_eq!(
                bytes,
                format!("sent {} characters", got.text.chars().count()).into_bytes()
            );
            assert_eq!(sent.round.number, 1);
        } else {
            assert!(
                matches!(sent, Err(EngineError::HandOut(_))),
                "{name}: {sent:?}"
            );
            assert_eq!(
                reviews.review(&id).expect("read it back"),
                attached,
                "{name}"
            );
        }
        handed.push((*name, instruction, got));
    }

    // Four distinct instructions: taking each off the front of its own text must leave one
    // body, the same for all four.
    let instructions: Vec<&str> = handed
        .iter()
        .map(|(_, instruction, _)| *instruction)
        .collect();
    for (at, instruction) in instructions.iter().enumerate() {
        assert!(!instruction.is_empty());
        assert!(
            !instructions[..at].contains(instruction),
            "two ways share an instruction: {instructions:?}"
        );
    }
    assert_eq!(instructions[3], OWN_INSTRUCTION);
    let bodies: Vec<&str> = handed
        .iter()
        .map(|(name, instruction, got)| {
            got.text
                .strip_prefix(instruction)
                .unwrap_or_else(|| panic!("{name}'s text does not start with its instruction"))
        })
        .collect();
    assert!(bodies[0].contains("Why is this `one`?"), "{}", bodies[0]);
    assert!(bodies[0].contains("a line\nbreak?"), "{}", bodies[0]);
    assert!(
        bodies.iter().all(|body| *body == bodies[0]),
        "the four texts differ after their instructions: {bodies:#?}"
    );
    let [first, rest @ ..] = &handed[..] else {
        unreachable!("four ways were tried")
    };
    for (name, _, got) in rest {
        assert_eq!(got.review, first.2.review, "{name}");
        assert_eq!(got.round, first.2.round, "{name}");
        assert_eq!(got.threads, first.2.threads, "{name}");
    }
    assert_eq!(
        (first.2.review.as_str(), first.2.round, first.2.threads),
        (review.id.as_str(), 1, 2)
    );
    drop(ways);
    drop(reviews);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let after = window.reviews(&workspace);
    let [sent] = &after.reviews[..] else {
        panic!("not one review: {:?}", after.reviews);
    };
    let [round] = &sent.rounds[..] else {
        panic!("not one round: {:?}", sent.rounds);
    };
    assert_eq!((round.number, round.sent_by), (1, SentBy::Delivered));
    assert_eq!(sent.attached, None);
    assert_eq!(sent.threads.len(), 2);
    for thread in &sent.threads {
        let [comment] = &thread.comments[..] else {
            panic!("not one comment: {:?}", thread.comments);
        };
        assert_eq!(
            (comment.round, comment.author),
            (Some(1), Author::Reviewer),
            "{comment:?}"
        );
    }
    window.stop(keeper);
}
