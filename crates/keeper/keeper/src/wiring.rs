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
//! | `OpenTerminal` | `Dock::open`, in the folder of the workspace | none; later `TerminalEnded` |
//! | `ResizeTerminal` | `Dock::resize` | none |
//! | `GetReviews` | `Reviews::of_workspace`, with the worktree when it is there | none |
//! | `SaveDraft` | `Reviews::save_draft` | `ReviewsChanged` of its workspace when it made a comment |
//! | `DeleteDraft` | `Reviews::delete_draft` | `ReviewsChanged` of its workspace |
//! | `CopyReview` | `Reviews::copy_as_text` | `ReviewsChanged` of its workspace |
//! | `MarkReviewed` | `Reviews::mark_reviewed` | `ReviewsChanged` of its workspace |
//! | `SetViewed` | `Reviews::set_viewed` | `ReviewsChanged` of that workspace |
//! | `Commit` | `Worktree::commit_all`, the worktree from `Workspaces::worktree` | `ChangesChanged` of that workspace |
//! | `Report` | `Agents::report`: queued, and answered once the agents' own thread has taken it (`Answer` or `Done`) | later `AgentsChanged`, `Step`, `TurnEnded`, `ReviewsChanged` |
//! | `ListAgents` | `Agents::list` | none |
//! | `GetHistory` | `Agents::history` | none |
//! | `GetUncommitted` | `Agents::uncommitted` | none |
//! | `CompareVersions` | `Agents::compare` | none |
//! | `ReadVersions` | `Agents::read_versions` | none |
//! | `GetTurn` | `Agents::turn` | none |
//! | `ShowWorkspace` | the watchers of the worktrees on screen | later `ChangesChanged`, `Workspaces` |
//! | `ListTerminals` | the terminals the Dock holds, with their workspace and program | none |
//! | `AttachTerminal` | `Dock::attach` adds the Window to the terminal's route, then `Dock::resize` and `Dock::redraw` | none |
//! | `GetSettings` | `settings::get`, from the Store | none |
//! | `SetSetting` | `settings::set`, into the Store | `SettingChanged` with the setting |
//! | `ListClis` | `Agents::supported`, the Adapters' names and titles | none |
//! | `CompareForReview` | `Agents::compare_for_review`, through `Reviews::credited_change` | none |
//! | `ListConsents` | `Agents::consents`, each Adapter's `consent` | none |
//! | `AnswerConsent` | `Agents::answer_consent`, the Adapter's `answer_consent` | `ConsentsChanged` with every Adapter's consent |
//! | `ExportVersion` | `Agents::export_version`, through `StateStore::export` | none |
//! | `CreateWorktree` | `Parts::create_worktree` (`wiring/worktrees.rs`): `Workspaces::plan_worktree`, `Planned::add` once the parts are let go, then under the Keeper's turn `Workspaces::open_added` and the agent's launcher as `OpenTerminal` opens it | `Workspaces`, the list as it is then, told under the turn before the reply |
//! | `GetStateSizes` | `cleaning::sizes`, through `Reviews::state_sizes` | none |
//! | `CleanUpStates` | `cleaning::clean_up_now`: `Reviews::clean_up` of every workspace, then `Agents::remove_files_of_ended` | `StatesDeleted` of each workspace that lost a state |
//!
//! `SetBase` and `GetChanges` also tell `base_moves` the commit the changes are counted from,
//! so that the threads on the old side follow the base when it moves; a thread that moved is
//! told as `ReviewsChanged` of its workspace.
//!
//! A Window is also told `ConsentsChanged` alone at its first request when an Adapter's
//! question waits for an answer or its CLI's settings could not be changed: the question
//! it shows (`Parts::consents`).
//!
//! A piece of the app inside an agent CLI connects as a Window does, sends one request of
//! the origin `Agent`, and is let go once it is answered.
//!
//! The worktree of a request of reviews is found with `Workspaces::worktree`, which reads
//! the Store and the folder and runs no git: never through the list of every repository,
//! which runs git in each. A save that changes a draft's text needs no worktree at all.
//!
//! A copy of a review and a mark of reviewed need the worktree of the review's workspace,
//! and are refused with the words "the worktree is gone" when there is none. So is a
//! commit, and so is the mark of a viewed file. Reading the reviews of a gone worktree
//! works, and so does the save of a draft's text.
//!
//! The commit is not answered by [`Parts::answer`] itself: that gives the work, which the
//! Keeper runs once it has let its parts go, so that the user's hooks, which may run for
//! minutes, hold up nothing.
//!
//! A terminal's bytes are no request. What a Window sends as `Message::Input` is handed
//! to `Dock::write`; what the program writes goes, as `Message::Output`, to every Window
//! the terminal is shown to: the one that opened it, and each that attached to it later.
//! A Window gets it only after the reply that named the terminal or attached it, and an
//! attached Window first gets the kept output of the terminal, so that nothing is lost or
//! sent twice where that ends. When the program ends, every Window is told
//! `Fact::TerminalEnded`, after the last output of each that was sent any.
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
//! Requests that change the list of workspaces, and the one whose reply carries that
//! list, are answered one at a time: see [`waits_its_turn`]. The Review engine keeps its
//! own changes one at a time. Where each request is answered, so that what takes long
//! holds up nothing else of its Window, is [`answerer`]'s to say.
//!
//! The worktrees a Window shows, and those an agent works in, are watched: what changes in
//! them is told as `ChangesChanged`, and a change of what is checked out as `Workspaces`
//! too. The list of every repository is read from git when a Window asks for it.

// A worktree made from a branch with an agent in it, which `Parts::open_terminal` serves.
mod worktrees;

pub(crate) use worktrees::set_worktrees_folder;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use yaar_dock::{Dock, DockError, Entry, Event, Start};
use yaar_link::{
    Ending, Fact, Message, Program, Refusal, Reply, Request, RequestKind, SOCKET_VARIABLE,
    TERMINAL_VARIABLE, TerminalSize,
};
use yaar_recorder::{CommitError, CommitId, Git, Worktree};
use yaar_review_engine::{EngineError, ReviewError, ReviewId, Reviews, Saved, StateWatch};
use yaar_store::Store;
use yaar_workspaces::{WorkspaceError, WorkspaceId, Workspaces};

use crate::agents::Agents;
use crate::base_moves;
use crate::watching::Watching;
use crate::{Options, Shared, Window, frame_of};

/// How many of its first bytes say whether a file is text: git's own number.
const TEXT_PROBE: usize = 8000;

/// The most bytes of a program's output that one frame carries.
const OUTPUT_FRAME: usize = 64 * 1024;

/// The parts of the Keeper, started and connected.
pub(crate) struct Parts {
    // Workspaces and the reviews, both on git and the Store, or the sentence that says
    // why there are none.
    kept: Result<Kept, String>,
    // The agents' side: the Adapters, the Switchboard and what follows from them. Held by
    // the routes of the terminals too, which tell it the marks they find, so that the
    // Dock's threads never wait for the parts.
    agents: Arc<Agents>,
    // The worktrees watched: those a Window shows, and those an agent runs in.
    watching: Arc<Watching>,
    // The terminals and their programs. Dropped with the parts, which ends every program.
    dock: Dock,
    // The terminals the Dock runs, with who is shown each: what `ListTerminals` and
    // `AttachTerminal` read, and what a Window that goes leaves.
    terminals: Arc<Terminals>,
    // What the programs of the Dock tell the Windows goes through here.
    shared: Weak<Shared>,
    // Where the Keeper was asked to listen: what every shell of the app is told is its
    // socket.
    socket: PathBuf,
}

