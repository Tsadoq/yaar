//! A tree is a recorded state or a commit, named by an id; only the Recorder tells them apart.
//! [`Review`] holds the rules and no file; [`Reviews`] is the engine over the Store and the
//! Recorder.

mod attaching;
mod base_moves;
mod credits;
mod engine;
mod expiry;
pub mod follow;
mod following;
mod replies;
mod review;
mod rounds;
mod sending;
mod takeover;
mod text;
mod uncommitted;
mod viewed;
mod workspace;

pub use credits::CreditedChange;
pub use expiry::{Expired, KEEP_STATES, Presence, StoreRoom, TurnMark};
pub use follow::{FileStep, MOVED_BLOCK, Notice, Place, Placed, carry, carry_file};
pub use replies::ThreadReply;
pub use review::{Offer, SessionNow, ThreadState};

pub use engine::{
    Copied, DraftOn, DraftSave, EngineError, NewAnchor, Reviews, Saved, StateWatch, TurnEnded,
    WorkspaceView,
};
pub use review::{Attachment, Review, ReviewError, Scope, Sent, Unsent};
pub use takeover::{Takeover, takeover};
pub use text::{ReviewText, as_text};
pub use uncommitted::{Credited, uncommitted_of};
pub use viewed::{ViewedFiles, fingerprint};
pub use workspace::with_review_of_no_agent;
pub use yaar_store::{
    Anchor, AttachedRecord, Author, Comment, CommentId, NoticeRecord, ReviewId, ReviewRecord,
    Round, SentBy, SessionId, SessionRecord, Side, Thread, ThreadId, ThreadPlace, WorkspaceId,
};
