//! What the Store keeps of reviews: the records, and the calls that keep and find them.
//!
//! The records are plain data. What may be done to a review, and in which order, is the
//! Review engine's to say; the Store keeps what it is given, each change in one
//! transaction, and finds it again.
//!
//! Two ways of waiting for the drive:
//! - [`Store::save_draft`] is the call made at every keystroke. It returns when the text
//!   is in the database, where a killed Keeper finds it again, and does not wait for the
//!   drive to empty its cache: that costs a few milliseconds, and the typing would feel
//!   it. The writer empties it by itself, with one full flush, once no draft was saved
//!   for [`FLUSH_AFTER`], and when the last clone of the Store is dropped.
//! - Every other call that changes something returns when the change is on the drive. A
//!   call that sends comments, [`Store::put_round`], also moves everything the
//!   write-ahead file holds into the database file before it returns, so that a
//!   write-ahead file damaged later costs at most the drafts typed since.

use std::path::PathBuf;
use std::time::Duration;

use crate::{Store, StoreError, WorkspaceId};

/// How long the writer waits after the last draft save before it flushes the drive.
pub const FLUSH_AFTER: Duration = Duration::from_secs(1);

/// Declares an id of a review's part: 32 lowercase hexadecimal digits.
macro_rules! review_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub(crate) String);

        impl $name {
            /// The id with this text.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The text of the id.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

review_id! {
    /// Names one review: 32 lowercase hexadecimal digits, drawn when the review is made.
    ReviewId
}

review_id! {
    /// Names one thread of a review: 32 lowercase hexadecimal digits.
    ThreadId
}

review_id! {
    /// Names one comment of a thread: 32 lowercase hexadecimal digits.
    CommentId
}

/// The side of a diff a line is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The version the change is compared with: a removed line is on it.
    Old,
    /// The version the change made: an added or an unchanged line is on it.
    New,
}

/// Where a thread sits: lines of one version of one file, and their text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The version the lines are read in: the full id of a commit, a recorded state's or
    /// one of the real repository's, as the Recorder names it; never the id of a git tree
    /// object.
    pub tree: String,
    /// The path of the file in that version, relative to the folder of the worktree: on
    /// the old side of a renamed file its old path. Kept as its bytes.
    pub path: PathBuf,
    /// The side of the diff the lines were on where the comment was written.
    pub side: Side,
    /// The first line, counted from 1.
    pub first: u32,
    /// The last line, counted from 1; at least `first`.
    pub last: u32,
    /// The text of those lines as the reviewer saw them, joined by line feeds.
    pub text: String,
}

/// Who wrote a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    /// The reviewer.
    Reviewer,
    /// The agent the review belongs to.
    Agent,
}

/// One comment of a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Its id.
    pub id: CommentId,
    /// Who wrote it.
    pub author: Author,
    /// Its text.
    pub text: String,
    /// The number of the round it was sent in; none while it is a draft. A draft is a
    /// comment of the reviewer with no round: the calls that edit, delete and send drafts
    /// touch no comment of the agent, whatever its round.
    pub round: Option<u32>,
}

/// One thread of a review, with its comments oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// Its id.
    pub id: ThreadId,
    /// Where its first comment was written, in the version it was written in. It is not
    /// moved when the code changes: where the thread sits now is worked out from it.
    pub anchor: Anchor,
    /// Whether the reviewer resolved it.
    pub resolved: bool,
    /// Whether the agent said it addressed it.
    pub addressed: bool,
    /// Its comments, in the order they were first kept.
    pub comments: Vec<Comment>,
}

/// How the comments of a round left the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentBy {
    /// Copied as text by the reviewer.
    Copied,
    /// Delivered with a message of the user.
    Delivered,
}

/// One round of a review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// Its number, from 1.
    pub number: u32,
    /// The full id of the state of the worktree recorded at the send.
    pub state: String,
    /// How the comments left.
    pub sent_by: SentBy,
}

/// One review, whole: what [`Store::reviews_of`] and [`Store::review`] give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRecord {
    /// Its id.
    pub id: ReviewId,
    /// The workspace it belongs to.
    pub workspace: WorkspaceId,
    /// The agent session it belongs to, by the session's id; none for the review of no
    /// agent.
    pub agent: Option<String>,
    /// The full id of the state last marked reviewed; none before the first mark.
    pub reviewed: Option<String>,
    /// Its threads, in the order they were first kept.
    pub threads: Vec<Thread>,
    /// Its rounds, by number.
    pub rounds: Vec<Round>,
}

/// A file marked viewed: the mark holds for the content it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewedRecord {
    /// The path of the file, relative to the folder of the worktree. Kept as its bytes.
    pub path: PathBuf,
    /// What the Review engine wrote down of the content when the file was marked: the
    /// Store keeps it and compares nothing.
    pub content: String,
}

