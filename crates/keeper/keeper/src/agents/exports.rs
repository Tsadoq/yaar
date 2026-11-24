use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use yaar_link::{Refusal, Reply, WorkspaceId};

use crate::agents::Agents;
use crate::agents::reading::{states_of, versions_refusal};
use crate::wiring::{Kept, failed, refusal};

impl Agents {
    /// `into` must be absolute, inside `yaar_link::cache_dir()` with its parts resolved but the
    /// last, which must not exist.
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

/// Links of every existing part are resolved so that a link inside the cache folder cannot lead a
/// write out.
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
    // A dangling link is a taken name too: looked at, not followed.
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
