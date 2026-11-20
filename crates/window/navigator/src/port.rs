//! What the Window and the Navigator say to each other: the asks the Frame's model hands
//! the Navigator, and what the Navigator tells back.
//!
//! The Navigator runs on a thread of its own and is reached through one [`Port`]: a
//! channel of [`Ask`]s in, a channel of [`Told`]s out, both unbounded, so that neither side
//! ever waits for the other. The Frame's model `yaar_frame::Navigation` holds the [`Port`];
//! [`crate::Navigator::start`] takes the other end, the [`PortEnd`]. A pane never holds
//! either: it asks through the Frame's model, as it asks the Keeper.
//!
//! Every ask that wants an answer carries an [`AskId`] the asker chose, and is answered
//! once, by a [`Told`] that carries the same id, unless it was cancelled first
//! ([`Ask::Cancel`]): a cancelled ask is never answered. Everything else the Navigator
//! tells unasked: the state of its servers after each change ([`Told::Servers`]), the
//! languages its registry knows once at its start ([`Told::Languages`]), and the old side
//! it needs the Keeper to write ([`Told::ExportWanted`]).
//!
//! An answer found by text match, while no language server is ready for the file, says so
//! ([`Answered::guessed`]): the Window marks every such answer as possibly wrong.
//!
//! These types are plain data: a test plays either end with them, as the tests of the
//! panes play the Keeper with the Link's.

use std::path::PathBuf;
use std::sync::Arc;

use yaar_link::{RepositoryId, WorkspaceId};

/// Names one ask that wants an answer; the asker chooses it, and no two asks of one
/// [`Port`] that still wait for their answer share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AskId(pub u64);

/// The files a place lies in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tree {
    /// The files on disk of the workspace's worktree: the new side of a change that ends
    /// at the files on disk.
    Worktree,
    /// A version of the workspace, a commit of its repository or a recorded state, by its
    /// full id: the old side of a change, the version the change is counted from. Served by
    /// a second server on the export of that version, started at its first use.
    Version(String),
    /// The new side of a change that ends at a version and not at the files on disk, as a
    /// past turn's: that version by its full id, and the text of the file in it, which the
    /// pane holds. A line unchanged between it and the file on disk is answered by the
    /// server of the files on disk, its position carried through the diff of the two; any
    /// other line by text match.
    Past {
        /// The version, by its full id.
        version: String,
        /// The whole text of the file in that version.
        text: Arc<str>,
    },
}

/// A name the reviewer points at in a file of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The files the place lies in.
    pub tree: Tree,
    /// The file, relative to the folder of the worktree: its path in the version of
    /// `tree`. With [`Tree::Worktree`] only, it may be absolute instead: a file outside the
    /// worktree, or inside it and ignored by git, that the reviewer reached through an
    /// answer ([`FileKind::Outside`]); the server of the files on disk of the row of its
    /// extension is asked about it.
    pub path: PathBuf,
    /// The line, counted from 1, as the Link counts lines.
    pub line: u32,
    /// Where the name starts in the line: the offset of its first byte in the line's
    /// UTF-8 text, from 0.
    pub column: u32,
    /// The name itself, as the line holds it: what text match searches for.
    pub name: String,
    /// The files of the change on screen, relative to the folder of the worktree: text
    /// match lists their matches after those of the file under the pointer and before the
    /// rest.
    pub shown: Vec<PathBuf>,
}

/// One place an answer leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The file, by its absolute path, as the server or the search found it.
    pub path: PathBuf,
    /// The line, counted from 1.
    pub line: u32,
    /// Where the place starts in the line: the offset of its first byte in the line's
    /// UTF-8 text, from 0.
    pub column: u32,
    /// The text of the line, with no line end: what a list of references shows.
    pub text: String,
}

/// An answer, and whether it was found by text match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered<T> {
    /// What was found.
    pub value: T,
    /// True when no language server was ready for the file and the answer was found by
    /// text match: it may be wrong, and the Window says so.
    pub guessed: bool,
}

