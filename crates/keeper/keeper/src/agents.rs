//! The agents' side of the Keeper: the Adapters of the supported CLIs and their pieces,
//! the Switchboard, the sessions, turns and steps kept in the Store, the review each
//! session takes, the state recorded at each turn's end, and what the Windows ask of them:
//! the agents, a turn, the history of a worktree and the versions of a change.
//!
//! How it fits the rest of the Keeper. A report of a CLI's piece comes in as
//! `Request::Report`, with the moment it arrived, and is put on the queue of this side's
//! own thread before it is answered with `Reply::Done`; the marks the Dock finds in a
//! terminal, the ends of terminals and the processes found gone go on the same queue. That
//! thread works them out in the order they came, so that no hook of a CLI waits for a
//! recording, and so that a mark printed after a hook returned is always taken after that
//! hook's report. Every change of a session is kept in the Store before it is told, and
//! told to every Window under the Keeper's turn, so that a Window that asks for the
//! agents never takes an older list for a newer one.
//!
//! Three kinds of thread work here, each started once: the one that works the queue, the
//! one that looks every [`PID_WATCH`] for processes that are gone, and, for each workspace
//! where a turn ended, the one that records the state of the worktree, which takes up to a
//! second on a large repository and so holds up nothing else.
//!
//! The parts are reached from those threads through `Shared`, never held: a thread asks
//! for them each time it works an item, and takes the Keeper's turn before it does, as
//! every request that waits its turn does, so that the two never wait for each other.
//!
//! What a Window reads of agents, turns and the history of a worktree is answered in
//! `reading.rs`, from the Store, the state stores and Workspaces, through `Kept`: it holds
//! nothing of this side's thread.

mod reading;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_adapter_claude::ClaudeCode;
use yaar_agent::{Adapter, CliSession, Event, Phase, Report, Reported, ShellEntry, WriteTool};
use yaar_link::{
    AgentReport, Fact, Refusal, Reply, ReviewId, Session, SessionState, Step, TerminalId,
    WorkspaceAgents,
};
use yaar_store::{
    EndedBy, SessionId, SessionRecord, StepRecord, TurnEndRecord, TurnId, TurnRecord, WorkspaceId,
};
use yaar_switchboard::{Change, State, Switchboard};

use crate::watching::Watching;
use crate::wiring::{Kept, Outcome, Parts};
use crate::{Options, Shared, lock};

/// The reason given for a request this side does not answer yet.
pub(crate) const NOT_WIRED: &str = "the Keeper does not answer this yet";

/// How often the processes of the CLIs that run are looked at, to see that they are not
/// gone.
const PID_WATCH: Duration = Duration::from_millis(250);

/// What one Adapter is to the rest of the Keeper, as far as it never changes.
struct Described {
    name: &'static str,
    title: &'static str,
    command: &'static str,
    marks: Vec<Vec<u8>>,
}

/// What the threads of this side share.
struct Core {
    shared: Weak<Shared>,
    watching: Arc<Watching>,
    described: Vec<Described>,
    entries: Vec<yaar_dock::Entry>,
    // The terminals the Keeper holds, each with the workspace it was opened in: put in by
    // whoever opens a terminal before its program starts, taken out in queue order when its
    // end is worked out. A report from any other terminal is followed no further.
    terminals: Mutex<HashMap<String, WorkspaceId>>,
    // Called from the one thread that works the queue.
    adapters: Mutex<Vec<Box<dyn Adapter>>>,
    switchboard: Mutex<Switchboard>,
    // The thread of each workspace that records the states of its turns, begun at the
    // first turn that ends there, and the channel it is given turns on.
    recorders: Mutex<HashMap<WorkspaceId, Sender<Recording>>>,
    recorder_threads: Mutex<Vec<JoinHandle<()>>>,
    this: Weak<Core>,
}

/// What the thread that works the queue is given.
enum Work {
    /// A report of a CLI's piece, with the moment it reached the Keeper.
    Report { report: AgentReport, at: Instant },
    /// The Dock found mark `mark` of [`Agents::marks`] in the output of a terminal.
    Mark {
        terminal: TerminalId,
        mark: usize,
        at: Instant,
    },
    /// The program of a terminal ended.
    TerminalEnded(TerminalId),
    /// The watch of processes found that one is gone.
    Gone(u32),
    /// A review was marked reviewed.
    Reviewed(ReviewId),
    /// The agents' side ends.
    Stop,
}

/// A turn whose state is to be recorded.
struct Recording {
    workspace: WorkspaceId,
    session: SessionId,
    turn: TurnId,
    how: TurnEndRecord,
}

/// Why changes of the Switchboard happened.
#[derive(Clone, Copy)]
enum Cause {
    /// A CLI's report or a mark, which reached the Keeper at this moment: the end of the
    /// session it tells is the CLI's own.
    Told(Instant),
    /// The Keeper found out itself: a process is gone, a terminal ended, a review was
    /// marked reviewed.
    Own,
}

