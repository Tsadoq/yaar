//! What the two programs say about reviews: the reviews of a workspace with their threads,
//! comments and rounds, where a comment sits, and the files marked viewed.
//!
//! A review is the Keeper's: it keeps every review in its database, and a Window reads
//! them with [`crate::Request::GetReviews`] and changes them through the requests that
//! name a review. A draft is saved as it is typed, with [`crate::Request::SaveDraft`],
//! which carries the whole text each time.
//!
//! Lines are counted from 1, as git and editors count them, and a range of lines holds
//! both its ends.

use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::WorkspaceId;

/// Declares an opaque id of the Link: a text the other side treats as a name.
macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

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

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id! {
    /// Names one review, on the Link and across restarts of both programs: 32 lowercase
    /// hexadecimal digits the Keeper drew when it made the review.
    ReviewId
}

opaque_id! {
    /// Names one thread of a review: 32 lowercase hexadecimal digits. The Window draws the
    /// id of a thread it starts, before the first save of its first comment, so that every
    /// save of that comment names the same thread; an agent's reply names it too.
    ThreadId
}

opaque_id! {
    /// Names one comment of a thread: 32 lowercase hexadecimal digits. The Window draws
    /// the id of a draft it starts, so that every save of the draft, from the first
    /// keystroke on, names the same comment; the Keeper draws those of an agent's replies.
    CommentId
}

opaque_id! {
    /// Names one session of an agent CLI, as the Keeper knows it: 32 lowercase hexadecimal
    /// digits the Keeper drew when the session first reported in. It is the app's name for
    /// the session, not the CLI's own: a session the CLI resumes keeps it. A review belongs
    /// to one session, or to no agent.
    SessionId
}

opaque_id! {
    /// Names one turn of an agent session: 32 lowercase hexadecimal digits the Keeper drew
    /// when the turn started. A turn runs from the user's message to the agent's last word
    /// in answer to it.
    TurnId
}

/// The side of a diff a line is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The version the change is compared with: a removed line is on this side.
    Old,
    /// The version the change made: an added line, and an unchanged line, are on this
    /// side.
    New,
}

/// A range of lines of one version of a file, counted from 1, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lines {
    /// The first line.
    pub first: u32,
    /// The last line; at least `first`.
    pub last: u32,
}

/// Where a thread sits: lines of one version of one file, and their text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The version the lines are read in: the full id of a commit, a recorded state of the
    /// worktree or a commit of the repository, never the id of a git tree object. In a
    /// [`crate::Request::SaveDraft`] that starts a thread it is none for lines of the files
    /// on disk: the Keeper records the state of the worktree then and puts the thread on
    /// it. In what the Keeper sends it is some, but for a thread on a file outside the
    /// worktree, whose path is absolute: that thread has no version.
    pub tree: Option<String>,
    /// The path of the file in that version, relative to the folder of the worktree. On
    /// the old side of a renamed file it is the old path, where the file lies in the
    /// version the lines are read in; on the new side it is the path now.
    ///
    /// An absolute path names a file outside the worktree, or one inside it that git
    /// ignores, which the reviewer reached through go to definition or a reference: a
    /// library's source, a dependency's types. Such a thread has no version (`tree` is
    /// none, in a save and in what the Keeper sends), sits on the new side, and keeps its
    /// path, lines and text: no state is recorded for it, it is never carried through a
    /// diff, and the Window shows it on its lines while their text is still the text kept.
    pub path: PathBuf,
    /// The side of the diff the lines were on where the comment was written.
    pub side: Side,
    /// The lines.
    pub lines: Lines,
    /// The text of those lines as the reviewer saw them, joined by line feeds, with no
    /// line end after the last.
    pub text: String,
}

/// What the draft of a [`crate::Request::SaveDraft`] belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOn {
    /// The draft is the first comment of a new thread, which sits here. Every save of the
    /// draft says so again, with the same anchor; the Keeper makes the thread at the
    /// first.
    NewThread {
        /// Where the thread sits.
        anchor: Anchor,
    },
    /// The draft is a comment of a thread that exists: an edit of a draft, or a reply.
    Thread,
}

/// Who wrote a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    /// The reviewer, in the app.
    Reviewer,
    /// The agent the review belongs to, through the reply tool.
    Agent,
}

/// One comment of a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Its id.
    pub id: CommentId,
    /// Who wrote it.
    pub author: Author,
    /// Its text, as written.
    pub text: String,
    /// The number of the round it was sent in; none while it is a draft. A comment of the
    /// agent is never a draft.
    pub round: Option<u32>,
}

/// One thread of a review: comments on one anchor, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// Its id.
    pub id: ThreadId,
    /// Where its first comment was written, in the version it was written in. It is kept
    /// as written when the code changes later.
    pub anchor: Anchor,
    /// Whether the reviewer resolved it. A resolved thread is sent no more, an unsent
    /// comment on it included.
    pub resolved: bool,
    /// Whether the agent said it addressed it, in its last reply. Only the reviewer
    /// resolves.
    pub addressed: bool,
    /// Its comments, oldest first; at least one.
    pub comments: Vec<Comment>,
    /// Its state, as the Keeper works it out from the fields above, by the one rule of
    /// [`ThreadState`]: what the review pane lists for it.
    pub state: ThreadState,
    /// Where it sits in the newest recorded state of the worktree the Keeper carried it
    /// to, with what that state did to the lines it was written on; none while it sits
    /// where its anchor says, as before the first state after it, and for a thread the
    /// Keeper does not carry: one on the old side, or on a file outside the worktree.
    pub now: Option<Placed>,
}