/// What the Keeper keeps for its Windows: the workspaces, the reviews of their worktrees,
/// and the Store both keep their records in.
pub(crate) struct Kept {
    pub(crate) workspaces: Workspaces,
    pub(crate) reviews: Reviews,
    pub(crate) store: Store,
}

/// A step that waits for a reply to be queued, and runs after it.
pub(crate) type Step = Box<dyn FnOnce() + Send>;

/// What one request leads to.
pub(crate) struct Outcome {
    /// For the Window that asked.
    pub(crate) reply: Result<Reply, Refusal>,
    /// For every attached Window, the one that asked included, after the reply.
    pub(crate) facts: Vec<Fact>,
    /// Run once the reply is queued, before the facts are told.
    pub(crate) then: Option<Step>,
}

impl Outcome {
    /// A request that was refused, with nothing to tell anybody else.
    pub(crate) fn failed(reason: impl Into<String>) -> Outcome {
        Outcome {
            reply: Err(Refusal::Failed {
                reason: reason.into(),
            }),
            facts: Vec::new(),
            then: None,
        }
    }
}

/// Where a request is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answerer {
    /// On the thread that reads the Window's connection, before its next message is read.
    Here,
    /// By the Window's one thread for reviews, one after the other in the order they came.
    InOrder,
    /// On a thread of its own, started for this request.
    Apart,
}

/// Where requests of this kind are answered.
pub(crate) fn answerer(kind: RequestKind) -> Answerer {
    // No arm for "the rest": a new kind of request has to say which it is.
    match kind {
        // Quick, or ordered with the Window's other requests of slices 1 and 2.
        RequestKind::ListWorkspaces
        | RequestKind::OpenFolder
        | RequestKind::SetBase
        | RequestKind::GetChanges
        | RequestKind::ReadFile
        | RequestKind::OpenTerminal
        | RequestKind::ResizeTerminal => Answerer::Here,
        // The saves of a draft must be kept in the order they were typed, and a reading
        // that follows a save must find it. A copy or a mark of reviewed records a state,
        // up to a second or more on a large repository: the Window's typing into its
        // terminals, read on the connection's thread, must not wait for it.
        RequestKind::GetReviews
        | RequestKind::SaveDraft
        | RequestKind::DeleteDraft
        | RequestKind::CopyReview
        | RequestKind::MarkReviewed
        | RequestKind::SetViewed => Answerer::InOrder,
        // The user's own git, with their hooks, which may run for minutes: neither the
        // Window's typing nor its saves may wait for it.
        RequestKind::Commit => Answerer::Apart,
        // A CLI waits for its piece, which waits for this answer: it is given on the
        // piece's own connection as soon as the agents' side has taken the report, and that
        // connection has nothing else to answer meanwhile.
        RequestKind::Report => Answerer::Here,
        // Read from what the Keeper holds and the Store.
        RequestKind::ListAgents | RequestKind::GetTurn | RequestKind::ListTerminals => {
            Answerer::Here
        }
        // The watcher of a worktree is started on a thread of its own: the request only
        // says which workspace is on screen. The kept output of a terminal must reach its
        // Window after the reply, as the output of a terminal opened.
        RequestKind::ShowWorkspace | RequestKind::AttachTerminal => Answerer::Here,
        // git in the worktree or in its state store, up to a second on a large repository,
        // reading the files on disk: the Window's typing into its terminals must not wait
        // for it, nor its saves.
        RequestKind::GetHistory
        | RequestKind::GetUncommitted
        | RequestKind::CompareVersions
        | RequestKind::ReadVersions => Answerer::Apart,
        // One read or one write of the Store, which a user's key waits for no longer than
        // the drive takes.
        RequestKind::GetSettings | RequestKind::SetSetting => Answerer::Here,
        // They change a review, or read what it would carry: in the order of the
        // Window's other requests of reviews, so that a save typed before is in it.
        RequestKind::AttachReview
        | RequestKind::DetachReview
        | RequestKind::ResolveThread
        | RequestKind::ReopenThread
        | RequestKind::ResolveAddressed
        | RequestKind::GetAttachment => Answerer::InOrder,
        // The Adapters' names, which never change while the Keeper runs.
        RequestKind::ListClis => Answerer::Here,
        // git in the state store and the Store's credits of each state in the range: as a
        // comparison of two versions.
        RequestKind::CompareForReview => Answerer::Apart,
        // A few small files of the CLIs' settings, read or written once, which a user's
        // key waits for no longer than the drive takes.
        RequestKind::ListConsents | RequestKind::AnswerConsent => Answerer::Here,
        // git's checkout of a whole tree, a second or more on a large repository: as a
        // comparison of two versions.
        RequestKind::ExportVersion => Answerer::Apart,
        // The user's git checks a whole branch out and runs the user's hooks, which may take
        // a minute: apart, so that the Window's other requests and its typing go on.
        RequestKind::CreateWorktree => Answerer::Apart,
        // A walk of every folder of states, and a repack of every store: apart.
        RequestKind::GetStateSizes | RequestKind::CleanUpStates => Answerer::Apart,
    }
}

/// What [`Parts::answer`] gives.
pub(crate) enum Answered {
    /// The outcome, ready.
    Now(Outcome),
    /// Work to run once the parts are let go, which gives the outcome: what may take as
    /// long as the user's own git takes, and must hold up neither the Keeper's stop nor
    /// anything else that waits for the parts.
    Later(Job),
}

/// Work that gives the outcome of a request, run after the parts are let go.
pub(crate) type Job = Box<dyn FnOnce() -> Outcome + Send>;

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
        // They lead to no fact, and a program that takes its time to start must not hold
        // up the Windows' other questions.
        RequestKind::OpenTerminal | RequestKind::ResizeTerminal => false,
        // Each may change what the Keeper keeps, `GetReviews` included: it keeps the review
        // of no agent of a workspace that has none, and takes away the marks of files that
        // changed. The Review engine makes those changes one at a time itself. The facts
        // they lead to carry no more than the workspace's id, and a Window told one asks
        // again and reads the newest: their order does not matter, and none of them may
        // wait behind a recording or the user's git.
        RequestKind::GetReviews
        | RequestKind::SaveDraft
        | RequestKind::DeleteDraft
        | RequestKind::CopyReview
        | RequestKind::MarkReviewed
        | RequestKind::SetViewed
        | RequestKind::Commit => false,
        // Its reply carries what `AgentsChanged` carries: read between two changes and
        // sent after the fact of the second, it would be taken for the newer list. The
        // agents' side tells its facts under the same turn.
        RequestKind::ListAgents => true,
        // It leads to facts only later, from the agents' own thread, which takes the turn
        // to tell them; and its answer waits for that thread, which the turn held here would
        // keep waiting for ever.
        RequestKind::Report => false,
        // What these read, a fact makes a Window read again.
        RequestKind::GetHistory
        | RequestKind::GetUncommitted
        | RequestKind::CompareVersions
        | RequestKind::ReadVersions
        | RequestKind::GetTurn
        | RequestKind::ListTerminals => false,
        // They lead to no fact of their own.
        RequestKind::ShowWorkspace | RequestKind::AttachTerminal => false,
        // A setting kept leads to `SettingChanged`, and the answer of `GetSettings` carries
        // what that fact carries: read between two settings kept and sent after the fact
        // of the second, it would be taken for the newer list.
        RequestKind::GetSettings | RequestKind::SetSetting => true,
        // As the other requests of reviews: their facts name a workspace, and a Window
        // told one asks again. What attaching tells of the sessions is told by the agents'
        // side under the Keeper's turn.
        RequestKind::AttachReview
        | RequestKind::DetachReview
        | RequestKind::ResolveThread
        | RequestKind::ReopenThread
        | RequestKind::ResolveAddressed
        | RequestKind::GetAttachment => false,
        // No fact carries what they read: the CLIs never change, and a Window told of a
        // turn's end or of a change of reviews asks again for what a review shows.
        RequestKind::ListClis | RequestKind::CompareForReview => false,
        // An answer leads to `ConsentsChanged`, and the list carries what that fact
        // carries: read between two answers and sent after the fact of the second, it
        // would be taken for the newer one.
        RequestKind::ListConsents | RequestKind::AnswerConsent => true,
        // It leads to no fact, and no fact carries what it writes.
        RequestKind::ExportVersion => false,
        // It changes the list of workspaces, and tells it: not for the whole of its run,
        // which waits for the user's git, but for its last steps, the opening and the fact,
        // which take the turn themselves (`wiring/worktrees.rs`).
        RequestKind::CreateWorktree => false,
        // The sizes change nothing; the clean-up's facts name workspaces, not the list.
        RequestKind::GetStateSizes | RequestKind::CleanUpStates => false,
    }
}