impl Cause {
    /// The moment to give Workspaces for a change: the one the report was stamped with,
    /// or now for a change the Keeper made itself.
    fn at(self) -> Instant {
        match self {
            Cause::Told(at) => at,
            Cause::Own => Instant::now(),
        }
    }
}

/// What one item of the queue left to tell, in the order it is told: the workspaces whose
/// sessions changed, the steps that were taken, and the turns whose states are to be
/// recorded.
#[derive(Default)]
struct Told {
    workspaces: Vec<WorkspaceId>,
    steps: Vec<Fact>,
    recordings: Vec<Recording>,
}

impl Told {
    fn changed(&mut self, workspace: &WorkspaceId) {
        if !self.workspaces.contains(workspace) {
            self.workspaces.push(workspace.clone());
        }
    }
}

/// The agents' side of the Keeper. Shared by the threads that answer Windows and by the
/// Dock's threads, which tell it the marks they found and the terminals that ended: every
/// call takes `&self`, and none waits for a recording.
pub(crate) struct Agents {
    core: Arc<Core>,
    queue: Sender<Work>,
    // Dropped to end the thread that watches the processes.
    stop_watching_pids: Option<Sender<()>>,
    // The thread that works the queue and the one that watches the processes.
    threads: Vec<JoinHandle<()>>,
}

impl Agents {
    /// Starts this side: every supported Adapter, each with its pieces written by
    /// `Adapter::install` into `<data folder>/agents/<its name>`, and `options.companions`
    /// as where the pieces find the companions; the Switchboard, told of every session the
    /// Store holds; and those the Store holds as running marked ended by
    /// `EndedBy::KeeperStopped`, since a Keeper that starts runs none and they were cut
    /// off. `kept` is none for a Keeper with no git or no database, which follows no agent.
    /// A workspace is watched, through `watching`, while a session runs in it.
    pub(crate) fn start(
        options: &Options,
        kept: Option<&Kept>,
        watching: Arc<Watching>,
        shared: Weak<Shared>,
    ) -> Agents {
        let adapters: Vec<Box<dyn Adapter>> = vec![Box::new(ClaudeCode::new())];
        let companions = options.companions.clone().unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_owned))
                .unwrap_or_default()
        });
        let mut entries = Vec::new();
        for adapter in &adapters {
            let folder = options.data_dir.join("agents").join(adapter.name());
            let installed = std::fs::create_dir_all(&folder)
                .and_then(|()| adapter.install(&folder, &companions));
            // A Keeper that cannot write the pieces still runs the terminals: the CLI is
            // then run as any program, and the Keeper says why in its own words.
            match installed {
                Ok(shell) => entries.extend(shell.into_iter().map(entry)),
                Err(e) => eprintln!(
                    "yaar-keeper: the pieces of {} could not be written into {}: {e}",
                    adapter.title(),
                    folder.display()
                ),
            }
        }
        let described = adapters
            .iter()
            .map(|adapter| Described {
                name: adapter.name(),
                title: adapter.title(),
                command: adapter.command(),
                marks: adapter.marks(),
            })
            .collect();

        let mut switchboard = Switchboard::new();
        if let Some(kept) = kept {
            remember_and_cut_off(kept, &mut switchboard);
        }

        let core = Arc::new_cyclic(|this| Core {
            shared,
            watching,
            described,
            entries,
            terminals: Mutex::new(HashMap::new()),
            adapters: Mutex::new(adapters),
            switchboard: Mutex::new(switchboard),
            recorders: Mutex::new(HashMap::new()),
            recorder_threads: Mutex::new(Vec::new()),
            this: this.clone(),
        });
        let (queue, work) = mpsc::channel();
        let (stop_watching_pids, stopped) = mpsc::channel();
        let worker = Arc::clone(&core);
        let watcher = Arc::clone(&core);
        let to_watch = queue.clone();
        let threads = vec![
            spawn("works the queue", move || worker.work_the(&work)),
            spawn("watches the processes", move || {
                watcher.watch_pids(&to_watch, &stopped);
            }),
        ];
        Agents {
            core,
            queue,
            stop_watching_pids: Some(stop_watching_pids),
            threads,
        }
    }

    /// What every shell of the app gets from the Adapters, in the Dock's words.
    pub(crate) fn entries(&self) -> Vec<yaar_dock::Entry> {
        self.core.entries.clone()
    }

    /// The runs of bytes every terminal is watched for: the marks of every Adapter, one
    /// after the other in the order of the Adapters. [`Agents::marked`] is told the index
    /// in this list.
    pub(crate) fn marks(&self) -> Vec<Vec<u8>> {
        self.core
            .described
            .iter()
            .flat_map(|adapter| adapter.marks.iter().cloned())
            .collect()
    }

    /// What the launcher of the CLI named `agent` runs in the user's shell; none for a
    /// name no Adapter has.
    pub(crate) fn launcher(&self, agent: &str) -> Option<&'static str> {
        self.core
            .described
            .iter()
            .find(|adapter| adapter.name == agent)
            .map(|adapter| adapter.command)
    }

    /// Takes one report of a CLI's piece: stamps the moment it arrived, puts it on the
    /// queue of this side's thread, and only then answers `Reply::Done`; refused only for
    /// an Adapter the Keeper does not have. Nothing else is done on the caller's thread: a
    /// report the Adapter cannot read is written down by that thread. A report from
    /// outside the app's terminals, or from a terminal the Keeper does not hold, is
    /// answered and followed no further.
    pub(crate) fn report(&self, kept: &Kept, report: AgentReport) -> Outcome {
        let _ = kept;
        if !self
            .core
            .described
            .iter()
            .any(|adapter| adapter.name == report.agent)
        {
            return Outcome::failed(format!("the app knows no agent CLI named {}", report.agent));
        }
        // Stamped before the answer: a command the CLI runs at once cannot write before
        // its window opens.
        self.push(Work::Report {
            report,
            at: Instant::now(),
        });
        Outcome {
            reply: Ok(Reply::Done),
            facts: Vec::new(),
            then: None,
        }
    }

    /// The Dock found mark `mark` of [`Agents::marks`] in the output of `terminal`: put on
    /// the same queue as the reports, and nothing else on the Dock's thread.
    pub(crate) fn marked(&self, terminal: &TerminalId, mark: usize) {
        self.push(Work::Mark {
            terminal: terminal.clone(),
            mark,
            at: Instant::now(),
        });
    }

    /// The Keeper opens `terminal` in `workspace`. Called before the program of the terminal
    /// starts, so that no report from it can come first: a report is followed only when its
    /// terminal is one the Keeper holds, and a session whose folder no worktree holds belongs
    /// to the workspace of its terminal.
    pub(crate) fn terminal_opened(
        &self,
        terminal: &TerminalId,
        workspace: &yaar_link::WorkspaceId,
    ) {
        lock(&self.core.terminals).insert(
            terminal.as_str().to_owned(),
            WorkspaceId::new(workspace.as_str()),
        );
    }

    /// The program of `terminal` ended: a session that ran in it runs no more.
    pub(crate) fn terminal_ended(&self, terminal: &TerminalId) {
        self.push(Work::TerminalEnded(terminal.clone()));
    }

    /// `Request::ListAgents`: the sessions as the Switchboard holds them.
    pub(crate) fn list(&self, kept: &Kept) -> Result<Reply, Refusal> {
        let workspaces = kept.store.workspaces().map_err(failed)?;
        let mut agents = Vec::new();
        for workspace in workspaces {
            if let Some(found) = self.core.workspace_agents(kept, &workspace.id)? {
                agents.push(found);
            }
        }
        Ok(Reply::Agents(agents))
    }

    /// The review `review` was marked reviewed: the session it belongs to, if any, has
    /// nothing unreviewed any more, and every Window is told.
    pub(crate) fn reviewed(&self, kept: &Kept, review: &ReviewId) {
        let _ = kept;
        self.push(Work::Reviewed(review.clone()));
    }

    fn push(&self, work: Work) {
        // The thread that takes it is gone only when the Keeper is.
        let _ = self.queue.send(work);
    }
}

