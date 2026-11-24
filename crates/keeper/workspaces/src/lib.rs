//! A workspace is one worktree, identified by [`ID_FILE`] in its git folder; the id
//! survives a move. The watcher never says who wrote: a change not named by an agent's own
//! tool is credited to a session only if exactly one session's shell command ran then.

mod base;
mod create;
mod credit;
mod identity;
mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use yaar_recorder::{Branch, Commits, Git, GitError, OpenError};
use yaar_store::{RepositoryRecord, Store, StoreError, WorkspaceRecord};

pub use base::{default_base, on_base_branch};
pub use create::{Created, Planned};
pub use credit::{CreditedPath, Ledger, SETTLE};
pub use watch::Seen;
pub use yaar_recorder::{Change, ChangedPath, Commit, CommitId, Head, Refs, Worktree};
pub use yaar_store::{RepositoryId, WorkspaceId};

/// The workspace id in a worktree's git folder: 32 lowercase hex digits and a line feed.
pub const ID_FILE: &str = "ade-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: RepositoryId,
    pub name: String,
    pub git_dir: PathBuf,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub path: PathBuf,
    /// None when the worktree is gone.
    pub head: Option<Head>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub repository: RepositoryId,
    pub workspace: WorkspaceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// A branch by its short name, as [`Refs`] lists it, or a commit by its full id.
    pub name: String,
    pub source: BaseSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    Default,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    SinceBase {
        base: Base,
    },
    NotOnUpstream {
        base: Base,
        upstream: String,
    },
    OnNoRemoteBranch {
        base: Base,
    },
    NothingToCompare {
        base: Option<Base>,
        why: NoComparison,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoComparison {
    NoRemote,
    NoBase,
    BaseGone,
    NoSharedCommit,
}

/// | [`Comparison`] | `counted_from` | `commits` |
/// | --- | --- | --- |
/// | `SinceBase` | the point where the branch left the base | of the branch since that point |
/// | `NotOnUpstream` | the commit checked out | not on the upstream |
/// | `OnNoRemoteBranch` | the commit checked out | on no branch of any remote |
/// | `NothingToCompare` | the commit checked out, if any | none |
///
/// `files` differ between `counted_from` and the files on disk, untracked ones included. With
/// nothing committed every file git does not ignore is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub comparison: Comparison,
    pub counted_from: Option<CommitId>,
    pub commits: Vec<Commit>,
    pub files: Vec<ChangedPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSides {
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NoSuchFolder(PathBuf),
    NotARepository(PathBuf),
    BareRepository(PathBuf),
    CopiedWorktree(PathBuf),
    UnknownWorkspace(WorkspaceId),
    UnknownRevision(String),
    OutsideWorkspace(PathBuf),
    UnknownRepository(RepositoryId),
    NotABranchName(String),
    BranchExists(String),
    FolderExists(PathBuf),
    FolderNotAbsolute(PathBuf),
    FolderInsideRepository(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFolder(path) => write!(f, "there is no folder at {}", path.display()),
            Self::NotARepository(path) => write!(
                f,
                "{} holds no git repository and no worktree",
                path.display()
            ),
            Self::BareRepository(path) => write!(
                f,
                "{} is a repository with no files checked out",
                path.display()
            ),
            Self::CopiedWorktree(path) => write!(
                f,
                "{} is a copy of a worktree and shares its git folder",
                path.display()
            ),
            Self::UnknownWorkspace(id) => write!(f, "there is no workspace {id}"),
            Self::UnknownRevision(name) => {
                write!(f, "git knows no branch and no commit named {name}")
            }
            Self::OutsideWorkspace(path) => {
                write!(f, "{} is outside the worktree", path.display())
            }
            Self::UnknownRepository(id) => write!(f, "there is no repository {id}"),
            Self::NotABranchName(name) => {
                write!(f, "{name:?} is not a name git takes for a branch")
            }
            Self::BranchExists(name) => write!(f, "a branch named {name} exists already"),
            Self::FolderExists(path) => write!(f, "{} is there already", path.display()),
            Self::FolderNotAbsolute(path) => write!(
                f,
                "{} is not an absolute folder: name one from / on, with no ..",
                path.display()
            ),
            Self::FolderInsideRepository(path) => write!(
                f,
                "{} lies inside a worktree or the git folder of the repository",
                path.display()
            ),
        }
    }
}

