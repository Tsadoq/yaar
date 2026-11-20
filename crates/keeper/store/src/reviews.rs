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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{Failed, Store, StoreError, WorkspaceId, Writer, bytes, path};

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
    /// Whether the agent said it addressed it, in its last reply.
    pub addressed: bool,
    /// Its comments, in the order they were first kept.
    pub comments: Vec<Comment>,
    /// Where it sits now, in the newest state it was carried to; none while it sits where
    /// its anchor says.
    pub now: Option<ThreadPlace>,
}

/// Where a thread sits now, after the code it was written on changed: kept by
/// [`Store::set_places`], read back with the thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlace {
    /// The version it sits in now: the full id of a recorded state.
    pub tree: String,
    /// The path of the file in that version.
    pub path: PathBuf,
    /// The side, as the anchor's.
    pub side: Side,
    /// The first line, counted from 1.
    pub first: u32,
    /// The last line.
    pub last: u32,
    /// What became of the lines it was written on; none when they only moved.
    pub notice: Option<NoticeRecord>,
}

/// What became of the lines a thread was written on, as the Store keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeRecord {
    /// They were rewritten.
    Changed,
    /// They were deleted.
    Deleted,
}

/// What of a review is attached to the user's next message, as the Store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachedRecord {
    /// The review.
    Review,
    /// One thread alone.
    Thread(ThreadId),
}

