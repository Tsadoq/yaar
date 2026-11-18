//! Between the Link and the parts of the Keeper.
//!
//! The parts speak in their own words and know nothing of the Link. Here each request of a
//! Window is handed to the part that answers it, and what the part returns is put into the
//! words of the Link: a reply for the Window that asked, and the facts that every attached
//! Window is told.
//!
//! | Request | Answered by | Facts told to every Window |
//! | --- | --- | --- |
//! | `ListWorkspaces` | `Workspaces::repositories` | none |
//! | `OpenFolder` | `Workspaces::open_folder` | `Workspaces`, the list as it is then |
//! | `SetBase` | `Workspaces::set_base` | `ChangesChanged` of that workspace |
//! | `GetChanges` | `Workspaces::changes` | none |
//! | `ReadFile` | `Workspaces::file` | none |
//!
//! The reply goes out before the facts. A Window that opened a folder therefore learns
//! the id of the new workspace before the list that holds it.
//!
//! The list is told after every folder that was opened, also one that was open already.
//! Whether it differs from what a Window holds, the Keeper cannot say: the list is read
//! from git each time, and a worktree made outside the app is in it before any Window
//! heard of it. A Window that opens such a worktree waits for the list that holds it.
//!
//! A version of a file is sent as text unless it is larger than `yaar_link::MAX_CONTENT`,
//! or holds a zero byte among its first 8,000, which is how git tells that a file is not
//! text; of such a version only the size is sent.
//!
//! A reply too large for one frame, as the changes of a folder with a hundred thousand
//! new files can be, is not sent: the Window that asked gets `Refusal::Failed` with the
//! reason in its place, so that no request stays without an answer. That is done where
//! the reply is written, in the serve loop.
//!
//! Requests that change what the Keeper keeps, and the one whose reply carries what a
//! fact carries, are answered one at a time: see [`waits_its_turn`].
//!
//! Nothing is watched yet. The list of workspaces is read from git when a Window asks
//! for it, and no fact tells of a worktree or a branch that changed outside the app.

use yaar_link::{Fact, Refusal, Reply, Request, RequestKind};
use yaar_recorder::{CommitId, Git};
use yaar_store::Store;
use yaar_workspaces::{WorkspaceError, WorkspaceId, Workspaces};

use crate::Options;

/// How many of its first bytes say whether a file is text: git's own number.
const TEXT_PROBE: usize = 8000;

/// The parts of the Keeper, started and connected.
pub(crate) struct Parts {
    // Workspaces on git and the Store, or the sentence that says why there is none.
    workspaces: Result<Workspaces, String>,
}

/// What one request leads to.
pub(crate) struct Outcome {
    /// For the Window that asked.
    pub(crate) reply: Result<Reply, Refusal>,
    /// For every attached Window, the one that asked included, after the reply.
    pub(crate) facts: Vec<Fact>,
}

impl Outcome {
    /// A request that was refused, with nothing to tell anybody else.
    pub(crate) fn failed(reason: impl Into<String>) -> Outcome {
        Outcome {
            reply: Err(Refusal::Failed {
                reason: reason.into(),
            }),
            facts: Vec::new(),
        }
    }
}

/// Whether requests of this kind are answered one at a time, each with its reply and its
/// facts sent before the next is begun.
pub(crate) fn waits_its_turn(kind: RequestKind) -> bool {
    // No arm for "the rest": a new kind of request has to say which it is.
    match kind {
        // Each can change what the Keeper keeps, and so lead to a fact. The facts have to
        // reach the Windows in the order in which the changes were made, or a Window is
        // left with the older list.
        RequestKind::OpenFolder | RequestKind::SetBase => true,
        // Its reply carries what a fact carries. Read between two changes and sent after
        // the fact of the second, it would be taken for the newer list.
        RequestKind::ListWorkspaces => true,
        // What these read, no fact carries: a Window that is told of a change asks again.
        RequestKind::GetChanges | RequestKind::ReadFile => false,
    }
}

