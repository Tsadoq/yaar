//! The watcher of the worktrees: the system's notices of changed files, one watcher for
//! each workspace watched, through the `notify` crate.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use yaar_store::WorkspaceId;

use crate::credit::Ledger;

/// What the watcher of a worktree saw change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seen {
    /// Files of the worktree: what is uncommitted may have changed.
    Files,
    /// What is checked out, the list of files or a ref: the name of the workspace, its
    /// commits and what is uncommitted may have changed.
    Head,
}

/// Where a worktree keeps its files and its git folders, as the watcher reads a path.
#[derive(Debug, Clone)]
pub(crate) struct Places {
    /// The folder of the worktree.
    pub(crate) root: PathBuf,
    /// The git folder of this worktree alone.
    pub(crate) git_dir: PathBuf,
    /// The git folder all worktrees of the repository share.
    pub(crate) common_dir: PathBuf,
}

/// What one path of a system notice is, to the worktree it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    /// A file of the worktree, by its path relative to the folder of the worktree.
    File(PathBuf),
    /// What is checked out, the list of files or a ref.
    Head,
}

impl Places {
    /// What a changed `path` is: the files of the worktree are files; in the git folder,
    /// `HEAD`, the list of files (`index`), the refs and `packed-refs` are [`Item::Head`];
    /// anything else git keeps there, and the git folder of a repository inside the
    /// worktree, is nothing the app cares about.
    pub(crate) fn classify(&self, path: &Path) -> Option<Item> {
        // This worktree's own git folder first: it lies inside the shared one for a
        // linked worktree, and is the shared one for the main worktree.
        if let Ok(inside) = path.strip_prefix(&self.git_dir) {
            let own = ["HEAD", "index"]
                .iter()
                .any(|name| inside == Path::new(name));
            return (own || is_ref(inside)).then_some(Item::Head);
        }
        if let Ok(inside) = path.strip_prefix(&self.common_dir) {
            return is_ref(inside).then_some(Item::Head);
        }
        let inside = path.strip_prefix(&self.root).ok()?;
        // A folder `.git` anywhere in the worktree is git's, a nested repository's too.
        if inside.as_os_str().is_empty()
            || inside.components().any(|part| part.as_os_str() == ".git")
        {
            return None;
        }
        Some(Item::File(inside.to_owned()))
    }
}

/// Whether a path inside a git folder is a ref: under `refs/`, or `packed-refs`.
fn is_ref(inside: &Path) -> bool {
    inside == Path::new("packed-refs")
        || (inside.starts_with("refs") && inside != Path::new("refs"))
}

/// The watcher and the ledger of one workspace.
#[derive(Debug, Default)]
struct Entry {
    // Shared with the thread of the watcher, which notes each path it sees.
    ledger: Arc<Mutex<Ledger>>,
    // None while the workspace is not watched; the ledger is kept all the same.
    watcher: Option<RecommendedWatcher>,
}

/// The watchers of the workspaces watched and the ledger of each workspace.
#[derive(Debug, Default)]
pub(crate) struct Watching {
    entries: Mutex<BTreeMap<WorkspaceId, Entry>>,
}

impl Watching {
    /// Starts a watcher on the worktree at `places`, replacing the one of `workspace`
    /// when it has one. `told` hears what each notice was, after the ledger has noted
    /// every file of it.
    pub(crate) fn watch(
        &self,
        workspace: &WorkspaceId,
        places: Places,
        told: Box<dyn Fn(Seen) + Send + Sync>,
    ) -> Result<(), (PathBuf, io::Error)> {
        let ledger = lock(&self.entries)
            .entry(workspace.clone())
            .or_default()
            .ledger
            .clone();
        let made = {
            let places = places.clone();
            let ledger = Arc::clone(&ledger);
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else { return };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                let mut files = false;
                let mut head = false;
                {
                    let mut ledger = lock(&ledger);
                    // One moment for every path of the notice: it is when it arrived.
                    let now = Instant::now();
                    for path in &event.paths {
                        match places.classify(path) {
                            Some(Item::File(file)) => {
                                ledger.seen(&file, now);
                                files = true;
                            }
                            Some(Item::Head) => head = true,
                            None => {}
                        }
                    }
                }
                // The ledger is let go of before anyone is told, and has noted the paths
                // first: whoever hears of a file can ask who made the change.
                if files {
                    told(Seen::Files);
                }
                if head {
                    told(Seen::Head);
                }
            })
        };
        let mut watcher = made.map_err(|e| (places.root.clone(), io::Error::other(e)))?;

        // The files, and the git folder where it lies beside them or elsewhere: for a
        // linked worktree, its own git folder and the refs of the repository.
        let mut folders = vec![(places.root.as_path(), RecursiveMode::Recursive)];
        if !places.git_dir.starts_with(&places.root) {
            folders.push((places.git_dir.as_path(), RecursiveMode::Recursive));
        }
        if !places.common_dir.starts_with(&places.root) {
            folders.push((places.common_dir.as_path(), RecursiveMode::NonRecursive));
        }
        let refs = places.common_dir.join("refs");
        if !refs.starts_with(&places.root) {
            folders.push((refs.as_path(), RecursiveMode::Recursive));
        }
        for (folder, mode) in folders {
            watcher
                .watch(folder, mode)
                .map_err(|e| (folder.to_owned(), io::Error::other(e)))?;
        }

        // The new watcher is running before the old one goes: nothing falls between. The
        // old one is dropped with no lock held, since its thread may be telling `told`,
        // which may ask this for a ledger.
        let old = lock(&self.entries)
            .entry(workspace.clone())
            .or_default()
            .watcher
            .replace(watcher);
        drop(old);
        Ok(())
    }

    /// Stops the watcher of `workspace`; its ledger is kept.
    pub(crate) fn unwatch(&self, workspace: &WorkspaceId) {
        // Dropped after the lock is let go of: the watcher's own thread may be waiting
        // for the ledger, never for this.
        let stopped = lock(&self.entries)
            .get_mut(workspace)
            .and_then(|entry| entry.watcher.take());
        drop(stopped);
    }

    /// The workspaces that have a watcher.
    pub(crate) fn watched(&self) -> Vec<WorkspaceId> {
        lock(&self.entries)
            .iter()
            .filter(|(_, entry)| entry.watcher.is_some())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Runs `f` on the ledger of `workspace`, made empty when there is none yet.
    pub(crate) fn with_ledger<T>(
        &self,
        workspace: &WorkspaceId,
        f: impl FnOnce(&mut Ledger) -> T,
    ) -> T {
        let ledger = lock(&self.entries)
            .entry(workspace.clone())
            .or_default()
            .ledger
            .clone();
        let mut ledger = lock(&ledger);
        f(&mut ledger)
    }
}

/// Takes a lock, also after a thread that held it panicked: the ledger and the list are
/// only ever added to or taken from, whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
