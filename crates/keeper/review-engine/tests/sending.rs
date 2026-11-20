//! The one send of a review, through the engine with the real Store and the real Recorder
//! beneath, on a repository the test makes.

mod support;

use std::io;
use std::path::Path;

use support::{
    Scratch, commit_all, engine, files_of, git, id, init, new_thread, open, session, store,
    workspace, write,
};
use yaar_agent::{CopyAsText, Delivery, Rendered};
use yaar_recorder::Worktree;
use yaar_review_engine::{
    Attachment, Author, Comment, CommentId, DraftOn, DraftSave, EngineError, NewAnchor,
    ReviewError, ReviewId, Reviews, Scope, SentBy, SessionNow, Side, ThreadId,
};
use yaar_store::{AttachedRecord, Store};

/// What a delivery was handed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Handed {
    text: String,
    review: String,
    round: u32,
    threads: usize,
}

/// A delivery module written in the test: it has an instruction of its own, keeps what it
/// is handed, and gives the text back, or fails.
struct Keeper {
    instruction: &'static str,
    handed: Vec<Handed>,
    fails: bool,
}

impl Keeper {
    fn new(instruction: &'static str) -> Keeper {
        Keeper {
            instruction,
            handed: Vec::new(),
            fails: false,
        }
    }
}

impl Delivery for Keeper {
    fn instruction(&self) -> &'static str {
        self.instruction
    }

    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        if self.fails {
            return Err(io::Error::other("the hand-out failed"));
        }
        self.handed.push(Handed {
            text: rendered.text.to_owned(),
            review: rendered.review.to_owned(),
            round: rendered.round,
            threads: rendered.threads,
        });
        Ok(rendered.text.as_bytes().to_vec())
    }
}

const INSTRUCTION: &str = "Answer each thread with the reply tool.";

// A repository with one commit, a worktree changed on disk, and the engine over a Store
// and the Recorder: the review of the workspace with a draft on thread 1, attached.
struct Round {
    scratch: Scratch,
    repo: std::path::PathBuf,
    worktree: Worktree,
    store: Store,
    reviews: Reviews,
    review: ReviewId,
}

fn attached_review_in_a_repository() -> Round {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "two\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "one changed\n");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let review = reviews
        .of_workspace(&workspace(), Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "one changed", "why changed"),
        )
        .expect("a draft");
    reviews
        .attach(&review, Attachment::Review, &[])
        .expect("attach");
    Round {
        scratch,
        repo,
        worktree,
        store,
        reviews,
        review,
    }
}

// What git says of a repository that a send must not change: its HEAD, its branch, its
// index as bytes and the status of its worktree. The index is read before the status is
// asked for, since a status may refresh it.
fn repository_as_git_has_it(repo: &Path) -> (String, String, Vec<u8>, String) {
    let head = git(repo, &["rev-parse", "HEAD"]);
    let branch = git(repo, &["symbolic-ref", "HEAD"]);
    let index = std::fs::read(repo.join(".git/index")).expect("read the index");
    let status = git(repo, &["status", "--porcelain"]);
    (head, branch, index, status)
}

fn round_of(reviews: &Reviews, review: &ReviewId, comment: u128) -> Option<u32> {
    reviews
        .review(review)
        .expect("read")
        .comment(&CommentId::new(id(comment)))
        .expect("the comment is kept")
        .1
        .round
}

