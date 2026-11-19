//! Review engine (`H-REVIEW`).
//!
//! Owns: reviews, threads, comments, drafts, rounds, turns, attachment, sent and resolved states, thread following, the review rendered as text.
//!
//! Surface:
//! - [`Review`], one review and its rules: a comment is a draft until it is sent, and a
//!   draft can be edited and deleted; a sent comment carries its round and refuses both; a
//!   follow-up is a draft reply in the same thread; a resolved thread is sent no more; what
//!   is attached to the next message, and what a send carries; the reviewed state. It
//!   holds no file and starts no process;
//! - [`as_text`], the review rendered as text, with the [`COPY_INSTRUCTION`] that goes with
//!   a review copied by hand;
//! - [`ViewedFiles`], the files marked viewed, each mark holding for the content it was
//!   given;
//! - [`with_review_of_no_agent`], the rule that a workspace with no review of an agent
//!   session has one review, of no agent;
//! - [`Reviews`], the engine over the Store and the Recorder: what the Keeper calls. It
//!   keeps every change in the Store and records the state of the worktree where a review
//!   needs one: at a send, at a mark of reviewed, and for a new thread on the files on
//!   disk;
//! - [`takeover`], the rule of a review per agent session: which review a session that
//!   starts keeps, takes over, or gets new, as a [`Takeover`];
//! - [`uncommitted_of`], the uncommitted changes a review lists: those of its session and
//!   those no agent made, each a [`Credited`];
//! - [`Reviews::session_started`] and [`Reviews::turn_ended`], what the Keeper calls when a
//!   session starts and when a turn ends, with [`TurnEnded`]; [`StateWatch`], by which
//!   the engine records a state for a new thread only when files changed;
//! - the records it reads and writes, from the Store: [`Anchor`], [`Thread`], [`Comment`],
//!   [`Round`], [`SessionRecord`] and their ids.
//!
//! It knows nothing of what kind of change a tree is: a recorded state and a commit are
//! both a tree, named by an id, and only the Recorder tells them apart. It holds no
//! interface code and opens no database: the Store keeps, the Recorder records.

mod engine;
mod review;
mod takeover;
mod text;
mod uncommitted;
mod viewed;
mod workspace;

pub use engine::{
    Copied, DraftOn, DraftSave, EngineError, NewAnchor, Reviews, Saved, StateWatch, TurnEnded,
    WorkspaceView,
};
pub use review::{Attachment, Review, ReviewError, Scope, Sent, Unsent};
pub use takeover::{Takeover, takeover};
pub use text::{COPY_INSTRUCTION, ReviewText, as_text};
pub use uncommitted::{Credited, uncommitted_of};
pub use viewed::{ViewedFiles, fingerprint};
pub use workspace::with_review_of_no_agent;
pub use yaar_store::{
    Anchor, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, SessionId,
    SessionRecord, Side, Thread, ThreadId, WorkspaceId,
};
