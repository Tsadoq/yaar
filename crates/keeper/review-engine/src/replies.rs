//! The agent's replies to threads, and what only the reviewer does: resolving a thread,
//! reopening one, and resolving every addressed thread at once. Built by the engine's
//! package "replies and the states of a thread" of slice 6.

use yaar_store::{CommentId, ReviewId, SessionId, ThreadId};

use crate::engine::{EngineError, Reviews};

/// A reply of the agent to one thread, as its reply tool gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReply {
    /// The thread, by its id.
    pub thread: ThreadId,
    /// The reply, byte for byte.
    pub text: String,
    /// Whether the agent says it addressed the thread.
    pub addressed: bool,
}

impl Reviews {
    /// Keeps a reply of the agent of `session` in the review of that session, by
    /// [`crate::Review::agent_reply`], with a comment id the engine draws, in one
    /// transaction of the Store; gives the review and the comment. Refused, with nothing
    /// kept, as `agent_reply` refuses, a thread of another session's review among them,
    /// and with [`EngineError::NoReviewOfSession`] for a session that has no review.
    pub fn agent_replied(
        &self,
        session: &SessionId,
        reply: &ThreadReply,
    ) -> Result<(ReviewId, CommentId), EngineError> {
        let _ = (session, reply);
        todo!()
    }

    /// Resolves a thread of `review`, and keeps it.
    pub fn resolve(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _ = (review, thread);
        todo!()
    }

    /// Reopens a resolved thread of `review`, and keeps it.
    pub fn reopen(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _ = (review, thread);
        todo!()
    }

    /// Resolves every addressed thread of `review` that is not resolved, by
    /// [`crate::Review::resolve_addressed`], in one transaction of the Store; gives them.
    pub fn resolve_addressed(&self, review: &ReviewId) -> Result<Vec<ThreadId>, EngineError> {
        let _ = review;
        todo!()
    }
}
