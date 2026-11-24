//! Each request carries an id the Window chose and is answered by one reply with that id, or a
//! refusal. A fact goes to every attached Window when something changed.

use std::fmt;
use std::path::PathBuf;

use crate::LinkError;
use crate::agent::{
    AgentCli, AgentReport, Consent, ConsentAnswer, CreditedFile, History, Step, TerminalInfo, Turn,
    WorkspaceAgents, read_agent_cli, read_consent, read_consent_answer, read_credited,
    read_history, read_report, read_step, read_terminal_info, read_turn, read_workspace_agents,
    write_agent_cli, write_consent, write_consent_answer, write_credited, write_history,
    write_report, write_step, write_terminal_info, write_turn, write_workspace_agents,
};
use crate::review::{
    CommentId, DraftOn, ReviewId, SessionId, ThreadId, TurnId, WorkspaceReviews, read_draft_on,
    read_workspace_reviews, write_draft_on, write_workspace_reviews,
};
use crate::setting::{Setting, read_setting, write_setting};
use crate::states::{StateSizes, read_state_sizes, write_state_sizes};
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

/// Unique among the requests of one connection still waiting for a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Window,
    /// A piece of the app inside an agent CLI: hello, one request, leave. Nothing is told for it; a
    /// fact may still reach it before its reply, and it reads past it.
    Agent,
}

