//! Hands each request of a Window to the part that answers it and puts the result into the Link's
//! words. The reply goes out before the facts: a Window that opened a folder learns the new
//! workspace's id before the list that holds it.

mod worktrees;

pub(crate) use worktrees::set_worktrees_folder;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use yaar_dock::{Dock, DockError, Entry, Event, Start};
use yaar_link::{
    Ending, Fact, Message, Program, Refusal, Reply, Request, RequestKind, SOCKET_VARIABLE,
    TERMINAL_VARIABLE, TerminalSize,
};
use yaar_recorder::{CommitError, CommitId, Git, GitEnvironment, Worktree};
use yaar_review_engine::{EngineError, ReviewError, ReviewId, Reviews, Saved, StateWatch};
use yaar_store::Store;
use yaar_workspaces::{WorkspaceError, WorkspaceId, Workspaces};

use crate::agents::Agents;
use crate::base_moves;
use crate::watching::Watching;
use crate::{Options, Shared, Window, frame_of};

/// git's own number: how many first bytes say whether a file is text.
const TEXT_PROBE: usize = 8000;

const OUTPUT_FRAME: usize = 64 * 1024;

pub(crate) struct Parts {
    kept: Result<Kept, String>,
    // Also held by the terminals' routes, so the Dock's threads never wait for the parts.
    agents: Arc<Agents>,
    watching: Arc<Watching>,
    // Dropped with the parts, which ends every program.
    dock: Dock,
    terminals: Arc<Terminals>,
    shared: Weak<Shared>,
    socket: PathBuf,
}

pub(crate) struct Kept {
    pub(crate) workspaces: Workspaces,
    pub(crate) reviews: Reviews,
    pub(crate) store: Store,
}

pub(crate) type Step = Box<dyn FnOnce() + Send>;

pub(crate) struct Outcome {
    pub(crate) reply: Result<Reply, Refusal>,
    pub(crate) facts: Vec<Fact>,
    // Run once the reply is queued, before the facts are told.
    pub(crate) then: Option<Step>,
}