impl Parts {
    /// Starts the parts: finds git, opens the database `yaar_store::FILE_NAME` in the
    /// folder of `options.data_dir`, and puts Workspaces and the Review engine on both,
    /// the engine recording the states of worktrees in `states` beside the database; and
    /// makes the Dock, whose programs tell what they do to the Windows of `shared`.
    ///
    /// A Keeper with no git, or with a database it cannot open, still starts and answers
    /// the hello, so that a Window can say what is wrong; each request is then refused
    /// with the reason.
    pub(crate) fn start(
        options: &Options,
        clock: Arc<dyn yaar_recorder::Clock>,
        shared: Weak<Shared>,
    ) -> Parts {
        let watching = Arc::new(Watching::new(shared.clone()));
        let kept = Git::find().map_err(|e| e.to_string()).and_then(|git| {
            let file = options.data_dir.join(yaar_store::FILE_NAME);
            let store = Store::open(&file).map_err(|e| e.to_string())?;
            // The engine asks the watching whether files changed since its newest state,
            // and tells it of every state it takes.
            let reviews = Reviews::new(store.clone(), git.clone(), options.data_dir.join("states"))
                .watching(Arc::clone(&watching) as Arc<dyn StateWatch>)
                .with_clock(clock);
            Ok(Kept {
                workspaces: Workspaces::new(git, store.clone()),
                reviews,
                store,
            })
        });
        let agents = Arc::new(Agents::start(
            options,
            kept.as_ref().ok(),
            Arc::clone(&watching),
            shared.clone(),
        ));
        Parts {
            kept,
            agents,
            watching,
            dock: Dock::new(),
            terminals: Arc::new(Terminals::default()),
            shared,
            socket: options.socket.clone(),
        }
    }

    /// What the Keeper keeps for its Windows, unless it has no git or no database.
    pub(crate) fn kept(&self) -> Option<&Kept> {
        self.kept.as_ref().ok()
    }

    /// `window` has gone, whatever the way: it is no longer shown any terminal, and what
    /// it showed on screen is no longer needed on its account.
    pub(crate) fn window_gone(&self, window: &Arc<Window>) {
        self.terminals.window_gone(window, &self.shared);
        if let Ok(kept) = &self.kept {
            self.watching.window_gone(kept, window);
        }
    }

    /// How the Adapters that need the user's leave stand: what a Window is told at its
    /// connect when one waits for an answer.
    pub(crate) fn consents(&self) -> Vec<yaar_link::Consent> {
        self.agents.consents()
    }

    /// How many programs of terminals run.
    pub(crate) fn running(&self) -> usize {
        self.dock.running()
    }

