//! The old side of a diff: its export, the links to the worktree's dependencies, and its
//! lifetime (FR-055, FR-066, NFR-021, ADR-016).
//!
//! # The export
//!
//! At the first ask about the old side of `version` in a workspace ([`root`]), the
//! Navigator tells [`crate::Told::ExportWanted`] once, into a folder of its own cache that
//! does not exist yet, `<own cache>/old/<n>`, `n` counting up from 1 in this Navigator
//! (the own cache, `<cache>/<pid>-<k>`, is this Navigator's alone).
//! The Keeper writes it and the Frame hands the answer back ([`exported`]). On success,
//! for every row's `old_side.link`, each folder of that name the worktree holds, at any
//! depth but inside `.git` and inside a folder already linked, is linked into the export
//! at the same relative path, where the export holds the folder above it; then
//! `answers::export_ready`. On failure, the folder is removed if it is there, the reason
//! written to the standard error, and `answers::export_failed`; the next ask about that
//! version asks again. Meanwhile every ask about the old side is answered by text match
//! over the worktree, marked (`answers.rs`).
//!
//! No server of the old side runs before the first ask about it: the server is ensured by
//! `answers::export_ready` once the export is written, as `lifecycle::ensure` of the old
//! side's key with the export as its root (OLD-05). A link is never made through a folder of
//! the export that is itself a link, which a version may hold: the folder above the link is
//! resolved first and must lie inside the export. A folder the version holds under the name
//! of a link is the version's own and is left as it is.
//!
//! # Its lifetime
//!
//! One export a workspace. [`shown`] with another version than the export's, or none,
//! stops the old side's servers of that workspace (`lifecycle::stop` of every key of the
//! workspace whose side is that version, which returns once each process is reaped) and
//! then removes the export's folder, both before it returns (OLD-06); an export still being
//! written is removed when its answer comes.
//! [`workspaces_changed`] does the same for a workspace that left the list or whose
//! worktree is gone, and [`close`] for every one, and then removes `<own cache>`; the loop
//! calls it once every server is stopped, so that no export is removed under a process that
//! reads it. An
//! idle workspace's old servers stop by the rule of `lifecycle.rs`; its export stays until
//! another change is shown or the workspace closes.
//!
//! Nothing here writes into a worktree: links are made in the export, pointing at the
//! worktree's folders.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use yaar_link::WorkspaceId;

use crate::answers;
use crate::lifecycle::{state_of, stop};
use crate::navigator::{Core, ServerKey, Side};
use crate::port::{ExportAsk, Told};

/// What this module keeps in [`Core::old`]: the export of each workspace, written or
/// asked for.
#[derive(Default)]
pub(crate) struct OldSides {
    /// The export of each workspace, one at most.
    exports: HashMap<WorkspaceId, Export>,
    /// How many exports this Navigator asked for: the number of the next one's folder is one
    /// more.
    asked: u64,
}

/// One export, written or asked for.
struct Export {
    /// The version, by its full id.
    version: String,
    /// Its folder, `<own cache>/old/<n>`, by an absolute path.
    into: PathBuf,
    /// Whether the Keeper answered that the folder holds the version.
    written: bool,
}

/// [`crate::Ask::OldSide`]: the version shown in a workspace now. An export of another
/// version, or any when none is shown, is dropped as [`drop_export`] says; the export of this
/// version stays, and none is asked for before the first ask about a place in it.
pub(crate) fn shown(core: &mut Core, workspace: WorkspaceId, version: Option<String>) {
    let stays = core
        .old
        .exports
        .get(&workspace)
        .is_some_and(|export| Some(&export.version) == version.as_ref());
    if !stays {
        drop_export(core, &workspace);
    }
}

/// The root of the export of `version` of `workspace` when it is written; else asks for it
/// (once) and gives none. An export of another version of the workspace is dropped first.
pub(crate) fn root(core: &mut Core, workspace: &WorkspaceId, version: &str) -> Option<PathBuf> {
    if let Some(export) = core.old.exports.get(workspace) {
        if export.version == version {
            return export.written.then(|| export.into.clone());
        }
        drop_export(core, workspace);
    }
    core.old.asked += 1;
    let into = absolute(&core.own_cache.join("old").join(core.old.asked.to_string()));
    core.old.exports.insert(
        workspace.clone(),
        Export {
            version: version.to_owned(),
            into: into.clone(),
            written: false,
        },
    );
    core.tell(Told::ExportWanted(ExportAsk {
        workspace: workspace.clone(),
        version: version.to_owned(),
        into,
    }));
    None
}

/// [`crate::Ask::Exported`]: the Keeper's answer. One that is not for the export a workspace
/// holds now, because the change was dropped or another selected while it was written, only
/// removes the folder.
pub(crate) fn exported(core: &mut Core, asked: ExportAsk, failed: Option<String>) {
    let current = core
        .old
        .exports
        .get(&asked.workspace)
        .is_some_and(|export| export.into == asked.into && !export.written);
    if !current {
        remove_folder(&asked.into);
        return;
    }
    if let Some(reason) = failed {
        remove_folder(&asked.into);
        core.old.exports.remove(&asked.workspace);
        eprintln!(
            "language servers: the old side {} of the workspace {} could not be written: {reason}",
            asked.version, asked.workspace
        );
        answers::export_failed(core, &asked.workspace, &asked.version);
        return;
    }
    link_dependencies(core, &asked.workspace, &asked.into);
    if let Some(export) = core.old.exports.get_mut(&asked.workspace) {
        export.written = true;
    }
    answers::export_ready(core, &asked.workspace, &asked.version, &asked.into);
}

