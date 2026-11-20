//! `Request::ExportVersion`: the old side of a diff, written for the Window's language
//! server into a folder of the app's cache.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use yaar_link::{Refusal, Reply, WorkspaceId};

use crate::agents::Agents;
use crate::agents::reading::{failed, states_of, versions_refusal};
use crate::wiring::{Kept, refusal};

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
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = kept.workspaces.worktree(&id).map_err(refusal)?;
        let states = states_of(kept, &id)?;
        let into = place_in_cache(into)?;
        let refused =
            |error| versions_refusal(error, &states, worktree.as_deref(), [Some(version), None]);
        // The version is made held, and refused if it is none, before anything is made on
        // disk: a refusal leaves no folder behind, not even the ones above `into`. The
        // export finds it held and copies nothing.
        states
            .compare(worktree.as_deref(), version, Some(version))
            .map_err(refused)?;
        let parent = into.parent().ok_or_else(|| failed("no folder above it"))?;
        fs::create_dir_all(parent).map_err(|e| {
            failed(format_args!(
                "the folders above {} cannot be made: {e}",
                into.display()
            ))
        })?;
        states
            .export(worktree.as_deref(), version, &into)
            .map_err(refused)?;
        Ok(Reply::Done)
    }
}

/// Where `into` is, once the links of its path are resolved, when it is a place the Keeper
/// may write to: absolute, with no `.` or `..` in it, below the app's cache folder and not
/// that folder itself, and not there yet. The links of every part that exists are
/// resolved, so that a link inside the cache folder that leads out of it does not lead a
/// write out; the last part, which must not exist, is added as it is.
fn place_in_cache(into: &Path) -> Result<PathBuf, Refusal> {
    let refuse = |why: &str| failed(format_args!("{}: {why}", into.display()));
    if !into.is_absolute() {
        return Err(refuse("not an absolute path"));
    }
    if into
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(refuse("a path with . or .. in it"));
    }
    // A link that points nowhere is a taken name too: it is looked at, not followed.
    if into.symlink_metadata().is_ok() {
        return Err(refuse("it exists"));
    }
    let (Some(parent), Some(name)) = (into.parent(), into.file_name()) else {
        return Err(refuse("no folder above it"));
    };
    let cache = resolved(&yaar_link::cache_dir()).ok_or_else(|| refuse("no cache folder"))?;
    let parent = resolved(parent).ok_or_else(|| refuse("a folder above it cannot be read"))?;
    if !parent.starts_with(&cache) {
        return Err(refuse("outside the app's cache folder"));
    }
    Ok(parent.join(name))
}

/// `path` with the links of the part that exists resolved, and the rest, which does not
/// exist yet, as it is; none for a path that is not absolute, or whose rest holds a part
/// that is not a plain name.
fn resolved(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut rest = Vec::new();
    let mut here = path;
    let mut found = loop {
        match here.canonicalize() {
            Ok(found) => break found,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                rest.push(here.file_name()?);
                here = here.parent()?;
            }
            Err(_) => return None,
        }
    };
    found.extend(rest.into_iter().rev());
    Some(found)
}
