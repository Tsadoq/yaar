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
    let mut reviews = reviews;
    if reviews.is_empty() {
        reviews.push(Review::new(new_id(), workspace.clone(), None));
    }
    reviews
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use yaar_store::{Anchor, CommentId, Side, ThreadId};

    fn id(n: u32) -> ReviewId {
        ReviewId::new(format!("{n:032x}"))
    }

    // REV-17A
    #[test]
    fn a_workspace_with_no_review_has_one_of_no_agent_and_one_that_has_it_keeps_it() {
        let w = WorkspaceId::new("w");
        let calls = Cell::new(0);
        let made = with_review_of_no_agent(&w, vec![], || {
            calls.set(calls.get() + 1);
            id(7)
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].id(), &id(7));
        assert_eq!(made[0].workspace(), &w);
        assert_eq!(made[0].agent(), None);

        // Given back, with a thread in it, it comes back unchanged and draws no id.
        let mut review = made.into_iter().next().expect("the review");
        review
            .start_thread(
                ThreadId::new(format!("{:032x}", 1)),
                CommentId::new(format!("{:032x}", 2)),
                Anchor {
                    tree: "S1".to_string(),
                    path: "a.rs".into(),
                    side: Side::New,
                    first: 1,
                    last: 1,
                    text: "x".to_string(),
                },
                "a draft",
            )
            .expect("start a thread");
        let again = with_review_of_no_agent(&w, vec![review.clone()], || {
            panic!("a workspace that has its review draws no id")
        });
        assert_eq!(again, vec![review]);
    }

    #[test]
    fn a_workspace_that_has_the_reviews_of_agent_sessions_gets_no_other_review() {
        let w = WorkspaceId::new("w");
        let reviews = vec![
            Review::new(id(1), w.clone(), Some("session one".to_string())),
            Review::new(id(2), w.clone(), Some("session two".to_string())),
        ];
        let again = with_review_of_no_agent(&w, reviews.clone(), || {
            panic!("a workspace with a review of an agent session draws no id")
        });
        assert_eq!(again, reviews);
    }
}
