//! Who is credited with each path, kept with each recorded state, and what one review shows
//! of a change between two versions: the paths of its session and those no agent made, a
//! path two sessions changed whole in both reviews, marked. Built by the engine's package
//! "credits with recorded states" of slice 6.

use std::path::PathBuf;

use yaar_recorder::Change;
use yaar_store::{ReviewId, WorkspaceId};

use crate::engine::{EngineError, Reviews};
use crate::uncommitted::Credited;

/// One path of a change between two versions as a review shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedChange {
    /// The path in the newer version, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it between the two versions.
    pub change: Change,
    /// The sessions credited with it in any state after the older version up to the newer
    /// one, by their ids, in the order first credited; empty for a change no agent made.
    pub by: Vec<String>,
    /// Whether a session other than the review's own changed it too: the review shows the
    /// file whole all the same, and marks it.
    pub shared: bool,
}

impl Reviews {
    /// Keeps, with the recorded state `state` of `workspace`, who is credited with each
    /// path in `credits`: the paths that changed into it, as Workspaces credited them when
    /// it was recorded. What was kept with that state before is replaced.
    pub fn keep_credits(
        &self,
        workspace: &WorkspaceId,
        state: &str,
        credits: &[Credited],
    ) -> Result<(), EngineError> {
        let _ = (workspace, state, credits);
        todo!()
    }

    /// The paths that differ from version `from` to the recorded state `to` that `review`
    /// shows, in the order of the paths: those credited to its session in the states
    /// recorded after `from` up to `to`, those no agent made, and for the review of no
    /// agent every one. A path credited to its session and to another is in it whole, with
    /// [`CreditedChange::shared`]; the engine never divides the changes of one file.
    pub fn credited_change(
        &self,
        review: &ReviewId,
        from: &str,
        to: &str,
    ) -> Result<Vec<CreditedChange>, EngineError> {
        let _ = (review, from, to);
        todo!()
    }
}