/// The state of a thread, as the review pane lists it. Worked out in this order, the first
/// that holds:
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadState {
    /// The reviewer resolved it. It is sent no more.
    Resolved,
    /// It holds a comment of the reviewer that was not sent: a new thread, or a follow-up
    /// written after the last send.
    Draft,
    /// The agent's last reply marked it addressed. It waits for the reviewer to resolve it,
    /// or to answer.
    Addressed,
    /// The agent replied last, without the addressed mark: it waits for the reviewer's
    /// answer.
    WaitsForReviewer,
    /// Its comments were sent, and the agent has not replied since.
    Sent,
}

/// Where a thread sits now, after the code it was written on changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// The version it sits in now: the full id of the recorded state it was carried to.
    pub tree: String,
    /// The path of the file in that version: the path a renamed file has now. A file that
    /// was deleted keeps the path it had.
    pub path: PathBuf,
    /// The side, as the anchor's.
    pub side: Side,
    /// The lines it sits on in that version: the lines that replaced the ones it was
    /// written on, or, where they were deleted, the one line at the point of deletion.
    pub lines: Lines,
    /// What the code it was written on became; none when it only moved, or did not
    /// change at all. Once said, it stays said in later states.
    pub notice: Option<Notice>,
}

/// What became of the lines a thread was written on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    /// They were rewritten, or a line was put inside them: the thread sits on the lines
    /// that replaced them. The lines it was written on are the anchor's text.
    Changed,
    /// They were deleted, or their file was: the thread sits at the point of deletion.
    Deleted,
}

/// How the comments of a round left the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentBy {
    /// The reviewer copied the review as text, to paste it by hand.
    Copied,
    /// The review went with a message of the user to the agent.
    Delivered,
}

/// One round of a review: one send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// Its number: 1 for the first send of the review, one more for each later one.
    pub number: u32,
    /// The state of the worktree recorded at the send, by its full id; none until it is
    /// recorded. A delivered review is handed to the agent at once and its state is
    /// recorded after, off the agent's way; a copy records it first.
    pub state: Option<String>,
    /// How the comments left.
    pub sent_by: SentBy,
    /// Whether the agent the review belongs to reported an edit, with its edit or write
    /// tool, since the comments were sent: set on the newest round only, at the first such
    /// edit. Until then the review pane shows the round as sent.
    pub edited: bool,
    /// The turn of the review's session the round was sent in: for a review delivered with
    /// a message, the turn that message started or, for a message the CLI queued, the turn
    /// that ran. None for a copy, and for a round kept before the Keeper kept the turn of
    /// each round (a Store of version 5 or older). While that turn runs, the newest state
    /// of the history is older than the round's own, and what changed since the round
    /// cannot be counted to it.
    pub turn: Option<TurnId>,
}

/// What of a review goes with the user's next message to its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Every unsent comment on an unresolved thread, those written after the attaching
    /// included.
    Review,
    /// The unsent comments of this one thread alone.
    Thread(ThreadId),
}

/// One review: threads written for one agent session in one worktree, or for no agent.
/// A review has no votes and no labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// Its id.
    pub id: ReviewId,
    /// The agent session it belongs to; none for the review of no agent.
    pub agent: Option<SessionId>,
    /// The state of the worktree the reviewer last marked reviewed, by its full id;
    /// none before the first mark. What changed since the last review counts from it.
    pub reviewed: Option<String>,
    /// What is attached to the user's next message; none while nothing is.
    pub attached: Option<Attachment>,
    /// Its threads, in the order they were started.
    pub threads: Vec<Thread>,
    /// Its rounds, the first first.
    pub rounds: Vec<Round>,
}

/// The reviews of one workspace, and its files marked viewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceReviews {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// Its reviews: at least one, the review of no agent when no agent session has one.
    pub reviews: Vec<Review>,
    /// The paths of the files marked viewed whose content has not changed since, relative
    /// to the folder of the worktree, ordered by their bytes. Empty while the worktree is
    /// gone.
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

fn write_flag(out: &mut Out, flag: bool) {
    out.u8(u8::from(flag));
}

fn read_flag(input: &mut In<'_>) -> Result<bool, LinkError> {
    match input.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(LinkError::Malformed("a flag is neither 0 nor 1")),
    }
}

fn write_thread(out: &mut Out, thread: &Thread) {
    out.text(thread.id.as_str());
    write_anchor(out, &thread.anchor);
    write_flag(out, thread.resolved);
    write_flag(out, thread.addressed);
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
        resolved: read_flag(input)?,
        addressed: read_flag(input)?,
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
    write_flag(out, round.edited);
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
        edited: read_flag(input)?,
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