// REV-08
#[test]
fn when_the_message_that_carries_a_review_is_submitted_its_comments_are_sent_a_round_starts_and_the_state_of_the_worktree_is_recorded()
 {
    let Round {
        scratch,
        repo,
        worktree,
        store,
        reviews,
        review,
    } = attached_review_in_a_repository();
    let w = workspace();
    // Read once to leave the index as git has it before the send.
    git(&repo, &["status", "--porcelain"]);
    let before = repository_as_git_has_it(&repo);
    // The worktree moves on after the draft was written.
    write(&repo, "a.txt", "one changed again\n");
    write(&repo, "c.txt", "a new file\n");
    let mut keeper = Keeper::new(INSTRUCTION);

    // When the message that carries a review is submitted.
    let (bytes, sent) = reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send")
        .expect("something was sent");
    let text = String::from_utf8(bytes).expect("the keeper gave the text back");
    assert!(text.starts_with(INSTRUCTION), "{text}");
    assert!(text.contains("why changed"), "{text}");
    assert!(text.contains(&format!("Thread {}", id(1))), "{text}");
    assert_eq!(
        keeper.handed,
        vec![Handed {
            text: text.clone(),
            review: review.as_str().to_owned(),
            round: 1,
            threads: 1,
        }]
    );

    // The attached comments are sent, nothing is attached: read by a second engine on the
    // same file, as after a restart.
    drop(reviews);
    drop(store);
    let store = self::store(&scratch);
    let reviews = engine(&scratch, store.clone());
    assert_eq!(sent.comments, vec![CommentId::new(id(1001))]);
    assert_eq!(round_of(&reviews, &review, 1001), Some(1));
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.attached(), None);
    assert_eq!(
        store.review(&review).expect("read").expect("kept").attached,
        None
    );

    // A round starts: the first, delivered, with no state yet.
    assert_eq!(kept.rounds().len(), 1);
    let round = &kept.rounds()[0];
    assert_eq!(
        (
            round.number,
            round.sent_by,
            round.state.as_deref(),
            round.edited
        ),
        (1, SentBy::Delivered, None, false)
    );
    assert_eq!(sent.round, *round);

    // And the state of the worktree is recorded, after, off the agent's way.
    let states = reviews.states_of(&w).expect("the state store");
    let known = states.states().expect("list the states").len();
    let recorded = reviews
        .record_round(&worktree, &review, 1)
        .expect("record the state");
    let listed = states.states().expect("list the states");
    assert_eq!(listed.len(), known + 1);
    assert_eq!(listed.last().expect("a state").id, recorded);
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.rounds()[0].state.as_deref(), Some(recorded.as_str()));
    let on_disk = files_of(&repo);
    assert_eq!(
        states.paths(recorded.as_str()).expect("the files"),
        on_disk.keys().cloned().collect::<Vec<_>>()
    );
    for (path, content) in &on_disk {
        assert_eq!(
            states.content(recorded.as_str(), path).expect("read"),
            Some(content.clone()),
            "{}",
            path.display()
        );
    }
    assert_eq!(
        states
            .content(recorded.as_str(), Path::new("a.txt"))
            .expect("read"),
        Some(b"one changed again\n".to_vec())
    );

    // The repository's HEAD, branch, index and status are as before the send; the status
    // lists what the test changed, and nothing the app made.
    let after = repository_as_git_has_it(&repo);
    assert_eq!(
        (&after.0, &after.1, &after.2),
        (&before.0, &before.1, &before.2)
    );
    // (The helper trims the white space around what git prints.)
    assert_eq!(after.3, "M a.txt\n?? c.txt");
}

#[test]
fn a_send_whose_hand_out_fails_sends_nothing_and_the_attachment_stays() {
    let Round {
        scratch,
        worktree: _,
        store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    let mut keeper = Keeper::new(INSTRUCTION);
    keeper.fails = true;

    let failed = reviews.send_attached(&review, SentBy::Delivered, &mut keeper);

    assert!(matches!(failed, Err(EngineError::HandOut(_))), "{failed:?}");
    drop(reviews);
    drop(store);
    let store = self::store(&scratch);
    let reviews = engine(&scratch, store);
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.attached(), Some(&Attachment::Review));
    assert!(kept.rounds().is_empty());
    assert_eq!(round_of(&reviews, &review, 1001), None);
}

#[test]
fn a_delivery_with_an_instruction_of_its_own_is_handed_the_attached_text_with_that_instruction_and_the_round_it_starts()
 {
    let Round {
        scratch: _scratch,
        worktree,
        store: _store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "b.txt", 1, "two", "and why two"),
        )
        .expect("a second thread");
    let mut keeper = Keeper::new("Reply with the tool, never in prose.");

    // What the view shows, then what is sent: the same text, byte for byte.
    let viewed = reviews
        .attached_text(&review, keeper.instruction())
        .expect("read")
        .expect("something is attached");
    reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send")
        .expect("something was sent");

    assert_eq!(keeper.handed.len(), 1);
    let handed = &keeper.handed[0];
    assert_eq!(handed.text, viewed.text);
    assert!(
        handed
            .text
            .starts_with("Reply with the tool, never in prose.")
    );
    assert_eq!(
        viewed.threads,
        vec![ThreadId::new(id(1)), ThreadId::new(id(2))]
    );
    assert_eq!((handed.threads, handed.round), (2, 1));

    // Nothing is attached after: the view has nothing, and a second send hands nothing.
    assert_eq!(
        reviews
            .attached_text(&review, keeper.instruction())
            .expect("read"),
        None
    );
    assert!(
        reviews
            .send_attached(&review, SentBy::Delivered, &mut keeper)
            .expect("send again")
            .is_none()
    );
    assert_eq!(keeper.handed.len(), 1);
}