    /// Answers one request of `window`, or gives the work that answers it once the parts
    /// are let go.
    pub(crate) fn answer(&self, window: &Arc<Window>, request: Request) -> Answered {
        let kept = match &self.kept {
            Ok(kept) => kept,
            Err(reason) => return Answered::Now(Outcome::failed(reason.clone())),
        };
        let Kept {
            workspaces,
            reviews,
            ..
        } = kept;
        let mut facts = Vec::new();
        let mut then = None;
        let reply = match request {
            Request::ListWorkspaces => list(workspaces).map(Reply::Workspaces),
            Request::OpenFolder { path } => open_folder(workspaces, path, &mut facts),
            Request::SetBase { workspace, base } => {
                let before = base_moves::before_set(kept, &workspace);
                workspaces
                    .set_base(&WorkspaceId::new(workspace.as_str()), &base)
                    .map(|_| {
                        base_moves::after_set(kept, &workspace, before, &mut facts);
                        facts.push(Fact::ChangesChanged { workspace });
                        Reply::Done
                    })
                    .map_err(refusal)
            }
            Request::GetChanges { workspace } => workspaces
                .changes(&WorkspaceId::new(workspace.as_str()))
                .map(|found| {
                    let answered = changes(workspace.clone(), found);
                    base_moves::changes_seen(
                        kept,
                        &workspace,
                        answered.counted_from.as_deref(),
                        &mut facts,
                    );
                    Reply::Changes(answered)
                })
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
            Request::OpenTerminal {
                workspace,
                program,
                size: at,
            } => self
                .open_terminal(kept, window, &workspace, &program, at)
                .map(|(reply, release)| {
                    then = Some(release);
                    reply
                }),
            Request::ResizeTerminal { terminal, size: to } => self
                .dock
                .resize(&yaar_dock::TerminalId::new(terminal.as_str()), size(to))
                .map(|()| Reply::Done)
                .map_err(|error| match error {
                    DockError::UnknownTerminal(_) => Refusal::UnknownTerminal { terminal },
                    other => Refusal::Failed {
                        reason: other.to_string(),
                    },
                }),
            Request::GetReviews { workspace } => {
                get_reviews(workspaces, reviews, &kept.store, workspace).map(Reply::Reviews)
            }
            Request::SaveDraft {
                review,
                thread,
                comment,
                on,
                text,
            } => save_draft(
                workspaces,
                reviews,
                &review,
                &draft_save(&thread, &comment, on, text),
                &mut facts,
            ),
            Request::DeleteDraft { review, comment } => {
                let review = ReviewId::new(review.as_str());
                reviews
                    .review(&review)
                    .and_then(|found| {
                        reviews.delete_draft(
                            &review,
                            &yaar_review_engine::CommentId::new(comment.as_str()),
                        )?;
                        Ok(found.workspace().clone())
                    })
                    .map(|workspace| {
                        facts.push(reviews_changed(&workspace));
                        Reply::Done
                    })
                    .map_err(engine_refusal)
            }
            Request::CopyReview { review } => copy_review(workspaces, reviews, &review, &mut facts),
            Request::MarkReviewed { review } => {
                let marked = mark_reviewed(workspaces, reviews, &review, &mut facts);
                // A session whose review is marked has nothing unreviewed any more.
                if marked.is_ok() {
                    self.agents.reviewed(kept, &review);
                }
                marked
            }
            Request::SetViewed {
                workspace,
                path,
                viewed,
            } => set_viewed(workspaces, reviews, &workspace, &path, viewed, &mut facts),
            Request::Commit { workspace, message } => {
                return match worktree_of(workspaces, &workspace) {
                    Ok(Some(worktree)) => {
                        Answered::Later(Box::new(move || commit(&worktree, &workspace, &message)))
                    }
                    Ok(None) => Answered::Now(Outcome::failed(EngineError::NoWorktree.to_string())),
                    Err(refused) => Answered::Now(Outcome {
                        reply: Err(refused),
                        facts: Vec::new(),
                        then: None,
                    }),
                };
            }
            // Answered once the agents' side has taken it, by work that runs after the
            // parts are let go: that side takes them to work the report out.
            Request::Report { report } => return self.agents.report(kept, report),
            Request::ListAgents => self.agents.list(kept),
            Request::GetHistory { workspace } => self.agents.history(kept, &workspace),
            Request::GetUncommitted { workspace, review } => {
                self.agents.uncommitted(kept, &workspace, review.as_ref())
            }
            Request::CompareVersions {
                workspace,
                from,
                to,
            } => self.agents.compare(kept, &workspace, &from, to.as_deref()),
            Request::ReadVersions {
                workspace,
                old,
                old_path,
                new,
                path,
            } => self.agents.read_versions(
                kept,
                &workspace,
                old.as_deref().map(|old| (old, old_path.as_path())),
                (new.as_deref(), path.as_path()),
            ),
            Request::GetTurn { turn } => self.agents.turn(kept, &turn),
            // The terminals' side of slice 4: the watchers, the list of terminals and
            // attaching to one.
            Request::ShowWorkspace { workspace } => {
                self.watching.show(kept, window, workspace.as_ref())
            }
            Request::ListTerminals => Ok(Reply::Terminals(self.terminals.list())),
            Request::AttachTerminal { terminal, size: at } => {
                self.attach(window, &terminal, at).map(|(reply, release)| {
                    then = Some(release);
                    reply
                })
            }
            Request::GetSettings => crate::settings::get(kept),
            Request::SetSetting { setting } => crate::settings::set(kept, setting, &mut facts),
            // The requests of slice 6 on reviews: `reviewing.rs`.
            Request::AttachReview { review, thread } => {
                crate::reviewing::attach(kept, &self.agents, &review, thread.as_ref(), &mut facts)
            }
            Request::DetachReview { review } => {
                crate::reviewing::detach(kept, &self.agents, &review, &mut facts)
            }
            Request::ResolveThread { review, thread } => {
                crate::reviewing::resolve(kept, &review, &thread, true, &mut facts)
            }
            Request::ReopenThread { review, thread } => {
                crate::reviewing::resolve(kept, &review, &thread, false, &mut facts)
            }
            Request::ResolveAddressed { review } => {
                crate::reviewing::resolve_addressed(kept, &review, &mut facts)
            }
            Request::GetAttachment { review } => {
                crate::reviewing::attachment(kept, &self.agents, &review)
            }
            // The requests of slice 7: the CLIs to offer a launcher of, and what a review
            // shows of a change with its credits.
            Request::ListClis => Ok(Reply::Clis(self.agents.supported())),
            Request::CompareForReview {
                workspace,
                review,
                from,
                to,
            } => self
                .agents
                .compare_for_review(kept, &workspace, &review, &from, &to),
            // The questions for leave of the Adapters that write into a CLI's settings.
            Request::ListConsents => Ok(Reply::Consents(self.agents.consents())),
            Request::AnswerConsent { agent, answer } => {
                self.agents.answer_consent(&agent, answer).map(|consents| {
                    facts.push(Fact::ConsentsChanged(consents.clone()));
                    Reply::Consents(consents)
                })
            }
            // The old side of a diff, for the Window's language server.
            Request::ExportVersion {
                workspace,
                version,
                into,
            } => self
                .agents
                .export_version(kept, &workspace, &version, &into),
            // A worktree made from a branch, and an agent started in it: the user's git runs
            // after the parts are let go.
            Request::CreateWorktree {
                repository,
                from,
                branch,
                agent,
                size: at,
            } => {
                return self.create_worktree(kept, window, &repository, &from, &branch, &agent, at);
            }
            // The room the recorded states take, and their clean-up at once.
            Request::GetStateSizes => crate::cleaning::sizes(kept),
            Request::CleanUpStates => crate::cleaning::clean_up_now(kept, &self.agents, &mut facts),
        };
        Answered::Now(Outcome { reply, facts, then })
    }

    /// Starts `program` in the folder of `workspace`, in a terminal of the Dock. The
    /// reply names the terminal, and the step that goes with it lets the output of the
    /// program through: until the reply is queued, nothing of it is sent.
    fn open_terminal(
        &self,
        kept: &Kept,
        window: &Arc<Window>,
        workspace: &yaar_link::WorkspaceId,
        program: &Program,
        at: TerminalSize,
    ) -> Result<(Reply, Step), Refusal> {
        let folder = list(&kept.workspaces)?
            .into_iter()
            .flat_map(|repository| repository.workspaces)
            .find(|found| found.id == *workspace)
            .map(|found| found.path)
            .ok_or_else(|| Refusal::UnknownWorkspace {
                workspace: workspace.clone(),
            })?;
        // The id is made first, so that the program's environment can name it.
        let id = yaar_dock::TerminalId::random().map_err(|error| Refusal::Failed {
            reason: format!("no name for the terminal could be made: {error}"),
        })?;
        let terminal = yaar_link::TerminalId::new(id.as_str());
        // A launcher for a review is refused, as for an Adapter the Keeper does not have,
        // before anything is listed or started: the terminal would bind a review nobody holds.
        if let Program::AgentFor { review, .. } = program {
            let held = kept
                .reviews
                .review(&yaar_review_engine::ReviewId::new(review.as_str()));
            match held {
                Ok(_) => {}
                Err(yaar_review_engine::EngineError::UnknownReview(_)) => {
                    return Err(Refusal::Failed {
                        reason: format!("the app holds no review {review}"),
                    });
                }
                Err(error) => return Err(engine_refusal(error)),
            }
        }
        let start = match program {
            Program::Shell => Start::shell(folder, size(at)),
            Program::Agent { agent } | Program::AgentFor { agent, .. } => {
                match self.agents.launcher(agent) {
                    Some(command) => Start::launcher(folder, size(at), command),
                    None => {
                        return Err(Refusal::Failed {
                            reason: format!("the app knows no agent CLI named {agent}"),
                        });
                    }
                }
            }
            // The CLI's own command for bringing the session back, through the user's
            // shell as a launcher's command.
            Program::Resume { session } => {
                let command = self.agents.resume_command(session, &terminal)?;
                Start::launcher(folder, size(at), &command)
            }
        };
        // Every shell of the app is told where the Keeper listens and which terminal it
        // is in, so that a piece of an agent CLI started in it finds its way back, and
        // gets what each agent CLI's piece asks for. A CLI typed in a shell of the app
        // finds all of it as one started from its launcher does.
        let mut entries = vec![
            Entry::Variable {
                name: SOCKET_VARIABLE.into(),
                value: self.socket.clone().into_os_string(),
            },
            Entry::Variable {
                name: TERMINAL_VARIABLE.into(),
                value: terminal.as_str().into(),
            },
        ];
        entries.extend(self.agents.entries());
        let mut start = start.with(&entries);
        // Every terminal is watched for the marks of every agent CLI, for the same reason.
        start.marks = self.agents.marks();
        let route = Arc::new(Route::new(window, Arc::clone(&self.agents)));
        // Listed before the program starts, so that its end, which may come at once,
        // finds it there to take it out; shown to nobody until it runs.
        self.terminals.add(Held {
            id: terminal.clone(),
            workspace: workspace.clone(),
            program: program.clone(),
            route: Arc::clone(&route),
            opened: false,
        });
        let events = {
            let (route, terminals, shared, terminal) = (
                Arc::clone(&route),
                Arc::clone(&self.terminals),
                self.shared.clone(),
                terminal.clone(),
            );
            Box::new(move |event| {
                // Gone from the list before the Windows are told, so that one that asks
                // when it hears of the end does not find it.
                if matches!(event, Event::Ended(_)) {
                    terminals.remove(&terminal);
                }
                route.event(&shared, &terminal, event);
            })
        };
        // Told before the program starts: a CLI in it reports from its first moment, and
        // is followed only in a terminal the agents' side holds.
        self.agents.terminal_opened(&terminal, workspace);
        if let Program::AgentFor { review, .. } = program {
            self.agents.terminal_opened_for(&terminal, review);
        }
        if let Err(error) = self.dock.open(id, start, events) {
            self.agents.terminal_ended(&terminal);
            self.terminals.remove(&terminal);
            return Err(Refusal::Failed {
                reason: error.to_string(),
            });
        }
        self.terminals.opened(&terminal);
        let (shared, opener) = (self.shared.clone(), Arc::clone(window));
        Ok((
            Reply::TerminalOpened {
                terminal: terminal.clone(),
            },
            Box::new(move || route.release(&opener, &shared, &terminal)),
        ))
    }

