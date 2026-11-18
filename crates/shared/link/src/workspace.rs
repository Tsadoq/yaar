//! What the two programs say about repositories, workspaces and their changes.

use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::wire::{In, Out};

/// Names one workspace, on the Link and across restarts of both programs.
///
/// It is the id the Keeper wrote once into the git folder of the worktree, 32 lowercase
/// hexadecimal digits. The Window treats it as a name and reads nothing into it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Names one repository, on the Link and across restarts of both programs. Made by the
/// Keeper; the Window treats it as a name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(String);

impl RepositoryId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository the user opened, with its worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Its name on the Link.
    pub id: RepositoryId,
    /// The name to show: the name of the folder that holds the repository.
    pub name: String,
    /// Its worktrees, the main one first, then in the order git lists them.
    pub workspaces: Vec<Workspace>,
}

/// One worktree of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Its name on the Link.
    pub id: WorkspaceId,
    /// The folder of the worktree.
    pub path: PathBuf,
    /// What is checked out in it.
    pub head: Head,
}

/// What is checked out in a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// A branch, by its short name: `main`, `feature/login`.
    Branch(String),
    /// No branch: a commit, by its full id.
    Detached(String),
}

impl Head {
    /// The name a workspace is listed by: its branch, or `detached at` and the first seven
    /// digits of the commit.
    pub fn label(&self) -> String {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Detached(commit) => {
                format!("detached at {}", commit.get(..7).unwrap_or(commit))
            }
        }
    }
}

/// What the changes of a workspace are counted from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// A branch by its short name, `origin/main` or `main`, or a commit by its full id.
    pub name: String,
    /// Who chose it.
    pub source: BaseSource,
}

/// Who chose the base of a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// Nobody: it is the default branch of the repository, as the Keeper found it.
    Default,
    /// The user set it.
    User,
}

/// The changes of one workspace: what the Review pane shows for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// The workspace they belong to.
    pub workspace: WorkspaceId,
    /// What they are compared with.
    pub comparison: Comparison,
    /// The commit the files are counted from, by its full id: the point where the branch
    /// left its base in [`Comparison::SinceBase`], the commit that is checked out in every
    /// other comparison. None only where nothing is committed yet.
    ///
    /// A Window hands it back in [`crate::Request::ReadFile`] to name the old version of
    /// a file, so the two versions it gets belong to this list whatever happened to the
    /// base or the branch since. It is a name to the Window, which reads nothing into it.
    pub counted_from: Option<String>,
    /// The commits of the comparison, newest first.
    pub commits: Vec<Commit>,
    /// The changed files, ordered by path.
    pub files: Vec<ChangedFile>,
}

/// What the changes of a workspace are compared with. It decides what
/// [`Changes::commits`] and [`Changes::files`] hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// The branch of the workspace against its base. The commits are those of the branch
    /// since the point where it left the base, which is [`Changes::counted_from`]; the
    /// files are everything that differs between that point and the worktree as it is on
    /// disk, uncommitted and untracked files included.
    SinceBase {
        /// The base.
        base: Base,
    },
    /// The workspace is on its base branch, which has an upstream. The commits are those
    /// not on the upstream; the files are the uncommitted changes.
    NotOnUpstream {
        /// The base, as the Keeper found it or the user set it. The branch checked out is
        /// that branch, or the local branch of the same name when the base is a branch of
        /// a remote: `main` against `origin/main`.
        base: Base,
        /// The upstream of the branch checked out, by its short name: `origin/main`.
        upstream: String,
    },
    /// The workspace is on its base branch, which has no upstream, in a repository with a
    /// remote. The commits are those on no branch of any remote; the files are the
    /// uncommitted changes.
    OnNoRemoteBranch {
        /// The base, as the Keeper found it or the user set it. The branch checked out is
        /// that branch or its local twin, as in [`Comparison::NotOnUpstream`].
        base: Base,
    },
    /// There is nothing to compare the branch with. No commits; the files are the
    /// uncommitted changes. The Window says why and offers to set a base.
    NothingToCompare {
        /// The base, when there is one.
        base: Option<Base>,
        /// Why there is nothing to compare with.
        why: NoComparison,
    },
}

/// Why there is nothing to compare a workspace with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoComparison {
    /// The workspace is on its base branch, in a repository with no remote.
    NoRemote,
    /// No base was found and the user set none.
    NoBase,
    /// The base the user set names nothing git knows any more: the branch was deleted,
    /// or the commit is gone.
    BaseGone,
    /// The branch and its base share no commit, or nothing is committed on the branch
    /// yet.
    NoSharedCommit,
}

/// One commit, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Its full id.
    pub id: String,
    /// The first line of its message.
    pub summary: String,
}

/// One changed file of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Its path now, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it.
    pub status: FileStatus,
}

/// What happened to a changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// It is new; an untracked file counts as new.
    Added,
    /// Its content changed.
    Modified,
    /// It is gone.
    Deleted,
    /// It had another path before, and its content may have changed too.
    Renamed {
        /// The path it had.
        from: PathBuf,
    },
}

/// The two versions of one file of a comparison: before and now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSides {
    /// The file as it was in the commit the request named, at the path it had there.
    pub old: Content,
    /// The file as it is in the worktree.
    pub new: Content,
}

/// One version of a file, as far as it can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The file does not exist on this side.
    Absent,
    /// The bytes of the file.
    Text(Vec<u8>),
    /// The file is not text; only its size in bytes is sent.
    Binary {
        /// Its size in bytes.
        size: u64,
    },
    /// The file is larger than [`MAX_CONTENT`]; only its size in bytes is sent.
    TooLarge {
        /// Its size in bytes.
        size: u64,
    },
}

/// The largest version of a file that travels over the Link, in bytes. Two of them fit in
/// one frame with room to spare.
pub const MAX_CONTENT: u64 = 4 * 1024 * 1024;

pub(crate) fn write_repository(out: &mut Out, repository: &Repository) {
    out.text(repository.id.as_str());
    out.text(&repository.name);
    out.list(&repository.workspaces, |out, workspace| {
        out.text(workspace.id.as_str());
        out.path(&workspace.path);
        match &workspace.head {
            Head::Branch(name) => {
                out.u8(0);
                out.text(name);
            }
            Head::Detached(commit) => {
                out.u8(1);
                out.text(commit);
            }
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
                    0 => Head::Branch(input.text()?),
                    1 => Head::Detached(input.text()?),
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
    out.list(&changes.commits, |out, commit| {
        out.text(&commit.id);
        out.text(&commit.summary);
    });
    out.list(&changes.files, |out, file| {
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
    });
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
    let commits = input.list(|input| {
        Ok(Commit {
            id: input.text()?,
            summary: input.text()?,
        })
    })?;
    let files = input.list(|input| {
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
    })?;
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