#[test]
fn what_the_attachment_carries_is_read_at_the_send_so_a_new_draft_joins_and_a_resolved_thread_leaves()
 {
    let Round {
        scratch: _scratch,
        worktree,
        store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "b.txt", 1, "two", "a late thread"),
        )
        .expect("a draft after the attach");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(3, "b.txt", 1, "two", "a thread to resolve"),
        )
        .expect("a third thread");
    store
        .set_resolved(&ThreadId::new(id(3)), true)
        .expect("resolve it");
    let mut keeper = Keeper::new(INSTRUCTION);

    let (_, sent) = reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send")
        .expect("something was sent");

    assert_eq!(
        sent.comments,
        vec![CommentId::new(id(1001)), CommentId::new(id(1002))]
    );
    assert_eq!(round_of(&reviews, &review, 1003), None);
}

#[test]
fn a_send_whose_attachment_carries_nothing_hands_nothing_out_and_the_attachment_stays() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let started = session(&store, 1);
    let review = reviews.session_started(&started).expect("start");
    let mut keeper = Keeper::new(INSTRUCTION);

    // Nothing attached at all.
    assert!(
        reviews
            .send_attached(&review, SentBy::Delivered, &mut keeper)
            .expect("send")
            .is_none()
    );
    assert_eq!(
        reviews.attached_text(&review, INSTRUCTION).expect("read"),
        None
    );

    // Attached, and then the one thread resolved: nothing to carry, and still attached.
    reviews
        .save_draft(
            None,
            &review,
            &DraftSave {
                thread: ThreadId::new(id(1)),
                comment: CommentId::new(id(1001)),
                on: DraftOn::NewThread(NewAnchor {
                    tree: Some(id(0xface)),
                    path: "a.txt".into(),
                    side: Side::New,
                    first: 1,
                    last: 1,
                    text: "one".to_owned(),
                }),
                text: "why".to_owned(),
            },
        )
        .expect("a draft");
    reviews
        .attach(
            &review,
            Attachment::Review,
            &[SessionNow {
                id: started.id.clone(),
                cli: "claude".to_owned(),
                running: true,
                waits_for_permission: false,
            }],
        )
        .expect("attach");
    store
        .set_resolved(&ThreadId::new(id(1)), true)
        .expect("resolve");
    assert!(
        reviews
            .send_attached(&review, SentBy::Delivered, &mut keeper)
            .expect("send")
            .is_none()
    );
    assert!(keeper.handed.is_empty());
    assert_eq!(
        store.review(&review).expect("read").expect("kept").attached,
        Some(AttachedRecord::Review)
    );
}

#[test]
fn a_follow_up_sent_in_a_new_round_takes_the_addressed_mark_of_its_thread_away() {
    let Round {
        scratch: _scratch,
        worktree,
        store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    let mut keeper = Keeper::new(INSTRUCTION);
    reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send round 1");
    // The agent replies in round 1 and says it addressed the thread.
    store
        .put_reply(
            &ThreadId::new(id(1)),
            &Comment {
                id: CommentId::new(id(2001)),
                author: Author::Agent,
                text: "done".to_owned(),
                round: Some(1),
            },
            true,
        )
        .expect("keep the reply");
    let thread = |reviews: &Reviews| {
        reviews
            .review(&review)
            .expect("read")
            .thread(&ThreadId::new(id(1)))
            .expect("the thread")
            .clone()
    };
    assert!(thread(&reviews).addressed);

    // The reviewer follows up, and the follow-up rides with the next message.
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &DraftSave {
                thread: ThreadId::new(id(1)),
                comment: CommentId::new(id(3001)),
                on: DraftOn::Thread,
                text: "not quite".to_owned(),
            },
        )
        .expect("a follow-up");
    reviews
        .attach(&review, Attachment::Review, &[])
        .expect("attach");
    reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send round 2")
        .expect("something was sent");

    // The agent has a follow-up to answer: the thread is not addressed any more.
    let after = thread(&reviews);
    assert!(!after.addressed);
    assert_eq!(after.comments.last().expect("a comment").round, Some(2));
}