impl Outcome {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answerer {
    /// On the thread that reads the Window's connection, before its next message is read.
    Here,
    // By the Window's one thread for reviews, one after the other in the order they came.
    InOrder,
    Apart,
}

pub(crate) fn answerer(kind: RequestKind) -> Answerer {
    // No arm for "the rest": a new kind of request has to say which it is.
    match kind {
        RequestKind::ListWorkspaces
        | RequestKind::OpenFolder
        | RequestKind::SetBase
        | RequestKind::GetChanges
        | RequestKind::ReadFile
        | RequestKind::OpenTerminal
        | RequestKind::ResizeTerminal
        | RequestKind::CloseTerminal => Answerer::Here,
        // Saves keep typing order and a reading after a save must find it. A copy or a mark of
        // reviewed can take a second on a large repository, which the connection's thread must not
        // wait for.
        RequestKind::GetReviews
        | RequestKind::SaveDraft
        | RequestKind::DeleteDraft
        | RequestKind::CopyReview
        | RequestKind::MarkReviewed
        | RequestKind::SetViewed => Answerer::InOrder,
        RequestKind::Commit => Answerer::Apart,
        // Here although it waits: the CLI blocks on its piece, and the piece's connection has
        // nothing else to answer.
        RequestKind::Report => Answerer::Here,
        RequestKind::ListAgents
        | RequestKind::GetTurn
        | RequestKind::ListTerminals
        | RequestKind::DismissResumable => Answerer::Here,
        // The kept output of an attached terminal must reach its Window after the reply.
        RequestKind::ShowWorkspace | RequestKind::AttachTerminal => Answerer::Here,
        RequestKind::GetHistory
        | RequestKind::GetUncommitted
        | RequestKind::CompareVersions
        | RequestKind::ReadVersions => Answerer::Apart,
        RequestKind::GetSettings | RequestKind::SetSetting => Answerer::Here,
        RequestKind::AttachReview
        | RequestKind::DetachReview
        | RequestKind::ResolveThread
        | RequestKind::ReopenThread
        | RequestKind::ResolveAddressed
        | RequestKind::GetAttachment => Answerer::InOrder,
        RequestKind::ListClis => Answerer::Here,
        RequestKind::CompareForReview => Answerer::Apart,
        RequestKind::ListConsents | RequestKind::AnswerConsent => Answerer::Here,
        RequestKind::ExportVersion => Answerer::Apart,
        RequestKind::CreateWorktree => Answerer::Apart,
        RequestKind::GetStateSizes | RequestKind::CleanUpStates => Answerer::Apart,
    }
}

pub(crate) enum Answered {
    Now(Outcome),
    Later(Job),
}

pub(crate) type Job = Box<dyn FnOnce() -> Outcome + Send>;

/// Whether requests of this kind take the Keeper's turn: answered one at a time with the others
/// that take it, each with its reply and facts sent before the next is begun.
pub(crate) fn waits_its_turn(kind: RequestKind) -> bool {
    match kind {
        // Each can change what the Keeper keeps; its facts must reach the Windows in order.
        RequestKind::OpenFolder | RequestKind::SetBase => true,
        // Its reply carries what a fact carries (also `GetSettings`, `ListAgents`, `ListConsents`):
        // read between two changes and sent after the later fact, it would pass for the newer list.
        RequestKind::ListWorkspaces => true,
        RequestKind::GetChanges | RequestKind::ReadFile => false,
        RequestKind::OpenTerminal | RequestKind::ResizeTerminal | RequestKind::CloseTerminal => {
            false
        }
        // Their facts carry only the workspace's id and a Window told one reads again, so their
        // order does not matter; none may wait behind a recording or the user's git.
        RequestKind::GetReviews
        | RequestKind::SaveDraft
        | RequestKind::DeleteDraft
        | RequestKind::CopyReview
        | RequestKind::MarkReviewed
        | RequestKind::SetViewed
        | RequestKind::Commit => false,
        RequestKind::ListAgents | RequestKind::DismissResumable => true,
        // Its facts come from the agents' thread, which takes the turn, and the answer waits for
        // that thread: a turn held here would wait for ever.
        RequestKind::Report => false,
        RequestKind::GetHistory
        | RequestKind::GetUncommitted
        | RequestKind::CompareVersions
        | RequestKind::ReadVersions
        | RequestKind::GetTurn
        | RequestKind::ListTerminals => false,
        RequestKind::ShowWorkspace | RequestKind::AttachTerminal => false,
        RequestKind::GetSettings | RequestKind::SetSetting => true,
        RequestKind::AttachReview
        | RequestKind::DetachReview
        | RequestKind::ResolveThread
        | RequestKind::ReopenThread
        | RequestKind::ResolveAddressed
        | RequestKind::GetAttachment => false,
        RequestKind::ListClis | RequestKind::CompareForReview => false,
        RequestKind::ListConsents | RequestKind::AnswerConsent => true,
        RequestKind::ExportVersion => false,
        RequestKind::CreateWorktree => false,
        RequestKind::GetStateSizes | RequestKind::CleanUpStates => false,
    }
}

impl Parts {
    /// A Keeper with no git or an unopenable database still starts and answers the hello, so that a
    /// Window can say what is wrong; each request is then refused with the reason.
    pub(crate) fn start(
        options: &Options,
        clock: Arc<dyn yaar_recorder::Clock>,
        environment: GitEnvironment,
        shared: Weak<Shared>,
    ) -> Parts {
        let watching = Arc::new(Watching::new(shared.clone()));
        let kept = Git::find().map_err(|e| e.to_string()).and_then(|git| {
            let git = git.with_environment(environment);
            let file = options.data_dir.join(yaar_store::FILE_NAME);
            let store = Store::open(&file).map_err(|e| e.to_string())?;
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

    pub(crate) fn kept(&self) -> Option<&Kept> {
        self.kept.as_ref().ok()
    }

    pub(crate) fn agents(&self) -> &Agents {
        &self.agents
    }

    pub(crate) fn window_gone(&self, window: &Arc<Window>) {
        self.terminals.window_gone(window, &self.shared);
        if let Ok(kept) = &self.kept {
            self.watching.window_gone(kept, window);
        }
    }

    pub(crate) fn consents(&self) -> Vec<yaar_link::Consent> {
        self.agents.consents()
    }

    pub(crate) fn running(&self) -> usize {
        self.dock.running()
    }

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
            Request::CloseTerminal { terminal } => {
                if self.terminals.remove(&terminal) {
                    self.dock
                        .stop_one(&yaar_dock::TerminalId::new(terminal.as_str()));
                    Ok(Reply::Done)
                } else {
                    Err(Refusal::UnknownTerminal { terminal })
                }
            }
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
                if marked.is_ok() {
                    self.agents.reviewed(&review);
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
            Request::Report { report } => return self.agents.report(report),
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
            Request::ListClis => Ok(Reply::Clis(self.agents.supported())),
            Request::CompareForReview {
                workspace,
                review,
                from,
                to,
            } => self
                .agents
                .compare_for_review(kept, &workspace, &review, &from, &to),
            Request::ListConsents => Ok(Reply::Consents(self.agents.consents())),
            Request::AnswerConsent { agent, answer } => {
                self.agents.answer_consent(&agent, answer).map(|consents| {
                    facts.push(Fact::ConsentsChanged(consents.clone()));
                    Reply::Consents(consents)
                })
            }
            Request::ExportVersion {
                workspace,
                version,
                into,
            } => self
                .agents
                .export_version(kept, &workspace, &version, &into),
            Request::CreateWorktree {
                repository,
                from,
                branch,
                agent,
                size: at,
            } => {
                return self.create_worktree(kept, window, &repository, &from, &branch, &agent, at);
            }
            Request::GetStateSizes => crate::cleaning::sizes(kept),
            Request::CleanUpStates => crate::cleaning::clean_up_now(kept, &self.agents, &mut facts),
            Request::DismissResumable { workspace, session } => {
                self.agents
                    .dismiss_resumable(kept, &workspace, session.as_ref(), &mut facts)
            }
        };
        Answered::Now(Outcome { reply, facts, then })
    }

    /// The step that goes with the reply lets the program's output through: until the reply is
    /// queued, nothing of it is sent.
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
        // Made before the start so that the program's environment can name it.
        let id = yaar_dock::TerminalId::random().map_err(|error| Refusal::Failed {
            reason: format!("no name for the terminal could be made: {error}"),
        })?;
        let terminal = yaar_link::TerminalId::new(id.as_str());
        // Refused before anything is listed or started: the terminal would bind a review nobody
        // holds.
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
            Program::Resume { session } => {
                let command = self.agents.resume_command(session, &terminal)?;
                Start::launcher(folder, size(at), &command)
            }
        };
        // Every shell is told the Keeper's socket and its terminal, so a piece of an agent CLI
        // finds its way back.
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
        start.marks = self.agents.marks();
        let route = Arc::new(Route::new(window, Arc::clone(&self.agents)));
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
                if matches!(event, Event::Ended(_)) {
                    terminals.remove(&terminal);
                }
                route.event(&shared, &terminal, event);
            })
        };
        // Told before the program starts: a CLI reports from its first moment.
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

    /// Shows `window` the terminal as well: the reply, the kept output, then live output, with
    /// nothing lost or sent twice. The resize and redraw mend the cut at the start of the kept
    /// output.
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
        // Joined while the Dock holds the output still, so the kept output ends where the live
        // begins.
        self.dock
            .attach(&id, |kept, dropped| {
                cut = dropped;
                shown = route.attach(window, kept);
            })
            .map_err(|_| unknown())?;
        if !shown {
            return Err(unknown());
        }
        // A program that ended meanwhile has nothing to draw; its end is told as it comes.
        let _ = self.dock.resize(&id, size(at));
        let _ = self.dock.redraw(&id);
        let (shared, window, terminal) =
            (self.shared.clone(), Arc::clone(window), terminal.clone());
        Ok((
            Reply::Attached { cut },
            Box::new(move || route.release(&window, &shared, &terminal)),
        ))
    }

    pub(crate) fn input(&self, terminal: &yaar_link::TerminalId, bytes: &[u8]) {
        let _ = self
            .dock
            .write(&yaar_dock::TerminalId::new(terminal.as_str()), bytes);
    }
}

