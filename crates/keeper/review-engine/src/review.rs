//! One review and its rules, with no file and no process: what can be done to its threads
//! and comments, what is attached, and what a send carries.

use std::fmt;

use yaar_store::{
    Anchor, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, Thread, ThreadId,
    WorkspaceId,
};

/// What of a review goes with the user's next message to its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// The review: every unsent comment on an unresolved thread at the moment of the send,
    /// drafts written after the attaching included, and a thread resolved meanwhile not.
    Review,
    /// One thread alone: its unsent comments, while it is unresolved. A draft on another
    /// thread does not join.
    Thread(ThreadId),
}

/// Which comments a send, or the text of a review, takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every unsent comment on an unresolved thread, whatever is attached: what copying a
    /// review as text takes.
    Everything,
    /// What is attached: everything as [`Scope::Everything`] when the review is attached,
    /// the one thread when one thread is attached alone, and nothing when nothing is.
    Attached,
}

/// The unsent comments of one thread that a send would carry, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsent<'a> {
    /// The thread.
    pub thread: &'a Thread,
    /// Its unsent comments, oldest first: at least one.
    pub comments: Vec<&'a Comment>,
}

/// What a send did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The round it started.
    pub round: Round,
    /// The comments it sent, now carrying that round, in the order of [`Review::unsent`].
    pub comments: Vec<CommentId>,
}

/// Why a review refused a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    /// The review holds no thread by this id.
    UnknownThread(ThreadId),
    /// The review holds no comment by this id.
    UnknownComment(CommentId),
    /// The comment was sent: it carries its round and cannot be edited or deleted.
    NotADraft(CommentId),
    /// The id is taken by a thread or a comment of the review.
    TakenId(String),
    /// A reply was asked for in a thread with no sent comment: there is nothing to follow
    /// up on, and its draft is edited instead.
    NothingSent(ThreadId),
    /// A reply was asked for in a thread that holds a draft already: that draft is edited
    /// instead.
    DraftPending {
        /// The thread.
        thread: ThreadId,
        /// Its draft.
        draft: CommentId,
    },
    /// The anchor is not a range of lines: the first line is 0, the last is before the
    /// first, or its text holds another number of lines than the range.
    BadAnchor,
    /// A send was asked for with nothing to carry.
    NothingToSend,
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownThread(thread) => write!(f, "there is no thread {thread}"),
            Self::UnknownComment(comment) => write!(f, "there is no comment {comment}"),
            Self::NotADraft(comment) => {
                write!(
                    f,
                    "the comment {comment} was sent and can no longer be changed"
                )
            }
            Self::TakenId(id) => write!(f, "the id {id} is taken"),
            Self::NothingSent(thread) => {
                write!(f, "the thread {thread} has no sent comment to reply to")
            }
            Self::DraftPending { thread, draft } => {
                write!(f, "the thread {thread} holds the draft {draft} already")
            }
            Self::BadAnchor => write!(f, "the lines of the comment are not a range of lines"),
            Self::NothingToSend => write!(f, "there is nothing to send"),
        }
    }
}

impl std::error::Error for ReviewError {}

/// One review: threads written for one agent session in one worktree, or for no agent.
///
/// A comment is a draft until a send carries it; a draft can be edited and deleted. A send
/// starts a round: every comment it carries gets the round's number and refuses to be
/// edited or deleted from then on. A follow-up on a sent comment is a new draft, a reply,
/// in the same thread. A resolved thread is not sent, an unsent comment on it included.
/// A review has no votes and no labels.
///
/// The ids of threads and comments are given by the caller, which draws them; an id the
/// review holds already is refused. Every change is checked before anything is changed: a
/// change that is refused leaves the review as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    record: ReviewRecord,
    attached: Option<Attachment>,
}

impl Review {
    /// A new review with no thread, no round and no reviewed state.
    pub fn new(id: ReviewId, workspace: WorkspaceId, agent: Option<String>) -> Review {
        let _ = (id, workspace, agent);
        todo!()
    }

    /// The review the Store kept, with nothing attached: the Store keeps no attachment yet.
    pub fn from_record(record: ReviewRecord) -> Review {
        let _ = record;
        todo!()
    }

    /// The review as the Store keeps it.
    pub fn record(&self) -> &ReviewRecord {
        &self.record
    }

    /// Its id.
    pub fn id(&self) -> &ReviewId {
        todo!()
    }

    /// The workspace it belongs to.
    pub fn workspace(&self) -> &WorkspaceId {
        todo!()
    }

    /// The agent session it belongs to; none for the review of no agent.
    pub fn agent(&self) -> Option<&str> {
        todo!()
    }

