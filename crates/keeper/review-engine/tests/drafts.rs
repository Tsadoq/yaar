//! The engine's calls for the review of a workspace and its drafts, through the engine
//! with the real Store and the real Recorder beneath, on a repository the test makes.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use support::{
    Scratch, commit_all, engine, git, id, init, new_thread, open, store, workspace, write,
};
use yaar_review_engine::{
    CommentId, DraftOn, DraftSave, EngineError, NewAnchor, ReviewError, ReviewId, Saved, Side,
    ThreadId,
};

fn repo_with_one_file(scratch: &Scratch) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\ntwo\nthree\n");
    commit_all(&repo, "first");
    repo
}

fn comment(n: u128) -> CommentId {
    CommentId::new(id(n + 1000))
}

#[test]
fn the_scratch_folder_drops_the_configuration_a_shell_hands_git_in_its_environment() {
    // SAFETY: changing the environment is sound only while no other thread reads it.
    // This is the first statement of a test, and these tests are run by `cargo
    // nextest`, which gives each test a process of its own.
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", "1");
        std::env::set_var("GIT_CONFIG_KEY_0", "alias.status");
        std::env::set_var("GIT_CONFIG_VALUE_0", "log");
        std::env::set_var("GIT_CONFIG_PARAMETERS", "'alias.status'='log'");
        std::env::set_var("GIT_CONFIG", "/nonexistent/config");
    }
    let _scratch = Scratch::new();
    for name in ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"] {
        assert_eq!(std::env::var_os(name), None, "{name} is still set");
    }
}

#[test]
fn eight_readings_at_once_of_a_new_workspace_make_one_review_and_give_its_id() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let store = store(&scratch);
    let reviews = Arc::new(engine(&scratch, store.clone()));
    let w = workspace();

    let readers: Vec<_> = (0..8)
        .map(|_| {
            let (reviews, repo, w) = (Arc::clone(&reviews), repo.clone(), w.clone());
            std::thread::spawn(move || {
                let worktree = open(&repo);
                let view = reviews.of_workspace(&w, Some(&worktree)).expect("read");
                assert_eq!(view.reviews.len(), 1);
                view.reviews[0].id().clone()
            })
        })
        .collect();
    let given: Vec<ReviewId> = readers
        .into_iter()
        .map(|reader| reader.join().expect("a reading"))
        .collect();

    let kept = store.reviews_of(&w).expect("the Store's reviews");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].agent, None);
    assert!(given.iter().all(|review| review == &kept[0].id));
}

#[test]
fn the_review_of_no_agent_is_kept_at_the_first_reading_and_is_the_same_after_a_restart() {
    let scratch = Scratch::new();
    let w = workspace();
    let first = {
        let reviews = engine(&scratch, store(&scratch));
        let view = reviews.of_workspace(&w, None).expect("read");
        assert_eq!(view.reviews.len(), 1);
        assert_eq!(view.reviews[0].workspace(), &w);
        assert!(view.viewed.is_empty());
        view.reviews[0].id().clone()
    };
    let reviews = engine(&scratch, store(&scratch));
    let again = reviews.of_workspace(&w, None).expect("read again");
    assert_eq!(again.reviews.len(), 1);
    assert_eq!(again.reviews[0].id(), &first);
    assert_eq!(reviews.review(&first).expect("read one").id(), &first);
}

#[test]
fn a_review_that_does_not_exist_is_refused_whatever_is_asked_of_it() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let nobody = ReviewId::new(id(0xdead));

    assert!(matches!(
        reviews.review(&nobody),
        Err(EngineError::UnknownReview(_))
    ));
    assert!(matches!(
        reviews.save_draft(
            Some(&worktree),
            &nobody,
            &new_thread(1, "a.txt", 1, "one", "x")
        ),
        Err(EngineError::UnknownReview(_))
    ));
    assert!(matches!(
        reviews.delete_draft(&nobody, &comment(1)),
        Err(EngineError::UnknownReview(_))
    ));
    assert!(matches!(
        reviews.copy_as_text(&worktree, &nobody),
        Err(EngineError::UnknownReview(_))
    ));
    assert!(matches!(
        reviews.mark_reviewed(&worktree, &nobody),
        Err(EngineError::UnknownReview(_))
    ));
    assert!(matches!(
        reviews.changed_since_review(&worktree, &nobody),
        Err(EngineError::UnknownReview(_))
    ));
}