/// The list of workspaces changed: the exports of those gone are removed.
pub(crate) fn workspaces_changed(core: &mut Core) {
    let gone: Vec<WorkspaceId> = core
        .old
        .exports
        .keys()
        .filter(|workspace| core.folder_of(workspace).is_none())
        .cloned()
        .collect();
    for workspace in gone {
        drop_export(core, &workspace);
    }
}

/// The workspace, version and relative path of a file that lies in an export, its links
/// resolved; none for any other file.
pub(crate) fn version_of(core: &Core, path: &Path) -> Option<(WorkspaceId, String, PathBuf)> {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| absolute(path));
    core.old.exports.iter().find_map(|(workspace, export)| {
        let folder = std::fs::canonicalize(&export.into).unwrap_or_else(|_| export.into.clone());
        let relative = resolved.strip_prefix(&folder).ok()?;
        Some((
            workspace.clone(),
            export.version.clone(),
            relative.to_path_buf(),
        ))
    })
}

/// At the end: removes every export and the Navigator's own cache folder.
pub(crate) fn close(core: &mut Core) {
    let workspaces: Vec<WorkspaceId> = core.old.exports.keys().cloned().collect();
    for workspace in workspaces {
        drop_export(core, &workspace);
    }
    // What is left is the folders of exports that were still being written: their answer
    // will not come.
    let _ = std::fs::remove_dir_all(&core.own_cache);
}

/// Forgets the export of `workspace`: the rows asked about its old side are forgotten, every
/// server of that old side is stopped, which returns once each process is reaped, and then
/// its folder is removed. The folder of an export still being written is removed when the
/// Keeper's answer comes ([`exported`]).
fn drop_export(core: &mut Core, workspace: &WorkspaceId) {
    let Some(export) = core.old.exports.remove(workspace) else {
        return;
    };
    answers::export_failed(core, workspace, &export.version);
    let languages: Vec<String> = core
        .registry
        .rows()
        .iter()
        .map(|row| row.name.clone())
        .collect();
    for language in languages {
        let key = ServerKey {
            workspace: workspace.clone(),
            language,
            side: Side::Old {
                version: export.version.clone(),
            },
        };
        if state_of(core, &key).is_some() {
            stop(core, &key);
        }
    }
    if export.written {
        remove_folder(&export.into);
    }
}

/// Removes `folder` and what it holds, if it is there. A link in it is removed and its target
/// is not.
fn remove_folder(folder: &Path) {
    if let Err(error) = std::fs::remove_dir_all(folder)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "language servers: cannot remove the export {}: {error}",
            folder.display()
        );
    }
}

/// Links into the export at `export`, for every name some row's `old_side.link` holds, each
/// folder of that name the worktree of `workspace` holds.
fn link_dependencies(core: &Core, workspace: &WorkspaceId, export: &Path) {
    let names: BTreeSet<&str> = core
        .registry
        .rows()
        .iter()
        .flat_map(|row| row.old_side.link.iter().map(String::as_str))
        .collect();
    let Some(worktree) = core.folder_of(workspace).map(|folder| absolute(&folder)) else {
        return;
    };
    if names.is_empty() {
        return;
    }
    let Ok(inside) = std::fs::canonicalize(export) else {
        return;
    };
    link_in(&names, &worktree, Path::new(""), export, &inside);
}

/// Walks the folder `relative` of `worktree`: a folder whose name is in `names` is linked into
/// `export` at the same place, and not entered; any other real folder but `.git` is entered.
/// `inside` is `export` with its links resolved.
fn link_in(names: &BTreeSet<&str>, worktree: &Path, relative: &Path, export: &Path, inside: &Path) {
    let Ok(entries) = std::fs::read_dir(worktree.join(relative)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let here = relative.join(&name);
        let linked = name.to_str().is_some_and(|name| names.contains(name));
        if linked {
            // A folder, or a link to one.
            if std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir()) {
                link(&worktree.join(&here), &export.join(&here), inside);
            }
        } else if name != ".git" && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            link_in(names, worktree, &here, export, inside);
        }
    }
}

/// Makes `at` a link to `target`, when the export holds the folder above it, as a real folder
/// inside the export (`inside`), and nothing is at `at` yet.
fn link(target: &Path, at: &Path, inside: &Path) {
    let Some(above) = at.parent() else {
        return;
    };
    let holds_the_folder_above =
        std::fs::canonicalize(above).is_ok_and(|above| above.starts_with(inside));
    if !holds_the_folder_above || std::fs::symlink_metadata(at).is_ok() {
        return;
    }
    if let Err(error) = std::os::unix::fs::symlink(target, at) {
        eprintln!(
            "language servers: cannot link {} into the export: {error}",
            target.display()
        );
    }
}

/// `path` as an absolute path: the working folder joined to a relative one.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
