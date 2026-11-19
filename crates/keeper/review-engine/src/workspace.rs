//! Which reviews a workspace has.

use yaar_store::{ReviewId, WorkspaceId};

use crate::review::Review;

/// The reviews of `workspace`, given those it has: a workspace with no review of an agent
/// session has exactly one review, which belongs to no agent.
///
/// When `reviews` holds no review of an agent session and none of no agent, one review of
/// no agent is added, with the id `new_id` draws, and nothing else changes; `new_id` is
/// called then and only then. When it holds one of no agent, it is given back as it is.
/// The reviews of agent sessions, and what becomes of the review of no agent when a session
/// starts, are a later slice's: `reviews` is given back as it is when it holds one.
pub fn with_review_of_no_agent(
    workspace: &WorkspaceId,
    reviews: Vec<Review>,
    new_id: impl FnOnce() -> ReviewId,
) -> Vec<Review> {
    let _ = (workspace, reviews, new_id);
    todo!()
}