impl Drop for Agents {
    fn drop(&mut self) {
        let _ = self.queue.send(Work::Stop);
        drop(self.stop_watching_pids.take());
        // The thread that works the queue first: only it begins recorders.
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        // A recorder ends when it is told nothing more; one that is recording is waited
        // for.
        lock(&self.core.recorders).clear();
        let recorders = std::mem::take(&mut *lock(&self.core.recorder_threads));
        for thread in recorders {
            let _ = thread.join();
        }
    }
}

impl Core {
    /// Works the queue, an item at a time, until told to stop or until there is nobody to
    /// give work.
    fn work_the(&self, work: &Receiver<Work>) {
        for item in work {
            if matches!(item, Work::Stop) {
                return;
            }
            self.take(item);
        }
    }

    /// Every [`PID_WATCH`], puts on the queue each process of a running session that is
    /// gone, until `stopped` is dropped.
    fn watch_pids(&self, queue: &Sender<Work>, stopped: &Receiver<()>) {
        while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(PID_WATCH) {
            let pids = lock(&self.switchboard).pids();
            for pid in pids.into_iter().filter(|pid| is_gone(*pid)) {
                if queue.send(Work::Gone(pid)).is_err() {
                    return;
                }
            }
        }
    }

    /// Works one item: what it changes is kept, handed to Workspaces and the engine, and
    /// told, all under the Keeper's turn.
    fn take(&self, work: Work) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        // The turn before the parts, as every request that waits its turn takes them.
        let _turn = lock(&shared.turn);
        let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
        let Some(kept) = parts.as_ref().and_then(Parts::kept) else {
            return;
        };
        let mut told = Told::default();
        match work {
            Work::Report { report, at } => self.report_taken(kept, &report, at, &mut told),
            Work::Mark { terminal, mark, at } => {
                self.mark_taken(kept, &terminal, mark, at, &mut told);
            }
            Work::TerminalEnded(terminal) => self.terminal_gone(kept, &terminal, &mut told),
            Work::Gone(pid) => {
                let changes = lock(&self.switchboard).gone(pid);
                self.apply(kept, changes, None, Cause::Own, &mut told);
            }
            Work::Reviewed(review) => self.review_marked(kept, &review, &mut told),
            Work::Stop => {}
        }
        let mut facts = Vec::new();
        for workspace in &told.workspaces {
            match self.workspace_agents(kept, workspace) {
                Ok(Some(agents)) => facts.push(Fact::AgentsChanged(agents)),
                Ok(None) => {}
                Err(e) => eprintln!("yaar-keeper: the agents of a workspace are unreadable: {e:?}"),
            }
        }
        facts.append(&mut told.steps);
        shared.tell(&facts);
        for recording in told.recordings {
            self.record_later(recording);
        }
    }

    fn report_taken(&self, kept: &Kept, report: &AgentReport, at: Instant, told: &mut Told) {
        let events = {
            let mut adapters = lock(&self.adapters);
            let Some(adapter) = adapters.iter_mut().find(|a| a.name() == report.agent) else {
                return;
            };
            let read = adapter.report(&Report {
                terminal: report.terminal.as_ref().map(TerminalId::as_str),
                pid: report.pid,
                payload: &report.payload,
            });
            match read {
                Ok(events) => events,
                // The piece was answered already, and the CLI goes on.
                Err(e) => {
                    eprintln!("yaar-keeper: {}: {e}", report.agent);
                    return;
                }
            }
        };
        self.follow(kept, &report.agent, events, Cause::Told(at), told);
    }

    fn mark_taken(
        &self,
        kept: &Kept,
        terminal: &TerminalId,
        mark: usize,
        at: Instant,
        told: &mut Told,
    ) {
        let mut first = 0;
        for described in &self.described {
            if mark < first + described.marks.len() {
                let events = {
                    let mut adapters = lock(&self.adapters);
                    let Some(adapter) = adapters.iter_mut().find(|a| a.name() == described.name)
                    else {
                        return;
                    };
                    adapter.marked(terminal.as_str(), mark - first)
                };
                self.follow(kept, described.name, events, Cause::Told(at), told);
                return;
            }
            first += described.marks.len();
        }
    }

    /// The program of `terminal` ended: every session that runs in it is gone with it.
    fn terminal_gone(&self, kept: &Kept, terminal: &TerminalId, told: &mut Told) {
        // Held no more: what its CLI still reports is followed no further.
        lock(&self.terminals).remove(terminal.as_str());
        let mut pids = Vec::new();
        for known in lock(&self.switchboard).sessions() {
            if known.state != State::NotRunning
                && known.terminal.as_deref() == Some(terminal.as_str())
                && let Some(pid) = known.pid
                && !pids.contains(&pid)
            {
                pids.push(pid);
            }
        }
        for pid in pids {
            let changes = lock(&self.switchboard).gone(pid);
            self.apply(kept, changes, None, Cause::Own, told);
        }
    }

    /// A review was marked reviewed: its session has nothing unreviewed any more.
    fn review_marked(&self, kept: &Kept, review: &ReviewId, told: &mut Told) {
        let review = yaar_store::ReviewId::new(review.as_str());
        let record = match kept.store.review(&review) {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(e) => {
                eprintln!("yaar-keeper: a review could not be read: {e}");
                return;
            }
        };
        let Some(session) = record.agent else {
            return;
        };
        let changes = lock(&self.switchboard).reviewed(&yaar_switchboard::SessionId(session));
        // Told whether or not the state changed: the Window that marked it waits for it.
        told.changed(&record.workspace);
        self.apply(kept, changes, None, Cause::Own, told);
    }

    /// Gives each event to the Switchboard, in the workspace its session belongs to.
    fn follow(
        &self,
        kept: &Kept,
        agent: &str,
        events: Vec<Reported>,
        cause: Cause,
        told: &mut Told,
    ) {
        for reported in events {
            // A CLI outside the app's terminals, or in one the Keeper does not hold, is
            // followed no further in this version.
            let Some(terminal) = reported.terminal.clone() else {
                continue;
            };
            let Some(of_terminal) = lock(&self.terminals).get(&terminal).cloned() else {
                continue;
            };
            let workspace = match kept.store.session_of_cli(agent, &reported.session.0) {
                Ok(Some(record)) => Some(record.workspace),
                Ok(None) => self.workspace_of(kept, reported.folder.as_deref(), of_terminal),
                Err(e) => {
                    eprintln!("yaar-keeper: a session could not be read: {e}");
                    continue;
                }
            };
            let Some(workspace) = workspace else {
                continue;
            };
            let event = reported.event.clone();
            let changes = lock(&self.switchboard).take(agent, reported, &mut random_id_text);
            self.apply(kept, changes, Some((&workspace, &event)), cause, told);
        }
    }

    /// The workspace a session that is new belongs to: the one whose worktree holds the
    /// folder its CLI works in, the deepest if several do; else `of_terminal`, the one its
    /// terminal was opened in.
    fn workspace_of(
        &self,
        kept: &Kept,
        folder: Option<&Path>,
        of_terminal: WorkspaceId,
    ) -> Option<WorkspaceId> {
        if let Some(folder) = folder {
            let records = kept.store.workspaces().ok()?;
            let folder = folder.canonicalize().unwrap_or_else(|_| folder.to_owned());
            let mut best: Option<(usize, WorkspaceId)> = None;
            for record in &records {
                let root = record
                    .path
                    .canonicalize()
                    .unwrap_or_else(|_| record.path.clone());
                let depth = root.components().count();
                if folder.starts_with(&root)
                    && best.as_ref().is_none_or(|(deepest, _)| depth > *deepest)
                    && matches!(kept.workspaces.worktree(&record.id), Ok(Some(_)))
                {
                    best = Some((depth, record.id.clone()));
                }
            }
            if let Some((_, workspace)) = best {
                return Some(workspace);
            }
        }
        Some(of_terminal)
    }

    /// Keeps each change in the Store, then gives it to Workspaces and the engine, and
    /// notes what is to be told. `starting` is the workspace a session that may be new
    /// starts in, and the event that led to the changes; the workspace of any other
    /// session is the one the Store keeps it in.
    fn apply(
        &self,
        kept: &Kept,
        changes: Vec<Change>,
        starting: Option<(&WorkspaceId, &Event)>,
        cause: Cause,
        told: &mut Told,
    ) {
        for change in changes {
            let session = session_of(&change);
            let Some(workspace) = self.workspace_for(kept, &session, starting.map(|s| s.0)) else {
                continue;
            };
            // What a step or the end of a shell command changes is no part of what a Window
            // is shown of the sessions: told for every step of a long turn, the list would
            // be read again and again for nothing.
            if !matches!(change, Change::Step { .. } | Change::CommandEnded { .. }) {
                told.changed(&workspace);
            }
            match change {
                Change::Started {
                    session,
                    agent,
                    cli,
                    terminal,
                    ..
                } => self.session_starts(kept, &workspace, &session, &agent, &cli, terminal),
                Change::State { .. } => {}
                Change::TurnStarted { session, turn } => {
                    let number = turns_of_session(kept, &workspace, &session) + 1;
                    keep(kept.store.put_turn(&TurnRecord {
                        id: TurnId::new(turn.0),
                        session: SessionId::new(session.0),
                        number,
                        ended: None,
                        state: None,
                    }));
                }
                Change::Step {
                    session,
                    turn,
                    number,
                    event,
                } => self.step_taken(
                    kept, &workspace, &session, &turn, number, &event, cause, told,
                ),
                Change::CommandEnded { session } => {
                    // The end the CLI reported is stamped with its report; one the
                    // Switchboard made for a turn or a session that ended is made now.
                    let reported = matches!(
                        starting.map(|(_, event)| event),
                        Some(Event::Command {
                            phase: Phase::Ended,
                            ..
                        })
                    );
                    let at = if reported { cause.at() } else { Instant::now() };
                    kept.workspaces.command_ended(&workspace, &session.0, at);
                }
                Change::TurnEnded { session, turn, how } => {
                    let how = match how {
                        yaar_agent::TurnEnd::Finished => TurnEndRecord::Finished,
                        yaar_agent::TurnEnd::Interrupted => TurnEndRecord::Interrupted,
                    };
                    // Its end is kept now, and its state when it is recorded: a Keeper
                    // that stops between knows the turn ended.
                    let turn = TurnId::new(turn.0);
                    keep(kept.store.end_turn(&turn, how, None));
                    told.recordings.push(Recording {
                        workspace: workspace.clone(),
                        session: SessionId::new(session.0),
                        turn,
                        how,
                    });
                }
                Change::Ended { session } => {
                    let how = match cause {
                        Cause::Told(_) => EndedBy::Cli,
                        Cause::Own => EndedBy::ProcessGone,
                    };
                    keep(kept.store.set_ended(&SessionId::new(session.0), how));
                    if !runs_in(kept, &workspace) {
                        self.watching.agent_runs(
                            kept,
                            &yaar_link::WorkspaceId::new(workspace.as_str()),
                            false,
                        );
                    }
                }
            }
        }
    }

    /// The workspace of a session: the one the Store keeps it in, or the one it is
    /// starting in.
    fn workspace_for(
        &self,
        kept: &Kept,
        session: &yaar_switchboard::SessionId,
        starting: Option<&WorkspaceId>,
    ) -> Option<WorkspaceId> {
        match kept.store.session(&SessionId::new(session.0.clone())) {
            Ok(Some(record)) => Some(record.workspace),
            Ok(None) => starting.cloned(),
            Err(e) => {
                eprintln!("yaar-keeper: a session could not be read: {e}");
                None
            }
        }
    }

    /// A session starts, or its CLI resumes it: it is kept as running, takes its review,
    /// and its worktree is watched while it runs.
    fn session_starts(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        session: &yaar_switchboard::SessionId,
        agent: &str,
        cli: &CliSession,
        terminal: Option<String>,
    ) {
        let id = SessionId::new(session.0.clone());
        // A session the Store holds is one its CLI resumed.
        if matches!(kept.store.session(&id), Ok(Some(_))) {
            keep(kept.store.set_running(&id, terminal.as_deref()));
        } else {
            let same_cli = kept
                .store
                .sessions_of(workspace)
                .map_or(0, |all| all.iter().filter(|s| s.agent == agent).count());
            keep(
                kept.store.put_session(&SessionRecord {
                    id: id.clone(),
                    workspace: workspace.clone(),
                    agent: agent.to_owned(),
                    cli_session: cli.0.clone(),
                    terminal,
                    number: u32::try_from(same_cli)
                        .unwrap_or(u32::MAX)
                        .saturating_add(1),
                    running: true,
                    ended: None,
                    ended_by: None,
                }),
            );
        }
        match kept.store.session(&id) {
            Ok(Some(record)) => keep(kept.reviews.session_started(&record).map(|_| ())),
            Ok(None) => {}
            Err(e) => eprintln!("yaar-keeper: a session could not be read: {e}"),
        }
        self.watching
            .agent_runs(kept, &yaar_link::WorkspaceId::new(workspace.as_str()), true);
    }

    /// The agent took a step in a turn: it is kept, then handed to Workspaces when it names
    /// a write or begins a command, and told.
    #[expect(clippy::too_many_arguments, reason = "what a Step change carries")]
    fn step_taken(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        session: &yaar_switchboard::SessionId,
        turn: &yaar_switchboard::TurnId,
        number: u32,
        event: &Event,
        cause: Cause,
        told: &mut Told,
    ) {
        let root = kept
            .workspaces
            .worktree(workspace)
            .ok()
            .flatten()
            .map(|worktree| worktree.root().to_owned());
        let inside = |path: &Path| match &root {
            Some(root) => relative_to(root, path),
            None => path.to_owned(),
        };
        // What Workspaces is told of, once the step is kept.
        enum Feed<'a> {
            Nothing,
            Wrote(&'a Path),
            Began,
        }
        let (record, feed) = match event {
            Event::FileRead { path } => (StepRecord::Read(inside(path)), Feed::Nothing),
            Event::FileWritten {
                path,
                tool: WriteTool::Edit,
                lines,
            } => (
                StepRecord::Edit(inside(path), lines.clone()),
                Feed::Wrote(path),
            ),
            Event::FileWritten {
                path,
                tool: WriteTool::Write,
                ..
            } => (StepRecord::Write(inside(path)), Feed::Wrote(path)),
            Event::Command {
                phase: Phase::Began,
                command,
            } => (StepRecord::Command(command.clone()), Feed::Began),
            // The Switchboard tells no other event as a step.
            _ => return,
        };
        keep(
            kept.store
                .put_step(&TurnId::new(turn.0.clone()), number, &record),
        );
        match feed {
            Feed::Nothing => {}
            Feed::Wrote(path) => kept.workspaces.agent_wrote(workspace, &session.0, path),
            Feed::Began => kept
                .workspaces
                .command_began(workspace, &session.0, cause.at()),
        }
        told.steps.push(Fact::Step {
            session: yaar_link::SessionId::new(session.0.clone()),
            turn: yaar_link::TurnId::new(turn.0.clone()),
            number,
            step: step_of(&record),
        });
    }

    /// The sessions of one workspace as a Window is shown them: those that run and those
    /// that ended and still hold a review, in the order they started. None for a
    /// workspace where no session ever reported.
    fn workspace_agents(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
    ) -> Result<Option<WorkspaceAgents>, Refusal> {
        let records = kept.store.sessions_of(workspace).map_err(failed)?;
        if records.is_empty() {
            return Ok(None);
        }
        let reviews = kept.store.reviews_of(workspace).map_err(failed)?;
        let board = lock(&self.switchboard);
        let mut shown = Vec::new();
        let mut sessions = Vec::new();
        for record in records {
            let known = board.session(&yaar_switchboard::SessionId(record.id.as_str().to_owned()));
            let state = known
                .as_ref()
                .map_or(State::NotRunning, |known| known.state);
            let review = reviews
                .iter()
                .find(|review| review.agent.as_deref() == Some(record.id.as_str()))
                .map(|review| ReviewId::new(review.id.as_str()));
            if state == State::NotRunning && review.is_none() {
                continue;
            }
            let title = self
                .described
                .iter()
                .find(|adapter| adapter.name == record.agent)
                .map_or(record.agent.as_str(), |adapter| adapter.title);
            shown.push(state);
            sessions.push(Session {
                id: yaar_link::SessionId::new(record.id.as_str()),
                agent: record.agent.clone(),
                title: format!("{title} {}", record.number),
                workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
                terminal: known
                    .as_ref()
                    .filter(|_| state != State::NotRunning)
                    .and_then(|known| known.terminal.as_deref())
                    .map(TerminalId::new),
                state: state_of(state),
                turn: known
                    .and_then(|known| known.turn)
                    .map(|(turn, _)| yaar_link::TurnId::new(turn.0)),
                review,
            });
        }
        Ok(Some(WorkspaceAgents {
            workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
            shown: yaar_switchboard::shown(shown).map(state_of),
            sessions,
        }))
    }

    /// Gives a turn that ended to the thread of its workspace that records states, begun
    /// when the first turn ends there.
    fn record_later(&self, recording: Recording) {
        let mut recorders = lock(&self.recorders);
        let sender = recorders
            .entry(recording.workspace.clone())
            .or_insert_with(|| {
                let (sender, received) = mpsc::channel::<Recording>();
                if let Some(core) = self.this.upgrade() {
                    let thread = spawn("records the states of turns", move || {
                        for recording in received {
                            core.record(&recording);
                        }
                    });
                    lock(&self.recorder_threads).push(thread);
                }
                sender
            });
        // A thread that is gone is the Keeper ending.
        let _ = sender.send(recording);
    }

    /// Records the state of the worktree at the end of a turn, then keeps it with the turn
    /// and tells every Window. The recording is taken with the parts held and no turn,
    /// since it can take a second; the telling is taken under the Keeper's turn.
    fn record(&self, recording: &Recording) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let (state, since) = {
            let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
            let Some(kept) = parts.as_ref().and_then(Parts::kept) else {
                return;
            };
            record_state(kept, recording)
        };
        let _turn = lock(&shared.turn);
        let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
        let Some(kept) = parts.as_ref().and_then(Parts::kept) else {
            return;
        };
        if state.is_some() {
            keep(
                kept.store
                    .end_turn(&recording.turn, recording.how, state.as_deref()),
            );
        }
        shared.tell(&[Fact::TurnEnded {
            workspace: yaar_link::WorkspaceId::new(recording.workspace.as_str()),
            session: yaar_link::SessionId::new(recording.session.as_str()),
            turn: yaar_link::TurnId::new(recording.turn.as_str()),
            state,
            since,
        }]);
    }
}

