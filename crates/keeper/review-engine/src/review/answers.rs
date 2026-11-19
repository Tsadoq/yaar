//! The agent's replies, the states of a thread, and resolving every addressed thread at
//! once. Built by the engine's package "replies and the states of a thread" of slice 6.

use yaar_store::{CommentId, SessionId, ThreadId};

use super::{Review, ReviewError};

/// The state of a thread, as the review pane lists it: the first of these that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadState {
    /// The reviewer resolved it.
    Resolved,
    /// It holds a comment of the reviewer that was not sent.
    Draft,
    /// The agent's last reply marked it addressed.
    Addressed,
    /// The agent replied last, without the addressed mark: it waits for the reviewer.
    WaitsForReviewer,
    /// Its comments were sent, and the agent has not replied since.
    Sent,
}

impl Review {
    /// The state of a thread, by [`ThreadState`]'s order; none for a thread the review
    /// does not hold.
    pub fn state_of(&self, thread: &ThreadId) -> Option<ThreadState> {
        let _ = thread;
        todo!()
    }

    /// Keeps a reply of the agent of `session` in a thread: a comment of the agent, sent
    /// in the newest round, and the thread addressed or not as the reply says. A reply
    /// without the mark takes an earlier mark away: the thread waits for the reviewer. A
    /// reply never resolves a thread, and leaves a resolved one resolved. Refused with
    /// [`ReviewError::OtherSession`] when the review does not belong to `session`,
    /// [`ReviewError::UnknownThread`] for a thread the review does not hold, a thread of
    /// another session's review among them, [`ReviewError::NothingSent`] when no round was
    /// sent, and [`ReviewError::TakenId`]; nothing changes then.
    pub fn agent_reply(
        &mut self,
        session: &SessionId,
        thread: &ThreadId,
        comment: CommentId,
        text: &str,
        addressed: bool,
    ) -> Result<(), ReviewError> {
        let _ = (session, thread, comment, text, addressed);
        todo!()
    }

    /// Resolves every thread the agent marked addressed that is not resolved, and gives
    /// them, in the order of the threads. The reviewer's action: nothing else resolves.
    pub fn resolve_addressed(&mut self) -> Vec<ThreadId> {
        todo!()
    }
}