#[derive(Default)]
pub(crate) struct Terminals {
    held: Mutex<Vec<Held>>,
}

struct Held {
    id: yaar_link::TerminalId,
    workspace: yaar_link::WorkspaceId,
    program: Program,
    route: Arc<Route>,
    opened: bool,
}

impl Terminals {
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Held>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, terminal: Held) {
        self.held().push(terminal);
    }

    fn opened(&self, terminal: &yaar_link::TerminalId) {
        if let Some(held) = self.held().iter_mut().find(|held| held.id == *terminal) {
            held.opened = true;
        }
    }

    fn remove(&self, terminal: &yaar_link::TerminalId) -> bool {
        let mut held = self.held();
        let before = held.len();
        held.retain(|held| held.id != *terminal);
        held.len() != before
    }

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

    fn route(&self, terminal: &yaar_link::TerminalId) -> Option<Arc<Route>> {
        self.held()
            .iter()
            .find(|held| held.opened && held.id == *terminal)
            .map(|held| Arc::clone(&held.route))
    }

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

/// Where the output of one terminal goes. The program starts before the reply is made, so what it
/// writes first is held until the reply is queued; a Window that attaches later waits behind both.
struct Route {
    state: Mutex<RouteState>,
    agents: Arc<Agents>,
}

#[derive(Default)]
struct RouteState {
    members: Vec<Member>,
    ended: Option<Ending>,
    over: bool,
}