/// Records the state of the worktree of a turn that ended, and says which state the
/// session's review last marked reviewed. Both none when the recording failed.
fn record_state(kept: &Kept, recording: &Recording) -> (Option<String>, Option<String>) {
    let worktree = match kept.workspaces.worktree(&recording.workspace) {
        Ok(Some(worktree)) => worktree,
        Ok(None) => {
            eprintln!("yaar-keeper: no state was recorded: the worktree is gone");
            return (None, None);
        }
        Err(e) => {
            eprintln!("yaar-keeper: no state was recorded: {e}");
            return (None, None);
        }
    };
    let start = kept
        .workspaces
        .counted_from(&recording.workspace)
        .ok()
        .and_then(|(start, _)| start);
    match kept
        .reviews
        .turn_ended(&worktree, &recording.session, start.as_ref())
    {
        Ok(ended) => (Some(ended.state.as_str().to_owned()), ended.since),
        Err(e) => {
            eprintln!("yaar-keeper: no state was recorded: {e}");
            (None, None)
        }
    }
}

/// Tells the Switchboard of every session the Store holds, and marks those the Store holds
/// as running ended by `KeeperStopped`: a Keeper that starts runs none.
fn remember_and_cut_off(kept: &Kept, switchboard: &mut Switchboard) {
    match kept.store.workspaces() {
        Ok(workspaces) => {
            for workspace in workspaces {
                for record in kept.store.sessions_of(&workspace.id).unwrap_or_default() {
                    switchboard.remember(
                        yaar_switchboard::SessionId(record.id.as_str().to_owned()),
                        &record.agent,
                        CliSession(record.cli_session),
                    );
                }
            }
        }
        Err(e) => eprintln!("yaar-keeper: the sessions could not be read: {e}"),
    }
    match kept.store.running_sessions() {
        Ok(running) => {
            for record in running {
                keep(kept.store.set_ended(&record.id, EndedBy::KeeperStopped));
            }
        }
        Err(e) => eprintln!("yaar-keeper: the sessions could not be read: {e}"),
    }
}