/// Who is credited with one uncommitted path at a recorded state, as the Store keeps it
/// with that state: see [`Store::put_credits`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditRecord {
    /// The path, relative to the folder of the worktree.
    pub path: PathBuf,
    /// The sessions it is credited to, by their ids, in the order given; empty for a
    /// change no agent made.
    pub sessions: Vec<String>,
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
    /// The full id of the state of the worktree recorded at the send; none until it is,
    /// as for a round delivered with a message, whose state is recorded after the
    /// message was handed to the agent ([`Store::set_round_state`]).
    pub state: Option<String>,
    /// How the comments left.
    pub sent_by: SentBy,
    /// Whether the agent reported an edit since the send ([`Store::set_round_edited`]).
    pub edited: bool,
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
    /// What is attached to the user's next message; none while nothing is.
    pub attached: Option<AttachedRecord>,
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
        let workspace = workspace.0.clone();
        self.ask(move |connection| {
            let heads = connection
                .prepare("SELECT id FROM review WHERE workspace = ?1 ORDER BY seq")?
                .query_map([workspace], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            heads
                .iter()
                .filter_map(|id| read_review(connection, id).transpose())
                .collect()
        })
    }

    /// One review, whole; none when the Store holds no review by this id.
    pub fn review(&self, review: &ReviewId) -> Result<Option<ReviewRecord>, StoreError> {
        let review = review.0.clone();
        self.ask(move |connection| read_review(connection, &review))
    }

    /// Keeps a new review, with no thread and no round: its id, its workspace, its agent
    /// and its reviewed state are taken from `record`, and its threads and rounds must be
    /// empty. An id the Store holds already is refused, and so is a second review of no
    /// agent for one workspace. Returns when it is on the drive.
    pub fn put_review(&self, record: &ReviewRecord) -> Result<(), StoreError> {
        if !record.threads.is_empty() || !record.rounds.is_empty() {
            return Err(refusal(format!(
                "review {} is new and cannot come with threads or rounds",
                record.id
            )));
        }
        let record = record.clone();
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if exists(&keeping, "SELECT 1 FROM review WHERE id = ?1", &record.id.0)? {
                return refused(format!("review {} is kept already", record.id));
            }
            if record.agent.is_none()
                && exists(
                    &keeping,
                    "SELECT 1 FROM review WHERE workspace = ?1 AND agent IS NULL",
                    &record.workspace.0,
                )?
            {
                return refused(format!(
                    "workspace {} has a review of no agent already",
                    record.workspace
                ));
            }
            keeping.execute(
                "INSERT INTO review (id, workspace, agent, reviewed) VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.id.0,
                    record.workspace.0,
                    record.agent,
                    record.reviewed
                ],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// Keeps a new thread of a review with its first comment, a draft of the reviewer.
    /// `thread.comments` holds exactly that comment. Ids the Store holds already are
    /// refused. Returns when it is on the drive.
    pub fn put_thread(&self, review: &ReviewId, thread: &Thread) -> Result<(), StoreError> {
        let anchor = &thread.anchor;
        if anchor.first < 1 || anchor.last < anchor.first {
            return Err(refusal(format!(
                "thread {} has the lines {} to {}, and lines count from 1, the last not before the first",
                thread.id, anchor.first, anchor.last
            )));
        }
        let [first] = thread.comments.as_slice() else {
            return Err(refusal(format!(
                "thread {} is new and holds one comment, not {}",
                thread.id,
                thread.comments.len()
            )));
        };
        if first.author != Author::Reviewer || first.round.is_some() {
            return Err(refusal(format!(
                "the first comment of thread {} is a draft of the reviewer",
                thread.id
            )));
        }
        let (review, thread) = (review.0.clone(), thread.clone());
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if !exists(&keeping, "SELECT 1 FROM review WHERE id = ?1", &review)? {
                return refused(format!("review {review} is not kept"));
            }
            if exists(&keeping, "SELECT 1 FROM thread WHERE id = ?1", &thread.id.0)? {
                return refused(format!("thread {} is kept already", thread.id));
            }
            let comment = &thread.comments[0];
            if exists(
                &keeping,
                "SELECT 1 FROM comment WHERE id = ?1",
                &comment.id.0,
            )? {
                return refused(format!("comment {} is kept already", comment.id));
            }
            let anchor = &thread.anchor;
            keeping.execute(
                "INSERT INTO thread
                 (id, review, tree, path, side, first_line, last_line, text, resolved, addressed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    thread.id.0,
                    review,
                    anchor.tree,
                    bytes(&anchor.path),
                    side_number(anchor.side),
                    anchor.first,
                    anchor.last,
                    anchor.text,
                    thread.resolved,
                    thread.addressed,
                ],
            )?;
            insert_comment(&keeping, &thread.id.0, comment)?;
            Ok(keeping.commit()?)
        })
    }

    /// Keeps a new comment at the end of a thread: a draft reply of the reviewer, or, in a
    /// later slice, a reply of the agent, whose round that slice settles. An id the Store
    /// holds already is refused. Returns when it is on the drive.
    pub fn put_comment(&self, thread: &ThreadId, comment: &Comment) -> Result<(), StoreError> {
        let (thread, comment) = (thread.0.clone(), comment.clone());
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if !exists(&keeping, "SELECT 1 FROM thread WHERE id = ?1", &thread)? {
                return refused(format!("thread {thread} is not kept"));
            }
            if exists(
                &keeping,
                "SELECT 1 FROM comment WHERE id = ?1",
                &comment.id.0,
            )? {
                return refused(format!("comment {} is kept already", comment.id));
            }
            insert_comment(&keeping, &thread, &comment)?;
            Ok(keeping.commit()?)
        })
    }

    /// Changes the text of a draft that is kept. Made at every keystroke: it returns when
    /// the text is in the database and does not wait for the drive, which the writer
    /// flushes once no draft was saved for [`FLUSH_AFTER`]. A comment that was sent, or
    /// that is not kept, is left alone and refused.
    pub fn save_draft(&self, comment: &CommentId, text: &str) -> Result<(), StoreError> {
        let (comment, text) = (comment.0.clone(), text.to_owned());
        self.with_writer(move |writer| {
            // A pragma cannot run inside a transaction, so each is a statement of its
            // own, and the default is put back whatever the update did.
            writer
                .connection
                .execute_batch("PRAGMA synchronous = NORMAL")?;
            let saved = writer.connection.execute(
                "UPDATE comment SET text = ?2 WHERE id = ?1 AND round IS NULL AND author = 0",
                params![comment, text],
            );
            writer
                .connection
                .execute_batch("PRAGMA synchronous = FULL")?;
            if saved? == 0 {
                return refused(not_a_draft(&writer.connection, &comment)?);
            }
            writer.unflushed_since = Some(Instant::now());
            Ok(())
        })
    }

    /// Deletes a draft; a thread left with no comment goes with it, and with it the
    /// attachment of that thread alone. A comment that was
    /// sent, or that is not kept, is left alone and refused. Returns when it is on the
    /// drive.
    pub fn delete_draft(&self, comment: &CommentId) -> Result<(), StoreError> {
        let comment = comment.0.clone();
        self.try_ask(move |connection| {
            let deleting = connection.transaction()?;
            let thread: Option<String> = deleting
                .query_row(
                    "SELECT thread FROM comment WHERE id = ?1 AND round IS NULL AND author = 0",
                    [&comment],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(thread) = thread else {
                return refused(not_a_draft(&deleting, &comment)?);
            };
            // The comment first: the thread is what its row refers to.
            deleting.execute("DELETE FROM comment WHERE id = ?1", [&comment])?;
            if !exists(
                &deleting,
                "SELECT 1 FROM comment WHERE thread = ?1",
                &thread,
            )? {
                deleting.execute("DELETE FROM thread WHERE id = ?1", [&thread])?;
                // A thread attached alone has nothing left to attach.
                deleting.execute(
                    "UPDATE review SET attached = NULL, attached_thread = NULL
                     WHERE attached_thread = ?1",
                    [&thread],
                )?;
            }
            Ok(deleting.commit()?)
        })
    }

    /// Keeps a new round of a review and marks these comments sent in it, in one
    /// transaction: either all of it is kept or none. Every thread one of whose comments it
    /// sends loses its addressed mark in the same transaction: the agent has a follow-up to
    /// answer. A round may be kept with no state, which [`Store::set_round_state`] gives it
    /// later. The round's number must be the next
    /// of the review, and every comment a draft of the review. Returns when it is on the
    /// drive and the write-ahead file has been moved into the database file. When that
    /// move could not be made, because another connection reads the file, the call fails
    /// though the round is kept: it is then in the write-ahead file only, which the writer
    /// goes on trying to move.
    pub fn put_round(
        &self,
        review: &ReviewId,
        round: &Round,
        comments: &[CommentId],
    ) -> Result<(), StoreError> {
        self.send_round(review, round, comments, false)
    }

    /// The one send behind [`Store::put_round`] and [`Store::put_round_detached`]: `detach`
    /// takes the review's attachment away in the same transaction.
    fn send_round(
        &self,
        review: &ReviewId,
        round: &Round,
        comments: &[CommentId],
        detach: bool,
    ) -> Result<(), StoreError> {
        let (review, round) = (review.0.clone(), round.clone());
        let comments: Vec<String> = comments.iter().map(|c| c.0.clone()).collect();
        self.with_writer(move |writer| {
            let sending = writer.connection.transaction()?;
            if !exists(&sending, "SELECT 1 FROM review WHERE id = ?1", &review)? {
                return refused(format!("review {review} is not kept"));
            }
            let last: u32 = sending.query_row(
                "SELECT COALESCE(MAX(number), 0) FROM round WHERE review = ?1",
                [&review],
                |row| row.get(0),
            )?;
            if round.number != last + 1 {
                return refused(format!(
                    "round {} is not the next of review {review}, which is round {}",
                    round.number,
                    last + 1
                ));
            }
            sending.execute(
                "INSERT INTO round (review, number, state, sent_by) VALUES (?1, ?2, ?3, ?4)",
                params![
                    review,
                    round.number,
                    round.state,
                    sent_by_number(round.sent_by)
                ],
            )?;
            for comment in &comments {
                // A comment named twice is a draft only the first time.
                let sent = sending.execute(
                    "UPDATE comment SET round = ?3
                     WHERE id = ?1 AND round IS NULL AND author = 0
                       AND thread IN (SELECT id FROM thread WHERE review = ?2)",
                    params![comment, review, round.number],
                )?;
                if sent == 0 {
                    return refused(format!(
                        "comment {comment} is not a draft of review {review}, and is not sent"
                    ));
                }
                // The agent has a follow-up to answer, whatever it said of the thread.
                sending.execute(
                    "UPDATE thread SET addressed = 0
                     WHERE id = (SELECT thread FROM comment WHERE id = ?1)",
                    [comment],
                )?;
            }
            if detach {
                sending.execute(
                    "UPDATE review SET attached = NULL, attached_thread = NULL WHERE id = ?1",
                    [&review],
                )?;
            }
            sending.commit()?;
            // The sent comments reach the database file before the call returns, so
            // that a damaged write-ahead file costs no more than the drafts typed since.
            writer.flush()?;
            Ok(())
        })
    }

    /// Sets whether a thread is resolved. Returns when it is on the drive.
    pub fn set_resolved(&self, thread: &ThreadId, resolved: bool) -> Result<(), StoreError> {
        let thread = thread.0.clone();
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE thread SET resolved = ?2 WHERE id = ?1",
                params![thread, resolved],
            )?;
            if changed == 0 {
                return refused(format!("thread {thread} is not kept"));
            }
            Ok(())
        })
    }

    /// Sets the state a review was last marked reviewed at. Returns when it is on the
    /// drive.
    pub fn set_reviewed(&self, review: &ReviewId, state: &str) -> Result<(), StoreError> {
        let (review, state) = (review.0.clone(), state.to_owned());
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE review SET reviewed = ?2 WHERE id = ?1",
                params![review, state],
            )?;
            if changed == 0 {
                return refused(format!("review {review} is not kept"));
            }
            Ok(())
        })
    }

    /// The files of a workspace marked viewed, ordered by the bytes of their paths.
    pub fn viewed(&self, workspace: &WorkspaceId) -> Result<Vec<ViewedRecord>, StoreError> {
        let workspace = workspace.0.clone();
        self.ask(move |connection| {
            connection
                .prepare("SELECT path, content FROM viewed WHERE workspace = ?1 ORDER BY path")?
                .query_map([workspace], |row| {
                    Ok(ViewedRecord {
                        path: path(row.get(0)?),
                        content: row.get(1)?,
                    })
                })?
                .collect()
        })
    }

    /// Marks a file of a workspace viewed, for the content `mark` says; with none, the
    /// mark is taken away. Returns when it is on the drive.
    pub fn set_viewed(
        &self,
        workspace: &WorkspaceId,
        path: &Path,
        mark: Option<&str>,
    ) -> Result<(), StoreError> {
        let (workspace, path, mark) = (workspace.0.clone(), bytes(path), mark.map(str::to_owned));
        self.ask(move |connection| {
            match mark {
                Some(content) => connection.execute(
                    "INSERT INTO viewed (workspace, path, content) VALUES (?1, ?2, ?3)
                     ON CONFLICT (workspace, path) DO UPDATE SET content = excluded.content
                     WHERE content IS NOT excluded.content",
                    params![workspace, path, content],
                ),
                None => connection.execute(
                    "DELETE FROM viewed WHERE workspace = ?1 AND path = ?2",
                    params![workspace, path],
                ),
            }
            .map(drop)
        })
    }

    /// The workspaces one of whose reviews holds a thread, in the order the workspaces
    /// were first kept. A worktree that is gone stays listed while it is among them.
    pub fn workspaces_with_threads(&self) -> Result<Vec<WorkspaceId>, StoreError> {
        self.ask(|connection| {
            // A workspace the table does not hold comes after those it does, by the
            // first of its reviews.
            connection
                .prepare(
                    "SELECT r.workspace FROM review r
                     LEFT JOIN workspace w ON w.id = r.workspace
                     WHERE EXISTS (SELECT 1 FROM thread t WHERE t.review = r.id)
                     GROUP BY r.workspace
                     ORDER BY w.seq IS NULL, w.seq, MIN(r.seq)",
                )?
                .query_map([], |row| Ok(WorkspaceId(row.get(0)?)))?
                .collect()
        })
    }

    /// Keeps a new round of a review as [`Store::put_round`] does, and takes away what of
    /// the review is attached to the user's next message, all in the same transaction:
    /// the send of what was attached. Either all of it is kept or none, so that a Keeper
    /// killed in between never leaves a review attached whose comments were sent. Refused
    /// as `put_round` is.
    pub fn put_round_detached(
        &self,
        review: &ReviewId,
        round: &Round,
        comments: &[CommentId],
    ) -> Result<(), StoreError> {
        self.send_round(review, round, comments, true)
    }

    /// Keeps what of a review is attached to the user's next message, in place of what
    /// was; none takes the attachment away. Returns when it is on the drive. Refused for a
    /// review that is not kept, and for a thread the review does not hold.
    pub fn set_attached(
        &self,
        review: &ReviewId,
        attached: Option<&AttachedRecord>,
    ) -> Result<(), StoreError> {
        let (review, attached) = (review.0.clone(), attached.cloned());
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if !exists(&keeping, "SELECT 1 FROM review WHERE id = ?1", &review)? {
                return refused(format!("review {review} is not kept"));
            }
            let (kind, thread) = match &attached {
                None => (None, None),
                Some(AttachedRecord::Review) => (Some(0), None),
                Some(AttachedRecord::Thread(thread)) => {
                    if !keeping
                        .prepare("SELECT 1 FROM thread WHERE id = ?1 AND review = ?2")?
                        .exists([&thread.0, &review])?
                    {
                        return refused(format!("review {review} holds no thread {thread}"));
                    }
                    (Some(1), Some(thread.0.clone()))
                }
            };
            keeping.execute(
                "UPDATE review SET attached = ?2, attached_thread = ?3 WHERE id = ?1",
                params![review, kind, thread],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// Keeps a reply of the agent in a thread, and whether it marked the thread addressed,
    /// in one transaction: either both are kept or neither. The comment's author is the
    /// agent and its round the number given, which must be a round of the thread's
    /// review. Returns when it is on the drive. Refused for a thread that is not kept, an
    /// id that is taken, and a comment of the reviewer.
    pub fn put_reply(
        &self,
        thread: &ThreadId,
        reply: &Comment,
        addressed: bool,
    ) -> Result<(), StoreError> {
        if reply.author != Author::Agent {
            return Err(refusal(format!(
                "comment {} is the reviewer's, and a reply is the agent's",
                reply.id
            )));
        }
        let Some(round) = reply.round else {
            return Err(refusal(format!(
                "reply {} names no round, and a reply belongs to one",
                reply.id
            )));
        };
        let (thread, reply) = (thread.0.clone(), reply.clone());
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            let review: Option<String> = keeping
                .query_row(
                    "SELECT review FROM thread WHERE id = ?1",
                    [&thread],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(review) = review else {
                return refused(format!("thread {thread} is not kept"));
            };
            if exists(&keeping, "SELECT 1 FROM comment WHERE id = ?1", &reply.id.0)? {
                return refused(format!("comment {} is kept already", reply.id));
            }
            if !keeping
                .prepare("SELECT 1 FROM round WHERE review = ?1 AND number = ?2")?
                .exists(params![review, round])?
            {
                return refused(format!(
                    "review {review} has no round {round}, which reply {} names",
                    reply.id
                ));
            }
            insert_comment(&keeping, &thread, &reply)?;
            keeping.execute(
                "UPDATE thread SET addressed = ?2 WHERE id = ?1",
                params![thread, addressed],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// Resolves each of `threads`, in one transaction. Returns when it is on the drive.
    /// Refused, with nothing changed, when one is not kept.
    pub fn resolve_threads(&self, threads: &[ThreadId]) -> Result<(), StoreError> {
        let threads: Vec<String> = threads.iter().map(|t| t.0.clone()).collect();
        self.try_ask(move |connection| {
            let resolving = connection.transaction()?;
            for thread in &threads {
                let changed =
                    resolving.execute("UPDATE thread SET resolved = 1 WHERE id = ?1", [thread])?;
                if changed == 0 {
                    return refused(format!("thread {thread} is not kept"));
                }
            }
            Ok(resolving.commit()?)
        })
    }

    /// Keeps the state recorded for a round that was kept with none. Returns when it is on
    /// the drive. Refused for a round that is not kept, and one that has a state already.
    pub fn set_round_state(
        &self,
        review: &ReviewId,
        round: u32,
        state: &str,
    ) -> Result<(), StoreError> {
        let (review, state) = (review.0.clone(), state.to_owned());
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE round SET state = ?3 WHERE review = ?1 AND number = ?2 AND state IS NULL",
                params![review, round, state],
            )?;
            if changed == 0 {
                return refused(if exists_round(connection, &review, round)? {
                    format!("round {round} of review {review} has a state already")
                } else {
                    format!("review {review} has no round {round}")
                });
            }
            Ok(())
        })
    }

    /// Marks a round as one its agent has edited since. Returns when it is on the drive.
    /// Refused for a round that is not kept.
    pub fn set_round_edited(&self, review: &ReviewId, round: u32) -> Result<(), StoreError> {
        let review = review.0.clone();
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE round SET edited = 1 WHERE review = ?1 AND number = ?2",
                params![review, round],
            )?;
            if changed == 0 {
                return refused(format!("review {review} has no round {round}"));
            }
            Ok(())
        })
    }

    /// Keeps where each thread of `places` sits now, in place of where it sat, in one
    /// transaction: either all are kept or none. None puts a thread back on its anchor.
    /// Following carries every thread of a workspace to each state it records, so one
    /// write to the drive serves them all. Returns when it is on the drive. Refused, with
    /// nothing kept, when one thread is not kept.
    pub fn set_places(&self, places: &[(ThreadId, Option<ThreadPlace>)]) -> Result<(), StoreError> {
        for (thread, place) in places {
            if let Some(place) = place
                && (place.first < 1 || place.last < place.first)
            {
                return Err(refusal(format!(
                    "thread {thread} is placed on the lines {} to {}, and lines count from 1, the last not before the first",
                    place.first, place.last
                )));
            }
        }
        let places = places.to_vec();
        self.try_ask(move |connection| {
            let placing = connection.transaction()?;
            for (thread, place) in &places {
                let changed = match place {
                    Some(place) => placing.execute(
                        "UPDATE thread SET now_tree = ?2, now_path = ?3, now_side = ?4,
                                now_first = ?5, now_last = ?6, notice = ?7
                         WHERE id = ?1",
                        params![
                            thread.0,
                            place.tree,
                            bytes(&place.path),
                            side_number(place.side),
                            place.first,
                            place.last,
                            place.notice.map(notice_number)
                        ],
                    )?,
                    None => placing.execute(
                        "UPDATE thread SET now_tree = NULL, now_path = NULL, now_side = NULL,
                                now_first = NULL, now_last = NULL, notice = NULL
                         WHERE id = ?1",
                        [&thread.0],
                    )?,
                };
                if changed == 0 {
                    return refused(format!("thread {thread} is not kept"));
                }
            }
            Ok(placing.commit()?)
        })
    }

    /// Keeps, with the recorded state `state` of `workspace`, who is credited with each
    /// path of `credits`, in place of what was kept for that state: every uncommitted
    /// change of the worktree, as Workspaces credited them when the state was recorded.
    /// Returns when it is on the drive.
    pub fn put_credits(
        &self,
        workspace: &WorkspaceId,
        state: &str,
        credits: &[CreditRecord],
    ) -> Result<(), StoreError> {
        // An empty session id is how the table says that no agent made the change.
        if credits
            .iter()
            .any(|c| c.sessions.iter().any(String::is_empty))
        {
            return Err(refusal(
                "a credit names a session with an empty id, which stands for no session".to_owned(),
            ));
        }
        let (workspace, state, credits) = (workspace.0.clone(), state.to_owned(), credits.to_vec());
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            keeping.execute(
                "DELETE FROM state_credit WHERE workspace = ?1 AND state = ?2",
                params![workspace, state],
            )?;
            // A path named twice, or a session twice for it, is kept once.
            let mut insert = keeping.prepare(
                "INSERT OR IGNORE INTO state_credit (workspace, state, path, session)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for credit in &credits {
                let path = bytes(&credit.path);
                if credit.sessions.is_empty() {
                    insert.execute(params![workspace, state, path, ""])?;
                }
                for session in &credit.sessions {
                    insert.execute(params![workspace, state, path, session])?;
                }
            }
            drop(insert);
            // Kept with none, a state is still one whose credits were kept.
            keeping.execute(
                "INSERT OR IGNORE INTO state_kept (workspace, state) VALUES (?1, ?2)",
                params![workspace, state],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// Who is credited with each path kept with the recorded state `state` of `workspace`,
    /// ordered by the bytes of the paths; empty for a state nothing was kept with.
    pub fn credits_of(
        &self,
        workspace: &WorkspaceId,
        state: &str,
    ) -> Result<Vec<CreditRecord>, StoreError> {
        let (workspace, state) = (workspace.0.clone(), state.to_owned());
        self.ask(move |connection| {
            // The sessions of a path come in the order they were kept.
            let rows = connection
                .prepare(
                    "SELECT path, session FROM state_credit
                     WHERE workspace = ?1 AND state = ?2 ORDER BY path, rowid",
                )?
                .query_map([workspace, state], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut credits: Vec<CreditRecord> = Vec::new();
            for (file, session) in rows {
                let file = path(file);
                if credits.last().is_none_or(|last| last.path != file) {
                    credits.push(CreditRecord {
                        path: file,
                        sessions: Vec::new(),
                    });
                }
                if !session.is_empty() {
                    credits
                        .last_mut()
                        .expect("just pushed")
                        .sessions
                        .push(session);
                }
            }
            Ok(credits)
        })
    }

    /// Whether the credits of the recorded state `state` of `workspace` were kept with
    /// [`Store::put_credits`], none among them: a state of a worktree with nothing
    /// uncommitted is kept with no credit, and is told from a state nothing was kept with,
    /// as one recorded at a send. Every call of `put_credits` keeps this mark in its own
    /// transaction.
    pub fn credits_were_kept(
        &self,
        workspace: &WorkspaceId,
        state: &str,
    ) -> Result<bool, StoreError> {
        let (workspace, state) = (workspace.0.clone(), state.to_owned());
        self.ask(move |connection| {
            connection
                .prepare("SELECT 1 FROM state_kept WHERE workspace = ?1 AND state = ?2")?
                .exists([workspace, state])
        })
    }

    /// Keeps that round `round` of `review` was sent in the turn `turn` of its session, in
    /// place of what was kept before. Refused for a review or a round the Store does not
    /// hold. Returns when it is on the drive.
    pub fn set_round_turn(
        &self,
        review: &ReviewId,
        round: u32,
        turn: &str,
    ) -> Result<(), StoreError> {
        let (review, turn) = (review.0.clone(), turn.to_owned());
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE round SET turn = ?3 WHERE review = ?1 AND number = ?2",
                params![review, round, turn],
            )?;
            if changed == 0 {
                return refused(format!("review {review} has no round {round}"));
            }
            Ok(())
        })
    }

    /// The turn each round of `review` was sent in, as [`Store::set_round_turn`] kept it, by
    /// the number of the round, ordered by it; a round with none is left out. Empty for a
    /// review the Store does not hold.
    pub fn round_turns(&self, review: &ReviewId) -> Result<Vec<(u32, String)>, StoreError> {
        let review = review.0.clone();
        self.ask(move |connection| {
            connection
                .prepare(
                    "SELECT number, turn FROM round
                     WHERE review = ?1 AND turn IS NOT NULL ORDER BY number",
                )?
                .query_map([review], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
    }

    /// Flushes the drive now: every draft saved so far is then on it. The writer does
    /// this by itself after a pause in the typing, and when the last clone of the Store is
    /// dropped, as when the Keeper stops and lets its parts go.
    pub fn flush(&self) -> Result<(), StoreError> {
        self.with_writer(Writer::flush)
    }
}

/// The refusal of a call, before any work is handed to the writer.
pub(crate) fn refusal(sentence: String) -> StoreError {
    StoreError::Database(sentence)
}

/// The refusal of a piece of work, in a sentence.
pub(crate) fn refused<T>(sentence: String) -> Result<T, Failed> {
    Err(Failed::Refused(sentence))
}

/// Whether a query that takes one id finds a row.
pub(crate) fn exists(connection: &Connection, query: &str, id: &str) -> rusqlite::Result<bool> {
    connection.prepare(query)?.exists([id])
}

/// Whether a review holds a round of this number.
fn exists_round(connection: &Connection, review: &str, round: u32) -> rusqlite::Result<bool> {
    connection
        .prepare("SELECT 1 FROM round WHERE review = ?1 AND number = ?2")?
        .exists(params![review, round])
}

/// What to say of a comment that is not a draft that is kept: that it is not kept, or
/// that it was sent, or that it is not the reviewer's.
fn not_a_draft(connection: &Connection, comment: &str) -> rusqlite::Result<String> {
    let found: Option<(i64, Option<u32>)> = connection
        .query_row(
            "SELECT author, round FROM comment WHERE id = ?1",
            [comment],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        None => format!("comment {comment} is not kept"),
        Some((_, Some(round))) => {
            format!("comment {comment} was sent in round {round}, not a draft")
        }
        Some(_) => format!("comment {comment} is not the reviewer's, not a draft"),
    })
}

fn insert_comment(
    keeping: &Transaction<'_>,
    thread: &str,
    comment: &Comment,
) -> rusqlite::Result<()> {
    keeping
        .execute(
            "INSERT INTO comment (id, thread, author, text, round) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                comment.id.0,
                thread,
                author_number(comment.author),
                comment.text,
                comment.round
            ],
        )
        .map(drop)
}