#[test]
fn a_new_thread_on_the_files_on_disk_sits_on_the_state_recorded_for_it() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    write(&repo, "a.txt", "one\ntwo changed\nthree\n");
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    let saved = reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 2, "two changed", "why changed"),
        )
        .expect("save");
    assert_eq!(saved, Saved::Made);

    let kept = reviews.review(&review).expect("read");
    let thread = kept.thread(&ThreadId::new(id(1))).expect("the thread");
    let states = reviews.states_of(&w).expect("the state store");
    let listed = states.states().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(thread.anchor.tree, listed[0].id.as_str());
    assert_eq!(thread.anchor.path, PathBuf::from("a.txt"));
    assert_eq!(thread.anchor.side, Side::New);
    assert_eq!((thread.anchor.first, thread.anchor.last), (2, 2));
    assert_eq!(thread.anchor.text, "two changed");
    assert_eq!(thread.comments.len(), 1);
    assert_eq!(thread.comments[0].text, "why changed");
    assert_eq!(thread.comments[0].round, None);
    // The anchor's tree reads the lines the thread was written on, after the file moved on.
    write(&repo, "a.txt", "something else\n");
    assert_eq!(
        states
            .content(&thread.anchor.tree, Path::new("a.txt"))
            .expect("read"),
        Some(b"one\ntwo changed\nthree\n".to_vec())
    );

    // A second thread on files that did not change is on the same state: none is added.
    write(&repo, "a.txt", "one\ntwo changed\nthree\n");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "a.txt", 1, "one", "and here"),
        )
        .expect("save");
    assert_eq!(states.states().expect("list").len(), 1);
}

#[test]
fn a_thread_on_a_removed_line_names_a_commit_of_the_repository_and_keeps_it_readable() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    write(&repo, "a.txt", "one\nthree\n");
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    let save = DraftSave {
        thread: ThreadId::new(id(1)),
        comment: comment(1),
        on: DraftOn::NewThread(NewAnchor {
            tree: Some(commit.clone()),
            path: PathBuf::from("a.txt"),
            side: Side::Old,
            first: 2,
            last: 2,
            text: "two".to_string(),
        }),
        text: "why was this removed".to_string(),
    };
    assert_eq!(
        reviews
            .save_draft(Some(&worktree), &review, &save)
            .expect("save"),
        Saved::Made
    );

    let kept = reviews.review(&review).expect("read");
    let anchor = &kept.thread(&ThreadId::new(id(1))).expect("thread").anchor;
    assert_eq!(
        (anchor.tree.as_str(), anchor.side),
        (commit.as_str(), Side::Old)
    );
    // No state was needed for it, and the commit reads with the repository gone.
    let states = reviews.states_of(&w).expect("the state store");
    assert!(states.states().expect("list").is_empty());
    std::fs::remove_dir_all(&repo).expect("remove the repository");
    assert_eq!(
        states.content(&commit, Path::new("a.txt")).expect("read"),
        Some(b"one\ntwo\nthree\n".to_vec())
    );
}