/// Whether any session of `workspace` runs.
fn runs_in(kept: &Kept, workspace: &WorkspaceId) -> bool {
    kept.store
        .sessions_of(workspace)
        .is_ok_and(|all| all.iter().any(|session| session.running))
}

/// How many turns the session has had.
fn turns_of_session(
    kept: &Kept,
    workspace: &WorkspaceId,
    session: &yaar_switchboard::SessionId,
) -> u32 {
    let count = kept.store.turns_of(workspace).map_or(0, |turns| {
        turns
            .iter()
            .filter(|turn| turn.session.as_str() == session.0)
            .count()
    });
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The session a change is about.
fn session_of(change: &Change) -> yaar_switchboard::SessionId {
    match change {
        Change::Started { session, .. }
        | Change::State { session, .. }
        | Change::TurnStarted { session, .. }
        | Change::Step { session, .. }
        | Change::CommandEnded { session }
        | Change::TurnEnded { session, .. }
        | Change::Ended { session } => session.clone(),
    }
}

/// A shell entry of an Adapter in the Dock's words.
fn entry(shell: ShellEntry) -> yaar_dock::Entry {
    match shell {
        ShellEntry::PathList { name, path } => yaar_dock::Entry::PathList { name, path },
        ShellEntry::SearchFirst(folder) => yaar_dock::Entry::SearchFirst(folder),
    }
}

/// A state of the Switchboard in the Link's words.
fn state_of(state: State) -> SessionState {
    match state {
        State::WaitsForPermission => SessionState::WaitsForPermission,
        State::Unreviewed => SessionState::Unreviewed,
        State::Works => SessionState::Works,
        State::NotRunning => SessionState::NotRunning,
        State::Idle => SessionState::Idle,
    }
}

/// A step as the Store keeps it, in the Link's words.
fn step_of(record: &StepRecord) -> Step {
    match record {
        StepRecord::Read(path) => Step::Read { path: path.clone() },
        StepRecord::Edit(path, lines) => Step::Edit {
            path: path.clone(),
            lines: lines.clone(),
        },
        StepRecord::Write(path) => Step::Write { path: path.clone() },
        StepRecord::Command(command) => Step::Command {
            command: command.clone(),
        },
    }
}

/// `path` relative to `root` when it lies inside, as the CLI named it or once its links
/// are resolved; as it was given otherwise.
fn relative_to(root: &Path, path: &Path) -> PathBuf {
    if let Ok(inside) = path.strip_prefix(root) {
        return inside.to_owned();
    }
    // The CLI may name a folder through a link, as `/tmp` on macOS.
    let resolved = path.canonicalize().ok().or_else(|| {
        let name = path.file_name()?;
        Some(path.parent()?.canonicalize().ok()?.join(name))
    });
    match resolved.as_deref().map(|path| path.strip_prefix(root)) {
        Some(Ok(inside)) => inside.to_owned(),
        _ => path.to_owned(),
    }
}

/// Whether the process `pid` is gone: the system knows no such process. A process we may
/// not signal is not gone.
fn is_gone(pid: u32) -> bool {
    // Pid 0 and the negative ones name groups: not a process of a CLI.
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 sends nothing; it only asks whether the process exists.
    let found = unsafe { libc::kill(pid, 0) };
    found != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// A new id for a session or a turn: 32 lowercase hexadecimal digits.
fn random_id_text() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut from| from.read_exact(&mut bytes));
    if read.is_err() {
        // No random bytes: the clock and a counter never repeat within one Keeper.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        bytes[..8].copy_from_slice(&(now as u64).to_le_bytes());
        bytes[8..].copy_from_slice(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Starts a thread of this side.
fn spawn(what: &str, body: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("yaar-agents {what}"))
        .spawn(body)
        .expect("the system gives a thread")
}

/// Writes down a failure of the Store or of the engine that no Window can be told: the
/// work goes on, as the CLI does.
fn keep<E: std::fmt::Display>(result: Result<(), E>) {
    if let Err(e) = result {
        eprintln!("yaar-keeper: {e}");
    }
}

/// The refusal of a read that failed.
fn failed(error: impl std::fmt::Display) -> Refusal {
    Refusal::Failed {
        reason: error.to_string(),
    }
}

/// The refusal of a request this side does not answer yet.
pub(crate) fn not_wired() -> Refusal {
    Refusal::Failed {
        reason: NOT_WIRED.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_inside_the_worktree_is_given_relative_to_it() {
        let root = Path::new("/work/tree");
        assert_eq!(
            relative_to(root, Path::new("/work/tree/src/lib.rs")),
            Path::new("src/lib.rs")
        );
    }

    #[test]
    fn a_path_outside_the_worktree_is_given_as_it_was_named() {
        let root = Path::new("/work/tree");
        assert_eq!(
            relative_to(root, Path::new("/elsewhere/new.txt")),
            Path::new("/elsewhere/new.txt")
        );
        // A neighbour whose name starts with the worktree's is not inside it.
        assert_eq!(
            relative_to(root, Path::new("/work/tree2/a.txt")),
            Path::new("/work/tree2/a.txt")
        );
    }

    #[test]
    fn a_process_that_runs_is_not_gone_and_one_that_never_was_is() {
        assert!(!is_gone(std::process::id()));
        assert!(!is_gone(0));
        // Above any process number a system gives out.
        assert!(is_gone(0x7fff_ffff));
    }
}