#[test]
fn the_general_send_of_everything_records_its_round_at_the_state_it_is_given_and_refuses_none() {
    let Round {
        scratch: _scratch,
        worktree,
        store: _store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    let state = reviews
        .mark_reviewed(&worktree, &review)
        .expect("a state to send at");
    let mut copy = CopyAsText;

    // Everything with no state is refused, and nothing is handed out or changed.
    let refused = reviews.send(&review, Scope::Everything, SentBy::Copied, None, &mut copy);
    assert!(
        matches!(
            refused,
            Err(EngineError::Refused(ReviewError::NothingToSend))
        ),
        "{refused:?}"
    );
    assert!(reviews.review(&review).expect("read").rounds().is_empty());

    // With a state it is sent at that state, and what was attached stays attached: only a
    // send of what is attached takes it away.
    let (bytes, sent) = reviews
        .send(
            &review,
            Scope::Everything,
            SentBy::Copied,
            Some(&state),
            &mut copy,
        )
        .expect("send")
        .expect("something was sent");
    assert!(
        String::from_utf8(bytes)
            .expect("text")
            .contains("why changed")
    );
    assert_eq!(sent.round.state.as_deref(), Some(state.as_str()));
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.rounds()[0].state.as_deref(), Some(state.as_str()));
    assert_eq!(kept.rounds()[0].sent_by, SentBy::Copied);
    assert_eq!(kept.attached(), Some(&Attachment::Review));

    // Nothing left to carry: not an error, nothing handed out.
    assert!(
        reviews
            .send(
                &review,
                Scope::Everything,
                SentBy::Copied,
                Some(&state),
                &mut copy
            )
            .expect("send again")
            .is_none()
    );
}

#[test]
fn a_round_gets_its_state_once_and_a_round_that_is_not_there_or_has_one_records_none() {
    let Round {
        scratch: _scratch,
        worktree,
        store: _store,
        reviews,
        review,
        repo,
    } = attached_review_in_a_repository();
    let w = workspace();
    let mut keeper = Keeper::new(INSTRUCTION);
    reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send");
    let states = reviews.states_of(&w).expect("the state store");
    let known = states.states().expect("list").len();
    write(&repo, "a.txt", "changed once more\n");

    let missing = reviews.record_round(&worktree, &review, 2);
    assert!(
        matches!(
            missing,
            Err(EngineError::Refused(ReviewError::UnknownRound(2)))
        ),
        "{missing:?}"
    );
    reviews
        .record_round(&worktree, &review, 1)
        .expect("record round 1");
    let known = known + 1;
    let again = reviews.record_round(&worktree, &review, 1);
    assert!(
        matches!(
            again,
            Err(EngineError::Refused(ReviewError::RoundHasState(1)))
        ),
        "{again:?}"
    );
    // A refused call records no state: only the first call's is there.
    assert_eq!(states.states().expect("list").len(), known);
}

#[test]
fn the_review_of_a_session_is_the_one_whose_agent_it_is() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let first = session(&store, 1);
    let second = session(&store, 2);
    let r1 = reviews.session_started(&first).expect("start");
    let r2 = reviews.session_started(&second).expect("start");

    assert_eq!(
        reviews.review_of_session(&first.id).expect("find"),
        Some(r1.clone())
    );
    assert_eq!(
        reviews.review_of_session(&second.id).expect("find"),
        Some(r2)
    );
    // A session whose review the next session took over has none.
    store
        .set_ended(&first.id, yaar_store::EndedBy::Cli)
        .expect("end");
    let third = session(&store, 3);
    assert_eq!(reviews.session_started(&third).expect("start"), r1);
    assert_eq!(reviews.review_of_session(&first.id).expect("find"), None);
    assert_eq!(
        reviews.review_of_session(&third.id).expect("find"),
        Some(r1.clone())
    );
    // A session the Store does not hold has none.
    assert_eq!(
        reviews
            .review_of_session(&yaar_review_engine::SessionId::new(id(99)))
            .expect("find"),
        None
    );
}

#[test]
fn a_thread_attached_alone_is_sent_alone_as_the_view_shows_it_and_the_other_draft_stays_a_draft() {
    let Round {
        scratch,
        worktree,
        store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "b.txt", 1, "two", "and why two"),
        )
        .expect("a second thread");
    reviews
        .attach(&review, Attachment::Thread(ThreadId::new(id(1))), &[])
        .expect("attach one thread");
    let mut keeper = Keeper::new(INSTRUCTION);

    let viewed = reviews
        .attached_text(&review, keeper.instruction())
        .expect("read")
        .expect("something is attached");
    let (_, sent) = reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send")
        .expect("something was sent");

    // What was handed out is what the view showed: the one thread, and not the other's draft.
    assert_eq!(keeper.handed.len(), 1);
    let handed = &keeper.handed[0];
    assert_eq!(handed.text, viewed.text);
    assert!(handed.text.contains("why changed"), "{}", handed.text);
    assert!(!handed.text.contains("and why two"), "{}", handed.text);
    assert!(
        handed.text.contains(&format!("Thread {}", id(1))),
        "{}",
        handed.text
    );
    assert!(
        !handed.text.contains(&format!("Thread {}", id(2))),
        "{}",
        handed.text
    );
    assert_eq!((handed.threads, handed.round), (1, 1));
    assert_eq!(sent.comments, vec![CommentId::new(id(1001))]);

    // The other draft is still a draft, and nothing is attached, also in a Store opened anew.
    drop(reviews);
    drop(store);
    let store = self::store(&scratch);
    let reviews = engine(&scratch, store.clone());
    assert_eq!(round_of(&reviews, &review, 1001), Some(1));
    assert_eq!(round_of(&reviews, &review, 1002), None);
    assert_eq!(
        store.review(&review).expect("read").expect("kept").attached,
        None
    );
}