    /// Its threads, in the order they were started.
    pub fn threads(&self) -> &[Thread] {
        todo!()
    }

    /// One thread.
    pub fn thread(&self, thread: &ThreadId) -> Option<&Thread> {
        let _ = thread;
        todo!()
    }

    /// One comment, with the thread it is in.
    pub fn comment(&self, comment: &CommentId) -> Option<(&Thread, &Comment)> {
        let _ = comment;
        todo!()
    }

    /// Its rounds, the first first.
    pub fn rounds(&self) -> &[Round] {
        todo!()
    }

    /// The state last marked reviewed, by its full id; none before the first mark.
    pub fn reviewed(&self) -> Option<&str> {
        todo!()
    }

    /// What is attached to the user's next message; none while nothing is.
    pub fn attached(&self) -> Option<&Attachment> {
        self.attached.as_ref()
    }

    /// Starts a thread on `anchor` with a first comment, a draft of the reviewer with
    /// `text`. The anchor is kept as given: the tree, the path, the side, the lines and
    /// their text, whatever line it is and wherever it was written. Refused with
    /// [`ReviewError::BadAnchor`] for lines that are no range, and
    /// [`ReviewError::TakenId`] for an id the review holds.
    pub fn start_thread(
        &mut self,
        thread: ThreadId,
        comment: CommentId,
        anchor: Anchor,
        text: &str,
    ) -> Result<(), ReviewError> {
        let _ = (thread, comment, anchor, text);
        todo!()
    }

    /// Adds a draft reply of the reviewer at the end of a thread that holds a sent comment
    /// and no draft. Refused with [`ReviewError::NothingSent`] in a thread with no sent
    /// comment, [`ReviewError::DraftPending`] in one that holds a draft, and
    /// [`ReviewError::UnknownThread`] and [`ReviewError::TakenId`].
    pub fn reply(
        &mut self,
        thread: &ThreadId,
        comment: CommentId,
        text: &str,
    ) -> Result<(), ReviewError> {
        let _ = (thread, comment, text);
        todo!()
    }

    /// Changes the text of a draft. Refused with [`ReviewError::NotADraft`] for a sent
    /// comment, which keeps its text.
    pub fn edit(&mut self, comment: &CommentId, text: &str) -> Result<(), ReviewError> {
        let _ = (comment, text);
        todo!()
    }

    /// Deletes a draft; a thread left with no comment goes with it. Refused with
    /// [`ReviewError::NotADraft`] for a sent comment, which stays.
    pub fn delete(&mut self, comment: &CommentId) -> Result<(), ReviewError> {
        let _ = comment;
        todo!()
    }

    /// Resolves a thread: it is sent no more, an unsent comment on it included, until it
    /// is reopened. A thread attached alone stays attached and carries nothing.
    pub fn resolve(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        let _ = thread;
        todo!()
    }

    /// Reopens a resolved thread.
    pub fn reopen(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        let _ = thread;
        todo!()
    }

    /// Attaches the review, or one of its threads alone, to the user's next message, in
    /// place of what was attached. What the send will carry is read at the send, by
    /// [`Review::unsent`] with [`Scope::Attached`]: an edit changes it, a new draft joins
    /// an attached review, and a thread resolved meanwhile leaves it.
    pub fn attach(&mut self, attachment: Attachment) -> Result<(), ReviewError> {
        let _ = attachment;
        todo!()
    }

    /// Takes the attachment away. Every comment it would have carried stays an unsent
    /// draft.
    pub fn detach(&mut self) {
        todo!()
    }

    /// The unsent comments a send of `scope` would carry now, thread by thread, in the
    /// order of the threads: every unsent comment on an unresolved thread, or for
    /// [`Scope::Attached`] those of what is attached. A thread with none is left out.
    pub fn unsent(&self, scope: Scope) -> Vec<Unsent<'_>> {
        let _ = scope;
        todo!()
    }

    /// Sends what [`Review::unsent`] gives for `scope`: starts the next round, recorded at
    /// `state` and sent by `sent_by`, and gives each of those comments that round's
    /// number. A send of what was attached detaches it. One thread sent alone is a round
    /// like any other. Refused with [`ReviewError::NothingToSend`], and nothing changed,
    /// when there is nothing to carry.
    pub fn send(
        &mut self,
        scope: Scope,
        state: &str,
        sent_by: SentBy,
    ) -> Result<Sent, ReviewError> {
        let _ = (scope, state, sent_by);
        todo!()
    }

    /// Makes `state` the reviewed state, from which the next review starts.
    pub fn mark_reviewed(&mut self, state: &str) {
        let _ = state;
        todo!()
    }
}