struct Member {
    window: Weak<Window>,
    released: bool,
    kept: Vec<u8>,
    held: Vec<Vec<u8>>,
}

impl RouteState {
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

    fn event(&self, shared: &Weak<Shared>, terminal: &yaar_link::TerminalId, event: Event) {
        match event {
            Event::Output(bytes) => {
                let frames = frames_of(terminal, bytes);
                self.state()
                    .members
                    .retain_mut(|member| member.take(&frames));
            }
            Event::Mark(mark) => self.agents.marked(terminal, mark),
            Event::Ended(how) => {
                self.agents.terminal_ended(terminal);
                let mut state = self.state();
                state.ended = Some(ending(how));
                let told = Self::settle(&mut state);
                drop(state);
                if let Some(how) = told {
                    tell_ended(shared, terminal, how);
                }
            }
        }
    }

    fn settle(state: &mut RouteState) -> Option<Ending> {
        if state.members.iter().all(|member| member.released) {
            let told = state.ended.take();
            state.over |= told.is_some();
            told
        } else {
            None
        }
    }

    fn attach(&self, window: &Arc<Window>, kept: &[u8]) -> bool {
        self.state().admit(window, kept)
    }

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

    /// `window` has gone: it is shown the terminal no more, and an end held back for its release is
    /// told once every Window left has been released.
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

fn tell_ended(shared: &Weak<Shared>, terminal: &yaar_link::TerminalId, how: Ending) {
    if let Some(shared) = shared.upgrade() {
        shared.tell(&[Fact::TerminalEnded {
            terminal: terminal.clone(),
            ending: how,
        }]);
        shared.program_ended();
    }
}

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

fn size(size: TerminalSize) -> yaar_dock::Size {
    yaar_dock::Size {
        columns: size.columns,
        rows: size.rows,
    }
}

fn ending(ending: yaar_dock::Ending) -> Ending {
    match ending {
        yaar_dock::Ending::Exited(code) => Ending::Exited(code),
        yaar_dock::Ending::Signalled(signal) => Ending::Signalled(signal),
    }
}

/// None when the worktree is gone; `UnknownWorkspace` when the Keeper does not hold the workspace.
/// Runs no git: a save of a draft may ask at every key.
fn worktree_of(
    workspaces: &Workspaces,
    workspace: &yaar_link::WorkspaceId,
) -> Result<Option<Arc<Worktree>>, Refusal> {
    workspaces
        .worktree(&WorkspaceId::new(workspace.as_str()))
        .map_err(refusal)
}

fn worktree_needed(
    workspaces: &Workspaces,
    workspace: &yaar_link::WorkspaceId,
) -> Result<Arc<Worktree>, Refusal> {
    worktree_of(workspaces, workspace)?.ok_or_else(|| Refusal::Failed {
        reason: EngineError::NoWorktree.to_string(),
    })
}

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

fn reviews_changed(workspace: &WorkspaceId) -> Fact {
    Fact::ReviewsChanged {
        workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
    }
}

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

fn save_draft(
    workspaces: &Workspaces,
    reviews: &Reviews,
    review: &yaar_link::ReviewId,
    save: &yaar_review_engine::DraftSave,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let review = ReviewId::new(review.as_str());
    // A thread on a file outside the worktree (absolute path) skips the lookup, which may fail and
    // which the save does not depend on.
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

fn open_folder(
    workspaces: &Workspaces,
    path: std::path::PathBuf,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    if !path.is_absolute() {
        return Err(Refusal::NoSuchFolder { path });
    }
    let opened = workspaces.open_folder(&path).map_err(refusal)?;
    // Told even for a folder already open: the list is read from git, and a worktree made outside
    // the app may be new to every Window.
    if let Ok(list) = list(workspaces) {
        facts.push(Fact::Workspaces(list));
    }
    Ok(Reply::Opened {
        repository: yaar_link::RepositoryId::new(opened.repository.as_str()),
        workspace: yaar_link::WorkspaceId::new(opened.workspace.as_str()),
    })
}

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

/// One version of a file as the Link carries it: its bytes when it is text no larger than
/// `MAX_CONTENT`, else its size alone; a binary one is said to be so whatever its size.
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
    // A thread on a file outside the worktree has no version: the Store keeps the empty one, and
    // the Window is told none.
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

/// `turns` is the turn each round was sent in, by round number; a round with none, as a copy, has
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

fn side(side: yaar_review_engine::Side) -> yaar_link::Side {
    match side {
        yaar_review_engine::Side::Old => yaar_link::Side::Old,
        yaar_review_engine::Side::New => yaar_link::Side::New,
    }
}

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

pub(crate) fn failed(error: impl std::fmt::Display) -> Refusal {
    Refusal::Failed {
        reason: error.to_string(),
    }
}

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
            // The Link has no kind of its own for these; the sentence names what to mend.
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

        let largest = vec![b'a'; yaar_link::MAX_CONTENT as usize];
        assert_eq!(content(Some(largest.clone())), Content::Text(largest));
        let too_large = vec![b'a'; yaar_link::MAX_CONTENT as usize + 1];
        assert_eq!(
            content(Some(too_large)),
            Content::TooLarge {
                size: yaar_link::MAX_CONTENT + 1
            }
        );
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

        let waiting = window();
        assert!(state.admit(&waiting, b"kept"));
        assert!(Route::settle(&mut state).is_none());
        state.members[1].released = true;

        assert_eq!(Route::settle(&mut state), Some(Ending::Exited(0)));
        assert!(!state.admit(&late, b"kept"));
        assert!(!state.members.iter().any(|member| member.is(&late)));
    }
}
