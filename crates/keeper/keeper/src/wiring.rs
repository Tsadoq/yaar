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
//! | `Report` | `Agents::report`: queued, then answered; what it means on the agents' own thread | later `AgentsChanged`, `Step`, `TurnEnded` |
//! | `ListAgents` | `Agents::list` | none |
//! | `GetHistory` | `Agents::history` | none |
//! | `GetUncommitted` | `Agents::uncommitted` | none |
//! | `CompareVersions` | `Agents::compare` | none |
//! | `ReadVersions` | `Agents::read_versions` | none |
//! | `GetTurn` | `Agents::turn` | none |
//! | `ShowWorkspace` | the watchers of the worktrees on screen | later `ChangesChanged`, `Workspaces` |
//! | `ListTerminals` | the terminals the Dock holds, with their workspace and program | none |
//! | `AttachTerminal` | `Dock::attach`, then `Dock::resize` and `Dock::redraw` | none |
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
//! to `Dock::write`; what the program writes goes, as `Message::Output`, to the Window
//! that opened the terminal, and only after the reply that named it. When the program
//! ends, every Window is told `Fact::TerminalEnded`, after the last output.
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

use std::sync::{Arc, Mutex, PoisonError, Weak};

use yaar_dock::{Dock, DockError, Event, Start};
use yaar_link::{
    Ending, Fact, Message, Program, Refusal, Reply, Request, RequestKind, TerminalSize,
};
use yaar_recorder::{CommitError, CommitId, Git, Worktree};
use yaar_review_engine::{EngineError, ReviewError, ReviewId, Reviews, Saved};
use yaar_store::Store;
use yaar_workspaces::{WorkspaceError, WorkspaceId, Workspaces};

