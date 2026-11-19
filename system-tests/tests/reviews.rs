//! System tests of the reviews of the Keeper through the scripted client: the built
//! Keeper, real repositories, and a Keeper that is killed.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use yaar_link::{
    Anchor, Author, CommentId, DraftOn, Fact, Lines, Reply, Request, ReviewId, Side, TerminalSize,
    ThreadId, WorkspaceId, WorkspaceReviews,
};
use yaar_system_tests::{Client, DEADLINE, Repo, World, is_alive, wait_until};

const ONE: &str = "pub fn number() -> u32 {\n    1\n}\n";
const TWO: &str = "pub fn number() -> u32 {\n    2\n}\n";
const NEW: &str = "pub const NEW: bool = true;\n";

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// The id of the `n`th thing of a kind: 32 digits, as the Window draws them.
fn id(kind: u128, n: u128) -> String {
    format!("{:032x}", (kind << 64) + n)
}

/// A repository named `name` on a branch `feature` whose one commit changes `src/lib.rs`
/// and adds `src/new.rs`, with the commit of `main` it left. Gives the repository and that
/// commit, which the changes of its workspace are counted from.
fn a_feature_branch(world: &World, name: &str) -> (Repo, String) {
    let repo = world.repository(name);
    repo.write("src/lib.rs", ONE);
    let fork = repo.commit("Add the number");
    repo.git(&["switch", "--quiet", "-c", "feature"]);
    repo.write("src/lib.rs", TWO);
    repo.write("src/new.rs", NEW);
    repo.commit("Make the number two");
    (repo, fork)
}

/// Opens the folder of `repo` and gives its workspace.
fn open(client: &mut Client, repo: &Repo) -> WorkspaceId {
    client.open_folder(repo.path()).expect("open the folder").1
}

/// The one review of a workspace that has no review of an agent session: of no agent, with
/// no thread yet.
fn the_review_of_no_agent(client: &mut Client, workspace: &WorkspaceId) -> ReviewId {
    let reviews = client.reviews(workspace);
    assert_eq!(reviews.workspace, *workspace);
    let [review] = &reviews.reviews[..] else {
        panic!("not exactly one review: {:?}", reviews.reviews);
    };
    assert_eq!(review.agent, None);
    assert_eq!(review.threads, vec![]);
    assert_eq!(review.rounds, vec![]);
    review.id.clone()
}

/// A comment being typed: its thread, its id, and what it belongs to the first time.
struct Typed {
    review: ReviewId,
    thread: ThreadId,
    comment: CommentId,
    /// The anchor of the thread, for the first save only; none for a reply.
    anchor: Option<Anchor>,
}

impl Typed {
    /// Saves `text` as the comment now stands, as the Window does at every change.
    fn save(&mut self, client: &mut Client, text: &str) {
        client
            .save_draft(&self.review, &self.thread, &self.comment, self.on(), text)
            .unwrap_or_else(|refused| panic!("a save of {text:?} was refused: {refused}"));
        self.anchor = None;
    }

    /// What a save of this comment says it belongs to: the anchor for the first one.
    fn on(&self) -> DraftOn {
        match &self.anchor {
            Some(anchor) => DraftOn::NewThread {
                anchor: anchor.clone(),
            },
            None => DraftOn::Thread,
        }
    }

    /// Types `text` a character at a time, a save for each.
    fn type_out(&mut self, client: &mut Client, text: &str) {
        for end in text.char_indices().map(|(at, c)| at + c.len_utf8()) {
            self.save(client, &text[..end]);
        }
    }
}

fn anchor(tree: Option<&str>, path: &str, side: Side, line: u32, text: &str) -> Anchor {
    Anchor {
        tree: tree.map(str::to_owned),
        path: PathBuf::from(path),
        side,
        lines: Lines {
            first: line,
            last: line,
        },
        text: text.to_owned(),
    }
}

/// The text of the one comment of thread `thread` of `reviews`' first review.
fn draft_text(reviews: &WorkspaceReviews, thread: usize) -> String {
    let comments = &reviews.reviews[0].threads[thread].comments;
    assert_eq!(comments.len(), 1, "{comments:?}");
    comments[0].text.clone()
}

