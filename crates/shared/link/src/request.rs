//! What a Window asks of the Keeper, what the Keeper answers, and what it tells unasked.
//!
//! A request carries an id the Window chose; the Keeper answers each request with one
//! reply that carries the same id, either what was asked for or the reason it was refused.
//! A fact is sent to every attached Window when something changed, whoever caused it.

use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::agent::{
    AgentCli, AgentReport, CreditedFile, History, Step, TerminalInfo, Turn, WorkspaceAgents,
    read_agent_cli, read_credited, read_history, read_report, read_step, read_terminal_info,
    read_turn, read_workspace_agents, write_agent_cli, write_credited, write_history, write_report,
    write_step, write_terminal_info, write_turn, write_workspace_agents,
};
use crate::review::{
    CommentId, DraftOn, ReviewId, SessionId, ThreadId, TurnId, WorkspaceReviews, read_draft_on,
    read_workspace_reviews, write_draft_on, write_workspace_reviews,
};
use crate::setting::{Setting, read_setting, write_setting};
use crate::terminal::{
    Ending, Program, TerminalId, TerminalSize, read_ending, read_program, read_size, write_ending,
    write_program, write_size,
};
use crate::wire::{In, Out};
use crate::workspace::{
    ChangedFile, Changes, FileSides, Repository, RepositoryId, WorkspaceId, read_changed_file,
    read_changes, read_repository, read_sides, write_changed_file, write_changes, write_repository,
    write_sides,
};

/// Pairs a reply with its request. The Window chooses it; within one connection no two
/// requests that still wait for their reply share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u32);

/// Who starts a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The user, by a control of the Window. The Window has an action with a key for every
    /// such request.
    User,
    /// The Window itself, to have something to show.
    Window,
    /// A piece of the app inside an agent CLI, as the hook program of Claude Code, to
    /// report what the CLI does. Such a piece says hello, sends one request and leaves.
    /// It is no Window, and nothing is told for it; a fact told to every connection may
    /// still reach it before its reply, and it reads past it.
    Agent,
}

