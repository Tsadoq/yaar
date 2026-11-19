//! Which uncommitted changes a review lists: those of its session and those no agent
//! made.

use std::path::PathBuf;

use crate::review::Review;

/// One uncommitted change of a worktree with who made it, as Workspaces credits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credited {
    /// The path, relative to the folder of the worktree.
    pub path: PathBuf,
    /// The ids of the sessions it is credited to; empty for a change no agent made.
    pub by: Vec<String>,
}

/// The uncommitted changes `review` lists under uncommitted changes, in the order of
/// `changes`: every change no agent made, in every review of the worktree, while it is
/// uncommitted; and those credited to the review's own session. The review of no agent
/// lists them all.
pub fn uncommitted_of<'a>(review: &Review, changes: &'a [Credited]) -> Vec<&'a Credited> {
    let _ = (review, changes);
    todo!("the uncommitted changes a review lists")
}