impl Store {
    /// Every review of a workspace, whole, in the order they were first kept; none for a
    /// workspace with no review.
    pub fn reviews_of(&self, workspace: &WorkspaceId) -> Result<Vec<ReviewRecord>, StoreError> {
        let _ = workspace;
        todo!()
    }

    /// One review, whole; none when the Store holds no review by this id.
    pub fn review(&self, review: &ReviewId) -> Result<Option<ReviewRecord>, StoreError> {
        let _ = review;
        todo!()
    }

    /// Keeps a new review, with no thread and no round: its id, its workspace, its agent
    /// and its reviewed state are taken from `record`, and its threads and rounds must be
    /// empty. An id the Store holds already is refused, and so is a second review of no
    /// agent for one workspace. Returns when it is on the drive.
    pub fn put_review(&self, record: &ReviewRecord) -> Result<(), StoreError> {
        let _ = record;
        todo!()
    }

    /// Keeps a new thread of a review with its first comment, a draft of the reviewer.
    /// `thread.comments` holds exactly that comment. Ids the Store holds already are
    /// refused. Returns when it is on the drive.
    pub fn put_thread(&self, review: &ReviewId, thread: &Thread) -> Result<(), StoreError> {
        let _ = (review, thread);
        todo!()
    }

    /// Keeps a new comment at the end of a thread: a draft reply of the reviewer, or, in a
    /// later slice, a reply of the agent, whose round that slice settles. An id the Store
    /// holds already is refused. Returns when it is on the drive.
    pub fn put_comment(&self, thread: &ThreadId, comment: &Comment) -> Result<(), StoreError> {
        let _ = (thread, comment);
        todo!()
    }

    /// Changes the text of a draft that is kept. Made at every keystroke: it returns when
    /// the text is in the database and does not wait for the drive, which the writer
    /// flushes once no draft was saved for [`FLUSH_AFTER`]. A comment that was sent, or
    /// that is not kept, is left alone and refused.
    pub fn save_draft(&self, comment: &CommentId, text: &str) -> Result<(), StoreError> {
        let _ = (comment, text);
        todo!()
    }

    /// Deletes a draft; a thread left with no comment goes with it. A comment that was
    /// sent, or that is not kept, is left alone and refused. Returns when it is on the
    /// drive.
    pub fn delete_draft(&self, comment: &CommentId) -> Result<(), StoreError> {
        let _ = comment;
        todo!()
    }

    /// Keeps a new round of a review and marks these comments sent in it, in one
    /// transaction: either all of it is kept or none. The round's number must be the next
    /// of the review, and every comment a draft of the review. Returns when it is on the
    /// drive and the write-ahead file has been moved into the database file.
    pub fn put_round(
        &self,
        review: &ReviewId,
        round: &Round,
        comments: &[CommentId],
    ) -> Result<(), StoreError> {
        let _ = (review, round, comments);
        todo!()
    }

    /// Sets whether a thread is resolved. Returns when it is on the drive.
    pub fn set_resolved(&self, thread: &ThreadId, resolved: bool) -> Result<(), StoreError> {
        let _ = (thread, resolved);
        todo!()
    }

    /// Sets the state a review was last marked reviewed at. Returns when it is on the
    /// drive.
    pub fn set_reviewed(&self, review: &ReviewId, state: &str) -> Result<(), StoreError> {
        let _ = (review, state);
        todo!()
    }

    /// The files of a workspace marked viewed, ordered by the bytes of their paths.
    pub fn viewed(&self, workspace: &WorkspaceId) -> Result<Vec<ViewedRecord>, StoreError> {
        let _ = workspace;
        todo!()
    }

    /// Marks a file of a workspace viewed, for the content `mark` says; with none, the
    /// mark is taken away. Returns when it is on the drive.
    pub fn set_viewed(
        &self,
        workspace: &WorkspaceId,
        path: &std::path::Path,
        mark: Option<&str>,
    ) -> Result<(), StoreError> {
        let _ = (workspace, path, mark);
        todo!()
    }

    /// The workspaces one of whose reviews holds a thread, in the order the workspaces
    /// were first kept. A worktree that is gone stays listed while it is among them.
    pub fn workspaces_with_threads(&self) -> Result<Vec<WorkspaceId>, StoreError> {
        todo!()
    }

    /// Flushes the drive now: every draft saved so far is then on it. The writer does
    /// this by itself after a pause in the typing, and when the last clone of the Store is
    /// dropped, as when the Keeper stops and lets its parts go.
    pub fn flush(&self) -> Result<(), StoreError> {
        todo!()
    }
}
