//! Asks that want an answer carry an [`AskId`] and are answered once by a [`Told`] with the same
//! id, unless cancelled first.

use std::path::PathBuf;
use std::sync::Arc;

use yaar_link::{RepositoryId, WorkspaceId};

/// Chosen by the asker; unique among the waiting asks of one [`Port`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AskId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tree {
    Worktree,
    Version(String),
    /// The new side of a change that ends at a version. A line unchanged against the file on disk
    /// is answered by that file's server, its position carried through the diff; others by text
    /// match.
    Past {
        version: String,
        text: Arc<str>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub workspace: WorkspaceId,
    pub tree: Tree,
    /// Relative to the worktree. Only with [`Tree::Worktree`] may it be absolute: a file outside
    /// the worktree, or git-ignored inside it.
    pub path: PathBuf,
    /// From 1.
    pub line: u32,
    /// Byte offset in the line's UTF-8 text, from 0.
    pub column: u32,
    pub name: String,
    pub shown: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Absolute, unlike `Place::path`.
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered<T> {
    pub value: T,
    /// True when text match found the answer, not a server: it may be wrong.
    pub guessed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Worktree {
        workspace: WorkspaceId,
        relative: PathBuf,
    },
    Version {
        workspace: WorkspaceId,
        version: String,
        relative: PathBuf,
    },
    Outside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedFile {
    pub path: PathBuf,
    pub kind: FileKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub server: String,
    pub version: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Starting,
    Ready,
    /// An offer only when the row has a download for this machine and no registered server.
    Missing {
        offer: Option<Offer>,
    },
    /// The first server found lies in a repository with no answer yet: it is passed over, the user
    /// is asked, and later candidates are tried meanwhile.
    Asking {
        repository: RepositoryId,
        program: PathBuf,
    },
    Downloading {
        offer: Offer,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerState {
    pub workspace: WorkspaceId,
    pub language: String,
    pub title: String,
    pub old_side: Option<String>,
    pub registered: bool,
    pub state: State,
    pub server: Option<String>,
    pub program: Option<PathBuf>,
    pub pid: Option<u32>,
    /// The row's pinned version, when newer than the downloaded one that runs.
    pub update: Option<Offer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    pub name: String,
    pub title: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    pub id: WorkspaceId,
    pub repository: RepositoryId,
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConsent {
    pub repository: RepositoryId,
    pub allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportAsk {
    pub workspace: WorkspaceId,
    pub version: String,
    pub into: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Workspaces(Vec<WorkspaceRoot>),
    OnScreen(Option<WorkspaceId>),
    OldSide {
        workspace: WorkspaceId,
        version: Option<String>,
    },
    Consents(Vec<RepositoryConsent>),
    FilesChanged {
        workspace: WorkspaceId,
        paths: Vec<PathBuf>,
    },
    Hover {
        id: AskId,
        place: Place,
    },
    Definition {
        id: AskId,
        place: Place,
    },
    References {
        id: AskId,
        place: Place,
    },
    Open {
        id: AskId,
        location: Location,
    },
    Cancel {
        id: AskId,
    },
    Download {
        language: String,
    },
    Update {
        language: String,
    },
    Exported {
        asked: ExportAsk,
        failed: Option<String>,
    },
    StopAfter(std::time::Duration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    Languages(Vec<Language>),
    Servers(Vec<ServerState>),
    Hover {
        id: AskId,
        answer: Answered<Option<String>>,
    },
    Definition {
        id: AskId,
        answer: Answered<Vec<Location>>,
    },
    References {
        id: AskId,
        /// A server's: by path, line, column. Text match's: the file under the pointer, then the
        /// files of the change on screen in their order, then the rest by path; each by line,
        /// column.
        answer: Answered<Vec<Location>>,
    },
    Opened {
        id: AskId,
        file: Result<OpenedFile, String>,
    },
    ExportWanted(ExportAsk),
}

#[derive(Debug, Clone)]
pub struct Port {
    pub asks: async_channel::Sender<Ask>,
    pub told: async_channel::Receiver<Told>,
}

#[derive(Debug)]
pub struct PortEnd {
    pub asks: async_channel::Receiver<Ask>,
    pub told: async_channel::Sender<Told>,
}

pub fn port() -> (Port, PortEnd) {
    let (asks_in, asks_out) = async_channel::unbounded();
    let (told_in, told_out) = async_channel::unbounded();
    (
        Port {
            asks: asks_in,
            told: told_out,
        },
        PortEnd {
            asks: asks_out,
            told: told_in,
        },
    )
}
