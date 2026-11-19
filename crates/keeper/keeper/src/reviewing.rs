//! What the user does to a review of slice 6, and what a Window reads of it: attaching a
//! review or one thread alone, with the offer to start its agent; detaching it; resolving a
//! thread, reopening one, and resolving every addressed thread; and the text an attached
//! review would carry. Also how a thread of the engine is told to a Window: its state and
//! where it sits now.
//!
//! Each request is answered in the order of the Window's reviews, on its lane of them,
//! through `Kept`: the engine keeps every change, and each change is told to every Window
//! as `Fact::ReviewsChanged`; one that changes what is attached is also handed to the
//! agents' side, which tells the sessions of the workspace again so that the agent's
//! terminal shows it.

use yaar_link::{Fact, Refusal, Reply, ReviewId, ThreadId};

use crate::agents::Agents;
use crate::wiring::Kept;

/// Attaches `review`, or `thread` alone, through the engine, with the sessions of its
/// workspace as the agents' side knows them; answers with the CLIs to offer: none while
/// its agent runs, its agent's CLI when it does not, every supported CLI
/// (`Agents::supported`) for the review of no agent.
pub(crate) fn attach(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
    thread: Option<&ThreadId>,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, agents, review, thread, facts);
    todo!()
}

/// Takes the attachment of `review` away.
pub(crate) fn detach(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, agents, review, facts);
    todo!()
}

/// Resolves a thread, or reopens it when `resolved` is false.
pub(crate) fn resolve(
    kept: &Kept,
    review: &ReviewId,
    thread: &ThreadId,
    resolved: bool,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, review, thread, resolved, facts);
    todo!()
}

/// Resolves every addressed thread of `review` that is not resolved.
pub(crate) fn resolve_addressed(
    kept: &Kept,
    review: &ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, review, facts);
    todo!()
}

/// What the next message would carry to the agent of `review`: the engine's
/// `Reviews::attached_text` after the instruction of the delivery module of that agent's
/// CLI (`Agents::instruction`), which is the text that agent receives.
pub(crate) fn attachment(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
) -> Result<Reply, Refusal> {
    let _ = (kept, agents, review);
    todo!()
}

/// The state of a thread as a Window is told it: the engine's
/// `yaar_review_engine::Review::state_of`, in the Link's words. Until the package of slice
/// 6 that builds the Keeper's side of reviews fills it in, every thread is told as sent,
/// which no Window of slices 1 to 5 reads.
pub(crate) fn thread_state(
    review: &yaar_review_engine::Review,
    thread: &yaar_review_engine::ThreadId,
) -> yaar_link::ThreadState {
    let _ = (review, thread);
    yaar_link::ThreadState::Sent
}