#[derive(Debug)]
pub enum WorkspaceError {
    Refused(Refusal),
    Git(GitError),
    Store(StoreError),
    Id(PathBuf, std::io::Error),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::Git(e) => e.fmt(f),
            Self::Store(e) => e.fmt(f),
            Self::Id(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<Refusal> for WorkspaceError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<GitError> for WorkspaceError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<StoreError> for WorkspaceError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

#[derive(Debug)]
pub struct Workspaces {
    git: Git,
    store: Store,
    worktrees: Mutex<BTreeMap<WorkspaceId, Arc<Worktree>>>,
    // Held while ids are read, made and kept, so that two Windows that open or list at once do
    // not give one worktree two ids.
    naming: Mutex<()>,
    watching: watch::Watching,
}

impl Workspaces {
    pub fn new(git: Git, store: Store) -> Workspaces {
        Workspaces {
            git,
            store,
            worktrees: Mutex::new(BTreeMap::new()),
            naming: Mutex::new(()),
            watching: watch::Watching::default(),
        }
    }

    /// `told` runs on the watcher's own thread; watching again replaces it. After lost notices it
    /// hears both [`Seen::Files`] and [`Seen::Head`]. The ledger notes a change before `told`
    /// hears of it, so [`Workspaces::credits`] judges that write.
    pub fn watch(
        &self,
        workspace: &WorkspaceId,
        told: Box<dyn Fn(Seen) + Send + Sync>,
    ) -> Result<(), WorkspaceError> {
        let (_, worktree) = self.workspace(workspace)?;
        let places = watch::Places {
            root: worktree.root().to_owned(),
            git_dir: worktree.git_dir().to_owned(),
            common_dir: worktree.common_dir().to_owned(),
        };
        // Watching has no error variant of its own: the folder it could not watch comes as `Id`.
        self.watching
            .watch(workspace, places, told)
            .map_err(|(folder, e)| WorkspaceError::Id(folder, e))
    }

    pub fn unwatch(&self, workspace: &WorkspaceId) {
        self.watching.unwatch(workspace);
    }

    pub fn watched(&self) -> Vec<WorkspaceId> {
        self.watching.watched()
    }

    pub fn agent_wrote(&self, workspace: &WorkspaceId, session: &str, path: &Path) {
        let Some(path) = self.inside(workspace, path) else {
            return;
        };
        self.watching.with_ledger(workspace, |ledger| {
            ledger.wrote(session, &path, Instant::now());
        });
    }

    /// `at` is when the report reached the Keeper, taken before it answered, so that a command
    /// the CLI runs at once cannot write before its window opens.
    pub fn command_began(&self, workspace: &WorkspaceId, session: &str, at: Instant) {
        self.watching
            .with_ledger(workspace, |ledger| ledger.began(session, at));
    }

    pub fn command_ended(&self, workspace: &WorkspaceId, session: &str, at: Instant) {
        self.watching
            .with_ledger(workspace, |ledger| ledger.ended(session, at));
    }

    /// A change made before the Keeper started, or while not watched, is no agent's unless an
    /// agent's own tool named it. A path committed since it changed leaves the ledger.
    pub fn credits(&self, workspace: &WorkspaceId) -> Result<Vec<CreditedPath>, WorkspaceError> {
        let (_, worktree) = self.workspace(workspace)?;
        let head = match worktree.head()? {
            Head::Branch { commit, .. } => commit,
            Head::Detached(commit) => Some(commit),
        };
        // Taken before git is asked: a write noted while git reads the files is not in what it
        // says, and must not be forgotten for that.
        let asked = Instant::now();
        let changed = worktree.changed_paths(head.as_ref())?;
        let now = Instant::now();
        Ok(self.watching.with_ledger(workspace, |ledger| {
            let listed: BTreeSet<&Path> = changed.iter().map(|one| one.path.as_path()).collect();
            ledger.forget_unchanged(&listed, asked);
            let credits = changed
                .into_iter()
                .map(|ChangedPath { path, change }| CreditedPath {
                    by: ledger.credit(&path, now),
                    path,
                    change,
                })
                .collect();
            ledger.compact(now);
            credits
        }))
    }

    /// / `counted_from` and `commits` as [`Workspaces::changes`] gives them, but runs no
    /// / `git status`, so a window that opens can draw the history at once.
    pub fn counted_from(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<(Option<CommitId>, Vec<Commit>), WorkspaceError> {
        let (record, worktree) = self.workspace(workspace)?;
        let counted = counted(&record, &worktree)?;
        Ok((counted.counted_from, counted.commits))
    }

    /// A known id found at another path is the same workspace moved if that path no longer holds
    /// it; else a copy: a whole repository copy gets a new id, a copy of one linked worktree is
    /// refused as [`Refusal::CopiedWorktree`].
    pub fn open_folder(&self, folder: &Path) -> Result<Opened, WorkspaceError> {
        let worktree = Worktree::open(&self.git, folder).map_err(|e| match e {
            OpenError::NoSuchFolder => Refusal::NoSuchFolder(folder.to_owned()).into(),
            OpenError::NotARepository => Refusal::NotARepository(folder.to_owned()).into(),
            OpenError::Bare => Refusal::BareRepository(folder.to_owned()).into(),
            OpenError::Git(e) => WorkspaceError::Git(e),
        })?;
        let _naming = lock(&self.naming);

        // git lists one folder per git folder: another one still there means this is a copy,
        // gone means this worktree moved without git's knowing.
        let listed = worktree.list()?;
        if listed.iter().all(|other| other.path != worktree.root())
            && listed
                .iter()
                .any(|other| other.git_dir == worktree.git_dir())
        {
            return Err(Refusal::CopiedWorktree(folder.to_owned()).into());
        }

        let kept = self.store.workspaces()?;
        let here = identity::read(worktree.git_dir())?;
        let known = here
            .as_ref()
            .and_then(|id| kept.iter().find(|record| &record.id == id));
        let moved = known.filter(|record| record.path != worktree.root());
        let workspace = match (here, moved) {
            (_, Some(elsewhere)) if self.still_holds(elsewhere) => {
                identity::give(worktree.git_dir())?
            }
            (Some(id), _) => id,
            (None, _) => identity::give(worktree.git_dir())?,
        };

        let repositories = self.store.repositories()?;
        let at_home = repositories
            .iter()
            .find(|record| record.git_dir == worktree.common_dir());
        let repository = match (at_home, moved) {
            (Some(record), _) => record.id.clone(),
            (None, Some(before)) if before.id == workspace => before.repository.clone(),
            (None, _) => RepositoryId::new(identity::random()?),
        };
        self.store.put_repository(&RepositoryRecord {
            id: repository.clone(),
            git_dir: worktree.common_dir().to_owned(),
        })?;
        self.store
            .put_workspace(&workspace, &repository, worktree.root())?;
        lock(&self.worktrees).insert(workspace.clone(), Arc::new(worktree));
        Ok(Opened {
            repository,
            workspace,
        })
    }

    /// A repository that cannot be read (git fails, an id unreadable or unwritable) is listed with
    /// no worktrees. A worktree gone or moved without git's knowing is listed last; a gone one
    /// has no head and is listed only while one of its reviews holds a thread.
    pub fn repositories(&self) -> Result<Vec<Repository>, WorkspaceError> {
        let _naming = lock(&self.naming);
        let kept = self.store.workspaces()?;
        let with_threads = self.store.workspaces_with_threads()?;
        let mut repositories = Vec::new();
        for record in self.store.repositories()? {
            let workspaces = match self.worktrees_of(&record, &kept, &with_threads) {
                Ok(workspaces) => workspaces,
                Err(e) if of_one_repository(&e) => Vec::new(),
                Err(e) => return Err(e),
            };
            repositories.push(Repository {
                id: record.id,
                name: name_of(&record.git_dir),
                git_dir: record.git_dir,
                workspaces,
            });
        }
        Ok(repositories)
    }

    fn worktrees_of(
        &self,
        record: &RepositoryRecord,
        kept: &[WorkspaceRecord],
        with_threads: &[WorkspaceId],
    ) -> Result<Vec<Workspace>, WorkspaceError> {
        let mut workspaces = Vec::new();
        for listed in Worktree::list_of(&self.git, &record.git_dir)? {
            let id = match identity::read(&listed.git_dir)? {
                Some(id) => id,
                None => identity::give(&listed.git_dir)?,
            };
            self.store.put_workspace(&id, &record.id, &listed.path)?;
            workspaces.push(Workspace {
                id,
                path: listed.path,
                head: Some(listed.head),
            });
        }
        let mut gone = Vec::new();
        for known in kept.iter().filter(|known| known.repository == record.id) {
            if workspaces.iter().any(|listed| listed.id == known.id) {
                continue;
            }
            match self.worktree_of(known) {
                Ok(worktree) if worktree.common_dir() == record.git_dir => {
                    workspaces.push(Workspace {
                        id: known.id.clone(),
                        path: known.path.clone(),
                        head: Some(worktree.head()?),
                    });
                }
                Ok(_) => {}
                Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(_))) => {
                    if with_threads.contains(&known.id) {
                        gone.push(Workspace {
                            id: known.id.clone(),
                            path: known.path.clone(),
                            head: None,
                        });
                    }
                }
                Err(e) => return Err(e),
            }
        }
        workspaces.extend(gone);
        Ok(workspaces)
    }

    pub fn base(&self, workspace: &WorkspaceId) -> Result<Option<Base>, WorkspaceError> {
        let (record, worktree) = self.workspace(workspace)?;
        Ok(base_of(&record, &worktree.head()?, &worktree.refs()?))
    }

    /// A branch, local or remote, is kept by its short name and followed. Anything else (a tag, a
    /// short id, `HEAD~3`) is kept as the full id of the commit it names now.
    pub fn set_base(
        &self,
        workspace: &WorkspaceId,
        revision: &str,
    ) -> Result<Base, WorkspaceError> {
        let (_, worktree) = self.workspace(workspace)?;
        let name = match branch_named(revision, &worktree.refs()?) {
            Some(branch) => branch,
            None => worktree
                .resolve(revision)?
                .ok_or_else(|| Refusal::UnknownRevision(revision.to_owned()))?
                .to_string(),
        };
        self.store.set_base(workspace, &name)?;
        Ok(Base {
            name,
            source: BaseSource::User,
        })
    }

    /// The first that applies: no base (`NoBase`); base gone (`BaseGone`); on the base branch:
    /// `NotOnUpstream` if it follows a branch, else `OnNoRemoteBranch` if a remote exists, else
    /// `NoRemote`; a commit shared with the base (`SinceBase`); else `NoSharedCommit`.
    pub fn changes(&self, workspace: &WorkspaceId) -> Result<Changes, WorkspaceError> {
        let (record, worktree) = self.workspace(workspace)?;
        let Counted {
            comparison,
            counted_from,
            commits,
        } = counted(&record, &worktree)?;
        Ok(Changes {
            comparison,
            files: worktree.changed_paths(counted_from.as_ref())?,
            counted_from,
            commits,
        })
    }

    /// Refused for a path leading out of the worktree and for a commit git does not know.
    pub fn file(
        &self,
        workspace: &WorkspaceId,
        since: Option<&CommitId>,
        old_path: &Path,
        path: &Path,
    ) -> Result<FileSides, WorkspaceError> {
        for asked in [old_path, path] {
            if leads_out(asked) {
                return Err(Refusal::OutsideWorkspace(asked.to_owned()).into());
            }
        }
        let (_, worktree) = self.workspace(workspace)?;
        let old = match since {
            Some(since) => {
                let old = worktree.content_at(since, old_path)?;
                // No content is the answer for a file the commit lacks and for a commit
                // git lacks; only then is it worth a second question to tell them apart.
                if old.is_none() && worktree.resolve(since.as_str())?.as_ref() != Some(since) {
                    return Err(Refusal::UnknownRevision(since.to_string()).into());
                }
                old
            }
            None => None,
        };
        Ok(FileSides {
            old,
            new: worktree.content_on_disk(path)?,
        })
    }

    /// The kept worktree with its running git; none when gone. Starts no git beyond opening one
    /// not seen before, so a request may call it every time, unlike [`Workspaces::repositories`].
    pub fn worktree(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Option<Arc<Worktree>>, WorkspaceError> {
        let record = self
            .store
            .workspaces()?
            .into_iter()
            .find(|record| &record.id == workspace)
            .ok_or_else(|| Refusal::UnknownWorkspace(workspace.clone()))?;
        match self.worktree_of(&record) {
            Ok(worktree) => Ok(Some(worktree)),
            Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(_))) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn workspace(
        &self,
        id: &WorkspaceId,
    ) -> Result<(WorkspaceRecord, Arc<Worktree>), WorkspaceError> {
        let record = self
            .store
            .workspaces()?
            .into_iter()
            .find(|record| &record.id == id)
            .ok_or_else(|| Refusal::UnknownWorkspace(id.clone()))?;
        let worktree = self.worktree_of(&record)?;
        Ok((record, worktree))
    }

    fn worktree_of(&self, record: &WorkspaceRecord) -> Result<Arc<Worktree>, WorkspaceError> {
        let unknown = || {
            lock(&self.worktrees).remove(&record.id);
            WorkspaceError::Refused(Refusal::UnknownWorkspace(record.id.clone()))
        };
        if !record.path.is_dir() {
            return Err(unknown());
        }
        let kept = lock(&self.worktrees)
            .get(&record.id)
            .filter(|worktree| worktree.root() == record.path)
            .cloned();
        let worktree = match kept {
            Some(worktree) => worktree,
            None => match Worktree::open(&self.git, &record.path) {
                Ok(worktree) if worktree.root() == record.path => Arc::new(worktree),
                Ok(_)
                | Err(OpenError::NoSuchFolder | OpenError::NotARepository | OpenError::Bare) => {
                    return Err(unknown());
                }
                Err(OpenError::Git(e)) => return Err(e.into()),
            },
        };
        if identity::read(worktree.git_dir())?.as_ref() != Some(&record.id) {
            return Err(unknown());
        }
        lock(&self.worktrees).insert(record.id.clone(), Arc::clone(&worktree));
        Ok(worktree)
    }

    fn inside(&self, workspace: &WorkspaceId, path: &Path) -> Option<PathBuf> {
        let worktree = self.worktree(workspace).ok().flatten()?;
        relative_to(worktree.root(), path)
    }

    fn still_holds(&self, record: &WorkspaceRecord) -> bool {
        Worktree::open(&self.git, &record.path).is_ok_and(|worktree| {
            worktree.root() == record.path
                && identity::read(worktree.git_dir())
                    .is_ok_and(|id| id.as_ref() == Some(&record.id))
        })
    }
}

struct Counted {
    comparison: Comparison,
    counted_from: Option<CommitId>,
    commits: Vec<Commit>,
}

fn counted(record: &WorkspaceRecord, worktree: &Worktree) -> Result<Counted, WorkspaceError> {
    let head = worktree.head()?;
    let refs = worktree.refs()?;
    let checked_out = match &head {
        Head::Branch { commit, .. } => commit.clone(),
        Head::Detached(commit) => Some(commit.clone()),
    };
    let nothing = |comparison: Comparison, commits: Vec<Commit>| Counted {
        comparison,
        counted_from: checked_out.clone(),
        commits,
    };
    let none = |base: Option<Base>, why: NoComparison| {
        nothing(Comparison::NothingToCompare { base, why }, Vec::new())
    };

    let Some(base) = base_of(record, &head, &refs) else {
        return Ok(none(None, NoComparison::NoBase));
    };
    let Some(base_commit) = commit_of_base(worktree, &base.name, &refs)? else {
        return Ok(none(Some(base), NoComparison::BaseGone));
    };
    if on_base_branch(&head, &base.name, &refs) {
        let upstream = match &head {
            Head::Branch { name, .. } => refs
                .local
                .iter()
                .find(|branch| &branch.name == name)
                .and_then(|branch| branch.upstream.clone()),
            Head::Detached(_) => None,
        };
        return Ok(match upstream {
            Some(upstream) => {
                let commits = worktree.commits(Commits::NotOn(followed(&upstream, &refs)))?;
                nothing(Comparison::NotOnUpstream { base, upstream }, commits)
            }
            None if !refs.remotes.is_empty() => {
                let commits = worktree.commits(Commits::OnNoRemoteBranch)?;
                nothing(Comparison::OnNoRemoteBranch { base }, commits)
            }
            None => none(Some(base), NoComparison::NoRemote),
        });
    }
    Ok(match worktree.fork_point(&base_commit)? {
        Some(fork) => Counted {
            comparison: Comparison::SinceBase { base },
            commits: worktree.commits(Commits::Since(&fork))?,
            counted_from: Some(fork),
        },
        None => none(Some(base), NoComparison::NoSharedCommit),
    })
}

fn of_one_repository(error: &WorkspaceError) -> bool {
    matches!(
        error,
        WorkspaceError::Git(GitError::Failed { .. } | GitError::Unreadable { .. })
            | WorkspaceError::Id(..)
    )
}

fn base_of(record: &WorkspaceRecord, head: &Head, refs: &Refs) -> Option<Base> {
    match &record.base {
        Some(name) => Some(Base {
            name: name.clone(),
            source: BaseSource::User,
        }),
        None => default_base(head, refs).map(|name| Base {
            name,
            source: BaseSource::Default,
        }),
    }
}

/// A base is a branch's short name or a full commit id: a tag named like a gone branch is not it.
fn commit_of_base(
    worktree: &Worktree,
    base: &str,
    refs: &Refs,
) -> Result<Option<CommitId>, GitError> {
    let local = refs.local.iter().any(|branch| branch.name == base);
    let remote = refs.remote.iter().any(|branch| branch.name == base);
    if local {
        worktree.commit_of(Branch::Local(base))
    } else if remote {
        worktree.commit_of(Branch::Remote(base))
    } else {
        // git reads a full id as that commit and as nothing else. Whatever else it finds
        // under this text is another commit than the text says.
        let named = worktree.resolve(base)?;
        Ok(named.filter(|commit| commit.as_str() == base))
    }
}

fn followed<'a>(upstream: &'a str, refs: &Refs) -> Branch<'a> {
    let local = refs.local.iter().any(|branch| branch.name == upstream);
    let remote = refs.remote.iter().any(|branch| branch.name == upstream);
    if local && !remote {
        Branch::Local(upstream)
    } else {
        Branch::Remote(upstream)
    }
}

