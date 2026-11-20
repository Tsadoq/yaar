//! `Request::ExportVersion`: the old side of a diff, written for the Window's language
//! server into a folder of the app's cache.

use std::path::Path;

use yaar_link::{Refusal, Reply, WorkspaceId};

use crate::agents::Agents;
use crate::wiring::Kept;

impl Agents {
    /// `Request::ExportVersion`: writes `version` of `workspace` into `into` with
    /// `yaar_recorder::StateStore::export`, through the workspace's state store and its
    /// worktree when it is there, and answers `Reply::Done`.
    ///
    /// `into` is checked before anything is written: it must be absolute, lie inside
    /// `yaar_link::cache_dir()` as this process finds it, its parts resolved but the last,
    /// which must not exist; else `Refusal::Failed` naming it, and nothing is made. The
    /// parent folders under the cache folder are made. A workspace the Keeper does not know
    /// is `Refusal::UnknownWorkspace`; a version neither a state of the workspace nor a
    /// commit of its repository `Refusal::UnknownRevision`, as `Request::ReadVersions`
    /// refuses one; a write that failed `Refusal::Failed`, with no folder left behind.
    pub(crate) fn export_version(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        version: &str,
        into: &Path,
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, version, into);
        todo!()
    }
}