impl Parts {
    /// Starts the parts: finds git, opens the database `yaar_store::FILE_NAME` in the
    /// folder of `options.data_dir`, and puts Workspaces on both.
    ///
    /// A Keeper with no git, or with a database it cannot open, still starts and answers
    /// the hello, so that a Window can say what is wrong; each request is then refused
    /// with the reason.
    pub(crate) fn start(options: &Options) -> Parts {
        let workspaces = Git::find().map_err(|e| e.to_string()).and_then(|git| {
            let file = options.data_dir.join(yaar_store::FILE_NAME);
            let store = Store::open(&file).map_err(|e| e.to_string())?;
            Ok(Workspaces::new(git, store))
        });
        Parts { workspaces }
    }

    /// Answers one request of a Window.
    pub(crate) fn answer(&self, request: Request) -> Outcome {
        let workspaces = match &self.workspaces {
            Ok(workspaces) => workspaces,
            Err(reason) => return Outcome::failed(reason.clone()),
        };
        let mut facts = Vec::new();
        let reply = match request {
            Request::ListWorkspaces => list(workspaces).map(Reply::Workspaces),
            Request::OpenFolder { path } => open_folder(workspaces, path, &mut facts),
            Request::SetBase { workspace, base } => workspaces
                .set_base(&WorkspaceId::new(workspace.as_str()), &base)
                .map(|_| {
                    facts.push(Fact::ChangesChanged { workspace });
                    Reply::Done
                })
                .map_err(refusal),
            Request::GetChanges { workspace } => workspaces
                .changes(&WorkspaceId::new(workspace.as_str()))
                .map(|found| Reply::Changes(changes(workspace, found)))
                .map_err(refusal),
            Request::ReadFile {
                workspace,
                since,
                old_path,
                path,
            } => workspaces
                .file(
                    &WorkspaceId::new(workspace.as_str()),
                    since.map(CommitId::new).as_ref(),
                    &old_path,
                    &path,
                )
                .map(|sides| {
                    Reply::File(yaar_link::FileSides {
                        old: content(sides.old),
                        new: content(sides.new),
                    })
                })
                .map_err(refusal),
        };
        Outcome { reply, facts }
    }
}

/// Opens a folder, and tells the list of repositories as it is then.
fn open_folder(
    workspaces: &Workspaces,
    path: std::path::PathBuf,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    // A Window names the folder in full. A path counted from wherever this process
    // happens to run names nothing the user chose.
    if !path.is_absolute() {
        return Err(Refusal::NoSuchFolder { path });
    }
    let opened = workspaces.open_folder(&path).map_err(refusal)?;
    // The folder is open now, whatever becomes of the list: a list that cannot be read
    // is told to nobody, and the Window that asks for it next learns why.
    if let Ok(list) = list(workspaces) {
        facts.push(Fact::Workspaces(list));
    }
    Ok(Reply::Opened {
        repository: yaar_link::RepositoryId::new(opened.repository.as_str()),
        workspace: yaar_link::WorkspaceId::new(opened.workspace.as_str()),
    })
}

/// Every opened repository with its worktrees, in the words of the Link.
fn list(workspaces: &Workspaces) -> Result<Vec<yaar_link::Repository>, Refusal> {
    let repositories = workspaces.repositories().map_err(refusal)?;
    Ok(repositories
        .into_iter()
        .map(|repository| yaar_link::Repository {
            id: yaar_link::RepositoryId::new(repository.id.as_str()),
            name: repository.name,
            workspaces: repository
                .workspaces
                .into_iter()
                .map(|workspace| yaar_link::Workspace {
                    id: yaar_link::WorkspaceId::new(workspace.id.as_str()),
                    path: workspace.path,
                    head: head(workspace.head),
                })
                .collect(),
        })
        .collect())
}

/// What is checked out, in the words of the Link. A branch with no commit yet is still
/// that branch.
fn head(head: yaar_workspaces::Head) -> yaar_link::Head {
    match head {
        yaar_workspaces::Head::Branch { name, .. } => yaar_link::Head::Branch(name),
        yaar_workspaces::Head::Detached(commit) => yaar_link::Head::Detached(commit.to_string()),
    }
}