/// What a file the reviewer opens through an answer is, for the comments on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    /// A file of the worktree's files on disk that git does not ignore.
    Worktree {
        /// The workspace.
        workspace: WorkspaceId,
        /// Its path relative to the folder of the worktree.
        relative: PathBuf,
    },
    /// A file of the export of a version: the old side of a change.
    Version {
        /// The workspace.
        workspace: WorkspaceId,
        /// The version, by its full id.
        version: String,
        /// Its path in that version, relative to the folder of the worktree.
        relative: PathBuf,
    },
    /// Anything else: a file outside the worktree, or inside it and ignored by git, as a
    /// dependency's or the standard library's. A comment on it keeps its absolute path.
    Outside,
}

/// A file opened through an answer, for the read-only file view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedFile {
    /// Its absolute path, as the answer named it.
    pub path: PathBuf,
    /// What it is, for the comments on it.
    pub kind: FileKind,
    /// Its whole text, as read when it was opened; a file that is not UTF-8 is refused.
    pub text: String,
}

/// The offer to download a server, which the review pane shows on the line of a missing
/// server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The server's name, as its row names it: `rust-analyzer`.
    pub server: String,
    /// The version the row pins.
    pub version: String,
    /// How many bytes the download is, as the row says.
    pub size: u64,
}

/// How a server stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// A server was found and is started, and its row's readiness rule is not met yet:
    /// names are resolved by text match meanwhile.
    Starting,
    /// Its answers are sound.
    Ready,
    /// No server was found, or every one found failed. The offer to download one, when
    /// the row has a download for this machine and the user registered no server of their
    /// own for the language; none otherwise.
    Missing {
        /// The offer, if any.
        offer: Option<Offer>,
    },
    /// The first server found lies inside a repository whose answer the Navigator has not
    /// been told: it is passed over, and the user is asked. The later candidates are tried
    /// meanwhile, and one of them that is ready answers; their state is not shown while
    /// this one is: the question takes the line until it is answered.
    Asking {
        /// The repository the server lies in.
        repository: RepositoryId,
        /// The server's program, by its absolute path.
        program: PathBuf,
    },
    /// The user accepted the offer, and the download is on its way.
    Downloading {
        /// What is downloaded.
        offer: Offer,
    },
}

/// The state of one server the Navigator runs or looks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerState {
    /// The workspace it serves.
    pub workspace: WorkspaceId,
    /// The registry row, by its name: `rust`.
    pub language: String,
    /// The row's title, to show: `Rust`.
    pub title: String,
    /// For a server of the old side of a change, the version it serves; none for the
    /// server of the files on disk.
    pub old_side: Option<String>,
    /// Whether the row is one the user registered in their own file.
    pub registered: bool,
    /// How it stands.
    pub state: State,
    /// The server of the row that runs or was found, by its name: `ty`; none while none
    /// was found.
    pub server: Option<String>,
    /// The program that runs, by its absolute path, while one runs.
    pub program: Option<PathBuf>,
    /// Its process, while one runs.
    pub pid: Option<u32>,
    /// A newer version the row pins than the one the app downloaded and runs: the update
    /// the user may ask for. None when nothing downloaded runs, or it is the version the row
    /// pins.
    pub update: Option<Offer>,
}

/// A language the registry knows, so that the Window can tell which state is the one of
/// the file on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// The row's name.
    pub name: String,
    /// The row's title.
    pub title: String,
    /// The file extensions of the row, with no dot.
    pub extensions: Vec<String>,
}

/// One workspace as the Navigator needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    /// The workspace.
    pub id: WorkspaceId,
    /// Its repository.
    pub repository: RepositoryId,
    /// The folder of its worktree; none when the worktree is gone.
    pub folder: Option<PathBuf>,
}

/// The user's answer on the servers that lie inside one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConsent {
    /// The repository.
    pub repository: RepositoryId,
    /// True for yes.
    pub allowed: bool,
}

/// The old side the Navigator needs the Keeper to write: a version of a workspace, into a
/// folder of the app's cache that does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportAsk {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The version, by its full id.
    pub version: String,
    /// The folder to write it into.
    pub into: PathBuf,
}