fn branch_named(text: &str, refs: &Refs) -> Option<String> {
    let local = |name: &str| refs.local.iter().any(|branch| branch.name == name);
    let remote = |name: &str| refs.remote.iter().any(|branch| branch.name == name);
    let stripped =
        |prefixes: [&str; 2]| prefixes.iter().find_map(|prefix| text.strip_prefix(prefix));
    let name = if local(text) || remote(text) {
        text
    } else if let Some(name) = stripped(["refs/heads/", "heads/"]).filter(|name| local(name)) {
        name
    } else {
        stripped(["refs/remotes/", "remotes/"]).filter(|name| remote(name))?
    };
    Some(name.to_owned())
}

/// `root` has every link resolved. An absolute `path` is compared as written and, failing that,
/// with its existing folders resolved (macOS keeps `/tmp` behind a link). None when outside.
fn relative_to(root: &Path, path: &Path) -> Option<PathBuf> {
    let inside = if path.is_relative() {
        path.to_owned()
    } else if let Ok(inside) = path.strip_prefix(root) {
        inside.to_owned()
    } else {
        let mut rest = Vec::new();
        let mut folder = path;
        let resolved = loop {
            if let Ok(resolved) = folder.canonicalize() {
                break resolved;
            }
            rest.push(folder.file_name()?);
            folder = folder.parent()?;
        };
        resolved
            .strip_prefix(root)
            .ok()?
            .iter()
            .chain(rest.into_iter().rev())
            .collect()
    };
    // The ledger's keys hold no `.` component.
    let inside: PathBuf = inside
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect();
    (!leads_out(&inside)).then_some(inside)
}

fn leads_out(path: &Path) -> bool {
    path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
}

fn name_of(git_dir: &Path) -> String {
    let folder = match git_dir.file_name() {
        Some(name) if name == ".git" => git_dir.parent().and_then(Path::file_name),
        name => name,
    };
    folder
        .unwrap_or(git_dir.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Takes a lock also after a panic of its holder: every call brings what it guards up to date.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
