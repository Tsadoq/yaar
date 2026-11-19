//! The watcher of the worktrees: the system's notices of changed files, one watcher for
//! each workspace watched, through the `notify` crate.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use notify::event::{CreateKind, ModifyKind};
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

/// What one notice of the system came to, to the worktree it came from.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Notice {
    /// The files that changed, by their paths relative to the folder of the worktree.
    pub(crate) files: Vec<PathBuf>,
    /// The folders that appeared, made there or moved in, by the same kind of path.
    pub(crate) folders: Vec<PathBuf>,
    /// What is checked out, the list of files or a ref changed.
    pub(crate) head: bool,
    /// The system lost notices, or says it could not give them: anything may have changed.
    /// What was lost cannot be credited to a session, and is a change no agent made.
    pub(crate) lost: bool,
}

impl Notice {
    /// What `result` of the system is, to the worktree at `places`. `is_folder` says
    /// whether a path that came into being, or was renamed, is a folder: a notice does not
    /// always say.
    pub(crate) fn of(
        places: &Places,
        result: &notify::Result<notify::Event>,
        is_folder: impl Fn(&Path) -> bool,
    ) -> Notice {
        let event = match result {
            Ok(event) => event,
            Err(_) => {
                return Notice {
                    lost: true,
                    ..Notice::default()
                };
            }
        };
        let mut notice = Notice {
            lost: event.need_rescan(),
            ..Notice::default()
        };
        // Reads are not changes.
        if matches!(event.kind, EventKind::Access(_)) {
            return notice;
        }
        for path in &event.paths {
            match places.classify(path) {
                Some(Item::File(file)) => {
                    let folder = match event.kind {
                        EventKind::Create(CreateKind::Folder) => true,
                        EventKind::Create(CreateKind::File) => false,
                        EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_)) => {
                            is_folder(path)
                        }
                        _ => false,
                    };
                    if folder {
                        notice.folders.push(file);
                    } else {
                        notice.files.push(file);
                    }
                }
                Some(Item::Head) => notice.head = true,
                None => {}
            }
        }
        notice
    }

    /// Whether what is uncommitted may have changed.
    pub(crate) fn tells_files(&self) -> bool {
        self.lost || !self.files.is_empty() || !self.folders.is_empty()
    }

    /// Whether what is checked out may have changed.
    pub(crate) fn tells_head(&self) -> bool {
        self.lost || self.head
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
                let notice = Notice::of(&places, &result, |path| {
                    path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
                });
                {
                    let mut ledger = lock(&ledger);
                    // One moment for every path of the notice: it is when it arrived.
                    let now = Instant::now();
                    for file in &notice.files {
                        ledger.seen(file, now);
                    }
                    for folder in &notice.folders {
                        ledger.appeared(folder, now);
                    }
                }
                // The ledger is let go of before anyone is told, and has noted the paths
                // first: whoever hears of a file can ask who made the change.
                if notice.tells_files() {
                    told(Seen::Files);
                }
                if notice.tells_head() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn main_worktree() -> Places {
        Places {
            root: PathBuf::from("/w/repo"),
            git_dir: PathBuf::from("/w/repo/.git"),
            common_dir: PathBuf::from("/w/repo/.git"),
        }
    }

    fn linked_worktree() -> Places {
        Places {
            root: PathBuf::from("/w/linked"),
            git_dir: PathBuf::from("/w/repo/.git/worktrees/linked"),
            common_dir: PathBuf::from("/w/repo/.git"),
        }
    }

    fn file(path: &str) -> Option<Item> {
        Some(Item::File(PathBuf::from(path)))
    }

    #[test]
    fn a_path_in_the_worktree_is_a_file_by_its_path_relative_to_the_worktree() {
        let places = main_worktree();
        assert_eq!(places.classify(Path::new("/w/repo/a.txt")), file("a.txt"));
        assert_eq!(
            places.classify(Path::new("/w/repo/src/deep/b.rs")),
            file("src/deep/b.rs")
        );
        // A name that only starts with `.git` is a file like any other.
        assert_eq!(
            places.classify(Path::new("/w/repo/.gitignore")),
            file(".gitignore")
        );
    }

    #[test]
    fn the_folder_of_the_worktree_and_paths_outside_it_are_nothing() {
        let places = main_worktree();
        assert_eq!(places.classify(Path::new("/w/repo")), None);
        assert_eq!(places.classify(Path::new("/w/repo-two/a.txt")), None);
        assert_eq!(places.classify(Path::new("/elsewhere/a.txt")), None);
    }

    #[test]
    fn in_the_git_folder_the_head_the_list_of_files_and_the_refs_are_the_head_and_the_rest_is_nothing()
     {
        let places = main_worktree();
        for told in [
            "/w/repo/.git/HEAD",
            "/w/repo/.git/index",
            "/w/repo/.git/packed-refs",
            "/w/repo/.git/refs/heads/main",
            "/w/repo/.git/refs/remotes/origin/main",
            "/w/repo/.git/refs/tags/v1",
        ] {
            assert_eq!(places.classify(Path::new(told)), Some(Item::Head), "{told}");
        }
        for not_told in [
            "/w/repo/.git",
            "/w/repo/.git/refs",
            "/w/repo/.git/index.lock",
            "/w/repo/.git/objects/ab/cdef",
            "/w/repo/.git/ORIG_HEAD",
            "/w/repo/.git/FETCH_HEAD",
            "/w/repo/.git/config",
            "/w/repo/.git/logs/HEAD",
        ] {
            assert_eq!(places.classify(Path::new(not_told)), None, "{not_told}");
        }
    }

    #[test]
    fn a_git_folder_inside_the_worktree_is_no_file_whatever_it_holds() {
        let places = main_worktree();
        assert_eq!(places.classify(Path::new("/w/repo/vendor/lib/.git")), None);
        assert_eq!(
            places.classify(Path::new("/w/repo/vendor/lib/.git/index")),
            None
        );
    }

    #[test]
    fn a_linked_worktree_has_its_own_head_and_list_and_shares_the_refs() {
        let places = linked_worktree();
        for told in [
            "/w/repo/.git/worktrees/linked/HEAD",
            "/w/repo/.git/worktrees/linked/index",
            "/w/repo/.git/refs/heads/main",
            "/w/repo/.git/packed-refs",
        ] {
            assert_eq!(places.classify(Path::new(told)), Some(Item::Head), "{told}");
        }
        for not_told in [
            // The main worktree's own head and list are its own.
            "/w/repo/.git/HEAD",
            "/w/repo/.git/index",
            "/w/repo/.git/worktrees/linked/index.lock",
            "/w/repo/.git/objects/ab/cdef",
            // The link to the git folder, a file of the linked worktree.
            "/w/linked/.git",
        ] {
            assert_eq!(places.classify(Path::new(not_told)), None, "{not_told}");
        }
        assert_eq!(places.classify(Path::new("/w/linked/a.txt")), file("a.txt"));
    }

    fn event(kind: EventKind, paths: &[&str]) -> notify::Result<notify::Event> {
        let mut event = notify::Event::new(kind);
        for path in paths {
            event = event.add_path(PathBuf::from(path));
        }
        Ok(event)
    }

    fn notice(result: &notify::Result<notify::Event>) -> Notice {
        // Nothing is a folder unless the notice says so.
        Notice::of(&main_worktree(), result, |_| false)
    }

    #[test]
    fn a_change_of_files_tells_the_files_and_of_the_git_folder_tells_the_head() {
        let seen = notice(&event(
            EventKind::Modify(ModifyKind::Any),
            &["/w/repo/a.txt", "/w/repo/.git/index", "/w/repo/.git/config"],
        ));
        assert_eq!(seen.files, [PathBuf::from("a.txt")]);
        assert!(seen.folders.is_empty());
        assert!(seen.tells_files() && seen.tells_head());
        assert!(!seen.lost);

        let only_head = notice(&event(EventKind::Any, &["/w/repo/.git/refs/heads/main"]));
        assert!(only_head.tells_head() && !only_head.tells_files());

        let nothing = notice(&event(EventKind::Any, &["/w/repo/.git/objects/ab/cd"]));
        assert!(!nothing.tells_head() && !nothing.tells_files());
    }

    #[test]
    fn a_read_tells_nothing() {
        let read = notice(&event(
            EventKind::Access(notify::event::AccessKind::Any),
            &["/w/repo/a.txt"],
        ));
        assert_eq!(read, Notice::default());
    }

    #[test]
    fn a_folder_that_appears_is_told_as_a_folder_by_the_notice_or_by_looking() {
        let made = notice(&event(
            EventKind::Create(CreateKind::Folder),
            &["/w/repo/out"],
        ));
        assert_eq!(made.folders, [PathBuf::from("out")]);
        assert!(made.files.is_empty() && made.tells_files());

        let made_a_file = notice(&event(
            EventKind::Create(CreateKind::File),
            &["/w/repo/out/f"],
        ));
        assert_eq!(made_a_file.files, [PathBuf::from("out/f")]);

        // A notice that says only that something appeared, or was renamed, is asked about.
        for kind in [
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To)),
        ] {
            let result = event(kind, &["/w/repo/moved-in", "/w/repo/file"]);
            let moved = Notice::of(&main_worktree(), &result, |path| {
                path == Path::new("/w/repo/moved-in")
            });
            assert_eq!(moved.folders, [PathBuf::from("moved-in")], "{kind:?}");
            assert_eq!(moved.files, [PathBuf::from("file")], "{kind:?}");
        }

        // A change of what a folder holds is no folder appearing.
        let result = event(EventKind::Modify(ModifyKind::Any), &["/w/repo/out"]);
        let changed = Notice::of(&main_worktree(), &result, |_| true);
        assert_eq!(changed.files, [PathBuf::from("out")]);
        assert!(changed.folders.is_empty());
    }

    #[test]
    fn notices_the_system_lost_tell_the_files_and_the_head_with_no_path_to_note() {
        // The system's queue overflowed, or it says it could not watch: after a burst of
        // writes, nothing says what changed, and whoever shows the changes must look.
        let overflow =
            Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan));
        let lost = notice(&overflow);
        assert!(lost.lost && lost.tells_files() && lost.tells_head());
        assert!(lost.files.is_empty() && lost.folders.is_empty());

        let failed = Err(notify::Error::generic("the system gave up"));
        let lost = notice(&failed);
        assert!(lost.lost && lost.tells_files() && lost.tells_head());
    }
}