// CLIENT-02.
#[test]
fn a_keeper_killed_while_a_draft_is_typed_comes_back_with_every_review_as_it_was_left() {
    let mut world = World::new();
    let first_keeper = world.start_keeper();
    let (repo_a, fork_a) = a_feature_branch(&world, "alpha");
    let (repo_b, fork_b) = a_feature_branch(&world, "beta");
    let mut client = world.client();
    let a = open(&mut client, &repo_a);
    let b = open(&mut client, &repo_b);
    assert_ne!(a, b);

    // Each workspace has one review, of no agent.
    let review_a = the_review_of_no_agent(&mut client, &a);
    let review_b = the_review_of_no_agent(&mut client, &b);
    assert_ne!(review_a, review_b);

    // A thread on a removed line of a file of A, in the version the changes are counted
    // from, typed and then copied: its comment is sent, in round 1. A reply in that
    // thread, typed. A second thread on an added line, in the files on disk, typed.
    let mut first = Typed {
        review: review_a.clone(),
        thread: ThreadId::new(id(1, 1)),
        comment: CommentId::new(id(2, 1)),
        anchor: Some(anchor(Some(&fork_a), "src/lib.rs", Side::Old, 2, "    1")),
    };
    first.type_out(&mut client, "first");
    let (copied, round) = client.copy_review(&review_a).expect("copy the review");
    assert_eq!(round, 1);
    assert!(copied.contains("first"), "{copied}");
    let mut reply = Typed {
        review: review_a.clone(),
        thread: ThreadId::new(id(1, 1)),
        comment: CommentId::new(id(2, 2)),
        anchor: None,
    };
    reply.type_out(&mut client, "reply");
    let mut second = Typed {
        review: review_a.clone(),
        thread: ThreadId::new(id(1, 2)),
        comment: CommentId::new(id(2, 3)),
        anchor: Some(anchor(None, "src/new.rs", Side::New, 1, NEW.trim_end())),
    };
    second.type_out(&mut client, "second");
    let mut other = Typed {
        review: review_b.clone(),
        thread: ThreadId::new(id(1, 3)),
        comment: CommentId::new(id(2, 4)),
        anchor: Some(anchor(Some(&fork_b), "src/lib.rs", Side::Old, 3, "}")),
    };
    other.type_out(&mut client, "other");

    // The reviewer marks the change of each review reviewed and a file of A viewed, so
    // that the reviewed states and the viewed marks the restart must keep are real ones.
    client.mark_reviewed(&review_a).expect("mark A reviewed");
    client.mark_reviewed(&review_b).expect("mark B reviewed");
    client
        .set_viewed(&a, Path::new("src/lib.rs"), true)
        .expect("mark a file of A viewed");

    // What the Keeper holds before the kill. The first thread's comment is sent, the
    // reply and the second thread's comment are drafts.
    let before_a = client.reviews(&a);
    let before_b = client.reviews(&b);
    let review = &before_a.reviews[0];
    assert!(review.reviewed.is_some(), "A's review is marked reviewed");
    assert!(before_b.reviews[0].reviewed.is_some());
    assert_eq!(before_a.viewed, vec![PathBuf::from("src/lib.rs")]);
    assert_eq!(review.threads.len(), 2);
    assert_eq!(review.rounds.len(), 1);
    assert_eq!(review.threads[0].comments.len(), 2);
    assert_eq!(review.threads[0].comments[0].round, Some(1));
    assert_eq!(review.threads[0].comments[0].author, Author::Reviewer);
    assert_eq!(review.threads[0].comments[1].text, "reply");
    assert_eq!(review.threads[0].comments[1].round, None);
    assert_eq!(draft_text(&before_a, 1), "second");

    // Killed while a draft is typed: thirty more saves of the second thread's comment,
    // the first fifteen waited for, the other fifteen on their way when the Keeper is
    // killed, with no reply awaited.
    let saves: Vec<String> = (1..=30u8)
        .map(|n| format!("second {}", letters(n)))
        .collect();
    let mut sent = Vec::new();
    for text in &saves[..15] {
        sent.push(client.send(Request::SaveDraft {
            review: second.review.clone(),
            thread: second.thread.clone(),
            comment: second.comment.clone(),
            on: DraftOn::Thread,
            text: text.clone(),
        }));
    }
    let last_kept = sent[14];
    assert_eq!(
        client.wait_for_reply(last_kept, DEADLINE),
        Ok(Reply::Done),
        "the save of {:?}",
        saves[14]
    );
    for text in &saves[15..] {
        client.send(Request::SaveDraft {
            review: second.review.clone(),
            thread: second.thread.clone(),
            comment: second.comment.clone(),
            on: DraftOn::Thread,
            text: text.clone(),
        });
    }
    world.kill_keeper();
    assert!(!is_alive(first_keeper), "the killed Keeper is gone");
    drop(client);

    // And started again.
    let second_keeper = world.start_keeper();
    assert_ne!(second_keeper, first_keeper);
    let mut client = world.client();

    // Every review, the draft included, is as it was left: the same ids and the same
    // threads, comments, rounds and reviewed state. The one thing that may differ is the
    // text of the draft that was being typed: that of the last save the Keeper answered,
    // or of one sent after it, which a killed Keeper may or may not have kept.
    let mut after_a = client.reviews(&a);
    let kept = draft_text(&after_a, 1);
    assert!(
        saves[14..].contains(&kept),
        "the draft reads {kept:?}, which no save from the last answered one on holds"
    );
    after_a.reviews[0].threads[1].comments[0].text = draft_text(&before_a, 1);
    assert_eq!(after_a, before_a);
    assert_eq!(client.reviews(&b), before_b);
    assert_eq!(draft_text(&client.reviews(&b), 0), "other");

    // Killed again right after a save's reply, with nothing asked in between: a save whose
    // reply has come is kept whole, so the draft after this restart is exactly that save's
    // text. This shows a Keeper that answers a save before it keeps it, however fast it
    // reads what follows a kill, and one that keeps it only when the next request comes.
    let last = "second, the last one";
    second.save(&mut client, last);
    world.kill_keeper();
    assert!(!is_alive(second_keeper), "the killed Keeper is gone");
    drop(client);
    let third_keeper = world.start_keeper();
    let mut client = world.client();
    let mut kept_a = client.reviews(&a);
    assert_eq!(draft_text(&kept_a, 1), last);
    kept_a.reviews[0].threads[1].comments[0].text = draft_text(&before_a, 1);
    assert_eq!(kept_a, before_a);
    assert_eq!(client.reviews(&b), before_b);

    // The Keeper of the world is the third one, and the others are gone; when the world
    // goes the third goes too.
    drop(client);
    drop(world);
    assert!(wait_until(DEADLINE, || !is_alive(third_keeper)));
}