use crate::agents::{Agents, not_wired};
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
    // What the programs of the Dock tell the Windows goes through here.
    shared: Weak<Shared>,
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
        // A CLI waits for its piece, which waits for this answer: it is given at once,
        // before anything the report leads to.
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
        // to tell them.
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
    pub(crate) fn start(options: &Options, shared: Weak<Shared>) -> Parts {
        let kept = Git::find().map_err(|e| e.to_string()).and_then(|git| {
            let file = options.data_dir.join(yaar_store::FILE_NAME);
            let store = Store::open(&file).map_err(|e| e.to_string())?;
            let reviews = Reviews::new(store.clone(), git.clone(), options.data_dir.join("states"));
            Ok(Kept {
                workspaces: Workspaces::new(git, store.clone()),
                reviews,
                store,
            })
        });
        let watching = Arc::new(Watching::new(shared.clone()));
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
            shared,
        }
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
            Request::OpenTerminal {
                workspace,
                program,
                size: at,
            } => self
                .open_terminal(workspaces, window, &workspace, &program, at)
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
                get_reviews(workspaces, reviews, workspace).map(Reply::Reviews)
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
            Request::Report { report } => return Answered::Now(self.agents.report(kept, report)),
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
            Request::ListTerminals | Request::AttachTerminal { .. } => Err(not_wired()),
        };
        Answered::Now(Outcome { reply, facts, then })
    }

    /// Starts `program` in the folder of `workspace`, in a terminal of the Dock. The
    /// reply names the terminal, and the step that goes with it lets the output of the
    /// program through: until the reply is queued, nothing of it is sent.
    fn open_terminal(
        &self,
        workspaces: &Workspaces,
        window: &Arc<Window>,
        workspace: &yaar_link::WorkspaceId,
        program: &Program,
        at: TerminalSize,
    ) -> Result<(Reply, Step), Refusal> {
        let folder = list(workspaces)?
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
        let mut start = match program {
            Program::Shell => Start::shell(folder, size(at)),
            Program::Agent { agent } => match self.agents.launcher(agent) {
                Some(command) => Start::launcher(folder, size(at), command),
                None => {
                    return Err(Refusal::Failed {
                        reason: format!("the app knows no agent CLI named {agent}"),
                    });
                }
            },
        };
        // Every terminal is watched for the marks of every agent CLI: a CLI typed in a
        // shell writes them as one started from its launcher does.
        start.marks = self.agents.marks();
        let route = Arc::new(Route::new(window, Arc::clone(&self.agents)));
        let events = {
            let (route, shared, id) = (
                Arc::clone(&route),
                self.shared.clone(),
                yaar_link::TerminalId::new(id.as_str()),
            );
            Box::new(move |event| route.event(&shared, &id, event))
        };
        self.dock
            .open(id.clone(), start, events)
            .map_err(|error| Refusal::Failed {
                reason: error.to_string(),
            })?;
        let terminal = yaar_link::TerminalId::new(id.as_str());
        let shared = self.shared.clone();
        Ok((
            Reply::TerminalOpened {
                terminal: terminal.clone(),
            },
            Box::new(move || route.release(&shared, &terminal)),
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

/// Where the output of one terminal goes: to the Window that opened it, and in order.
///
/// The reply that names a terminal must reach its Window before the terminal's first
/// output, and nothing may be lost in between. The program starts before the reply is
/// made, so what it writes first is held until the reply is queued, and then queued
/// behind it.
struct Route {
    // Not held for long: a Window that has gone away is gone, though its terminal runs
    // on, and its connection closes with it.
    opener: Weak<Window>,
    state: Mutex<RouteState>,
    // Told the marks found in the output and the end of the program.
    agents: Arc<Agents>,
}

#[derive(Default)]
struct RouteState {
    /// The reply is queued: output goes straight to the opener.
    released: bool,
    /// Frames of output that came before the reply was queued, in order.
    held: Vec<Vec<u8>>,
    /// The program ended before the reply was queued: the end is told after the output.
    ended: Option<Ending>,
}

impl Route {
    fn new(opener: &Arc<Window>, agents: Arc<Agents>) -> Route {
        Route {
            opener: Arc::downgrade(opener),
            state: Mutex::new(RouteState::default()),
            agents,
        }
    }

    /// Queues frames for the Window that opened the terminal. A Window that is gone takes
    /// nothing, and the program goes on.
    fn queue(&self, frames: Vec<Vec<u8>>) {
        if let Some(opener) = self.opener.upgrade() {
            for frame in frames {
                let _ = opener.queue(frame);
            }
        }
    }

    /// What the Dock tells of the terminal, on the Dock's thread for it.
    fn event(&self, shared: &Weak<Shared>, terminal: &yaar_link::TerminalId, event: Event) {
        match event {
            Event::Output(bytes) => {
                let frames = frames_of(terminal, bytes);
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.released {
                    drop(state);
                    self.queue(frames);
                } else {
                    state.held.extend(frames);
                }
            }
            // After the output that held it: what the mark means is the agents' side's.
            Event::Mark(mark) => self.agents.marked(terminal, mark),
            Event::Ended(how) => {
                self.agents.terminal_ended(terminal);
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.released {
                    drop(state);
                    // From this thread, which queued the last output: the end follows
                    // it on the connection of the Window that opened the terminal.
                    tell_ended(shared, terminal, ending(how));
                } else {
                    state.ended = Some(ending(how));
                }
            }
        }
    }

    /// The reply that names the terminal is queued: what was held goes out after it, in
    /// order, and from now on output goes out as it comes.
    fn release(&self, shared: &Weak<Shared>, terminal: &yaar_link::TerminalId) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.queue(std::mem::take(&mut state.held));
        state.released = true;
        let ended = state.ended.take();
        drop(state);
        if let Some(how) = ended {
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

/// The reviews of a workspace, with its files marked viewed when the worktree is there.
fn get_reviews(
    workspaces: &Workspaces,
    reviews: &Reviews,
    workspace: yaar_link::WorkspaceId,
) -> Result<yaar_link::WorkspaceReviews, Refusal> {
    let worktree = worktree_of(workspaces, &workspace)?;
    let view = reviews
        .of_workspace(&WorkspaceId::new(workspace.as_str()), worktree.as_deref())
        .map_err(engine_refusal)?;
    Ok(workspace_reviews(workspace, view))
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
    // Only a thread that starts here is put on the files of the worktree, and needs it.
    let worktree = match save.on {
        yaar_review_engine::DraftOn::NewThread(_) => {
            worktree_of_review(workspaces, reviews, &review)?.1
        }
        yaar_review_engine::DraftOn::Thread => None,
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

/// The reviews of a workspace, in the words of the Link.
fn workspace_reviews(
    workspace: yaar_link::WorkspaceId,
    view: yaar_review_engine::WorkspaceView,
) -> yaar_link::WorkspaceReviews {
    yaar_link::WorkspaceReviews {
        workspace,
        reviews: view.reviews.iter().map(review).collect(),
        viewed: view.viewed,
    }
}

/// One review, in the words of the Link.
fn review(review: &yaar_review_engine::Review) -> yaar_link::Review {
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
}