fn base(base: yaar_workspaces::Base) -> yaar_link::Base {
    yaar_link::Base {
        name: base.name,
        source: match base.source {
            yaar_workspaces::BaseSource::Default => yaar_link::BaseSource::Default,
            yaar_workspaces::BaseSource::User => yaar_link::BaseSource::User,
        },
    }
}

/// The changes of a workspace, in the words of the Link.
fn changes(
    workspace: yaar_link::WorkspaceId,
    found: yaar_workspaces::Changes,
) -> yaar_link::Changes {
    use yaar_workspaces::{Change, Comparison, NoComparison};

    let comparison = match found.comparison {
        Comparison::SinceBase { base: found } => {
            yaar_link::Comparison::SinceBase { base: base(found) }
        }
        Comparison::NotOnUpstream {
            base: found,
            upstream,
        } => yaar_link::Comparison::NotOnUpstream {
            base: base(found),
            upstream,
        },
        Comparison::OnNoRemoteBranch { base: found } => {
            yaar_link::Comparison::OnNoRemoteBranch { base: base(found) }
        }
        Comparison::NothingToCompare { base: found, why } => {
            yaar_link::Comparison::NothingToCompare {
                base: found.map(base),
                why: match why {
                    NoComparison::NoRemote => yaar_link::NoComparison::NoRemote,
                    NoComparison::NoBase => yaar_link::NoComparison::NoBase,
                    NoComparison::BaseGone => yaar_link::NoComparison::BaseGone,
                    NoComparison::NoSharedCommit => yaar_link::NoComparison::NoSharedCommit,
                },
            }
        }
    };
    yaar_link::Changes {
        workspace,
        comparison,
        counted_from: found.counted_from.map(|commit| commit.to_string()),
        commits: found
            .commits
            .into_iter()
            .map(|commit| yaar_link::Commit {
                id: commit.id.to_string(),
                summary: commit.summary,
            })
            .collect(),
        files: found
            .files
            .into_iter()
            .map(|file| yaar_link::ChangedFile {
                path: file.path,
                status: match file.change {
                    Change::Added => yaar_link::FileStatus::Added,
                    Change::Modified => yaar_link::FileStatus::Modified,
                    Change::Deleted => yaar_link::FileStatus::Deleted,
                    Change::Renamed { from } => yaar_link::FileStatus::Renamed { from },
                },
            })
            .collect(),
    }
}

/// One version of a file as far as it travels: its bytes when it is text of a size the
/// Link carries, else its size alone. What is not text is said to be so whatever its
/// size, since that is the better reason to show no lines.
fn content(version: Option<Vec<u8>>) -> yaar_link::Content {
    let Some(bytes) = version else {
        return yaar_link::Content::Absent;
    };
    let size = bytes.len() as u64;
    if bytes.iter().take(TEXT_PROBE).any(|byte| *byte == 0) {
        yaar_link::Content::Binary { size }
    } else if size > yaar_link::MAX_CONTENT {
        yaar_link::Content::TooLarge { size }
    } else {
        yaar_link::Content::Text(bytes)
    }
}

