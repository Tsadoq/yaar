//! Which review a session that starts takes: the rule of a review per agent session.

use yaar_store::{ReviewId, SessionRecord};

use crate::review::Review;

/// What a session that starts does with the reviews of its workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Takeover {
    /// The session keeps the review it held before: its CLI resumed it.
    Keeps(ReviewId),
    /// The session takes over this review: that of no agent, or one whose session of the
    /// same CLI has ended.
    TakesOver(ReviewId),
    /// The session gets a review of its own, new.
    New,
}

/// What `session`, which starts now in its workspace, does with that workspace's
/// `reviews`, given every session of the workspace as the Store holds them, `session`
/// among them or not.
///
/// In this order:
/// 1. a review that belongs to `session` already, as when its CLI resumes it: it keeps
///    it;
/// 2. a review whose session ended in the terminal `session` starts in and was of the
///    same CLI, as after a clear or a new start of the CLI there: it takes it over;
/// 3. the review of no agent: the first session of a supported CLI takes it over;
/// 4. a review whose session was of the same CLI and has ended, the one that ended last:
///    the next session of that CLI in the worktree takes it over;
/// 5. else a new review: two sessions that run at once in one worktree have a review
///    each.
///
/// A review whose session runs is never taken over, and a review whose session was of
/// another CLI is never taken over by this one.
pub fn takeover(
    reviews: &[Review],
    sessions: &[SessionRecord],
    session: &SessionRecord,
) -> Takeover {
    let _ = (reviews, sessions, session);
    todo!("which review a session that starts takes")
}