/// A review in a repository with two rounds sent and no state recorded for either.
fn two_rounds_waiting_for_their_state() -> Round {
    let round = attached_review_in_a_repository();
    let Round {
        worktree,
        reviews,
        review,
        ..
    } = &round;
    let mut keeper = Keeper::new(INSTRUCTION);
    reviews
        .send_attached(review, SentBy::Delivered, &mut keeper)
        .expect("send round 1")
        .expect("something was sent");
    reviews
        .save_draft(
            Some(worktree),
            review,
            &new_thread(2, "b.txt", 1, "two", "and why two"),
        )
        .expect("a second thread");
    reviews
        .attach(review, Attachment::Review, &[])
        .expect("attach again");
    reviews
        .send_attached(review, SentBy::Delivered, &mut keeper)
        .expect("send round 2")
        .expect("something was sent");
    let waiting = reviews.review(review).expect("read");
    assert_eq!(
        waiting
            .rounds()
            .iter()
            .map(|r| r.state.clone())
            .collect::<Vec<_>>(),
        vec![None, None]
    );
    round
}

/// Records the two waiting rounds in `order`, with a change of a file before each call, and
/// shows that each round holds the state its own call returned, also from a Store opened
/// anew.
fn each_round_gets_the_state_its_own_call_recorded(order: [u32; 2]) {
    let Round {
        scratch,
        repo,
        worktree,
        store,
        reviews,
        review,
    } = two_rounds_waiting_for_their_state();

    // Each call names its round, and the files differ between the two calls.
    let mut recorded = Vec::new();
    for round in order {
        write(
            &repo,
            "a.txt",
            &format!("one, for call {}\n", recorded.len() + 1),
        );
        let state = reviews
            .record_round(&worktree, &review, round)
            .unwrap_or_else(|error| panic!("record round {round}: {error}"));
        recorded.push((round, state));
    }
    assert_ne!(recorded[0].1, recorded[1].1);

    let mut expected: Vec<(u32, Option<String>)> = recorded
        .iter()
        .map(|(round, state)| (*round, Some(state.as_str().to_owned())))
        .collect();
    expected.sort();
    let states_of = |reviews: &Reviews| {
        reviews
            .review(&review)
            .expect("read")
            .rounds()
            .iter()
            .map(|round| (round.number, round.state.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(states_of(&reviews), expected);

    // Also from a Store opened anew.
    drop(reviews);
    drop(store);
    let reviews = engine(&scratch, self::store(&scratch));
    assert_eq!(states_of(&reviews), expected);
}

#[test]
fn each_of_two_rounds_that_wait_for_their_state_gets_the_state_its_own_call_recorded() {
    each_round_gets_the_state_its_own_call_recorded([1, 2]);
}

#[test]
fn the_second_of_two_rounds_recorded_first_gets_its_own_state_and_the_first_the_one_recorded_after()
{
    each_round_gets_the_state_its_own_call_recorded([2, 1]);
}

#[test]
fn the_threads_a_round_counts_are_threads_and_not_the_comments_of_a_thread_with_two_unsent() {
    let Round {
        scratch: _scratch,
        worktree: _worktree,
        store,
        reviews,
        review,
        repo: _,
    } = attached_review_in_a_repository();
    // A second unsent comment of the reviewer on the thread the draft started: the Store
    // keeps one, as a draft saved again after a restart could leave.
    store
        .put_comment(
            &ThreadId::new(id(1)),
            &Comment {
                id: CommentId::new(id(1003)),
                author: Author::Reviewer,
                text: "and one more thing".to_owned(),
                round: None,
            },
        )
        .expect("a second unsent comment");
    let mut keeper = Keeper::new(INSTRUCTION);

    let (_, sent) = reviews
        .send_attached(&review, SentBy::Delivered, &mut keeper)
        .expect("send")
        .expect("something was sent");

    assert_eq!(
        sent.comments,
        vec![CommentId::new(id(1001)), CommentId::new(id(1003))]
    );
    assert_eq!(keeper.handed.len(), 1);
    assert_eq!(keeper.handed[0].threads, 1);
}