macro_rules! request_kinds {
    ($( $(#[$doc:meta])* $name:ident = $code:literal, $origin:ident; )+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum RequestKind {
            $( $(#[$doc])* $name, )+
        }

        impl RequestKind {
            pub const ALL: &'static [RequestKind] = &[ $( RequestKind::$name, )+ ];

            pub const fn origin(self) -> Origin {
                match self {
                    $( RequestKind::$name => Origin::$origin, )+
                }
            }

            pub const fn code(self) -> u16 {
                match self {
                    $( RequestKind::$name => $code, )+
                }
            }

            pub const fn from_code(code: u16) -> Option<RequestKind> {
                match code {
                    $( $code => Some(RequestKind::$name), )+
                    _ => None,
                }
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $( RequestKind::$name => stringify!($name), )+
                }
            }
        }
    };
}

request_kinds! {
    ListWorkspaces = 1, Window;
    OpenFolder = 2, User;
    SetBase = 3, User;
    GetChanges = 4, Window;
    ReadFile = 5, Window;
    OpenTerminal = 6, User;
    ResizeTerminal = 7, Window;
    GetReviews = 8, Window;
    SaveDraft = 9, Window;
    DeleteDraft = 10, User;
    CopyReview = 11, User;
    MarkReviewed = 12, User;
    SetViewed = 13, User;
    Commit = 14, User;
    Report = 15, Agent;
    ListAgents = 16, Window;
    GetHistory = 17, Window;
    CompareVersions = 18, Window;
    ReadVersions = 19, Window;
    GetTurn = 20, Window;
    ShowWorkspace = 21, Window;
    ListTerminals = 22, Window;
    AttachTerminal = 23, Window;
    GetUncommitted = 24, Window;
    GetSettings = 25, Window;
    SetSetting = 26, User;
    AttachReview = 27, User;
    DetachReview = 28, User;
    ResolveThread = 29, User;
    ReopenThread = 30, User;
    ResolveAddressed = 31, User;
    GetAttachment = 32, Window;
    ListClis = 33, Window;
    CompareForReview = 34, Window;
    ListConsents = 35, Window;
    AnswerConsent = 36, User;
    ExportVersion = 37, Window;
    CreateWorktree = 38, User;
    GetStateSizes = 39, Window;
    CleanUpStates = 40, User;
}

impl RequestKind {
    pub fn started_by_user() -> impl Iterator<Item = RequestKind> {
        Self::ALL
            .iter()
            .copied()
            .filter(|kind| kind.origin() == Origin::User)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    ListWorkspaces,
    OpenFolder {
        path: PathBuf,
    },
    /// A branch is followed (the base moves when it does); anything else is kept as the commit it
    /// names now.
    SetBase {
        workspace: WorkspaceId,
        base: String,
    },
    GetChanges {
        workspace: WorkspaceId,
    },
    /// Both versions are named by the Window from the [`Changes`] it holds, so they belong to the
    /// list they were asked from, whatever happened to the base since.
    ReadFile {
        workspace: WorkspaceId,
        since: Option<String>,
        old_path: PathBuf,
        path: PathBuf,
    },
    /// The reply reaches the Window before the first output and none is lost in between;
    /// [`Fact::TerminalEnded`] follows the last output.
    OpenTerminal {
        workspace: WorkspaceId,
        program: Program,
        size: TerminalSize,
    },
    ResizeTerminal {
        terminal: TerminalId,
        size: TerminalSize,
    },
    GetReviews {
        workspace: WorkspaceId,
    },
    /// Done once the text is in the Keeper's database; the drive is flushed after a pause in the
    /// typing. Told as [`Fact::ReviewsChanged`] only when the save made a comment, so typing does
    /// not make every Window read again.
    SaveDraft {
        review: ReviewId,
        /// New, when `on` starts one.
        thread: ThreadId,
        comment: CommentId,
        on: DraftOn,
        text: String,
    },
    DeleteDraft {
        review: ReviewId,
        comment: CommentId,
    },
    /// Copy a review as text, to paste by hand to an agent the app cannot deliver to. Its unsent
    /// comments on unresolved threads are then sent as a message would send them: the worktree's
    /// state is recorded and a new round starts.
    CopyReview {
        review: ReviewId,
    },
    /// Records the worktree's state now as the reviewed state, from which the next review counts.
    /// No commit is made.
    MarkReviewed {
        review: ReviewId,
    },
    SetViewed {
        workspace: WorkspaceId,
        path: PathBuf,
        viewed: bool,
    },
    /// Made as the user's own git makes it: with the user's hooks, identity and signing. Nothing is
    /// marked reviewed.
    Commit {
        workspace: WorkspaceId,
        message: String,
    },
    /// Answered in the order reports and terminal marks came, once the agents' side of the Keeper
    /// has taken it: nothing the report leads to is waited for, since the CLI waits for its piece.
    /// The answer never decides a permission prompt.
    Report {
        report: AgentReport,
    },
    ListAgents,
    GetHistory {
        workspace: WorkspaceId,
    },
    /// Each change with who made it: those no agent made and those credited to the review's
    /// session; every change for the review of no agent, or none named.
    GetUncommitted {
        workspace: WorkspaceId,
        review: Option<ReviewId>,
    },
    CompareVersions {
        workspace: WorkspaceId,
        from: String,
        /// None for the files on disk.
        to: Option<String>,
    },
    ReadVersions {
        workspace: WorkspaceId,
        /// None reads no old side.
        old: Option<String>,
        old_path: PathBuf,
        /// None for the file on disk.
        new: Option<String>,
        path: PathBuf,
    },
    GetTurn {
        turn: TurnId,
    },
    /// The Window tells it whenever the workspace on screen changes; the Keeper watches the
    /// worktrees some Window shows or an agent works in.
    ShowWorkspace {
        workspace: Option<WorkspaceId>,
    },
    ListTerminals,
    /// After the reply the Window gets the newest output the Keeper kept, then what the program
    /// writes from then on, nothing lost or sent twice. The program is told the size and asked to
    /// draw its screen again.
    AttachTerminal {
        terminal: TerminalId,
        size: TerminalSize,
    },
    GetSettings,
    /// Told as [`Fact::SettingChanged`]. A value that makes no sense (a time of 0, an empty theme
    /// name, a worktrees folder not absolute or inside its repository) is Failed.
    SetSetting {
        setting: Setting,
    },
    /// Attach the review, or one thread alone, to the user's next message to the review's agent.
    /// Nothing is sent now: its unsent comments go as they are when that message is submitted.
    /// Drafts stay editable and a new one joins, unless one thread is attached alone.
    AttachReview {
        review: ReviewId,
        /// One thread to attach alone; none attaches the review.
        thread: Option<ThreadId>,
    },
    DetachReview {
        review: ReviewId,
    },
    ResolveThread {
        review: ReviewId,
        thread: ThreadId,
    },
    ReopenThread {
        review: ReviewId,
        thread: ThreadId,
    },
    ResolveAddressed {
        review: ReviewId,
    },
    /// What the next message would carry, as the agent will receive it. [`Refusal::NothingToSend`]
    /// also for the review of no agent.
    GetAttachment {
        review: ReviewId,
    },
    /// Every agent CLI the Keeper has an Adapter for, in the Adapters' order. Fixed while a Keeper
    /// runs: a Window asks once per connection.
    ListClis,
    /// The files [`Request::CompareVersions`] lists, minus those credited only to sessions other
    /// than the review's own (none left out for the review of no agent), each with who is credited.
    /// Credits are those kept with the states recorded after `from`, up to `to`.
    CompareForReview {
        workspace: WorkspaceId,
        review: ReviewId,
        from: String,
        to: String,
    },
    ListConsents,
    /// `NotAsked` removes the entry like `No` and has the Window ask again. Kept across restarts.
    AnswerConsent {
        agent: String,
        answer: ConsentAnswer,
    },
    /// Writes the files of one version (a commit or a recorded state) into `into`, which must not
    /// exist yet and must lie inside [`crate::cache_dir`]; the reviewed repository gains nothing.
    /// The Window removes the folder when it no longer needs it.
    ExportVersion {
        workspace: WorkspaceId,
        version: String,
        into: PathBuf,
    },
    /// One step: git makes the worktree (`git worktree add -b`) in a folder named after the branch
    /// (each `/` made `-`), in [`crate::Setting::WorktreesFolder`] or else beside the repository,
    /// then `agent` starts in a new terminal. A failure undoes it, or the refusal names what stays.
    CreateWorktree {
        repository: RepositoryId,
        /// The branch the worktree is made from, local (`main`) or of a remote (`origin/main`): the
        /// base of the new workspace.
        from: String,
        branch: String,
        agent: String,
        size: TerminalSize,
    },
    GetStateSizes,
    /// Deletes every state that no thread, round or reviewed mark refers to, whatever its age, with
    /// what only such states held and the files of long reviews of sessions that no longer run.
    /// Answered with the sizes after.
    CleanUpStates,
}

impl Request {
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
            Self::ListConsents => RequestKind::ListConsents,
            Self::AnswerConsent { .. } => RequestKind::AnswerConsent,
            Self::ExportVersion { .. } => RequestKind::ExportVersion,
            Self::CreateWorktree { .. } => RequestKind::CreateWorktree,
            Self::GetStateSizes => RequestKind::GetStateSizes,
            Self::CleanUpStates => RequestKind::CleanUpStates,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Workspaces(Vec<Repository>),
    Opened {
        repository: RepositoryId,
        workspace: WorkspaceId,
    },
    Changes(Changes),
    File(FileSides),
    TerminalOpened {
        terminal: TerminalId,
    },
    Reviews(WorkspaceReviews),
    Copied {
        text: String,
        round: u32,
    },
    Committed {
        commit: String,
    },
    Agents(Vec<WorkspaceAgents>),
    History(History),
    Compared(Vec<ChangedFile>),
    Turn(Turn),
    Terminals(Vec<TerminalInfo>),
    Attached {
        /// Whether the program wrote more than the Keeper keeps: what came before is gone.
        cut: bool,
    },
    Uncommitted(Vec<CreditedFile>),
    Settings(Vec<Setting>),
    Answer {
        bytes: Vec<u8>,
    },
    /// The CLIs the Window offers to start with the review attached: none while the review's agent
    /// runs, that agent's CLI when it does not, every supported CLI for the review of no agent.
    AttachedReview {
        start: Vec<AgentCli>,
    },
    Attachment {
        text: String,
    },
    Clis(Vec<AgentCli>),
    ComparedForReview(Vec<CreditedFile>),
    Consents(Vec<Consent>),
    WorktreeCreated {
        workspace: WorkspaceId,
        terminal: TerminalId,
    },
    StateSizes(StateSizes),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NoSuchFolder {
        path: PathBuf,
    },
    NotARepository {
        path: PathBuf,
    },
    BareRepository {
        path: PathBuf,
    },
    /// The folder shares the original worktree's git folder, so git cannot tell them apart.
    CopiedWorktree {
        path: PathBuf,
    },
    UnknownWorkspace {
        workspace: WorkspaceId,
    },
    UnknownRevision {
        revision: String,
    },
    OutsideWorkspace {
        path: PathBuf,
    },
    UnknownTerminal {
        terminal: TerminalId,
    },
    UnknownReview {
        review: ReviewId,
    },
    UnknownThread {
        thread: ThreadId,
    },
    UnknownComment {
        comment: CommentId,
    },
    NotADraft {
        comment: CommentId,
    },
    NothingToSend {
        review: ReviewId,
    },
    NothingToCommit {
        workspace: WorkspaceId,
    },
    Failed {
        reason: String,
    },
    UnknownTurn {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// Told when a folder was opened and when a worktree's watcher saw its checkout change. Only
    /// worktrees a Window shows or an agent works in are watched.
    Workspaces(Vec<Repository>),
    ChangesChanged {
        workspace: WorkspaceId,
    },
    /// Told after the last [`crate::Message::Output`] of the terminal.
    TerminalEnded {
        terminal: TerminalId,
        ending: Ending,
    },
    ReviewsChanged {
        workspace: WorkspaceId,
    },
    AgentsChanged(WorkspaceAgents),
    /// Told after the [`Fact::AgentsChanged`] that shows the session's new state.
    TurnEnded {
        workspace: WorkspaceId,
        session: SessionId,
        turn: TurnId,
        /// Full id of the state recorded at the turn's end; None when the recording failed.
        state: Option<String>,
        /// Full id of the state last marked reviewed in the session's review, from which what
        /// changed since then counts; None before the first mark, when it counts from the history's
        /// start.
        since: Option<String>,
    },
    /// Told in order after the [`Fact::AgentsChanged`] that shows the turn running. Each is kept
    /// before it is told, so [`Request::GetTurn`] gives every step told so far.
    Step {
        workspace: WorkspaceId,
        session: SessionId,
        turn: TurnId,
        /// The step's number in the turn, from 1.
        number: u32,
        step: Step,
    },
    SettingChanged(Setting),
    /// Told to every Window after an answer, and to one Window alone at its first request, before
    /// it is answered, when a question waits or a CLI's settings could not be changed.
    ConsentsChanged(Vec<Consent>),
    StatesDeleted {
        workspace: WorkspaceId,
    },
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
            out.flag(*viewed);
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
        Request::ListClis | Request::ListConsents => {}
        Request::AnswerConsent { agent, answer } => {
            out.text(agent);
            write_consent_answer(out, *answer);
        }
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
        Request::ExportVersion {
            workspace,
            version,
            into,
        } => {
            out.text(workspace.as_str());
            out.text(version);
            out.path(into);
        }
        Request::CreateWorktree {
            repository,
            from,
            branch,
            agent,
            size,
        } => {
            out.text(repository.as_str());
            out.text(from);
            out.text(branch);
            out.text(agent);
            write_size(out, *size);
        }
        Request::GetStateSizes | Request::CleanUpStates => {}
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
            viewed: input.flag()?,
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
        RequestKind::ListConsents => Request::ListConsents,
        RequestKind::AnswerConsent => Request::AnswerConsent {
            agent: input.text()?,
            answer: read_consent_answer(input)?,
        },
        RequestKind::ExportVersion => Request::ExportVersion {
            workspace: WorkspaceId::new(input.text()?),
            version: input.text()?,
            into: input.path()?,
        },
        RequestKind::CreateWorktree => Request::CreateWorktree {
            repository: RepositoryId::new(input.text()?),
            from: input.text()?,
            branch: input.text()?,
            agent: input.text()?,
            size: read_size(input)?,
        },
        RequestKind::GetStateSizes => Request::GetStateSizes,
        RequestKind::CleanUpStates => Request::CleanUpStates,
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
                    out.flag(*cut);
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
                Reply::Consents(consents) => {
                    out.u16(22);
                    out.list(consents, write_consent);
                }
                Reply::WorktreeCreated {
                    workspace,
                    terminal,
                } => {
                    out.u16(23);
                    out.text(workspace.as_str());
                    out.text(terminal.as_str());
                }
                Reply::StateSizes(sizes) => {
                    out.u16(24);
                    write_state_sizes(out, sizes);
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
            14 => Reply::Attached { cut: input.flag()? },
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
            22 => Reply::Consents(input.list(read_consent)?),
            23 => Reply::WorktreeCreated {
                workspace: WorkspaceId::new(input.text()?),
                terminal: TerminalId::new(input.text()?),
            },
            24 => Reply::StateSizes(read_state_sizes(input)?),
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
        Fact::ConsentsChanged(consents) => {
            out.u16(9);
            out.list(consents, write_consent);
        }
        Fact::StatesDeleted { workspace } => {
            out.u16(10);
            out.text(workspace.as_str());
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
        9 => Fact::ConsentsChanged(input.list(read_consent)?),
        10 => Fact::StatesDeleted {
            workspace: WorkspaceId::new(input.text()?),
        },
        _ => return Err(LinkError::Malformed("unknown kind of fact")),
    })
}
