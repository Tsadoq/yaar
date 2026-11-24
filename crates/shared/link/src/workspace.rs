use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::wire::{In, Out};

/// 32 lowercase hex digits, written once into the git folder of the worktree. Opaque to the Window.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(String);

impl RepositoryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: RepositoryId,
    pub name: String,
    /// The main worktree first, then in the order git lists them.
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    /// Kept when the worktree is gone.
    pub path: PathBuf,
    /// None when the worktree is gone but still listed because its reviews hold a thread.
    pub head: Option<Head>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    Detached(String),
}

impl Head {
    pub fn label(&self) -> String {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Detached(commit) => {
                format!("detached at {}", commit.get(..7).unwrap_or(commit))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// A branch by short name (`origin/main`, `main`), or a commit by its full id.
    pub name: String,
    pub source: BaseSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    Default,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub workspace: WorkspaceId,
    pub comparison: Comparison,
    /// Full id of the commit the files are counted from: where the branch left its base for
    /// `SinceBase`, the checked-out commit otherwise. None when nothing is committed.
    pub counted_from: Option<String>,
    pub commits: Vec<Commit>,
    pub files: Vec<ChangedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// Commits since the branch left the base; files are what differs between that point and the
    /// disk, uncommitted and untracked included.
    SinceBase { base: Base },
    /// On the base branch, which has an upstream: commits not on the upstream, files uncommitted.
    NotOnUpstream {
        /// The base. The branch checked out is that branch, or its local namesake when the base is
        /// a remote branch: `main` against `origin/main`.
        base: Base,
        upstream: String,
    },
    /// On the base branch with no upstream, in a repository with a remote: commits on no remote
    /// branch, files uncommitted.
    OnNoRemoteBranch { base: Base },
    /// No commits; files are the uncommitted changes. The Window says why and offers to set a base.
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
    /// No common commit, or nothing committed on the branch yet.
    NoSharedCommit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Relative to the folder of the worktree.
    pub path: PathBuf,
    pub status: FileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed { from: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSides {
    /// At the commit the request named, at the path it had there.
    pub old: Content,
    pub new: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Absent,
    Text(Vec<u8>),
    Binary { size: u64 },
    TooLarge { size: u64 },
}

/// Bytes; two versions fit in one frame.
pub const MAX_CONTENT: u64 = 4 * 1024 * 1024;

pub(crate) fn write_repository(out: &mut Out, repository: &Repository) {
    out.text(repository.id.as_str());
    out.text(&repository.name);
    out.list(&repository.workspaces, |out, workspace| {
        out.text(workspace.id.as_str());
        out.path(&workspace.path);
        match &workspace.head {
            Some(Head::Branch(name)) => {
                out.u8(0);
                out.text(name);
            }
            Some(Head::Detached(commit)) => {
                out.u8(1);
                out.text(commit);
            }
            None => out.u8(2),
        }
    });
}

pub(crate) fn read_repository(input: &mut In<'_>) -> Result<Repository, LinkError> {
    Ok(Repository {
        id: RepositoryId(input.text()?),
        name: input.text()?,
        workspaces: input.list(|input| {
            Ok(Workspace {
                id: WorkspaceId(input.text()?),
                path: input.path()?,
                head: match input.u8()? {
                    0 => Some(Head::Branch(input.text()?)),
                    1 => Some(Head::Detached(input.text()?)),
                    2 => None,
                    _ => return Err(LinkError::Malformed("unknown kind of head")),
                },
            })
        })?,
    })
}

fn write_base(out: &mut Out, base: &Base) {
    out.text(&base.name);
    out.u8(match base.source {
        BaseSource::Default => 0,
        BaseSource::User => 1,
    });
}

fn read_base(input: &mut In<'_>) -> Result<Base, LinkError> {
    Ok(Base {
        name: input.text()?,
        source: match input.u8()? {
            0 => BaseSource::Default,
            1 => BaseSource::User,
            _ => return Err(LinkError::Malformed("unknown source of a base")),
        },
    })
}

pub(crate) fn write_changes(out: &mut Out, changes: &Changes) {
    out.text(changes.workspace.as_str());
    match &changes.comparison {
        Comparison::SinceBase { base } => {
            out.u8(0);
            write_base(out, base);
        }
        Comparison::NotOnUpstream { base, upstream } => {
            out.u8(1);
            write_base(out, base);
            out.text(upstream);
        }
        Comparison::OnNoRemoteBranch { base } => {
            out.u8(2);
            write_base(out, base);
        }
        Comparison::NothingToCompare { base, why } => {
            out.u8(3);
            out.optional(base.as_ref(), write_base);
            out.u8(match why {
                NoComparison::NoRemote => 0,
                NoComparison::NoBase => 1,
                NoComparison::BaseGone => 2,
                NoComparison::NoSharedCommit => 3,
            });
        }
    }
    out.optional(changes.counted_from.as_deref(), Out::text);
    out.list(&changes.commits, write_commit);
    out.list(&changes.files, write_changed_file);
}

pub(crate) fn write_commit(out: &mut Out, commit: &Commit) {
    out.text(&commit.id);
    out.text(&commit.summary);
}

pub(crate) fn read_commit(input: &mut In<'_>) -> Result<Commit, LinkError> {
    Ok(Commit {
        id: input.text()?,
        summary: input.text()?,
    })
}

pub(crate) fn write_changed_file(out: &mut Out, file: &ChangedFile) {
    out.path(&file.path);
    match &file.status {
        FileStatus::Added => out.u8(0),
        FileStatus::Modified => out.u8(1),
        FileStatus::Deleted => out.u8(2),
        FileStatus::Renamed { from } => {
            out.u8(3);
            out.path(from);
        }
    }
}

pub(crate) fn read_changed_file(input: &mut In<'_>) -> Result<ChangedFile, LinkError> {
    Ok(ChangedFile {
        path: input.path()?,
        status: match input.u8()? {
            0 => FileStatus::Added,
            1 => FileStatus::Modified,
            2 => FileStatus::Deleted,
            3 => FileStatus::Renamed {
                from: input.path()?,
            },
            _ => return Err(LinkError::Malformed("unknown status of a file")),
        },
    })
}

pub(crate) fn read_changes(input: &mut In<'_>) -> Result<Changes, LinkError> {
    let workspace = WorkspaceId(input.text()?);
    let comparison = match input.u8()? {
        0 => Comparison::SinceBase {
            base: read_base(input)?,
        },
        1 => Comparison::NotOnUpstream {
            base: read_base(input)?,
            upstream: input.text()?,
        },
        2 => Comparison::OnNoRemoteBranch {
            base: read_base(input)?,
        },
        3 => Comparison::NothingToCompare {
            base: input.optional(read_base)?,
            why: match input.u8()? {
                0 => NoComparison::NoRemote,
                1 => NoComparison::NoBase,
                2 => NoComparison::BaseGone,
                3 => NoComparison::NoSharedCommit,
                _ => return Err(LinkError::Malformed("unknown reason for no comparison")),
            },
        },
        _ => return Err(LinkError::Malformed("unknown kind of comparison")),
    };
    let counted_from = input.optional(In::text)?;
    let commits = input.list(read_commit)?;
    let files = input.list(read_changed_file)?;
    Ok(Changes {
        workspace,
        comparison,
        counted_from,
        commits,
        files,
    })
}

fn write_content(out: &mut Out, content: &Content) {
    match content {
        Content::Absent => out.u8(0),
        Content::Text(bytes) => {
            out.u8(1);
            out.bytes(bytes);
        }
        Content::Binary { size } => {
            out.u8(2);
            out.u64(*size);
        }
        Content::TooLarge { size } => {
            out.u8(3);
            out.u64(*size);
        }
    }
}

fn read_content(input: &mut In<'_>) -> Result<Content, LinkError> {
    Ok(match input.u8()? {
        0 => Content::Absent,
        1 => Content::Text(input.bytes()?),
        2 => Content::Binary { size: input.u64()? },
        3 => Content::TooLarge { size: input.u64()? },
        _ => return Err(LinkError::Malformed("unknown kind of content")),
    })
}

pub(crate) fn write_sides(out: &mut Out, sides: &FileSides) {
    write_content(out, &sides.old);
    write_content(out, &sides.new);
}

pub(crate) fn read_sides(input: &mut In<'_>) -> Result<FileSides, LinkError> {
    Ok(FileSides {
        old: read_content(input)?,
        new: read_content(input)?,
    })
}