    /// Shows `window` the terminal `terminal` too, as it was opened for another Window or
    /// for this one before it was killed: the reply, then the newest output the Dock kept,
    /// then what the program writes from then on, with nothing lost or sent twice where
    /// the kept output ends. The terminal is given the size of this Window and asked to
    /// draw itself again, which mends what the cut at the start of the kept output spoiled.
    fn attach(
        &self,
        window: &Arc<Window>,
        terminal: &yaar_link::TerminalId,
        at: TerminalSize,
    ) -> Result<(Reply, Step), Refusal> {
        let unknown = || Refusal::UnknownTerminal {
            terminal: terminal.clone(),
        };
        let route = self.terminals.route(terminal).ok_or_else(unknown)?;
        let id = yaar_dock::TerminalId::new(terminal.as_str());
        let (mut cut, mut shown) = (false, false);
        // The Window is put among those the output goes to while the Dock holds the
        // output still, so that the kept output ends where the live output begins.
        self.dock
            .attach(&id, |kept, dropped| {
                cut = dropped;
                shown = route.attach(window, kept);
            })
            .map_err(|_| unknown())?;
        // The program ended between finding the route and the attach, and the Windows have
        // been told: the terminal is no more.
        if !shown {
            return Err(unknown());
        }
        // A program that ended meanwhile has nothing to draw: its end is told as it comes.
        // What the redraw writes is held behind the kept output like all that follows it.
        let _ = self.dock.resize(&id, size(at));
        let _ = self.dock.redraw(&id);
        let (shared, window, terminal) =
            (self.shared.clone(), Arc::clone(window), terminal.clone());
        Ok((
            Reply::Attached { cut },
            Box::new(move || route.release(&window, &shared, &terminal)),
        ))
    }

    /// Hands what a Window typed to the program of `terminal`. Bytes for a terminal the
    /// Dock does not hold, as one whose program has ended, are dropped: nothing answers
    /// input.
    pub(crate) fn input(&self, terminal: &yaar_link::TerminalId, bytes: &[u8]) {
        let _ = self
            .dock
            .write(&yaar_dock::TerminalId::new(terminal.as_str()), bytes);
    }
}

/// The terminals the Dock runs, in the order they were opened, with who they were opened
/// for and what runs in them.
#[derive(Default)]
pub(crate) struct Terminals {
    held: Mutex<Vec<Held>>,
}

/// One terminal of the Keeper.
struct Held {
    id: yaar_link::TerminalId,
    workspace: yaar_link::WorkspaceId,
    program: Program,
    route: Arc<Route>,
    /// The Dock runs its program: before that, it is shown to nobody.
    opened: bool,
}

impl Terminals {
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Held>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, terminal: Held) {
        self.held().push(terminal);
    }

    /// The Dock runs the program of `terminal` now.
    fn opened(&self, terminal: &yaar_link::TerminalId) {
        if let Some(held) = self.held().iter_mut().find(|held| held.id == *terminal) {
            held.opened = true;
        }
    }

    /// `terminal` is no more: its program ended, or could not start.
    fn remove(&self, terminal: &yaar_link::TerminalId) {
        self.held().retain(|held| held.id != *terminal);
    }

    /// Every terminal that runs, in the order opened.
    fn list(&self) -> Vec<yaar_link::TerminalInfo> {
        self.held()
            .iter()
            .filter(|held| held.opened)
            .map(|held| yaar_link::TerminalInfo {
                id: held.id.clone(),
                workspace: held.workspace.clone(),
                program: held.program.clone(),
            })
            .collect()
    }

    /// Where the output of `terminal` goes, if it runs.
    fn route(&self, terminal: &yaar_link::TerminalId) -> Option<Arc<Route>> {
        self.held()
            .iter()
            .find(|held| held.opened && held.id == *terminal)
            .map(|held| Arc::clone(&held.route))
    }

    /// `window` has gone: it is shown no terminal any more.
    fn window_gone(&self, window: &Arc<Window>, shared: &Weak<Shared>) {
        // The list is not held while a route is: a route tells of an end to Windows.
        let routes: Vec<(yaar_link::TerminalId, Arc<Route>)> = self
            .held()
            .iter()
            .map(|held| (held.id.clone(), Arc::clone(&held.route)))
            .collect();
        for (terminal, route) in routes {
            route.leave(window, shared, &terminal);
        }
    }
}

/// Where the output of one terminal goes: to every Window it is shown to, the one that
/// opened it first, and in order.
///
/// The reply that names a terminal must reach its Window before the terminal's first
/// output, and nothing may be lost in between. The program starts before the reply is
/// made, so what it writes first is held until the reply is queued, and then queued
/// behind it. A Window that attaches later is treated the same way: the output waits
/// behind the reply and the kept output that follows it.
struct Route {
    state: Mutex<RouteState>,
    // Told the marks found in the output and the end of the program.
    agents: Arc<Agents>,
}

#[derive(Default)]
struct RouteState {
    /// The Windows the terminal is shown to. A Window that has gone is dropped when the
    /// route finds it so, which is at the next output or at once when it is told of.
    members: Vec<Member>,
    /// The program ended, and the end is not told yet: it is told once every Window has
    /// what came before it.
    ended: Option<Ending>,
    /// The end has been told: no Window is shown the terminal any more, since nothing
    /// more would be told to one that came after.
    over: bool,
}

/// One Window a terminal is shown to.
struct Member {
    // Not held for long: a Window that has gone away is gone, though its terminal runs
    // on, and its connection closes with it.
    window: Weak<Window>,
    /// The reply is queued: output goes straight to the Window.
    released: bool,
    /// The kept output the Dock handed over at the attach, to go right behind the reply.
    kept: Vec<u8>,
    /// Frames of output that came before the reply was queued, in order.
    held: Vec<Vec<u8>>,
}

