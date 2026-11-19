//! What a Window reads of the agents' side but the agents themselves: one turn, the
//! history of a worktree, its uncommitted changes as a review lists them, and two
//! versions of a change. Each is read from the Store, the state stores, the Review engine
//! and Workspaces, through `Kept`, and from nothing of this side's thread.

use std::path::Path;

use yaar_link::{Refusal, Reply, ReviewId, TurnId, WorkspaceId};

use super::{Agents, not_wired};
use crate::wiring::Kept;

impl Agents {
    /// `Request::GetTurn`.
    pub(crate) fn turn(&self, kept: &Kept, turn: &TurnId) -> Result<Reply, Refusal> {
        let _ = (kept, turn);
        Err(not_wired())
    }

    /// `Request::GetHistory`.
    pub(crate) fn history(&self, kept: &Kept, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        let _ = (kept, workspace);
        Err(not_wired())
    }

    /// `Request::CompareVersions`.
    pub(crate) fn compare(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        from: &str,
        to: Option<&str>,
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, from, to);
        Err(not_wired())
    }

    /// `Request::ReadVersions`.
    pub(crate) fn read_versions(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        old: Option<(&str, &Path)>,
        new: (Option<&str>, &Path),
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, old, new);
        Err(not_wired())
    }

    /// `Request::GetUncommitted`: the review's list, by `yaar_review_engine::uncommitted_of`
    /// over `Workspaces::credits`.
    pub(crate) fn uncommitted(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        review: Option<&ReviewId>,
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, review);
        Err(not_wired())
    }
}
