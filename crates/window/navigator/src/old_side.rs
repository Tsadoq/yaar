//! The old side of a diff: its export, the links to the worktree's dependencies, and its
//! lifetime (FR-055, FR-066, NFR-021, ADR-016).
//!
//! # The export
//!
//! At the first ask about the old side of `version` in a workspace ([`root`]), the
//! Navigator tells [`crate::Told::ExportWanted`] once, into a folder of its own cache that
//! does not exist yet, `<own cache>/old/<n>`, `n` counting up from 1 in this Navigator.
//! The Keeper writes it and the Frame hands the answer back ([`exported`]). On success,
//! for every row's `old_side.link`, each folder of that name the worktree holds, at any
//! depth but inside `.git` and inside a folder already linked, is linked into the export
//! at the same relative path, where the export holds the folder above it; then
//! `answers::export_ready`. On failure, the folder is removed if it is there, the reason
//! written to the standard error, and `answers::export_failed`; the next ask about that
//! version asks again.
//!
//! No server of the old side runs before the first ask about it: the server is ensured by
//! `answers.rs` once the export is ready, as `lifecycle::ensure` of the old side's key with
//! the export as its root (OLD-05).
//!
//! # Its lifetime
//!
//! One export a workspace. [`shown`] with another version than the export's, or none,
//! stops the old side's servers of that workspace (`lifecycle::stop` of every key of the
//! workspace whose side is that version) and removes the export's folder, before it
//! returns (OLD-06); an export still being written is removed when its answer comes.
//! [`workspaces_changed`] does the same for a workspace that left the list or whose
//! worktree is gone, and [`close`] for every one, and then removes `<own cache>`. An
//! idle workspace's old servers stop by the rule of `lifecycle.rs`; its export stays until
//! another change is shown or the workspace closes.
//!
//! Nothing here writes into a worktree: links are made in the export, pointing at the
//! worktree's folders.

use std::path::PathBuf;

use yaar_link::WorkspaceId;

use crate::navigator::Core;
use crate::port::ExportAsk;

/// What this module keeps in [`Core::old`]: the export of each workspace, written or
/// asked for.
#[derive(Default)]
pub(crate) struct OldSides {}

/// [`crate::Ask::OldSide`]: the version shown in a workspace now.
pub(crate) fn shown(core: &mut Core, workspace: WorkspaceId, version: Option<String>) {
    let _ = (core, workspace, version);
    todo!()
}

/// The root of the export of `version` of `workspace` when it is written; else asks for it
/// (once) and gives none.
pub(crate) fn root(core: &mut Core, workspace: &WorkspaceId, version: &str) -> Option<PathBuf> {
    let _ = (core, workspace, version);
    todo!()
}

/// [`crate::Ask::Exported`]: the Keeper's answer.
pub(crate) fn exported(core: &mut Core, asked: ExportAsk, failed: Option<String>) {
    let _ = (core, asked, failed);
    todo!()
}

/// The list of workspaces changed: the exports of those gone are removed.
pub(crate) fn workspaces_changed(core: &mut Core) {
    let _ = core;
    todo!()
}

/// The workspace, version and relative path of a file that lies in an export, its links
/// resolved; none for any other file.
pub(crate) fn version_of(
    core: &Core,
    path: &std::path::Path,
) -> Option<(WorkspaceId, String, PathBuf)> {
    let _ = (core, path);
    todo!()
}

/// At the end: removes every export and the Navigator's own cache folder.
pub(crate) fn close(core: &mut Core) {
    let _ = core;
    todo!()
}
