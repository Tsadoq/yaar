//! Lines count from 1, as git and editors count them, and a range holds both its ends.

use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::WorkspaceId;

macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id! {
    ReviewId
}

opaque_id! {
    /// The Window draws the id of a thread it starts, so every save of its first comment names the
    /// same thread.
    ThreadId
}

opaque_id! {
    /// The Window draws the id of a draft it starts, so every save of the draft names the same
    /// comment.
    CommentId
}

opaque_id! {
    /// The app's name for the session, not the CLI's own: a session the CLI resumes keeps it.
    SessionId
}

opaque_id! {
    /// A turn runs from the user's message to the agent's last word in answer to it.
    TurnId
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    /// Added lines and unchanged lines are on this side.
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lines {
    pub first: u32,
    /// At least `first`.
    pub last: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// Full id of the commit or recorded state the lines are read in, never a git tree object. None
    /// for a draft that starts a thread on files on disk (the Keeper records the state then), and
    /// for a file outside the worktree.
    pub tree: Option<String>,
    /// Relative to the worktree folder; on the old side of a renamed file, the old path. An
    /// absolute path names a file outside the worktree or one git ignores: such a thread has no
    /// version, sits on the new side and is never carried through a diff.
    pub path: PathBuf,
    pub side: Side,
    pub lines: Lines,
    /// Joined by line feeds, no line end after the last.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOn {
    /// The draft is the first comment of a new thread at `anchor`. Every save repeats the same
    /// anchor; the Keeper makes the thread at the first.
    NewThread {
        anchor: Anchor,
    },
    Thread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    Reviewer,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: CommentId,
    pub author: Author,
    pub text: String,
    /// None while it is a draft. A comment of the agent is never a draft.
    pub round: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: ThreadId,
    /// Kept as written when the code changes later.
    pub anchor: Anchor,
    /// A resolved thread is sent no more, an unsent comment on it included.
    pub resolved: bool,
    /// Whether the agent's last reply marked the thread addressed. Only the reviewer resolves.
    pub addressed: bool,
    /// Oldest first; never empty.
    pub comments: Vec<Comment>,
    /// Worked out from the fields above by the one rule of [`ThreadState`].
    pub state: ThreadState,
    /// None while it sits where its anchor says, and for a file outside the worktree. On the old
    /// side it names a commit: an open thread placed at the commit the changes count from is placed
    /// at the new commit once that moved.
    pub now: Option<Placed>,
}

/// Worked out in this order, the first that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadState {
    Resolved,
    /// Holds a reviewer comment not yet sent: a new thread, or a follow-up after the last send.
    Draft,
    /// The agent's last reply marked it addressed; it waits for the reviewer to resolve or answer.
    Addressed,
    /// The agent replied last, without the addressed mark.
    WaitsForReviewer,
    /// Its comments were sent and the agent has not replied since.
    Sent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Full id of the recorded state it was carried to, or on the old side the commit the changes
    /// count from.
    pub tree: String,
    /// The file's path in that version: a renamed file's path now. A deleted file keeps its path.
    pub path: PathBuf,
    pub side: Side,
    /// The lines that replaced those it was written on; where those were deleted, the one line at
    /// the point of deletion.
    pub lines: Lines,
    /// None when it only moved or did not change. Once said, it stays said in later states.
    pub notice: Option<Notice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    /// The lines it was written on were rewritten, or a line was put inside them: the thread sits
    /// on the lines that replaced them.
    Changed,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentBy {
    Copied,
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// 1 for the first send of the review, one more for each later one.
    pub number: u32,
    /// Full id of the state recorded at the send; None until recorded. A delivered review is handed
    /// out first and its state recorded after; a copy records it first.
    pub state: Option<String>,
    pub sent_by: SentBy,
    /// Set on the newest round only, at the agent's first edit or write after the send.
    pub edited: bool,
    /// The turn the round's message started or, for a message the CLI queued, the turn that ran.
    /// None for a copy and for rounds kept by Store version 5 or older. While that turn runs, what
    /// changed since the round cannot be counted.
    pub turn: Option<TurnId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Every unsent comment on an unresolved thread, those written after attaching included.
    Review,
    Thread(ThreadId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub id: ReviewId,
    pub agent: Option<SessionId>,
    /// Full id of the state last marked reviewed; None before the first mark. What changed since
    /// the last review counts from it.
    pub reviewed: Option<String>,
    pub attached: Option<Attachment>,
    pub threads: Vec<Thread>,
    pub rounds: Vec<Round>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceReviews {
    pub workspace: WorkspaceId,
    /// At least one: the review of no agent when no agent session has one.
    pub reviews: Vec<Review>,
    /// Files marked viewed whose content has not changed since, relative to the worktree folder,
    /// ordered by bytes. Empty while the worktree is gone.
    pub viewed: Vec<PathBuf>,
}

pub(crate) fn write_lines(out: &mut Out, lines: Lines) {
    out.u32(lines.first);
    out.u32(lines.last);
}

pub(crate) fn read_lines(input: &mut In<'_>) -> Result<Lines, LinkError> {
    Ok(Lines {
        first: input.u32()?,
        last: input.u32()?,
    })
}

fn write_side(out: &mut Out, side: Side) {
    out.u8(match side {
        Side::Old => 0,
        Side::New => 1,
    });
}

fn read_side(input: &mut In<'_>) -> Result<Side, LinkError> {
    match input.u8()? {
        0 => Ok(Side::Old),
        1 => Ok(Side::New),
        _ => Err(LinkError::Malformed("unknown side of a diff")),
    }
}

pub(crate) fn write_anchor(out: &mut Out, anchor: &Anchor) {
    out.optional(anchor.tree.as_deref(), Out::text);
    out.path(&anchor.path);
    write_side(out, anchor.side);
    write_lines(out, anchor.lines);
    out.text(&anchor.text);
}

pub(crate) fn read_anchor(input: &mut In<'_>) -> Result<Anchor, LinkError> {
    Ok(Anchor {
        tree: input.optional(In::text)?,
        path: input.path()?,
        side: read_side(input)?,
        lines: read_lines(input)?,
        text: input.text()?,
    })
}

pub(crate) fn write_draft_on(out: &mut Out, on: &DraftOn) {
    match on {
        DraftOn::NewThread { anchor } => {
            out.u8(0);
            write_anchor(out, anchor);
        }
        DraftOn::Thread => out.u8(1),
    }
}

pub(crate) fn read_draft_on(input: &mut In<'_>) -> Result<DraftOn, LinkError> {
    match input.u8()? {
        0 => Ok(DraftOn::NewThread {
            anchor: read_anchor(input)?,
        }),
        1 => Ok(DraftOn::Thread),
        _ => Err(LinkError::Malformed("a draft belongs to no known place")),
    }
}

fn write_comment(out: &mut Out, comment: &Comment) {
    out.text(comment.id.as_str());
    out.u8(match comment.author {
        Author::Reviewer => 0,
        Author::Agent => 1,
    });
    out.text(&comment.text);
    out.optional(comment.round.as_ref(), |out, round| out.u32(*round));
}

fn read_comment(input: &mut In<'_>) -> Result<Comment, LinkError> {
    Ok(Comment {
        id: CommentId::new(input.text()?),
        author: match input.u8()? {
            0 => Author::Reviewer,
            1 => Author::Agent,
            _ => return Err(LinkError::Malformed("a comment by nobody known")),
        },
        text: input.text()?,
        round: input.optional(In::u32)?,
    })
}

fn write_thread(out: &mut Out, thread: &Thread) {
    out.text(thread.id.as_str());
    write_anchor(out, &thread.anchor);
    out.flag(thread.resolved);
    out.flag(thread.addressed);
    out.list(&thread.comments, write_comment);
    write_thread_state(out, thread.state);
    out.optional(thread.now.as_ref(), write_placed);
}

fn write_thread_state(out: &mut Out, state: ThreadState) {
    out.u8(match state {
        ThreadState::Resolved => 0,
        ThreadState::Draft => 1,
        ThreadState::Addressed => 2,
        ThreadState::WaitsForReviewer => 3,
        ThreadState::Sent => 4,
    });
}

fn read_thread_state(input: &mut In<'_>) -> Result<ThreadState, LinkError> {
    Ok(match input.u8()? {
        0 => ThreadState::Resolved,
        1 => ThreadState::Draft,
        2 => ThreadState::Addressed,
        3 => ThreadState::WaitsForReviewer,
        4 => ThreadState::Sent,
        _ => return Err(LinkError::Malformed("an unknown state of a thread")),
    })
}

fn write_placed(out: &mut Out, placed: &Placed) {
    out.text(&placed.tree);
    out.path(&placed.path);
    write_side(out, placed.side);
    write_lines(out, placed.lines);
    out.optional(placed.notice.as_ref(), |out, notice| {
        out.u8(match notice {
            Notice::Changed => 0,
            Notice::Deleted => 1,
        })
    });
}

fn read_placed(input: &mut In<'_>) -> Result<Placed, LinkError> {
    Ok(Placed {
        tree: input.text()?,
        path: input.path()?,
        side: read_side(input)?,
        lines: read_lines(input)?,
        notice: input.optional(|input| match input.u8()? {
            0 => Ok(Notice::Changed),
            1 => Ok(Notice::Deleted),
            _ => Err(LinkError::Malformed("an unknown notice of a thread")),
        })?,
    })
}

fn read_thread(input: &mut In<'_>) -> Result<Thread, LinkError> {
    Ok(Thread {
        id: ThreadId::new(input.text()?),
        anchor: read_anchor(input)?,
        resolved: input.flag()?,
        addressed: input.flag()?,
        comments: input.list(read_comment)?,
        state: read_thread_state(input)?,
        now: input.optional(read_placed)?,
    })
}

fn write_round(out: &mut Out, round: &Round) {
    out.u32(round.number);
    out.optional(round.state.as_deref(), Out::text);
    out.u8(match round.sent_by {
        SentBy::Copied => 0,
        SentBy::Delivered => 1,
    });
    out.flag(round.edited);
    out.optional(round.turn.as_ref(), |out, turn| out.text(turn.as_str()));
}

fn read_round(input: &mut In<'_>) -> Result<Round, LinkError> {
    Ok(Round {
        number: input.u32()?,
        state: input.optional(In::text)?,
        sent_by: match input.u8()? {
            0 => SentBy::Copied,
            1 => SentBy::Delivered,
            _ => return Err(LinkError::Malformed("a round sent in an unknown way")),
        },
        edited: input.flag()?,
        turn: input.optional(|input| input.text().map(TurnId::new))?,
    })
}

pub(crate) fn write_attachment(out: &mut Out, attachment: &Attachment) {
    match attachment {
        Attachment::Review => out.u8(0),
        Attachment::Thread(thread) => {
            out.u8(1);
            out.text(thread.as_str());
        }
    }
}

pub(crate) fn read_attachment(input: &mut In<'_>) -> Result<Attachment, LinkError> {
    match input.u8()? {
        0 => Ok(Attachment::Review),
        1 => Ok(Attachment::Thread(ThreadId::new(input.text()?))),
        _ => Err(LinkError::Malformed("an unknown kind of attachment")),
    }
}

fn write_review(out: &mut Out, review: &Review) {
    out.text(review.id.as_str());
    out.optional(review.agent.as_ref(), |out, session| {
        out.text(session.as_str())
    });
    out.optional(review.reviewed.as_deref(), Out::text);
    out.optional(review.attached.as_ref(), write_attachment);
    out.list(&review.threads, write_thread);
    out.list(&review.rounds, write_round);
}

fn read_review(input: &mut In<'_>) -> Result<Review, LinkError> {
    Ok(Review {
        id: ReviewId::new(input.text()?),
        agent: input.optional(|input| Ok(SessionId::new(input.text()?)))?,
        reviewed: input.optional(In::text)?,
        attached: input.optional(read_attachment)?,
        threads: input.list(read_thread)?,
        rounds: input.list(read_round)?,
    })
}

pub(crate) fn write_workspace_reviews(out: &mut Out, reviews: &WorkspaceReviews) {
    out.text(reviews.workspace.as_str());
    out.list(&reviews.reviews, write_review);
    out.list(&reviews.viewed, |out, path| out.path(path));
}

pub(crate) fn read_workspace_reviews(input: &mut In<'_>) -> Result<WorkspaceReviews, LinkError> {
    Ok(WorkspaceReviews {
        workspace: WorkspaceId::new(input.text()?),
        reviews: input.list(read_review)?,
        viewed: input.list(In::path)?,
    })
}