/// `n` letters, starting from `x` and going round the alphabet.
fn letters(n: u8) -> String {
    (0..n)
        .map(|i| char::from(b'a' + (i + 23) % 26))
        .collect::<String>()
}

/// A named pipe the hook of a repository waits on, and the one write that lets it go.
///
/// Held open for reading and writing by the test until it is dropped, so that the line
/// written stays in the pipe for the hook, whenever it comes to read (a pipe forgets its
/// data once no one holds it), and so that the write never waits for a reader. On every
/// path the line is written once, when the test lets the hook go or when this is dropped,
/// so that the hook never outlives the test.
struct Gate {
    fifo: File,
    released: bool,
    path: PathBuf,
}

impl Gate {
    fn new(path: &Path) -> Gate {
        let made = Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo {} failed", path.display());
        let fifo = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open the pipe");
        Gate {
            fifo,
            released: false,
            path: path.to_owned(),
        }
    }

    /// Lets the hook go.
    fn open(&mut self) {
        if !self.released {
            self.released = true;
            self.fifo.write_all(b"go\n").expect("write to the pipe");
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.open();
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn a_commit_whose_hook_waits_holds_up_neither_a_draft_save_nor_the_typing_into_a_terminal() {
    let mut world = World::new();
    world.start_keeper();
    let repo = world.repository("app");
    repo.git(&["config", "user.name", "A Test"]);
    repo.git(&["config", "user.email", "test@example.invalid"]);
    repo.write("src/new.rs", NEW);

    // The hook says it began and then waits for a line of the pipe.
    let fifo = world.home().join("hook.fifo");
    let begun = world.home().join("hook.begun");
    let mut gate = Gate::new(&fifo);
    let hook = repo.path().join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ntouch '{}'\nread line < '{}'\n",
            begun.display(),
            fifo.display()
        ),
    )
    .expect("write the hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("make the hook run");

    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let review = the_review_of_no_agent(&mut client, &workspace);
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");

    let commit = client.send(Request::Commit {
        workspace: workspace.clone(),
        message: "Add the constant".to_owned(),
    });
    assert!(
        wait_until(DEADLINE, || begun.exists()),
        "the hook did not begin"
    );

    // With the hook waiting: a new thread and two changes of its text are each answered
    // within five seconds.
    let mut typed = Typed {
        review,
        thread: ThreadId::new(id(1, 1)),
        comment: CommentId::new(id(2, 1)),
        anchor: Some(anchor(None, "src/new.rs", Side::New, 1, NEW.trim_end())),
    };
    for text in ["g", "go", "good"] {
        let asked = Instant::now();
        typed.save(&mut client, text);
        assert!(
            asked.elapsed() < Duration::from_secs(5),
            "the save of {text:?} waited for the hook"
        );
    }

    // And what is typed into a terminal opened before the commit reaches its program.
    client.type_into(&terminal, b"echo yaar-$((40 + 2))\r");
    client.wait_for_output(&terminal, DEADLINE, |bytes| {
        String::from_utf8_lossy(bytes).contains("yaar-42")
    });

    // The hook is let go and the commit is made.
    gate.open();
    let Ok(Reply::Committed { commit: made }) = client.wait_for_reply(commit, DEADLINE) else {
        panic!("the commit was not made");
    };
    assert_eq!(made, repo.git(&["rev-parse", "HEAD"]));
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
    client.wait_for_fact(DEADLINE, |fact| {
        *fact
            == Fact::ChangesChanged {
                workspace: workspace.clone(),
            }
    });
}