impl RouteState {
    /// Shows `window` the terminal from the start of `kept`; one shown it already is
    /// shown it again from there. False, and nothing done, when the end has been told.
    fn admit(&mut self, window: &Arc<Window>, kept: &[u8]) -> bool {
        if self.over {
            return false;
        }
        self.members.retain(|member| !member.is(window));
        self.members.push(Member::new(window, kept));
        true
    }
}

impl Member {
    fn new(window: &Arc<Window>, kept: &[u8]) -> Member {
        Member {
            window: Arc::downgrade(window),
            released: false,
            kept: kept.to_vec(),
            held: Vec::new(),
        }
    }

    fn is(&self, window: &Arc<Window>) -> bool {
        self.window
            .upgrade()
            .is_some_and(|ours| Arc::ptr_eq(&ours, window))
    }

    /// Gives the Window what the program wrote, or keeps it until the reply is queued.
    /// False when the Window has gone, and takes nothing more.
    fn take(&mut self, frames: &[Vec<u8>]) -> bool {
        let Some(window) = self.window.upgrade() else {
            return false;
        };
        if !self.released {
            self.held.extend_from_slice(frames);
            return true;
        }
        frames.iter().all(|frame| window.queue(frame.clone()))
    }
}

impl Route {
    fn new(opener: &Arc<Window>, agents: Arc<Agents>) -> Route {
        Route {
            state: Mutex::new(RouteState {
                members: vec![Member::new(opener, &[])],
                ended: None,
                over: false,
            }),
            agents,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RouteState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the Dock tells of the terminal, on the Dock's thread for it.
    fn event(&self, shared: &Weak<Shared>, terminal: &yaar_link::TerminalId, event: Event) {
        match event {
            Event::Output(bytes) => {
                let frames = frames_of(terminal, bytes);
                self.state()
                    .members
                    .retain_mut(|member| member.take(&frames));
            }
            // After the output that held it: what the mark means is the agents' side's.
            Event::Mark(mark) => self.agents.marked(terminal, mark),
            Event::Ended(how) => {
                self.agents.terminal_ended(terminal);
                let mut state = self.state();
                state.ended = Some(ending(how));
                let told = Self::settle(&mut state);
                drop(state);
                // From this thread, which queued the last output: the end follows it on
                // the connection of each Window that holds it.
                if let Some(how) = told {
                    tell_ended(shared, terminal, how);
                }
            }
        }
    }

    /// The end of the program, once every Window that is shown the terminal has what
    /// came before it: taken, so that it is told once. None before.
    fn settle(state: &mut RouteState) -> Option<Ending> {
        if state.members.iter().all(|member| member.released) {
            let told = state.ended.take();
            state.over |= told.is_some();
            told
        } else {
            None
        }
    }

    /// `window` is shown the terminal from now on, after the kept output `kept`. One that
    /// was shown it already is shown it from the start of the kept output again, as its
    /// reply says: what it was sent before the reply it has, and the kept output holds.
    /// Called with the Dock holding the output still. False when the end of the program
    /// has been told already: the Window is not shown it.
    fn attach(&self, window: &Arc<Window>, kept: &[u8]) -> bool {
        self.state().admit(window, kept)
    }

    /// The reply to `window`'s request is queued: what was held goes out after it, in
    /// order, the kept output first, and from now on output goes out as it comes.
    fn release(
        &self,
        window: &Arc<Window>,
        shared: &Weak<Shared>,
        terminal: &yaar_link::TerminalId,
    ) {
        let mut state = self.state();
        if let Some(at) = state.members.iter().position(|member| member.is(window)) {
            let member = &mut state.members[at];
            let mut frames = frames_of(terminal, std::mem::take(&mut member.kept));
            frames.append(&mut member.held);
            member.released = true;
            // A Window that takes nothing more is gone, and so is its place.
            if !frames.iter().all(|frame| window.queue(frame.clone())) {
                state.members.remove(at);
            }
        }
        let told = Self::settle(&mut state);
        drop(state);
        if let Some(how) = told {
            tell_ended(shared, terminal, how);
        }
    }

    /// `window` has gone: it is shown the terminal no more, and a program that ended
    /// while it waited for its reply no longer holds the end back.
    fn leave(&self, window: &Arc<Window>, shared: &Weak<Shared>, terminal: &yaar_link::TerminalId) {
        let mut state = self.state();
        state.members.retain(|member| !member.is(window));
        let told = Self::settle(&mut state);
        drop(state);
        if let Some(how) = told {
            tell_ended(shared, terminal, how);
        }
    }
}

/// Tells every Window that the program of a terminal has ended, and wakes the thread
/// that waits to see whether the Keeper is still needed.
fn tell_ended(shared: &Weak<Shared>, terminal: &yaar_link::TerminalId, how: Ending) {
    // The Keeper that is leaving has nobody to tell.
    if let Some(shared) = shared.upgrade() {
        shared.tell(&[Fact::TerminalEnded {
            terminal: terminal.clone(),
            ending: how,
        }]);
        shared.program_ended();
    }
}

/// What a program wrote, as the frames that carry it: one for each 64 KiB at most, which
/// is more than the Dock reads at once, and far under the length of a frame.
fn frames_of(terminal: &yaar_link::TerminalId, bytes: Vec<u8>) -> Vec<Vec<u8>> {
    bytes
        .chunks(OUTPUT_FRAME)
        .map(|run| {
            frame_of(&Message::Output {
                terminal: terminal.clone(),
                bytes: run.to_vec(),
            })
            .expect("64 KiB fit in a frame")
        })
        .collect()
}

/// A size of the Link in the words of the Dock.
fn size(size: TerminalSize) -> yaar_dock::Size {
    yaar_dock::Size {
        columns: size.columns,
        rows: size.rows,
    }
}

/// How a program ended, in the words of the Link.
fn ending(ending: yaar_dock::Ending) -> Ending {
    match ending {
        yaar_dock::Ending::Exited(code) => Ending::Exited(code),
        yaar_dock::Ending::Signalled(signal) => Ending::Signalled(signal),
    }
}

/// The worktree of a workspace: none when the worktree is gone, refused with
/// `UnknownWorkspace` when the Keeper does not hold the workspace. It reads the Store and
/// the folder and runs no git: a save of a draft may ask at every key.
fn worktree_of(
    workspaces: &Workspaces,
    workspace: &yaar_link::WorkspaceId,
) -> Result<Option<Arc<Worktree>>, Refusal> {
    workspaces
        .worktree(&WorkspaceId::new(workspace.as_str()))
        .map_err(refusal)
}

/// The worktree a request of reviews needs, which is refused in the words of "the
/// worktree is gone" when it is not there.
fn worktree_needed(
    workspaces: &Workspaces,
    workspace: &yaar_link::WorkspaceId,
) -> Result<Arc<Worktree>, Refusal> {
    worktree_of(workspaces, workspace)?.ok_or_else(|| Refusal::Failed {
        reason: EngineError::NoWorktree.to_string(),
    })
}

/// The workspace of a review as the Store holds it, and the worktree of that workspace,
/// none when it is gone. A review whose workspace the Store does not hold has no worktree
/// either.
fn worktree_of_review(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &ReviewId,
) -> Result<(WorkspaceId, Option<Arc<Worktree>>), Refusal> {
    let workspace = reviews.review(review).map_err(engine_refusal)?;
    let workspace = workspace.workspace().clone();
    match workspaces.worktree(&workspace) {
        Ok(worktree) => Ok((workspace, worktree)),
        Err(WorkspaceError::Refused(yaar_workspaces::Refusal::UnknownWorkspace(_))) => {
            Ok((workspace, None))
        }
        Err(other) => Err(refusal(other)),
    }
}

/// The same, for the requests that cannot go on without the worktree.
fn worktree_of_review_needed(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &ReviewId,
) -> Result<(WorkspaceId, Arc<Worktree>), Refusal> {
    match worktree_of_review(workspaces, reviews, review)? {
        (workspace, Some(worktree)) => Ok((workspace, worktree)),
        (_, None) => Err(Refusal::Failed {
            reason: EngineError::NoWorktree.to_string(),
        }),
    }
}

/// What every Window is told when the reviews of a workspace changed.
fn reviews_changed(workspace: &WorkspaceId) -> Fact {
    Fact::ReviewsChanged {
        workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
    }
}

/// The reviews of a workspace, with its files marked viewed when the worktree is there and
/// each round with the turn it was sent in, which `store` keeps.
fn get_reviews(
    workspaces: &Workspaces,
    reviews: &Reviews,
    store: &Store,
    workspace: yaar_link::WorkspaceId,
) -> Result<yaar_link::WorkspaceReviews, Refusal> {
    let worktree = worktree_of(workspaces, &workspace)?;
    let view = reviews
        .of_workspace(&WorkspaceId::new(workspace.as_str()), worktree.as_deref())
        .map_err(engine_refusal)?;
    workspace_reviews(workspace, view, store)
}

/// Keeps one save of a draft. A text that changes a comment of the review is kept with no
/// lookup of a worktree and told to nobody; one that makes a comment is told to every
/// Window.
fn save_draft(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &yaar_link::ReviewId,
    save: &yaar_review_engine::DraftSave,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let review = ReviewId::new(review.as_str());
    // Only a thread that starts here is put on the files of the worktree, and needs it; one
    // on a file outside the worktree, which has an absolute path, is put on nothing, and the
    // worktree is not even looked up: the lookup may fail, and the save does not depend on it.
    let worktree = match &save.on {
        yaar_review_engine::DraftOn::NewThread(at) if !at.path.is_absolute() => {
            worktree_of_review(workspaces, reviews, &review)?.1
        }
        yaar_review_engine::DraftOn::NewThread(_) | yaar_review_engine::DraftOn::Thread => None,
    };
    let saved = reviews
        .save_draft(worktree.as_deref(), &review, save)
        .map_err(engine_refusal)?;
    if saved == Saved::Made
        && let Ok(found) = reviews.review(&review)
    {
        facts.push(reviews_changed(found.workspace()));
    }
    Ok(Reply::Done)
}

/// Copies a review as text, and sends what was copied.
fn copy_review(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &yaar_link::ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let id = ReviewId::new(review.as_str());
    let (workspace, worktree) = worktree_of_review_needed(workspaces, reviews, &id)?;
    match reviews.copy_as_text(&worktree, &id) {
        Ok(copied) => {
            facts.push(reviews_changed(&workspace));
            Ok(Reply::Copied {
                text: copied.text,
                round: copied.round,
            })
        }
        // The error does not carry the review that has nothing to send; the request does.
        Err(EngineError::Refused(ReviewError::NothingToSend)) => Err(Refusal::NothingToSend {
            review: review.clone(),
        }),
        Err(other) => Err(engine_refusal(other)),
    }
}

/// Marks the change of a review reviewed.
fn mark_reviewed(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &yaar_link::ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let id = ReviewId::new(review.as_str());
    let (workspace, worktree) = worktree_of_review_needed(workspaces, reviews, &id)?;
    reviews
        .mark_reviewed(&worktree, &id)
        .map_err(engine_refusal)?;
    facts.push(reviews_changed(&workspace));
    Ok(Reply::Done)
}

/// Marks a file of a workspace viewed, or takes the mark away.
fn set_viewed(
    workspaces: &Workspaces,
    reviews: &Reviews,
    workspace: &yaar_link::WorkspaceId,
    path: &std::path::Path,
    viewed: bool,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let worktree = worktree_needed(workspaces, workspace)?;
    let id = WorkspaceId::new(workspace.as_str());
    reviews
        .set_viewed(&id, &worktree, path, viewed)
        .map_err(engine_refusal)?;
    facts.push(reviews_changed(&id));
    Ok(Reply::Done)
}

/// The commit: the user's own git, run once the parts are let go. Every Window is told
/// that the changes of the workspace changed when a commit was made.
fn commit(worktree: &Worktree, workspace: &yaar_link::WorkspaceId, message: &str) -> Outcome {
    let mut facts = Vec::new();
    let reply = match worktree.commit_all(message) {
        Ok(commit) => {
            facts.push(Fact::ChangesChanged {
                workspace: workspace.clone(),
            });
            Ok(Reply::Committed {
                commit: commit.to_string(),
            })
        }
        Err(CommitError::NothingToCommit) => Err(Refusal::NothingToCommit {
            workspace: workspace.clone(),
        }),
        Err(other) => Err(Refusal::Failed {
            reason: other.to_string(),
        }),
    };
    Outcome {
        reply,
        facts,
        then: None,
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
pub(crate) fn list(workspaces: &Workspaces) -> Result<Vec<yaar_link::Repository>, Refusal> {
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
                    head: workspace.head.map(head),
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

/// The reviews of a workspace, in the words of the Link; refused when `store` cannot say
/// which turn a round was sent in.
fn workspace_reviews(
    workspace: yaar_link::WorkspaceId,
    view: yaar_review_engine::WorkspaceView,
    store: &Store,
) -> Result<yaar_link::WorkspaceReviews, Refusal> {
    let mut reviews = view
        .reviews
        .iter()
        .map(|found| {
            let turns = store
                .round_turns(&yaar_store::ReviewId::new(found.id().as_str()))
                .map_err(|e| engine_refusal(EngineError::Store(e)))?;
            Ok(review(found, &turns))
        })
        .collect::<Result<Vec<yaar_link::Review>, Refusal>>()?;
    // A thread on a file outside the worktree, which has an absolute path, has no version:
    // the Store keeps the empty one, and the Window is told none.
    for thread in reviews.iter_mut().flat_map(|review| &mut review.threads) {
        if thread.anchor.path.is_absolute() {
            thread.anchor.tree = None;
        }
    }
    Ok(yaar_link::WorkspaceReviews {
        workspace,
        reviews,
        viewed: view.viewed,
    })
}

/// One review, in the words of the Link. `turns` is the turn each of its rounds was sent
/// in, by the round's number, as the Store keeps it: a round with none, as a copy, has
/// none.
fn review(review: &yaar_review_engine::Review, turns: &[(u32, String)]) -> yaar_link::Review {
    use yaar_review_engine::{Attachment, Author, SentBy};

    yaar_link::Review {
        id: yaar_link::ReviewId::new(review.id().as_str()),
        agent: review.agent().map(yaar_link::SessionId::new),
        reviewed: review.reviewed().map(str::to_owned),
        attached: review.attached().map(|attached| match attached {
            Attachment::Review => yaar_link::Attachment::Review,
            Attachment::Thread(thread) => {
                yaar_link::Attachment::Thread(yaar_link::ThreadId::new(thread.as_str()))
            }
        }),
        threads: review
            .threads()
            .iter()
            .map(|thread| yaar_link::Thread {
                id: yaar_link::ThreadId::new(thread.id.as_str()),
                anchor: yaar_link::Anchor {
                    tree: Some(thread.anchor.tree.clone()),
                    path: thread.anchor.path.clone(),
                    side: side(thread.anchor.side),
                    lines: yaar_link::Lines {
                        first: thread.anchor.first,
                        last: thread.anchor.last,
                    },
                    text: thread.anchor.text.clone(),
                },
                resolved: thread.resolved,
                addressed: thread.addressed,
                state: crate::reviewing::thread_state(review, &thread.id),
                now: thread.now.as_ref().map(|now| yaar_link::Placed {
                    tree: now.tree.clone(),
                    path: now.path.clone(),
                    side: side(now.side),
                    lines: yaar_link::Lines {
                        first: now.first,
                        last: now.last,
                    },
                    notice: now.notice.map(|notice| match notice {
                        yaar_review_engine::NoticeRecord::Changed => yaar_link::Notice::Changed,
                        yaar_review_engine::NoticeRecord::Deleted => yaar_link::Notice::Deleted,
                    }),
                }),
                comments: thread
                    .comments
                    .iter()
                    .map(|comment| yaar_link::Comment {
                        id: yaar_link::CommentId::new(comment.id.as_str()),
                        author: match comment.author {
                            Author::Reviewer => yaar_link::Author::Reviewer,
                            Author::Agent => yaar_link::Author::Agent,
                        },
                        text: comment.text.clone(),
                        round: comment.round,
                    })
                    .collect(),
            })
            .collect(),
        rounds: review
            .rounds()
            .iter()
            .map(|round| yaar_link::Round {
                number: round.number,
                state: round.state.clone(),
                sent_by: match round.sent_by {
                    SentBy::Copied => yaar_link::SentBy::Copied,
                    SentBy::Delivered => yaar_link::SentBy::Delivered,
                },
                edited: round.edited,
                turn: turns
                    .iter()
                    .find(|(number, _)| *number == round.number)
                    .map(|(_, turn)| yaar_link::TurnId::new(turn)),
            })
            .collect(),
    }
}

/// A side of a diff, in the words of the Link.
fn side(side: yaar_review_engine::Side) -> yaar_link::Side {
    match side {
        yaar_review_engine::Side::Old => yaar_link::Side::Old,
        yaar_review_engine::Side::New => yaar_link::Side::New,
    }
}

/// One save of a draft, in the words of the engine.
fn draft_save(
    thread: &yaar_link::ThreadId,
    comment: &yaar_link::CommentId,
    on: yaar_link::DraftOn,
    text: String,
) -> yaar_review_engine::DraftSave {
    use yaar_review_engine::{DraftOn, NewAnchor};

    yaar_review_engine::DraftSave {
        thread: yaar_review_engine::ThreadId::new(thread.as_str()),
        comment: yaar_review_engine::CommentId::new(comment.as_str()),
        on: match on {
            yaar_link::DraftOn::NewThread { anchor } => DraftOn::NewThread(NewAnchor {
                tree: anchor.tree,
                path: anchor.path,
                side: match anchor.side {
                    yaar_link::Side::Old => yaar_review_engine::Side::Old,
                    yaar_link::Side::New => yaar_review_engine::Side::New,
                },
                first: anchor.lines.first,
                last: anchor.lines.last,
                text: anchor.text,
            }),
            yaar_link::DraftOn::Thread => DraftOn::Thread,
        },
        text,
    }
}

/// Puts a failure of the Review engine into the words of the Link: what names something
/// the Keeper does not hold, a sent comment, and a path out of the worktree keep their
/// kind; the rest becomes `Refusal::Failed` with the sentence of the error. An empty send,
/// `ReviewError::NothingToSend`, is not mapped here: its refusal names the review, which
/// the error does not carry, so the caller that copies a review maps it to
/// `Refusal::NothingToSend` before it calls this.
fn engine_refusal(error: yaar_review_engine::EngineError) -> Refusal {
    use yaar_review_engine::{EngineError, ReviewError};

    match error {
        EngineError::UnknownReview(review) => Refusal::UnknownReview {
            review: yaar_link::ReviewId::new(review.as_str()),
        },
        EngineError::Refused(ReviewError::UnknownThread(thread)) => Refusal::UnknownThread {
            thread: yaar_link::ThreadId::new(thread.as_str()),
        },
        EngineError::Refused(ReviewError::UnknownComment(comment)) => Refusal::UnknownComment {
            comment: yaar_link::CommentId::new(comment.as_str()),
        },
        EngineError::Refused(ReviewError::NotADraft(comment)) => Refusal::NotADraft {
            comment: yaar_link::CommentId::new(comment.as_str()),
        },
        EngineError::OutsideWorktree(path) => Refusal::OutsideWorkspace { path },
        other => Refusal::Failed {
            reason: other.to_string(),
        },
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
            // Refusals of a worktree made from a branch: the Link has no kind of its own for
            // them, and the sentence names what to mend.
            other @ (Refused::UnknownRepository(_)
            | Refused::NotABranchName(_)
            | Refused::BranchExists(_)
            | Refused::FolderExists(_)
            | Refused::FolderNotAbsolute(_)
            | Refused::FolderInsideRepository(_)) => Refusal::Failed {
                reason: other.to_string(),
            },
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
    use std::os::unix::net::UnixStream;
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
    fn the_words_of_the_dock_become_those_of_the_link() {
        assert_eq!(
            size(TerminalSize {
                columns: 120,
                rows: 40
            }),
            yaar_dock::Size {
                columns: 120,
                rows: 40
            }
        );
        assert_eq!(ending(yaar_dock::Ending::Exited(3)), Ending::Exited(3));
        assert_eq!(
            ending(yaar_dock::Ending::Signalled(9)),
            Ending::Signalled(9)
        );
    }

    #[test]
    fn what_a_program_writes_is_cut_into_frames_of_at_most_64_kib_and_loses_nothing() {
        let terminal = yaar_link::TerminalId::new("0".repeat(32));
        let bytes: Vec<u8> = (0..150_000u32).map(|n| (n % 251) as u8).collect();
        let frames = frames_of(&terminal, bytes.clone());
        assert_eq!(frames.len(), 3);
        let mut joined = Vec::new();
        for frame in frames {
            let Ok(Message::Output {
                terminal: from,
                bytes,
            }) = yaar_link::read_message(&mut frame.as_slice())
            else {
                panic!("not a frame of output");
            };
            assert_eq!(from, terminal);
            assert!(bytes.len() <= OUTPUT_FRAME);
            joined.extend(bytes);
        }
        assert_eq!(joined, bytes);
        assert!(frames_of(&terminal, Vec::new()).is_empty());
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

    #[test]
    fn a_window_is_not_shown_a_terminal_whose_end_has_been_told() {
        let window = || Window::new(UnixStream::pair().expect("a pair of sockets").0);
        let (opener, late) = (window(), window());
        let mut state = RouteState::default();
        state.admit(&opener, &[]);
        state.members[0].released = true;
        state.ended = Some(Ending::Exited(0));

        // The end waits for a Window whose reply is not queued: one more may still be shown
        // the terminal, and is told of the end with the rest.
        let waiting = window();
        assert!(state.admit(&waiting, b"kept"));
        assert!(Route::settle(&mut state).is_none());
        state.members[1].released = true;

        // Once the end is told, the terminal is over, and no Window is shown it after that:
        // it would never hear what it is told of an end that has gone by.
        assert_eq!(Route::settle(&mut state), Some(Ending::Exited(0)));
        assert!(!state.admit(&late, b"kept"));
        assert!(!state.members.iter().any(|member| member.is(&late)));
    }
}