/// Puts a failure of Workspaces into the words of the Link: what the user can mend keeps
/// its kind, everything else becomes `Refusal::Failed` with the sentence of the error.
pub(crate) fn refusal(error: WorkspaceError) -> Refusal {
    use yaar_workspaces::Refusal as Refused;

    match error {
        WorkspaceError::Refused(refused) => match refused {
            Refused::NoSuchFolder(path) => Refusal::NoSuchFolder { path },
            Refused::NotARepository(path) => Refusal::NotARepository { path },
            Refused::BareRepository(path) => Refusal::BareRepository { path },
            Refused::CopiedWorktree(path) => Refusal::CopiedWorktree { path },
            Refused::UnknownWorkspace(workspace) => Refusal::UnknownWorkspace {
                workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
            },
            Refused::UnknownRevision(revision) => Refusal::UnknownRevision { revision },
            Refused::OutsideWorkspace(path) => Refusal::OutsideWorkspace { path },
        },
        failed @ (WorkspaceError::Git(_) | WorkspaceError::Store(_) | WorkspaceError::Id(..)) => {
            Refusal::Failed {
                reason: failed.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yaar_link::Content;

    #[test]
    fn a_version_travels_as_text_unless_it_is_not_text_or_too_large() {
        assert_eq!(content(None), Content::Absent);
        assert_eq!(content(Some(Vec::new())), Content::Text(Vec::new()));
        assert_eq!(
            content(Some(b"fn main() {}\n".to_vec())),
            Content::Text(b"fn main() {}\n".to_vec())
        );

        // A zero byte among the first 8,000 says "not text"; one after them says nothing.
        let mut zero_in_reach = vec![b'a'; TEXT_PROBE + 10];
        zero_in_reach[TEXT_PROBE - 1] = 0;
        assert_eq!(
            content(Some(zero_in_reach.clone())),
            Content::Binary {
                size: zero_in_reach.len() as u64
            }
        );
        let mut zero_out_of_reach = vec![b'a'; TEXT_PROBE + 10];
        zero_out_of_reach[TEXT_PROBE] = 0;
        assert_eq!(
            content(Some(zero_out_of_reach.clone())),
            Content::Text(zero_out_of_reach)
        );

        // The largest version that travels, and one byte more.
        let largest = vec![b'a'; yaar_link::MAX_CONTENT as usize];
        assert_eq!(content(Some(largest.clone())), Content::Text(largest));
        let too_large = vec![b'a'; yaar_link::MAX_CONTENT as usize + 1];
        assert_eq!(
            content(Some(too_large)),
            Content::TooLarge {
                size: yaar_link::MAX_CONTENT + 1
            }
        );
        // Large and not text: said to be not text.
        let mut large_and_binary = vec![b'a'; yaar_link::MAX_CONTENT as usize + 1];
        large_and_binary[0] = 0;
        assert_eq!(
            content(Some(large_and_binary)),
            Content::Binary {
                size: yaar_link::MAX_CONTENT + 1
            }
        );
    }

    #[test]
    fn what_the_user_can_mend_keeps_its_kind_and_the_rest_becomes_a_sentence() {
        use yaar_workspaces::Refusal as Refused;

        let path = PathBuf::from("/somewhere/else");
        let id = "0123456789abcdef0123456789abcdef";
        let pairs = [
            (
                Refused::NoSuchFolder(path.clone()),
                Refusal::NoSuchFolder { path: path.clone() },
            ),
            (
                Refused::NotARepository(path.clone()),
                Refusal::NotARepository { path: path.clone() },
            ),
            (
                Refused::BareRepository(path.clone()),
                Refusal::BareRepository { path: path.clone() },
            ),
            (
                Refused::CopiedWorktree(path.clone()),
                Refusal::CopiedWorktree { path: path.clone() },
            ),
            (
                Refused::UnknownWorkspace(WorkspaceId::new(id)),
                Refusal::UnknownWorkspace {
                    workspace: yaar_link::WorkspaceId::new(id),
                },
            ),
            (
                Refused::UnknownRevision("no-such-branch".to_owned()),
                Refusal::UnknownRevision {
                    revision: "no-such-branch".to_owned(),
                },
            ),
            (
                Refused::OutsideWorkspace(path.clone()),
                Refusal::OutsideWorkspace { path: path.clone() },
            ),
        ];
        for (theirs, ours) in pairs {
            assert_eq!(refusal(WorkspaceError::Refused(theirs)), ours);
        }

        assert_eq!(
            refusal(WorkspaceError::Git(yaar_recorder::GitError::NotFound)),
            Refusal::Failed {
                reason: "git was not found".to_owned()
            }
        );
        assert_eq!(
            refusal(WorkspaceError::Store(yaar_store::StoreError::Closed)),
            Refusal::Failed {
                reason: "the database is closed".to_owned()
            }
        );
    }
}