/// What the Frame's model hands the Navigator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Every workspace the Window knows, as its list changed: a workspace that leaves the
    /// list is closed, and its servers and exports go.
    Workspaces(Vec<WorkspaceRoot>),
    /// The workspace on screen now, or none. Its servers run; a workspace off screen for the
    /// set time has its servers stopped.
    OnScreen(Option<WorkspaceId>),
    /// The old side of the change on screen in a workspace: the version the change is
    /// counted from, or none. An export of another version of that workspace, and its
    /// server, go.
    OldSide {
        /// The workspace.
        workspace: WorkspaceId,
        /// The version, by its full id.
        version: Option<String>,
    },
    /// Every answer the user gave on the servers inside a repository, as the Keeper keeps
    /// them; a repository not listed has no answer.
    Consents(Vec<RepositoryConsent>),
    /// Files of a workspace changed on disk: the servers of the files on disk are told. A
    /// path may be told again, and may no longer exist.
    FilesChanged {
        /// The workspace.
        workspace: WorkspaceId,
        /// The files, relative to the folder of the worktree.
        paths: Vec<PathBuf>,
    },
    /// What the language server reports for the name at `place`, as hover text. Answered
    /// by [`Told::Hover`].
    Hover {
        /// The ask.
        id: AskId,
        /// Where.
        place: Place,
    },
    /// Where the name at `place` is defined. Answered by [`Told::Definition`].
    Definition {
        /// The ask.
        id: AskId,
        /// Where.
        place: Place,
    },
    /// Every place that refers to the name at `place`, its definition included. Answered
    /// by [`Told::References`].
    References {
        /// The ask.
        id: AskId,
        /// Where.
        place: Place,
    },
    /// The file of a location, read, for the file view. Answered by [`Told::Opened`].
    Open {
        /// The ask.
        id: AskId,
        /// The location.
        location: Location,
    },
    /// The reviewer moved on: the ask is not answered, and what it asked of a server is
    /// cancelled.
    Cancel {
        /// The ask.
        id: AskId,
    },
    /// The user accepted the offer to download the server of a row: download it into the
    /// app's folder and start it.
    Download {
        /// The row, by its name.
        language: String,
    },
    /// The user asked for the update of a server the app downloaded: download the version
    /// the row pins now and start it in place of the one that runs.
    Update {
        /// The row, by its name.
        language: String,
    },
    /// The Keeper's answer to [`Told::ExportWanted`]: the folder is written, or why not.
    Exported {
        /// What was asked.
        asked: ExportAsk,
        /// None when the folder holds the version; else the reason, as a sentence.
        failed: Option<String>,
    },
}

/// What the Navigator tells the Frame's model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    /// Every language the registry knows, once, at the Navigator's start.
    Languages(Vec<Language>),
    /// Every server the Navigator runs or looks for, after each change of any of them, in
    /// the order of the workspaces, then the rows, the server of the files on disk before
    /// that of an old side.
    Servers(Vec<ServerState>),
    /// The answer to [`Ask::Hover`]: the text the server reports, none where it reports
    /// nothing.
    Hover {
        /// The ask.
        id: AskId,
        /// The answer.
        answer: Answered<Option<String>>,
    },
    /// The answer to [`Ask::Definition`].
    Definition {
        /// The ask.
        id: AskId,
        /// The answer, in the server's order or text match's.
        answer: Answered<Vec<Location>>,
    },
    /// The answer to [`Ask::References`].
    References {
        /// The ask.
        id: AskId,
        /// The answer, by path, then line.
        answer: Answered<Vec<Location>>,
    },
    /// The answer to [`Ask::Open`]: the file, or why it could not be read.
    Opened {
        /// The ask.
        id: AskId,
        /// The file, or the reason, as a sentence.
        file: Result<OpenedFile, String>,
    },
    /// The Navigator needs a version of a workspace written into a folder: the Frame's
    /// model asks the Keeper with `yaar_link::Request::ExportVersion` and hands the answer
    /// back as [`Ask::Exported`].
    ExportWanted(ExportAsk),
}

/// The Frame's end of the Navigator's channels.
#[derive(Debug, Clone)]
pub struct Port {
    /// Where the asks go.
    pub asks: async_channel::Sender<Ask>,
    /// Where what the Navigator tells comes from.
    pub told: async_channel::Receiver<Told>,
}

/// The Navigator's end of its channels.
#[derive(Debug)]
pub struct PortEnd {
    /// Where the asks come from. When every sender is gone, the Navigator stops its
    /// servers and ends.
    pub asks: async_channel::Receiver<Ask>,
    /// Where what the Navigator tells goes.
    pub told: async_channel::Sender<Told>,
}

/// A new pair of ends, joined: what is sent into one comes out of the other.
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