#[test]
fn a_new_thread_on_the_files_on_disk_with_no_worktree_is_refused_and_a_text_save_needs_none() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    let first = new_thread(1, "a.txt", 1, "one", "h");
    assert!(matches!(
        reviews.save_draft(None, &review, &first),
        Err(EngineError::NoWorktree)
    ));
    assert!(reviews.review(&review).expect("read").threads().is_empty());

    reviews
        .save_draft(Some(&worktree), &review, &first)
        .expect("save with the worktree");
    // Each keystroke after it carries the whole text, and no worktree.
    for text in ["he", "hey"] {
        let more = DraftSave {
            text: text.to_string(),
            ..first.clone()
        };
        assert_eq!(
            reviews
                .save_draft(None, &review, &more)
                .expect("save the text"),
            Saved::Text
        );
    }
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.threads().len(), 1);
    assert_eq!(kept.threads()[0].comments[0].text, "hey");
}

#[test]
fn a_bad_anchor_is_refused_and_nothing_is_kept() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    // Two lines of text for one line.
    let refused = reviews.save_draft(
        Some(&worktree),
        &review,
        &new_thread(1, "a.txt", 1, "one\ntwo", "x"),
    );
    assert!(matches!(
        refused,
        Err(EngineError::Refused(ReviewError::BadAnchor))
    ));
    assert!(reviews.review(&review).expect("read").threads().is_empty());
}

#[test]
fn a_sent_comment_is_not_edited_or_deleted_and_a_reply_follows_it_in_its_thread() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let first = new_thread(1, "a.txt", 1, "one", "first");
    reviews
        .save_draft(Some(&worktree), &review, &first)
        .expect("save");

    // A reply to a comment not sent yet is no reply.
    let reply = DraftSave {
        thread: first.thread.clone(),
        comment: comment(2),
        on: DraftOn::Thread,
        text: "too early".to_string(),
    };
    assert!(matches!(
        reviews.save_draft(None, &review, &reply),
        Err(EngineError::Refused(ReviewError::NothingSent(_)))
    ));

    reviews.copy_as_text(&worktree, &review).expect("copy");

    // A sent comment keeps its text and stays.
    let edit = DraftSave {
        text: "changed".to_string(),
        ..first.clone()
    };
    assert!(matches!(
        reviews.save_draft(None, &review, &edit),
        Err(EngineError::Refused(ReviewError::NotADraft(_)))
    ));
    assert!(matches!(
        reviews.delete_draft(&review, &first.comment),
        Err(EngineError::Refused(ReviewError::NotADraft(_)))
    ));
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.threads()[0].comments[0].text, "first");

    // The reply is a draft in the same thread.
    assert_eq!(
        reviews.save_draft(None, &review, &reply).expect("reply"),
        Saved::Made
    );
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.threads().len(), 1);
    let comments = &kept.threads()[0].comments;
    assert_eq!(comments.len(), 2);
    assert_eq!(
        (comments[1].text.as_str(), comments[1].round),
        ("too early", None)
    );

    // The reply is a draft: it is edited, deleted, and the thread stays with the sent one.
    reviews
        .save_draft(
            None,
            &review,
            &DraftSave {
                text: "now on time".to_string(),
                ..reply.clone()
            },
        )
        .expect("edit the reply");
    assert_eq!(
        reviews.review(&review).expect("read").threads()[0].comments[1].text,
        "now on time"
    );
    reviews
        .delete_draft(&review, &reply.comment)
        .expect("delete");
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.threads().len(), 1);
    assert_eq!(kept.threads()[0].comments.len(), 1);
}

#[test]
fn deleting_the_only_draft_of_a_thread_takes_the_thread_with_it() {
    let scratch = Scratch::new();
    let repo = repo_with_one_file(&scratch);
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let first = new_thread(1, "a.txt", 1, "one", "first");
    reviews
        .save_draft(Some(&worktree), &review, &first)
        .expect("save");

    reviews
        .delete_draft(&review, &first.comment)
        .expect("delete");

    assert!(reviews.review(&review).expect("read").threads().is_empty());
    assert!(store.reviews_of(&w).expect("kept")[0].threads.is_empty());
    assert!(matches!(
        reviews.delete_draft(&review, &first.comment),
        Err(EngineError::Refused(ReviewError::UnknownComment(_)))
    ));
}
