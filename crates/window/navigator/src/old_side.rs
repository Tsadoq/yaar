//! The old side of a diff: a per-workspace export of one version, with the worktree's dependency
//! folders linked in. A link is never made through a link inside the export: the folder above must
//! resolve inside the export.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use yaar_link::WorkspaceId;

use crate::answers;
use crate::lifecycle::{state_of, stop};
use crate::navigator::{Core, ServerKey, Side};
use crate::port::{ExportAsk, Told};

#[derive(Default)]
pub(crate) struct OldSides {
    exports: HashMap<WorkspaceId, Export>,
    asked: u64,
}

struct Export {
    version: String,
    into: PathBuf,
    written: bool,
}

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

pub(crate) fn exported(core: &mut Core, asked: ExportAsk, failed: Option<String>) {
    let current = core
        .old
        .exports
        .get(&asked.workspace)
        .is_some_and(|export| export.into == asked.into && !export.written);
    if !current {
        if is_unused_export_folder(core, &asked.into) {
            remove_folder(&asked.into);
        }
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

fn is_unused_export_folder(core: &Core, folder: &Path) -> bool {
    let folder = absolute(folder);
    let old = absolute(&core.own_cache.join("old"));
    // Exactly one component below `old`: a deeper path may pass through a link to the worktree.
    let under_old = folder.strip_prefix(&old).is_ok_and(|rest| {
        let mut parts = rest.components();
        matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
    });
    under_old
        && !core
            .old
            .exports
            .values()
            .any(|export| export.into == folder)
}

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

pub(crate) fn close(core: &mut Core) {
    let workspaces: Vec<WorkspaceId> = core.old.exports.keys().cloned().collect();
    for workspace in workspaces {
        drop_export(core, &workspace);
    }
    let _ = std::fs::remove_dir_all(&core.own_cache);
}

/// Stops the old side's servers, each reaped, before the folder is removed. An export still
/// being written has no folder removed here: `exported` removes it when the Keeper answers.
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

/// A link in it is removed, not its target.
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

/// Links each folder named in `names` into `export` at the same place. Any other folder but
/// `.git` is entered only where the export holds a real folder too, so build output is skipped.
fn link_in(names: &BTreeSet<&str>, worktree: &Path, relative: &Path, export: &Path, inside: &Path) {
    let Ok(entries) = std::fs::read_dir(worktree.join(relative)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let here = relative.join(&name);
        let linked = name.to_str().is_some_and(|name| names.contains(name));
        if linked {
            if std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir()) {
                link(&worktree.join(&here), &export.join(&here), inside);
            }
        } else if name != ".git"
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
            && std::fs::symlink_metadata(export.join(&here)).is_ok_and(|meta| meta.is_dir())
        {
            link_in(names, worktree, &here, export, inside);
        }
    }
}

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

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