/// Declares every kind of request in one table: its name, the number it travels by, who
/// starts it, and the sentence that says what it is. [`RequestKind::ALL`] is built from
/// the same table, so a kind cannot exist without its origin and cannot be missing from
/// the list.
macro_rules! request_kinds {
    ($( $(#[$doc:meta])* $name:ident = $code:literal, $origin:ident; )+) => {
        /// The kinds of request, without their content. Every [`Request`] has one.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum RequestKind {
            $( $(#[$doc])* $name, )+
        }

        impl RequestKind {
            /// Every kind of request there is.
            pub const ALL: &'static [RequestKind] = &[ $( RequestKind::$name, )+ ];

            /// Who starts a request of this kind.
            pub const fn origin(self) -> Origin {
                match self {
                    $( RequestKind::$name => Origin::$origin, )+
                }
            }

            /// The number this kind travels by.
            pub const fn code(self) -> u16 {
                match self {
                    $( RequestKind::$name => $code, )+
                }
            }

            /// The kind that travels by this number.
            pub const fn from_code(code: u16) -> Option<RequestKind> {
                match code {
                    $( $code => Some(RequestKind::$name), )+
                    _ => None,
                }
            }

            /// The name of the kind, as written in the code.
            pub const fn name(self) -> &'static str {
                match self {
                    $( RequestKind::$name => stringify!($name), )+
                }
            }
        }
    };
}

request_kinds! {
    /// See [`Request::ListWorkspaces`].
    ListWorkspaces = 1, Window;
    /// See [`Request::OpenFolder`].
    OpenFolder = 2, User;
    /// See [`Request::SetBase`].
    SetBase = 3, User;
    /// See [`Request::GetChanges`].
    GetChanges = 4, Window;
    /// See [`Request::ReadFile`].
    ReadFile = 5, Window;
    /// See [`Request::OpenTerminal`].
    OpenTerminal = 6, User;
    /// See [`Request::ResizeTerminal`].
    ResizeTerminal = 7, Window;
    /// See [`Request::GetReviews`].
    GetReviews = 8, Window;
    /// See [`Request::SaveDraft`].
    SaveDraft = 9, Window;
    /// See [`Request::DeleteDraft`].
    DeleteDraft = 10, User;
    /// See [`Request::CopyReview`].
    CopyReview = 11, User;
    /// See [`Request::MarkReviewed`].
    MarkReviewed = 12, User;
    /// See [`Request::SetViewed`].
    SetViewed = 13, User;
    /// See [`Request::Commit`].
    Commit = 14, User;
    /// See [`Request::Report`].
    Report = 15, Agent;
    /// See [`Request::ListAgents`].
    ListAgents = 16, Window;
    /// See [`Request::GetHistory`].
    GetHistory = 17, Window;
    /// See [`Request::CompareVersions`].
    CompareVersions = 18, Window;
    /// See [`Request::ReadVersions`].
    ReadVersions = 19, Window;
    /// See [`Request::GetTurn`].
    GetTurn = 20, Window;
    /// See [`Request::ShowWorkspace`].
    ShowWorkspace = 21, Window;
    /// See [`Request::ListTerminals`].
    ListTerminals = 22, Window;
    /// See [`Request::AttachTerminal`].
    AttachTerminal = 23, Window;
    /// See [`Request::GetUncommitted`].
    GetUncommitted = 24, Window;
    /// See [`Request::GetSettings`].
    GetSettings = 25, Window;
    /// See [`Request::SetSetting`].
    SetSetting = 26, User;
    /// See [`Request::AttachReview`].
    AttachReview = 27, User;
    /// See [`Request::DetachReview`].
    DetachReview = 28, User;
    /// See [`Request::ResolveThread`].
    ResolveThread = 29, User;
    /// See [`Request::ReopenThread`].
    ReopenThread = 30, User;
    /// See [`Request::ResolveAddressed`].
    ResolveAddressed = 31, User;
    /// See [`Request::GetAttachment`].
    GetAttachment = 32, Window;
    /// See [`Request::ListClis`].
    ListClis = 33, Window;
    /// See [`Request::CompareForReview`].
    CompareForReview = 34, Window;
}

impl RequestKind {
    /// The kinds of request a user starts, in the order of [`RequestKind::ALL`].
    pub fn started_by_user() -> impl Iterator<Item = RequestKind> {
        Self::ALL
            .iter()
            .copied()
            .filter(|kind| kind.origin() == Origin::User)
    }
}

/// Something a Window asks of the Keeper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Every repository the user opened, with its worktrees. Answered by
    /// [`Reply::Workspaces`].
    ListWorkspaces,
    /// Add the repository that holds this folder, and with it the workspace of the
    /// worktree the folder lies in. Answered by [`Reply::Opened`], or refused when the
    /// folder holds no repository and no worktree.
    OpenFolder {
        /// The folder the user chose.
        path: PathBuf,
    },
    /// Count the changes of a workspace from this branch or commit from now on. Answered
    /// by [`Reply::Done`], or refused when git knows no such branch or commit.
    ///
    /// A branch is followed: the base moves when the branch does. Anything else the user
    /// wrote is kept as the commit it names now, and stays that commit.
    SetBase {
        /// The workspace.
        workspace: WorkspaceId,
        /// A branch or a commit, as the user wrote it.
        base: String,
    },
    /// The changes of a workspace as they are now. Answered by [`Reply::Changes`].
    GetChanges {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// The two versions of one changed file of a workspace. The Window names both, from
    /// the [`Changes`] it holds: the old one by the commit the changes are counted from
    /// and the path the file had there, the new one by its path in the worktree. The two
    /// versions therefore belong to the list they were asked from, whatever happened to
    /// the base or the branch since. Answered by [`Reply::File`]; refused when a path
    /// leads out of the worktree or git knows no such commit.
    ReadFile {
        /// The workspace.
        workspace: WorkspaceId,
        /// The commit the old version is read in: [`Changes::counted_from`]. With none,
        /// no old version is read and the reply holds none.
        since: Option<String>,
        /// The path the file had in that commit, relative to the folder of the worktree:
        /// its path now, or for a renamed file the path it had before.
        old_path: PathBuf,
        /// The path of the file now, relative to the folder of the worktree.
        path: PathBuf,
    },
    /// Open a new terminal for a workspace and start `program` in it, in the folder of
    /// the worktree, at `size`. Answered by [`Reply::TerminalOpened`]; refused when the
    /// Keeper knows no such workspace or the program cannot be started.
    ///
    /// From the reply on, what the program writes comes to the Window that asked, as
    /// [`crate::Message::Output`] frames, and what the Window sends as
    /// [`crate::Message::Input`] goes to the program. The reply reaches the Window before
    /// the first output of the terminal, and no output is lost in between. When the
    /// program ends, every Window is told [`Fact::TerminalEnded`], after the last output.
    OpenTerminal {
        /// The workspace the terminal belongs to.
        workspace: WorkspaceId,
        /// What to run in it.
        program: Program,
        /// How large the Window will show it.
        size: TerminalSize,
    },
    /// The Window shows a terminal at another size now: tell its program. Answered by
    /// [`Reply::Done`]; refused with [`Refusal::UnknownTerminal`] when the Keeper holds no
    /// such terminal, as after its program ended.
    ResizeTerminal {
        /// The terminal.
        terminal: TerminalId,
        /// Its new size.
        size: TerminalSize,
    },
    /// The reviews of a workspace, with their threads, comments and rounds, and its files
    /// marked viewed. Answered by [`Reply::Reviews`]; refused with
    /// [`Refusal::UnknownWorkspace`] when the Keeper knows no such workspace.
    ///
    /// A workspace whose worktree is gone still has its reviews, and no file viewed. A
    /// workspace with no review of an agent session has one review, of no agent, which the
    /// Keeper makes when it is first asked for.
    GetReviews {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// Keep the text of a draft as it is now. The Window sends it whenever the text in a
    /// comment box changes, with the whole text each time, and by itself: the user types,
    /// and no control stands for the save. Answered by [`Reply::Done`] once the text is in
    /// the Keeper's database, where a killed Keeper finds it again; the drive is flushed
    /// after a pause in the typing, not at each save.
    ///
    /// The first save of a comment makes it, as a draft: the first comment of a new
    /// thread, which sits where `on` says, or a reply in a thread that has a sent comment
    /// and no draft. A later save of the same comment changes its text, also of a draft
    /// that a reply made. Refused with [`Refusal::NotADraft`] when the comment was sent,
    /// [`Refusal::UnknownReview`] or [`Refusal::UnknownThread`] for what the Keeper does
    /// not hold, and [`Refusal::Failed`] when the ids contradict what is kept, as a
    /// thread id taken by another thread or a reply in a thread with nothing sent.
    ///
    /// A save that made a comment is told to every Window as [`Fact::ReviewsChanged`]; a
    /// save that only changed a text is told to nobody, so that typing does not make every
    /// Window read the reviews again. The Window that typed puts each text it saves into
    /// the reviews it shows itself.
    SaveDraft {
        /// The review.
        review: ReviewId,
        /// The thread: new, when `on` starts one.
        thread: ThreadId,
        /// The comment.
        comment: CommentId,
        /// What the draft belongs to.
        on: DraftOn,
        /// The whole text of the draft.
        text: String,
    },
    /// Delete a draft. A thread left with no comment goes with it. Answered by
    /// [`Reply::Done`]; refused with [`Refusal::NotADraft`] for a comment that was sent,
    /// and [`Refusal::UnknownReview`] or [`Refusal::UnknownComment`] for what the Keeper
    /// does not hold.
    DeleteDraft {
        /// The review.
        review: ReviewId,
        /// The draft.
        comment: CommentId,
    },
    /// Copy a review as text, to paste by hand to an agent the app cannot deliver to.
    /// Every unsent comment on an unresolved thread is rendered as text, with its path, its
    /// lines and the code it sits on, and is then sent: the state of the worktree is
    /// recorded and a new round starts, as when a message carries the review. Answered by
    /// [`Reply::Copied`], which the Window puts on the clipboard; refused with
    /// [`Refusal::NothingToSend`] when there is no such comment, and nothing is recorded.
    CopyReview {
        /// The review.
        review: ReviewId,
    },
    /// Mark the change reviewed: the state of the worktree as it is now is recorded and
    /// becomes the reviewed state of the review, from which the next review starts. No
    /// commit is made. Answered by [`Reply::Done`].
    MarkReviewed {
        /// The review.
        review: ReviewId,
    },
    /// Mark one file of a workspace viewed, or take the mark away. A mark holds for the
    /// content the file has now: when the file changes, the mark is gone. Answered by
    /// [`Reply::Done`]; refused when the path leads out of the worktree.
    SetViewed {
        /// The workspace.
        workspace: WorkspaceId,
        /// The file, relative to the folder of the worktree.
        path: PathBuf,
        /// Whether it is to be marked viewed.
        viewed: bool,
    },
    /// Make an ordinary git commit of every uncommitted change of the worktree, with this
    /// message, as the user's own git makes it: with the user's hooks, identity and
    /// signing. Nothing is marked reviewed, and what counts as changed since the last
    /// review stays as it was. Answered by [`Reply::Committed`]; refused with
    /// [`Refusal::NothingToCommit`] when nothing is uncommitted, and with
    /// [`Refusal::Failed`] and git's own words when git refuses, as a hook may.
    Commit {
        /// The workspace.
        workspace: WorkspaceId,
        /// The message, as the user wrote it.
        message: String,
    },
    /// What an agent CLI handed a piece of the app, passed on by that piece: the input of
    /// one of Claude Code's hooks, or a call of the app's reply tool. Answered once the
    /// agents' side of the Keeper has taken it, in the order reports and the marks of its
    /// terminals came: with [`Reply::Answer`], what the piece hands its CLI back, where the
    /// CLI's Adapter has something for it, else [`Reply::Done`]. Nothing the report leads
    /// to beyond that is waited for, a recording least of all: the CLI waits for its
    /// piece. Refused with [`Refusal::Failed`] only when the Keeper has no Adapter by the
    /// report's name. A report the Adapter cannot read is answered [`Reply::Done`], and
    /// written down by the Keeper.
    ///
    /// What the piece is answered with is never a decision on anything the CLI asks the
    /// user, a permission prompt above all: an attached review that rides with a message
    /// the user submitted; the allowing of a read of a review file the app wrote for that
    /// session, and of a call of the app's own reply tool, both of which the user allowed
    /// by installing the app; and what a reply to a thread became. What the report leads
    /// to is told to every Window as facts: [`Fact::AgentsChanged`], [`Fact::Step`],
    /// [`Fact::TurnEnded`] and [`Fact::ReviewsChanged`].
    Report {
        /// What the CLI handed the piece.
        report: AgentReport,
    },
    /// The agent sessions of every workspace, and the state each workspace shows. Answered
    /// by [`Reply::Agents`], one entry for each workspace in which a session of a supported
    /// agent CLI ever reported, in the order the workspaces were opened.
    ListAgents,
    /// The history of a workspace: its commits and the turns that ended in it. Answered by
    /// [`Reply::History`], from what the Keeper keeps and the commits, without asking git
    /// what changed on disk; refused with [`Refusal::UnknownWorkspace`], also for a
    /// workspace whose worktree is gone.
    GetHistory {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// The uncommitted changes of a workspace that a review lists, from the commit checked
    /// out to the files on disk, each with who made it: every change no agent made, and
    /// those credited to the review's session; for the review of no agent, or none named,
    /// every change. Answered by [`Reply::Uncommitted`], ordered by path; refused with
    /// [`Refusal::UnknownWorkspace`], and [`Refusal::UnknownReview`] for a review the
    /// Keeper does not hold.
    GetUncommitted {
        /// The workspace.
        workspace: WorkspaceId,
        /// The review whose list is asked for.
        review: Option<ReviewId>,
    },
    /// The files that differ between two versions of a workspace: commits of the
    /// repository or recorded states, named by their full ids, or the files on disk.
    /// Answered by [`Reply::Compared`], the files ordered by path; refused with
    /// [`Refusal::UnknownRevision`] for an id that is neither a commit nor a state of the
    /// workspace.
    CompareVersions {
        /// The workspace.
        workspace: WorkspaceId,
        /// The version the change is counted from.
        from: String,
        /// The version the change leads to; none for the files on disk.
        to: Option<String>,
    },
    /// The two versions of one file of a change between two versions of a workspace, as
    /// [`Request::CompareVersions`] names them. Answered by [`Reply::File`]; refused as
    /// [`Request::ReadFile`] and [`Request::CompareVersions`] are.
    ReadVersions {
        /// The workspace.
        workspace: WorkspaceId,
        /// The version the old side is read in; none reads no old side, and the reply
        /// holds none.
        old: Option<String>,
        /// The path of the file in that version, relative to the folder of the worktree.
        old_path: PathBuf,
        /// The version the new side is read in; none for the file on disk.
        new: Option<String>,
        /// The path of the file in that version, relative to the folder of the worktree.
        path: PathBuf,
    },
    /// One turn with its steps, as the Keeper kept them: also a turn that ended while no
    /// Window was attached. Answered by [`Reply::Turn`]; refused with
    /// [`Refusal::UnknownTurn`].
    GetTurn {
        /// The turn.
        turn: TurnId,
    },
    /// The Window shows this workspace now, or none. The Window tells it by itself
    /// whenever the workspace on screen changes; the Keeper watches the worktrees that
    /// some Window shows or an agent works in, and tells what changes in them as
    /// [`Fact::ChangesChanged`] and [`Fact::Workspaces`]. Answered by [`Reply::Done`];
    /// refused with [`Refusal::UnknownWorkspace`].
    ShowWorkspace {
        /// The workspace on screen.
        workspace: Option<WorkspaceId>,
    },
    /// Every terminal the Keeper holds, whoever opened it, in the order they were opened.
    /// Answered by [`Reply::Terminals`].
    ListTerminals,
    /// Show a terminal the Keeper holds in this Window too, as one opened again after a
    /// plain quit. Answered by [`Reply::Attached`]; refused with
    /// [`Refusal::UnknownTerminal`].
    ///
    /// After the reply the Window is sent, as [`crate::Message::Output`] frames, the
    /// newest output the Keeper kept of the terminal, up to its limit, and then what the
    /// program writes from then on, with nothing lost or sent twice in between. The
    /// program is told the size and asked to draw its screen again.
    AttachTerminal {
        /// The terminal.
        terminal: TerminalId,
        /// The size the Window shows it at.
        size: TerminalSize,
    },
    /// Every setting the user set, as the Keeper keeps them. Answered by
    /// [`Reply::Settings`]. The Window asks it when it connects, and holds the default of
    /// every setting the answer does not list.
    GetSettings,
    /// Keep a setting the user chose, in place of the one it is the same thing as. Answered
    /// by [`Reply::Done`] once the Keeper's database holds it, where a Keeper started again
    /// finds it; refused with [`Refusal::UnknownWorkspace`] for a setting of a workspace the
    /// Keeper does not know. Told to every Window as [`Fact::SettingChanged`].
    ///
    /// The user starts it by a control: switching the form of Flow keeps the form chosen
    /// for the workspace on screen.
    SetSetting {
        /// The setting, with its value.
        setting: Setting,
    },
    /// Attach the review, or one of its threads alone, to the user's next message to the
    /// agent the review belongs to, in place of what was attached. The comments are not
    /// sent now: the review rides with the next message the user submits to that agent,
    /// and its unsent comments on unresolved threads are then sent, as they are at that
    /// moment. While it is attached its drafts can be edited, and a new draft joins it,
    /// except when one thread is attached alone. Answered by [`Reply::AttachedReview`],
    /// which names the agent CLIs the Window offers to start with the review; refused with
    /// [`Refusal::NothingToSend`] when what would be attached holds no unsent comment on
    /// an unresolved thread, and [`Refusal::UnknownReview`], [`Refusal::UnknownThread`].
    /// Told as [`Fact::ReviewsChanged`], and as [`Fact::AgentsChanged`] of the session the
    /// review belongs to, whose terminal shows the attachment.
    AttachReview {
        /// The review.
        review: ReviewId,
        /// One thread to attach alone; none attaches the review.
        thread: Option<ThreadId>,
    },
    /// Take the attachment away: every comment it would have carried stays an unsent
    /// draft, and the next message carries no review. Answered by [`Reply::Done`], also
    /// when nothing was attached; refused with [`Refusal::UnknownReview`]. Told as
    /// [`Request::AttachReview`] is.
    DetachReview {
        /// The review.
        review: ReviewId,
    },
    /// Resolve a thread: the reviewer's own decision, which nothing the agent sends can
    /// take. A resolved thread is sent no more, an unsent comment on it included. Answered
    /// by [`Reply::Done`]; refused with [`Refusal::UnknownReview`],
    /// [`Refusal::UnknownThread`]. Told as [`Fact::ReviewsChanged`].
    ResolveThread {
        /// The review.
        review: ReviewId,
        /// The thread.
        thread: ThreadId,
    },
    /// Reopen a resolved thread: it is sent again with the next message, its unsent
    /// comments included. Answered and told as [`Request::ResolveThread`].
    ReopenThread {
        /// The review.
        review: ReviewId,
        /// The thread.
        thread: ThreadId,
    },
    /// Resolve every thread of the review that the agent marked addressed and that is not
    /// resolved, at once. Answered by [`Reply::Done`], also when there is none; refused
    /// with [`Refusal::UnknownReview`]. Told as [`Fact::ReviewsChanged`] when a thread was
    /// resolved.
    ResolveAddressed {
        /// The review.
        review: ReviewId,
    },
    /// What the user's next message would carry to the agent now, as the agent will
    /// receive it: what the control that views the attachment shows. Answered by
    /// [`Reply::Attachment`]; refused with [`Refusal::NothingToSend`] when nothing is
    /// attached, when what is attached holds no unsent comment on an unresolved thread,
    /// and for the review of no agent, which no agent receives until a session takes it;
    /// and with [`Refusal::UnknownReview`].
    GetAttachment {
        /// The review.
        review: ReviewId,
    },
    /// Every agent CLI the Keeper has an Adapter for, in the order of its Adapters: what
    /// the terminal pane offers a launcher of, next to the shell. Answered by
    /// [`Reply::Clis`]. The list does not change while a Keeper runs: a Window asks once
    /// for each connection.
    ListClis,
    /// What a review shows of the change between two versions of its workspace: the files
    /// that differ from `from` to `to`, as [`Request::CompareVersions`] lists them, left out
    /// those credited to sessions other than the review's own and to no session of no
    /// agent; each with who is credited with it, so that a file two sessions changed is
    /// marked. For the review of no agent, every file. Who is credited with a file is what
    /// the Keeper kept with the states recorded after `from` up to `to`. Answered by
    /// [`Reply::ComparedForReview`], ordered by path; refused with
    /// [`Refusal::UnknownWorkspace`], [`Refusal::UnknownReview`] and
    /// [`Refusal::UnknownRevision`], as [`Request::CompareVersions`] is.
    CompareForReview {
        /// The workspace.
        workspace: WorkspaceId,
        /// The review whose files are asked for.
        review: ReviewId,
        /// The version the change is counted from.
        from: String,
        /// The recorded state the change leads to.
        to: String,
    },
}

impl Request {
    /// The kind of this request.
    pub fn kind(&self) -> RequestKind {
        match self {
            Self::ListWorkspaces => RequestKind::ListWorkspaces,
            Self::OpenFolder { .. } => RequestKind::OpenFolder,
            Self::SetBase { .. } => RequestKind::SetBase,
            Self::GetChanges { .. } => RequestKind::GetChanges,
            Self::ReadFile { .. } => RequestKind::ReadFile,
            Self::OpenTerminal { .. } => RequestKind::OpenTerminal,
            Self::ResizeTerminal { .. } => RequestKind::ResizeTerminal,
            Self::GetReviews { .. } => RequestKind::GetReviews,
            Self::SaveDraft { .. } => RequestKind::SaveDraft,
            Self::DeleteDraft { .. } => RequestKind::DeleteDraft,
            Self::CopyReview { .. } => RequestKind::CopyReview,
            Self::MarkReviewed { .. } => RequestKind::MarkReviewed,
            Self::SetViewed { .. } => RequestKind::SetViewed,
            Self::Commit { .. } => RequestKind::Commit,
            Self::Report { .. } => RequestKind::Report,
            Self::ListAgents => RequestKind::ListAgents,
            Self::GetHistory { .. } => RequestKind::GetHistory,
            Self::CompareVersions { .. } => RequestKind::CompareVersions,
            Self::ReadVersions { .. } => RequestKind::ReadVersions,
            Self::GetTurn { .. } => RequestKind::GetTurn,
            Self::ShowWorkspace { .. } => RequestKind::ShowWorkspace,
            Self::ListTerminals => RequestKind::ListTerminals,
            Self::AttachTerminal { .. } => RequestKind::AttachTerminal,
            Self::GetUncommitted { .. } => RequestKind::GetUncommitted,
            Self::GetSettings => RequestKind::GetSettings,
            Self::SetSetting { .. } => RequestKind::SetSetting,
            Self::AttachReview { .. } => RequestKind::AttachReview,
            Self::DetachReview { .. } => RequestKind::DetachReview,
            Self::ResolveThread { .. } => RequestKind::ResolveThread,
            Self::ReopenThread { .. } => RequestKind::ReopenThread,
            Self::ResolveAddressed { .. } => RequestKind::ResolveAddressed,
            Self::GetAttachment { .. } => RequestKind::GetAttachment,
            Self::ListClis => RequestKind::ListClis,
            Self::CompareForReview { .. } => RequestKind::CompareForReview,
        }
    }
}

/// What the Keeper answers to a request it could fulfil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// It is done, and there is nothing to send back.
    Done,
    /// Every repository the user opened, in the order they were opened.
    Workspaces(Vec<Repository>),
    /// The folder is now a workspace.
    Opened {
        /// The repository that holds it.
        repository: RepositoryId,
        /// The workspace of the worktree the folder lies in.
        workspace: WorkspaceId,
    },
    /// The changes of the workspace asked about.
    Changes(Changes),
    /// The two versions of the file asked about.
    File(FileSides),
    /// The terminal is open and its program started.
    TerminalOpened {
        /// The name of the new terminal.
        terminal: TerminalId,
    },
    /// The reviews of the workspace asked about.
    Reviews(WorkspaceReviews),
    /// The review as text, and the round its comments were sent in.
    Copied {
        /// The text to put on the clipboard.
        text: String,
        /// The number of the round that started.
        round: u32,
    },
    /// The commit is made.
    Committed {
        /// Its full id.
        commit: String,
    },
    /// The agent sessions of every workspace in which one reported.
    Agents(Vec<WorkspaceAgents>),
    /// The history of the workspace asked about.
    History(History),
    /// The files that differ between the two versions asked about, ordered by path.
    Compared(Vec<ChangedFile>),
    /// The turn asked about.
    Turn(Turn),
    /// Every terminal the Keeper holds.
    Terminals(Vec<TerminalInfo>),
    /// The terminal is shown in this Window too; its kept output follows.
    Attached {
        /// Whether the program wrote more than the Keeper keeps: what came before the
        /// kept output is gone, and the Window says so where the kept output starts.
        cut: bool,
    },
    /// The uncommitted changes a review lists, ordered by path, each with who made it.
    Uncommitted(Vec<CreditedFile>),
    /// Every setting the user set, each once: those of the whole app first, then those of
    /// each workspace, in the order the workspaces were first kept.
    Settings(Vec<Setting>),
    /// What the piece of the app that sent a [`Request::Report`] hands its CLI back, byte
    /// for byte, as the CLI's Adapter wrote it: for Claude Code's hook program what it
    /// prints, for its reply tool's server the result of the call.
    Answer {
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// The review is attached. The Window offers to start each CLI named here, with the
    /// review attached to its first message: none while the agent the review belongs to
    /// runs, the CLI of that agent when it does not, whether or not another agent runs in
    /// the workspace, and every CLI the app supports for the review of no agent.
    AttachedReview {
        /// The CLIs to offer.
        start: Vec<AgentCli>,
    },
    /// What the next message would carry to the agent, as the agent will receive it: the
    /// instruction on how to answer each thread, then the threads. A review delivered as a
    /// file is this text in the file.
    Attachment {
        /// The text.
        text: String,
    },
    /// Every agent CLI the Keeper has an Adapter for, in the order of its Adapters.
    Clis(Vec<AgentCli>),
    /// The files a review shows of the change between two versions, ordered by path, each
    /// with the sessions credited with it.
    ComparedForReview(Vec<CreditedFile>),
}

/// Why the Keeper did not do what a request asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There is no folder at this path, or it cannot be read.
    NoSuchFolder {
        /// The path that was given.
        path: PathBuf,
    },
    /// The folder holds no git repository and no worktree.
    NotARepository {
        /// The folder that was given.
        path: PathBuf,
    },
    /// The folder is a repository with no files checked out: one of its worktrees has to
    /// be opened instead.
    BareRepository {
        /// The folder that was given.
        path: PathBuf,
    },
    /// The folder is a copy of a worktree and shares the original's git folder, so git
    /// cannot tell the two apart. A worktree made by git has to be opened instead.
    CopiedWorktree {
        /// The folder that was given.
        path: PathBuf,
    },
    /// The Keeper knows no workspace by this id.
    UnknownWorkspace {
        /// The id that was given.
        workspace: WorkspaceId,
    },
    /// git knows no branch and no commit by this name in the repository.
    UnknownRevision {
        /// The name that was given.
        revision: String,
    },
    /// The path leads out of the worktree.
    OutsideWorkspace {
        /// The path that was given.
        path: PathBuf,
    },
    /// The Keeper holds no terminal by this id: it never opened one, or its program has
    /// ended.
    UnknownTerminal {
        /// The id that was given.
        terminal: TerminalId,
    },
    /// The Keeper holds no review by this id.
    UnknownReview {
        /// The id that was given.
        review: ReviewId,
    },
    /// The review holds no thread by this id.
    UnknownThread {
        /// The id that was given.
        thread: ThreadId,
    },
    /// The review holds no comment by this id.
    UnknownComment {
        /// The id that was given.
        comment: CommentId,
    },
    /// The comment was sent: it carries its round and cannot be changed or deleted. A
    /// follow-up is a reply.
    NotADraft {
        /// The comment.
        comment: CommentId,
    },
    /// The review holds no unsent comment on an unresolved thread: there is nothing to
    /// send.
    NothingToSend {
        /// The review.
        review: ReviewId,
    },
    /// The worktree holds no uncommitted change: there is nothing to commit.
    NothingToCommit {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// Something the user cannot mend from here went wrong: git is missing or failed, the
    /// database could not be written, or a program could not be started.
    Failed {
        /// What went wrong, as a sentence.
        reason: String,
    },
    /// The Keeper holds no turn by this id.
    UnknownTurn {
        /// The id that was given.
        turn: TurnId,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFolder { path } => {
                write!(f, "there is no folder at {}", path.display())
            }
            Self::NotARepository { path } => write!(
                f,
                "{} holds no git repository and no worktree",
                path.display()
            ),
            Self::BareRepository { path } => write!(
                f,
                "{} is a repository with no files checked out; open one of its worktrees",
                path.display()
            ),
            Self::CopiedWorktree { path } => write!(
                f,
                "{} is a copy of a worktree and shares its git folder; make a worktree with git",
                path.display()
            ),
            Self::UnknownWorkspace { workspace } => {
                write!(f, "there is no workspace {workspace}")
            }
            Self::UnknownRevision { revision } => {
                write!(f, "git knows no branch and no commit named {revision}")
            }
            Self::OutsideWorkspace { path } => {
                write!(f, "{} is outside the worktree", path.display())
            }
            Self::UnknownTerminal { terminal } => {
                write!(f, "there is no terminal {terminal}")
            }
            Self::UnknownReview { review } => write!(f, "there is no review {review}"),
            Self::UnknownThread { thread } => write!(f, "there is no thread {thread}"),
            Self::UnknownComment { comment } => write!(f, "there is no comment {comment}"),
            Self::NotADraft { comment } => write!(
                f,
                "the comment {comment} was sent and can no longer be changed; reply to it instead"
            ),
            Self::NothingToSend { review } => write!(
                f,
                "the review {review} holds no unsent comment on an unresolved thread"
            ),
            Self::NothingToCommit { workspace } => {
                write!(f, "the workspace {workspace} has nothing uncommitted")
            }
            Self::Failed { reason } => f.write_str(reason),
            Self::UnknownTurn { turn } => write!(f, "there is no turn {turn}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Something the Keeper tells every attached Window, unasked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// The list of repositories and workspaces is now this. Told when a folder was opened,
    /// and when the watcher of a worktree saw what is checked out in it change.
    ///
    /// Only the worktrees that a Window shows or an agent works in are watched: a worktree
    /// made or removed outside the app, or a branch switched in a worktree nobody shows,
    /// is in the list the next time it is read.
    Workspaces(Vec<Repository>),
    /// The changes of this workspace are no longer what the Keeper last answered: its
    /// base was set, its commits changed, or the watcher saw its files change. A Window
    /// that shows them asks again. While files keep changing, as when an agent writes,
    /// the fact is told at most a few times a second.
    ChangesChanged {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// The program of this terminal has ended, and the Keeper holds the terminal no more.
    /// Told after the last [`crate::Message::Output`] of the terminal.
    TerminalEnded {
        /// The terminal.
        terminal: TerminalId,
        /// How its program ended.
        ending: Ending,
    },
    /// The reviews of this workspace, or its files marked viewed, are no longer what the
    /// Keeper last answered: a thread or a comment was made or deleted, a round started,
    /// the change was marked reviewed, or a file was marked viewed or unmarked. A Window
    /// that shows them asks again. A save that only changed the text of a draft is not
    /// told.
    ReviewsChanged {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// The agent sessions of this workspace, or the state it shows, are now these: a
    /// session started, ended or was taken by a review, or its state changed.
    AgentsChanged(WorkspaceAgents),
    /// A turn of an agent ended, and the state of the worktree was recorded. Told after the
    /// [`Fact::AgentsChanged`] that shows the session's new state.
    TurnEnded {
        /// The workspace.
        workspace: WorkspaceId,
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// The state recorded at its end, by its full id; none when the recording failed.
        state: Option<String>,
        /// The state the reviewer last marked reviewed in the session's review, by its
        /// full id: what changed since the last review counts from it. None before the
        /// first mark: it then counts from the start of the history.
        since: Option<String>,
    },
    /// An agent took one more step in a turn. Steps are told in the order they happened,
    /// each with its number in the turn, and after the [`Fact::AgentsChanged`] that shows
    /// the turn running. Each step is kept before it is told, so that
    /// [`Request::GetTurn`] gives every step told so far.
    Step {
        /// The workspace the session works in.
        workspace: WorkspaceId,
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// The number of the step in the turn, from 1.
        number: u32,
        /// The step.
        step: Step,
    },
    /// A Window set a setting: every Window is told it, the one that set it included, in
    /// the order the settings were kept.
    SettingChanged(Setting),
}

pub(crate) fn write_request(out: &mut Out, id: RequestId, request: &Request) {
    out.u32(id.0);
    out.u16(request.kind().code());
    match request {
        Request::ListWorkspaces => {}
        Request::OpenFolder { path } => out.path(path),
        Request::SetBase { workspace, base } => {
            out.text(workspace.as_str());
            out.text(base);
        }
        Request::GetChanges { workspace } => out.text(workspace.as_str()),
        Request::ReadFile {
            workspace,
            since,
            old_path,
            path,
        } => {
            out.text(workspace.as_str());
            out.optional(since.as_deref(), Out::text);
            out.path(old_path);
            out.path(path);
        }
        Request::OpenTerminal {
            workspace,
            program,
            size,
        } => {
            out.text(workspace.as_str());
            write_program(out, program);
            write_size(out, *size);
        }
        Request::ResizeTerminal { terminal, size } => {
            out.text(terminal.as_str());
            write_size(out, *size);
        }
        Request::GetReviews { workspace } => out.text(workspace.as_str()),
        Request::SaveDraft {
            review,
            thread,
            comment,
            on,
            text,
        } => {
            out.text(review.as_str());
            out.text(thread.as_str());
            out.text(comment.as_str());
            write_draft_on(out, on);
            out.text(text);
        }
        Request::DeleteDraft { review, comment } => {
            out.text(review.as_str());
            out.text(comment.as_str());
        }
        Request::CopyReview { review } | Request::MarkReviewed { review } => {
            out.text(review.as_str())
        }
        Request::SetViewed {
            workspace,
            path,
            viewed,
        } => {
            out.text(workspace.as_str());
            out.path(path);
            out.u8(u8::from(*viewed));
        }
        Request::Commit { workspace, message } => {
            out.text(workspace.as_str());
            out.text(message);
        }
        Request::Report { report } => write_report(out, report),
        Request::ListAgents | Request::ListTerminals => {}
        Request::GetHistory { workspace } => out.text(workspace.as_str()),
        Request::CompareVersions {
            workspace,
            from,
            to,
        } => {
            out.text(workspace.as_str());
            out.text(from);
            out.optional(to.as_deref(), Out::text);
        }
        Request::ReadVersions {
            workspace,
            old,
            old_path,
            new,
            path,
        } => {
            out.text(workspace.as_str());
            out.optional(old.as_deref(), Out::text);
            out.path(old_path);
            out.optional(new.as_deref(), Out::text);
            out.path(path);
        }
        Request::GetTurn { turn } => out.text(turn.as_str()),
        Request::GetUncommitted { workspace, review } => {
            out.text(workspace.as_str());
            out.optional(review.as_ref(), |out, review| out.text(review.as_str()));
        }
        Request::ShowWorkspace { workspace } => {
            out.optional(workspace.as_ref(), |out, workspace| {
                out.text(workspace.as_str())
            });
        }
        Request::AttachTerminal { terminal, size } => {
            out.text(terminal.as_str());
            write_size(out, *size);
        }
        Request::GetSettings => {}
        Request::SetSetting { setting } => write_setting(out, setting),
        Request::AttachReview { review, thread } => {
            out.text(review.as_str());
            out.optional(thread.as_ref(), |out, thread| out.text(thread.as_str()));
        }
        Request::DetachReview { review }
        | Request::ResolveAddressed { review }
        | Request::GetAttachment { review } => out.text(review.as_str()),
        Request::ResolveThread { review, thread } | Request::ReopenThread { review, thread } => {
            out.text(review.as_str());
            out.text(thread.as_str());
        }
        Request::ListClis => {}
        Request::CompareForReview {
            workspace,
            review,
            from,
            to,
        } => {
            out.text(workspace.as_str());
            out.text(review.as_str());
            out.text(from);
            out.text(to);
        }
    }
}

pub(crate) fn read_request(input: &mut In<'_>) -> Result<(RequestId, Request), LinkError> {
    let id = RequestId(input.u32()?);
    let kind = RequestKind::from_code(input.u16()?)
        .ok_or(LinkError::Malformed("unknown kind of request"))?;
    let request = match kind {
        RequestKind::ListWorkspaces => Request::ListWorkspaces,
        RequestKind::OpenFolder => Request::OpenFolder {
            path: input.path()?,
        },
        RequestKind::SetBase => Request::SetBase {
            workspace: WorkspaceId::new(input.text()?),
            base: input.text()?,
        },
        RequestKind::GetChanges => Request::GetChanges {
            workspace: WorkspaceId::new(input.text()?),
        },
        RequestKind::ReadFile => Request::ReadFile {
            workspace: WorkspaceId::new(input.text()?),
            since: input.optional(In::text)?,
            old_path: input.path()?,
            path: input.path()?,
        },
        RequestKind::OpenTerminal => Request::OpenTerminal {
            workspace: WorkspaceId::new(input.text()?),
            program: read_program(input)?,
            size: read_size(input)?,
        },
        RequestKind::ResizeTerminal => Request::ResizeTerminal {
            terminal: TerminalId::new(input.text()?),
            size: read_size(input)?,
        },
        RequestKind::GetReviews => Request::GetReviews {
            workspace: WorkspaceId::new(input.text()?),
        },
        RequestKind::SaveDraft => Request::SaveDraft {
            review: ReviewId::new(input.text()?),
            thread: ThreadId::new(input.text()?),
            comment: CommentId::new(input.text()?),
            on: read_draft_on(input)?,
            text: input.text()?,
        },
        RequestKind::DeleteDraft => Request::DeleteDraft {
            review: ReviewId::new(input.text()?),
            comment: CommentId::new(input.text()?),
        },
        RequestKind::CopyReview => Request::CopyReview {
            review: ReviewId::new(input.text()?),
        },
        RequestKind::MarkReviewed => Request::MarkReviewed {
            review: ReviewId::new(input.text()?),
        },
        RequestKind::SetViewed => Request::SetViewed {
            workspace: WorkspaceId::new(input.text()?),
            path: input.path()?,
            viewed: match input.u8()? {
                0 => false,
                1 => true,
                _ => return Err(LinkError::Malformed("a flag is neither 0 nor 1")),
            },
        },
        RequestKind::Commit => Request::Commit {
            workspace: WorkspaceId::new(input.text()?),
            message: input.text()?,
        },
        RequestKind::Report => Request::Report {
            report: read_report(input)?,
        },
        RequestKind::ListAgents => Request::ListAgents,
        RequestKind::GetHistory => Request::GetHistory {
            workspace: WorkspaceId::new(input.text()?),
        },
        RequestKind::CompareVersions => Request::CompareVersions {
            workspace: WorkspaceId::new(input.text()?),
            from: input.text()?,
            to: input.optional(In::text)?,
        },
        RequestKind::ReadVersions => Request::ReadVersions {
            workspace: WorkspaceId::new(input.text()?),
            old: input.optional(In::text)?,
            old_path: input.path()?,
            new: input.optional(In::text)?,
            path: input.path()?,
        },
        RequestKind::GetTurn => Request::GetTurn {
            turn: TurnId::new(input.text()?),
        },
        RequestKind::ShowWorkspace => Request::ShowWorkspace {
            workspace: input.optional(|input| Ok(WorkspaceId::new(input.text()?)))?,
        },
        RequestKind::ListTerminals => Request::ListTerminals,
        RequestKind::GetUncommitted => Request::GetUncommitted {
            workspace: WorkspaceId::new(input.text()?),
            review: input.optional(|input| Ok(ReviewId::new(input.text()?)))?,
        },
        RequestKind::AttachTerminal => Request::AttachTerminal {
            terminal: TerminalId::new(input.text()?),
            size: read_size(input)?,
        },
        RequestKind::GetSettings => Request::GetSettings,
        RequestKind::SetSetting => Request::SetSetting {
            setting: read_setting(input)?,
        },
        RequestKind::AttachReview => Request::AttachReview {
            review: ReviewId::new(input.text()?),
            thread: input.optional(|input| Ok(ThreadId::new(input.text()?)))?,
        },
        RequestKind::DetachReview => Request::DetachReview {
            review: ReviewId::new(input.text()?),
        },
        RequestKind::ResolveThread => Request::ResolveThread {
            review: ReviewId::new(input.text()?),
            thread: ThreadId::new(input.text()?),
        },
        RequestKind::ReopenThread => Request::ReopenThread {
            review: ReviewId::new(input.text()?),
            thread: ThreadId::new(input.text()?),
        },
        RequestKind::ResolveAddressed => Request::ResolveAddressed {
            review: ReviewId::new(input.text()?),
        },
        RequestKind::GetAttachment => Request::GetAttachment {
            review: ReviewId::new(input.text()?),
        },
        RequestKind::ListClis => Request::ListClis,
        RequestKind::CompareForReview => Request::CompareForReview {
            workspace: WorkspaceId::new(input.text()?),
            review: ReviewId::new(input.text()?),
            from: input.text()?,
            to: input.text()?,
        },
    };
    Ok((id, request))
}

pub(crate) fn write_reply(out: &mut Out, to: RequestId, reply: &Result<Reply, Refusal>) {
    out.u32(to.0);
    match reply {
        Ok(reply) => {
            out.u8(0);
            match reply {
                Reply::Done => out.u16(0),
                Reply::Workspaces(repositories) => {
                    out.u16(1);
                    out.list(repositories, write_repository);
                }
                Reply::Opened {
                    repository,
                    workspace,
                } => {
                    out.u16(2);
                    out.text(repository.as_str());
                    out.text(workspace.as_str());
                }
                Reply::Changes(changes) => {
                    out.u16(3);
                    write_changes(out, changes);
                }
                Reply::File(sides) => {
                    out.u16(4);
                    write_sides(out, sides);
                }
                Reply::TerminalOpened { terminal } => {
                    out.u16(5);
                    out.text(terminal.as_str());
                }
                Reply::Reviews(reviews) => {
                    out.u16(6);
                    write_workspace_reviews(out, reviews);
                }
                Reply::Copied { text, round } => {
                    out.u16(7);
                    out.text(text);
                    out.u32(*round);
                }
                Reply::Committed { commit } => {
                    out.u16(8);
                    out.text(commit);
                }
                Reply::Agents(agents) => {
                    out.u16(9);
                    out.list(agents, write_workspace_agents);
                }
                Reply::History(history) => {
                    out.u16(10);
                    write_history(out, history);
                }
                Reply::Compared(files) => {
                    out.u16(11);
                    out.list(files, write_changed_file);
                }
                Reply::Turn(turn) => {
                    out.u16(12);
                    write_turn(out, turn);
                }
                Reply::Terminals(terminals) => {
                    out.u16(13);
                    out.list(terminals, write_terminal_info);
                }
                Reply::Attached { cut } => {
                    out.u16(14);
                    out.u8(u8::from(*cut));
                }
                Reply::Uncommitted(files) => {
                    out.u16(15);
                    out.list(files, write_credited);
                }
                Reply::Settings(settings) => {
                    out.u16(16);
                    out.list(settings, write_setting);
                }
                Reply::Answer { bytes } => {
                    out.u16(17);
                    out.bytes(bytes);
                }
                Reply::AttachedReview { start } => {
                    out.u16(18);
                    out.list(start, write_agent_cli);
                }
                Reply::Attachment { text } => {
                    out.u16(19);
                    out.text(text);
                }
                Reply::Clis(clis) => {
                    out.u16(20);
                    out.list(clis, write_agent_cli);
                }
                Reply::ComparedForReview(files) => {
                    out.u16(21);
                    out.list(files, write_credited);
                }
            }
        }
        Err(refusal) => {
            out.u8(1);
            match refusal {
                Refusal::NoSuchFolder { path } => {
                    out.u16(1);
                    out.path(path);
                }
                Refusal::NotARepository { path } => {
                    out.u16(2);
                    out.path(path);
                }
                Refusal::BareRepository { path } => {
                    out.u16(3);
                    out.path(path);
                }
                Refusal::UnknownWorkspace { workspace } => {
                    out.u16(4);
                    out.text(workspace.as_str());
                }
                Refusal::UnknownRevision { revision } => {
                    out.u16(5);
                    out.text(revision);
                }
                Refusal::OutsideWorkspace { path } => {
                    out.u16(6);
                    out.path(path);
                }
                Refusal::Failed { reason } => {
                    out.u16(7);
                    out.text(reason);
                }
                Refusal::CopiedWorktree { path } => {
                    out.u16(8);
                    out.path(path);
                }
                Refusal::UnknownTerminal { terminal } => {
                    out.u16(9);
                    out.text(terminal.as_str());
                }
                Refusal::UnknownReview { review } => {
                    out.u16(10);
                    out.text(review.as_str());
                }
                Refusal::UnknownThread { thread } => {
                    out.u16(11);
                    out.text(thread.as_str());
                }
                Refusal::UnknownComment { comment } => {
                    out.u16(12);
                    out.text(comment.as_str());
                }
                Refusal::NotADraft { comment } => {
                    out.u16(13);
                    out.text(comment.as_str());
                }
                Refusal::NothingToSend { review } => {
                    out.u16(14);
                    out.text(review.as_str());
                }
                Refusal::NothingToCommit { workspace } => {
                    out.u16(15);
                    out.text(workspace.as_str());
                }
                Refusal::UnknownTurn { turn } => {
                    out.u16(16);
                    out.text(turn.as_str());
                }
            }
        }
    }
}

type Answer = (RequestId, Result<Reply, Refusal>);

pub(crate) fn read_reply(input: &mut In<'_>) -> Result<Answer, LinkError> {
    let to = RequestId(input.u32()?);
    let reply = match input.u8()? {
        0 => Ok(match input.u16()? {
            0 => Reply::Done,
            1 => Reply::Workspaces(input.list(read_repository)?),
            2 => Reply::Opened {
                repository: RepositoryId::new(input.text()?),
                workspace: WorkspaceId::new(input.text()?),
            },
            3 => Reply::Changes(read_changes(input)?),
            4 => Reply::File(read_sides(input)?),
            5 => Reply::TerminalOpened {
                terminal: TerminalId::new(input.text()?),
            },
            6 => Reply::Reviews(read_workspace_reviews(input)?),
            7 => Reply::Copied {
                text: input.text()?,
                round: input.u32()?,
            },
            8 => Reply::Committed {
                commit: input.text()?,
            },
            9 => Reply::Agents(input.list(read_workspace_agents)?),
            10 => Reply::History(read_history(input)?),
            11 => Reply::Compared(input.list(read_changed_file)?),
            12 => Reply::Turn(read_turn(input)?),
            13 => Reply::Terminals(input.list(read_terminal_info)?),
            14 => Reply::Attached {
                cut: match input.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(LinkError::Malformed("a flag is neither 0 nor 1")),
                },
            },
            15 => Reply::Uncommitted(input.list(read_credited)?),
            16 => Reply::Settings(input.list(read_setting)?),
            17 => Reply::Answer {
                bytes: input.bytes()?,
            },
            18 => Reply::AttachedReview {
                start: input.list(read_agent_cli)?,
            },
            19 => Reply::Attachment {
                text: input.text()?,
            },
            20 => Reply::Clis(input.list(read_agent_cli)?),
            21 => Reply::ComparedForReview(input.list(read_credited)?),
            _ => return Err(LinkError::Malformed("unknown kind of reply")),
        }),
        1 => Err(match input.u16()? {
            1 => Refusal::NoSuchFolder {
                path: input.path()?,
            },
            2 => Refusal::NotARepository {
                path: input.path()?,
            },
            3 => Refusal::BareRepository {
                path: input.path()?,
            },
            4 => Refusal::UnknownWorkspace {
                workspace: WorkspaceId::new(input.text()?),
            },
            5 => Refusal::UnknownRevision {
                revision: input.text()?,
            },
            6 => Refusal::OutsideWorkspace {
                path: input.path()?,
            },
            7 => Refusal::Failed {
                reason: input.text()?,
            },
            8 => Refusal::CopiedWorktree {
                path: input.path()?,
            },
            9 => Refusal::UnknownTerminal {
                terminal: TerminalId::new(input.text()?),
            },
            10 => Refusal::UnknownReview {
                review: ReviewId::new(input.text()?),
            },
            11 => Refusal::UnknownThread {
                thread: ThreadId::new(input.text()?),
            },
            12 => Refusal::UnknownComment {
                comment: CommentId::new(input.text()?),
            },
            13 => Refusal::NotADraft {
                comment: CommentId::new(input.text()?),
            },
            14 => Refusal::NothingToSend {
                review: ReviewId::new(input.text()?),
            },
            15 => Refusal::NothingToCommit {
                workspace: WorkspaceId::new(input.text()?),
            },
            16 => Refusal::UnknownTurn {
                turn: TurnId::new(input.text()?),
            },
            _ => return Err(LinkError::Malformed("unknown kind of refusal")),
        }),
        _ => return Err(LinkError::Malformed("a reply is neither done nor refused")),
    };
    Ok((to, reply))
}

pub(crate) fn write_fact(out: &mut Out, fact: &Fact) {
    match fact {
        Fact::Workspaces(repositories) => {
            out.u16(1);
            out.list(repositories, write_repository);
        }
        Fact::ChangesChanged { workspace } => {
            out.u16(2);
            out.text(workspace.as_str());
        }
        Fact::TerminalEnded { terminal, ending } => {
            out.u16(3);
            out.text(terminal.as_str());
            write_ending(out, *ending);
        }
        Fact::ReviewsChanged { workspace } => {
            out.u16(4);
            out.text(workspace.as_str());
        }
        Fact::AgentsChanged(agents) => {
            out.u16(5);
            write_workspace_agents(out, agents);
        }
        Fact::TurnEnded {
            workspace,
            session,
            turn,
            state,
            since,
        } => {
            out.u16(6);
            out.text(workspace.as_str());
            out.text(session.as_str());
            out.text(turn.as_str());
            out.optional(state.as_deref(), Out::text);
            out.optional(since.as_deref(), Out::text);
        }
        Fact::Step {
            workspace,
            session,
            turn,
            number,
            step,
        } => {
            out.u16(7);
            out.text(workspace.as_str());
            out.text(session.as_str());
            out.text(turn.as_str());
            out.u32(*number);
            write_step(out, step);
        }
        Fact::SettingChanged(setting) => {
            out.u16(8);
            write_setting(out, setting);
        }
    }
}

pub(crate) fn read_fact(input: &mut In<'_>) -> Result<Fact, LinkError> {
    Ok(match input.u16()? {
        1 => Fact::Workspaces(input.list(read_repository)?),
        2 => Fact::ChangesChanged {
            workspace: WorkspaceId::new(input.text()?),
        },
        3 => Fact::TerminalEnded {
            terminal: TerminalId::new(input.text()?),
            ending: read_ending(input)?,
        },
        4 => Fact::ReviewsChanged {
            workspace: WorkspaceId::new(input.text()?),
        },
        5 => Fact::AgentsChanged(read_workspace_agents(input)?),
        6 => Fact::TurnEnded {
            workspace: WorkspaceId::new(input.text()?),
            session: SessionId::new(input.text()?),
            turn: TurnId::new(input.text()?),
            state: input.optional(In::text)?,
            since: input.optional(In::text)?,
        },
        7 => Fact::Step {
            workspace: WorkspaceId::new(input.text()?),
            session: SessionId::new(input.text()?),
            turn: TurnId::new(input.text()?),
            number: input.u32()?,
            step: read_step(input)?,
        },
        8 => Fact::SettingChanged(read_setting(input)?),
        _ => return Err(LinkError::Malformed("unknown kind of fact")),
    })
}