/// Reads one review whole: its threads in the order they were first kept, each with its
/// comments in the order they were first kept, and its rounds by number.
fn read_review(connection: &Connection, id: &str) -> rusqlite::Result<Option<ReviewRecord>> {
    let head = connection
        .query_row(
            "SELECT workspace, agent, reviewed, attached, attached_thread
             FROM review WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((workspace, agent, reviewed, attached, attached_thread)) = head else {
        return Ok(None);
    };

    let mut threads: Vec<Thread> = connection
        .prepare(
            "SELECT id, tree, path, side, first_line, last_line, text, resolved, addressed,
                    now_tree, now_path, now_side, now_first, now_last, notice
             FROM thread WHERE review = ?1 ORDER BY seq",
        )?
        .query_map([id], |row| {
            // A thread sits on its anchor while it has no place of its own.
            let now = match row.get::<_, Option<String>>(9)? {
                None => None,
                Some(tree) => Some(ThreadPlace {
                    tree,
                    path: path(row.get(10)?),
                    side: side_of(row.get(11)?),
                    first: row.get(12)?,
                    last: row.get(13)?,
                    notice: row.get::<_, Option<i64>>(14)?.map(notice_of),
                }),
            };
            Ok(Thread {
                id: ThreadId(row.get(0)?),
                anchor: Anchor {
                    tree: row.get(1)?,
                    path: path(row.get(2)?),
                    side: side_of(row.get(3)?),
                    first: row.get(4)?,
                    last: row.get(5)?,
                    text: row.get(6)?,
                },
                resolved: row.get(7)?,
                addressed: row.get(8)?,
                comments: Vec::new(),
                now,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let place: HashMap<String, usize> = threads
        .iter()
        .enumerate()
        .map(|(i, thread)| (thread.id.0.clone(), i))
        .collect();

    let comments = connection
        .prepare(
            "SELECT c.thread, c.id, c.author, c.text, c.round
             FROM comment c JOIN thread t ON t.id = c.thread
             WHERE t.review = ?1 ORDER BY c.seq",
        )?
        .query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Comment {
                    id: CommentId(row.get(1)?),
                    author: author_of(row.get(2)?),
                    text: row.get(3)?,
                    round: row.get(4)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (thread, comment) in comments {
        threads[place[&thread]].comments.push(comment);
    }

    let rounds = connection
        .prepare(
            "SELECT number, state, sent_by, edited FROM round WHERE review = ?1 ORDER BY number",
        )?
        .query_map([id], |row| {
            Ok(Round {
                number: row.get(0)?,
                state: row.get(1)?,
                sent_by: sent_by_of(row.get(2)?),
                edited: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    Ok(Some(ReviewRecord {
        id: ReviewId(id.to_owned()),
        workspace: WorkspaceId(workspace),
        agent,
        reviewed,
        threads,
        rounds,
        attached: attached.map(|kind| match (kind, attached_thread) {
            (1, Some(thread)) => AttachedRecord::Thread(ThreadId(thread)),
            _ => AttachedRecord::Review,
        }),
    }))
}

// Sides, authors and ways of sending are kept as the numbers `TABLES_2` says, and the
// notice of a thread's place as `TABLES_5` says.

fn side_number(side: Side) -> i64 {
    match side {
        Side::Old => 0,
        Side::New => 1,
    }
}

fn side_of(number: i64) -> Side {
    if number == 0 { Side::Old } else { Side::New }
}

fn author_number(author: Author) -> i64 {
    match author {
        Author::Reviewer => 0,
        Author::Agent => 1,
    }
}

fn author_of(number: i64) -> Author {
    if number == 0 {
        Author::Reviewer
    } else {
        Author::Agent
    }
}

fn notice_number(notice: NoticeRecord) -> i64 {
    match notice {
        NoticeRecord::Changed => 0,
        NoticeRecord::Deleted => 1,
    }
}

fn notice_of(number: i64) -> NoticeRecord {
    if number == 0 {
        NoticeRecord::Changed
    } else {
        NoticeRecord::Deleted
    }
}

fn sent_by_number(sent_by: SentBy) -> i64 {
    match sent_by {
        SentBy::Copied => 0,
        SentBy::Delivered => 1,
    }
}

fn sent_by_of(number: i64) -> SentBy {
    if number == 0 {
        SentBy::Copied
    } else {
        SentBy::Delivered
    }
}
