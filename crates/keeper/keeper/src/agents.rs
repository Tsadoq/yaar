//! The agents' side of the Keeper: the Adapters of the supported CLIs and their pieces,
//! the Switchboard, the sessions, turns and steps kept in the Store, the review each
//! session takes, the state recorded at each turn's end, and what the Windows ask of them:
//! the agents, a turn, the history of a worktree and the versions of a change.
//!
//! How it fits the rest of the Keeper. A report of a CLI's piece comes in as
//! `Request::Report`, with the moment it arrived, and is put on the queue of this side's
//! own thread with a channel for its answer; the connection's thread waits on that channel,
//! holding nothing of the Keeper's, and answers the piece with what comes: `Reply::Answer`
//! where the report waited for something of the app's (a review that rides with the user's
//! message, the allowing of a read or of the reply tool, a reply kept), else `Reply::Done`.
//! The marks the Dock finds in a terminal, the ends of terminals and the processes found
//! gone go on the same queue. That thread works them out in the order they came, so that
//! no hook of a CLI waits for a recording, and so that a mark printed after a hook returned
//! is always taken after that hook's report. Every change of a session is kept in the
//! Store before it is told, and told to every Window under the Keeper's turn, so that a
//! Window that asks for the agents never takes an older list for a newer one.
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
//! nothing of this side's thread. What a report waits for, and the app's answer to it, is
//! worked out in `delivery.rs`: the review that rides with a message, the allows, the
//! agent's replies. What the Adapters that write into a CLI's own settings only with the
//! user's leave ask, and the user's answer, are handed on in `consents.rs`.

mod consents;
mod delivery;
mod reading;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_agent::{
    Adapter, Asked, Asks, CliSession, Event, Phase, Report, Reported, ShellEntry, WriteTool,
};
use yaar_link::{
    AgentReport, Fact, Refusal, Reply, ReviewId, Session, SessionState, Step, TerminalId,
    WorkspaceAgents,
};
use yaar_store::{
    EndedBy, SessionId, SessionRecord, StepRecord, TurnEndRecord, TurnId, TurnRecord, WorkspaceId,
};
use yaar_switchboard::{Change, Resumable, State, Switchboard};

use crate::watching::Watching;
use crate::wiring::{Answered, Kept, Outcome, Parts};
use crate::{Options, Shared, lock};

/// How often the processes of the CLIs that run are looked at, to see that they are not
/// gone.
const PID_WATCH: Duration = Duration::from_millis(250);

/// The longest the connection of a report waits for this side's answer: as long as the
/// piece that waits longest, the reply tool's server, waits for it. The hook program gives
/// up at 2 s; its connection is then answered into a socket nobody reads, which costs
/// nothing.
const REPORT_WAIT: Duration = Duration::from_secs(10);

/// What one Adapter is to the rest of the Keeper, as far as it never changes.
struct Described {
    name: &'static str,
    title: &'static str,
    command: &'static str,
    marks: Vec<Vec<u8>>,
    instruction: &'static str,
    // The Adapter's own folder of the app, where its pieces are written and its delivery
    // writes the reviews too long to ride inline.
    folder: PathBuf,
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
    // The review each terminal the app opened for a CLI was opened for, until the first
    // session that starts there takes it (or the terminal ends).
    opened_for: Mutex<HashMap<String, yaar_store::ReviewId>>,
    // Per session, whether the turn it has now began with a message of the user. Kept in
    // memory: a Keeper that starts has no turn running.
    turns_from_user: Mutex<HashMap<String, bool>>,
    // Called from the one thread that works the queue. Never held together with the
    // Switchboard's lock, in either order.
    adapters: Mutex<Vec<Box<dyn Adapter>>>,
    switchboard: Mutex<Switchboard>,
    // The sessions a terminal of the user's resume is open for, until that terminal ends.
    // Held while the offers are made, so taken before the Adapters and the Switchboard.
    resuming: Mutex<Vec<Hold>>,
    // The thread of each workspace that records the states of its turns and of its rounds,
    // begun at the first that is to be recorded there, and the channel it is given them on.
    recorders: Mutex<HashMap<WorkspaceId, Sender<ToRecord>>>,
    recorder_threads: Mutex<Vec<JoinHandle<()>>>,
    // Per workspace, which recordings of turn ends are in flight and which turns wait for
    // one to be kept before they have the state their change counts from. Read and written
    // only under the Keeper's turn, so that a turn's start and the end of a recording are
    // never taken in the same moment.
    starts: Mutex<HashMap<WorkspaceId, Starts>>,
    this: Weak<Core>,
}

/// A session a resume terminal was opened for.
struct Hold {
    /// The app's id of the session.
    session: String,
    /// The terminal the resume runs in, whose end frees the session.
    terminal: String,
}

/// The recordings of a workspace's turn ends and the turns that wait for them.
#[derive(Default)]
struct Starts {
    /// How many recordings were given out: the last one's number.
    issued: u64,
    /// The number of the last recording that is kept or failed. The thread of a workspace
    /// records in the order the turns ended, so every recording below it is done as well.
    done: u64,
    /// The turns that started while a recording was in flight, each with the number of the
    /// last one in flight then: the state of that recording is the state the worktree was
    /// in when the turn before left it.
    waiting: Vec<(TurnId, u64)>,
}

/// What the thread that works the queue is given.
enum Work {
    /// A report of a CLI's piece, with the moment it reached the Keeper, and where its
    /// answer goes: the connection of the piece waits for it.
    Report {
        report: AgentReport,
        at: Instant,
        answer: Sender<Reply>,
    },
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
    /// What is attached of a review changed.
    AttachmentChanged(ReviewId),
    /// The agents' side ends.
    Stop,
}

/// What the thread of a workspace that records states is given.
enum ToRecord {
    /// A turn that ended.
    TurnEnd(Recording),
    /// A round delivered with a message, kept with no state: the state of the worktree is
    /// recorded for it, off the agent's way.
    Round(RoundRecording),
}

/// A round whose state is to be recorded.
struct RoundRecording {
    workspace: WorkspaceId,
    review: yaar_store::ReviewId,
    round: u32,
}

/// A turn whose state is to be recorded.
struct Recording {
    /// Its place among the recordings of its workspace, from 1 in the order the turns
    /// ended.
    number: u64,
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

/// What a CLI's report was, as far as the changes it led to depend on it.
#[derive(Clone, Copy)]
struct Starting<'a> {
    /// The workspace a session that may be new starts in.
    workspace: &'a WorkspaceId,
    /// The event that led to the changes.
    event: &'a Event,
    /// Whether the report waited for a message of the user: a turn it starts came from the
    /// user.
    from_user: bool,
}

/// What a session that starts is, as its first event tells it.
struct Beginning<'a> {
    /// The name of its CLI's Adapter.
    agent: &'a str,
    /// The CLI's own name for it.
    cli: &'a CliSession,
    /// The terminal of the app it runs in, none outside the ones the Keeper holds.
    terminal: Option<String>,
}

/// What one item of the queue left to tell, in the order it is told: the workspaces whose
/// sessions changed, those whose reviews changed, the steps that were taken; and the turns
/// and the rounds whose states are to be recorded.
#[derive(Default)]
struct Told {
    workspaces: Vec<WorkspaceId>,
    reviews: Vec<WorkspaceId>,
    steps: Vec<Fact>,
    recordings: Vec<Recording>,
    rounds: Vec<RoundRecording>,
}

impl Told {
    fn changed(&mut self, workspace: &WorkspaceId) {
        if !self.workspaces.contains(workspace) {
            self.workspaces.push(workspace.clone());
        }
    }

    /// A review of `workspace` changed: `Fact::ReviewsChanged` is told for it.
    fn reviews_changed(&mut self, workspace: &WorkspaceId) {
        if !self.reviews.contains(workspace) {
            self.reviews.push(workspace.clone());
        }
    }

    /// Round `round` of `review`, of `workspace`, was kept with no state: its state is
    /// recorded on the thread of the workspace that records states, after this item is
    /// told, and `Fact::ReviewsChanged` is told again once it is kept.
    fn round_to_record(
        &mut self,
        workspace: &WorkspaceId,
        review: &yaar_store::ReviewId,
        round: u32,
    ) {
        self.rounds.push(RoundRecording {
            workspace: workspace.clone(),
            review: review.clone(),
            round,
        });
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
        let adapters: Vec<Box<dyn Adapter>> = crate::adapters(&options.home);
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
                instruction: adapter.instruction(),
                folder: options.data_dir.join("agents").join(adapter.name()),
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
            opened_for: Mutex::new(HashMap::new()),
            turns_from_user: Mutex::new(HashMap::new()),
            adapters: Mutex::new(adapters),
            switchboard: Mutex::new(switchboard),
            resuming: Mutex::new(Vec::new()),
            recorders: Mutex::new(HashMap::new()),
            recorder_threads: Mutex::new(Vec::new()),
            starts: Mutex::new(HashMap::new()),
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

    /// Takes one report of a CLI's piece: stamps the moment it arrived and puts it on the
    /// queue of this side's thread with a channel for its answer; refused only for an
    /// Adapter the Keeper does not have. Gives the work that waits for the answer, which the
    /// connection's thread runs once it let the Keeper's parts go: it waits up to
    /// [`REPORT_WAIT`] and answers what this side's thread sent, `Reply::Done` when nothing
    /// came, as when the Keeper stops first. Nothing else is done on the caller's thread: a
    /// report the Adapter cannot read is written down by this side's thread and answered
    /// `Reply::Done`.
    pub(crate) fn report(&self, kept: &Kept, report: AgentReport) -> Answered {
        let _ = kept;
        if !self
            .core
            .described
            .iter()
            .any(|adapter| adapter.name == report.agent)
        {
            return Answered::Now(Outcome::failed(format!(
                "the app knows no agent CLI named {}",
                report.agent
            )));
        }
        let (answer, answered) = mpsc::channel();
        // Stamped before the answer: a command the CLI runs at once cannot write before
        // its window opens.
        self.push(Work::Report {
            report,
            at: Instant::now(),
            answer,
        });
        // Waited for with no lock and no part held: this side's thread takes the Keeper's
        // turn and the parts to work the report out.
        Answered::Later(Box::new(move || Outcome {
            reply: Ok(answered.recv_timeout(REPORT_WAIT).unwrap_or(Reply::Done)),
            facts: Vec::new(),
            then: None,
        }))
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
    /// to the workspace of its terminal. A terminal that resumes a session has nothing more
    /// to be told: [`Agents::resume_command`] bound its hold to the terminal's id.
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

    /// The Keeper opened `terminal` for `review`, the launcher of a CLI offered when the
    /// review was attached ([`yaar_link::Program::AgentFor`]): the first session that
    /// starts in it takes that review (`Reviews::session_started_for`), and its first
    /// message carries what is attached. Called right after [`Agents::terminal_opened`].
    pub(crate) fn terminal_opened_for(&self, terminal: &TerminalId, review: &ReviewId) {
        lock(&self.core.opened_for).insert(
            terminal.as_str().to_owned(),
            yaar_store::ReviewId::new(review.as_str()),
        );
    }

    /// What a terminal runs in the user's shell to bring back `session`: the `command` of
    /// the session's offer in `yaar_switchboard::Switchboard::resumable`, the one place the
    /// command is made, from the Adapter's `Adapter::resume` with the CLI's own id and the
    /// CLI's own record of the session that the Keeper's start handed the Switchboard.
    /// Refused with [`Refusal::Failed`] for a session that has no offer (the Switchboard
    /// does not hold it, it runs, its CLI ended it, it reported since the Keeper started, or
    /// no Adapter has its CLI) and for one a resume terminal is open for, until that
    /// terminal ends or the session reports.
    ///
    /// The session is held for `terminal`, the one the caller made for the resume and is
    /// about to open: a refusal leaves no hold, and the end of that terminal, told as ended
    /// also when it cannot start, frees the session.
    pub(crate) fn resume_command(
        &self,
        session: &yaar_link::SessionId,
        terminal: &yaar_link::TerminalId,
    ) -> Result<String, Refusal> {
        let mut holds = lock(&self.core.resuming);
        let offer = self
            .core
            .offers()
            .into_iter()
            .find(|offer| offer.session.0 == session.as_str())
            .ok_or_else(|| Refusal::Failed {
                reason: format!(
                    "session {session} is not one to bring back: the app holds none by that id, \
                     it runs, or it ended by its CLI's word"
                ),
            })?;
        if holds.iter().any(|hold| hold.session == session.as_str()) {
            return Err(Refusal::Failed {
                reason: format!("a terminal is bringing session {session} back already"),
            });
        }
        holds.push(Hold {
            session: session.as_str().to_owned(),
            terminal: terminal.as_str().to_owned(),
        });
        Ok(offer.command)
    }

    /// The instruction on how to answer each thread of the CLI named `agent`, its
    /// delivery module's; none for a name no Adapter has.
    pub(crate) fn instruction(&self, agent: &str) -> Option<&'static str> {
        self.core
            .described
            .iter()
            .find(|adapter| adapter.name == agent)
            .map(|adapter| adapter.instruction)
    }

    /// Every agent CLI the Keeper has an Adapter for, in the order of the Adapters: what
    /// attaching the review of no agent offers to start.
    pub(crate) fn supported(&self) -> Vec<yaar_link::AgentCli> {
        self.core
            .described
            .iter()
            .map(|adapter| yaar_link::AgentCli {
                agent: adapter.name.to_owned(),
                title: adapter.title.to_owned(),
            })
            .collect()
    }

    /// The sessions of `workspace` as the Keeper knows them now, whether each runs and
    /// waits at a permission prompt: what attaching a review needs to say what to offer.
    /// Every session the Store holds there, those that ended among them; whether one runs
    /// is the Switchboard's. Empty, and written down, when the Store cannot be read.
    pub(crate) fn sessions_now(
        &self,
        kept: &Kept,
        workspace: &yaar_link::WorkspaceId,
    ) -> Vec<yaar_review_engine::SessionNow> {
        let records = match kept
            .store
            .sessions_of(&WorkspaceId::new(workspace.as_str()))
        {
            Ok(records) => records,
            Err(e) => {
                eprintln!("yaar-keeper: the sessions of {workspace} could not be read: {e}");
                return Vec::new();
            }
        };
        let board = lock(&self.core.switchboard);
        records
            .into_iter()
            .map(|record| {
                let state = board
                    .session(&yaar_switchboard::SessionId(record.id.as_str().to_owned()))
                    .map_or(State::NotRunning, |known| known.state);
                yaar_review_engine::SessionNow {
                    id: record.id,
                    cli: record.agent,
                    running: state != State::NotRunning,
                    waits_for_permission: state == State::WaitsForPermission,
                }
            })
            .collect()
    }

    /// What is attached of `review` changed: the sessions of its workspace are told again
    /// as [`Fact::AgentsChanged`], from this side's thread under the Keeper's turn, so that
    /// the agent's terminal shows the attachment. Pushed on the queue; nothing else is done
    /// on the caller's thread.
    pub(crate) fn attachment_changed(&self, review: &ReviewId) {
        self.push(Work::AttachmentChanged(review.clone()));
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
            Work::Report { report, at, answer } => {
                let reply = self.report_taken(kept, &report, at, &mut told);
                // The piece is answered at once; what the report changed is told after.
                let _ = answer.send(reply);
            }
            Work::Mark { terminal, mark, at } => {
                self.mark_taken(kept, &terminal, mark, at, &mut told);
            }
            Work::TerminalEnded(terminal) => self.terminal_gone(kept, &terminal, &mut told),
            Work::Gone(pid) => {
                let changes = lock(&self.switchboard).gone(pid);
                self.apply(kept, changes, None, Cause::Own, &mut told);
            }
            Work::Reviewed(review) => self.review_marked(kept, &review, &mut told),
            Work::AttachmentChanged(review) => self.attachment_told(kept, &review, &mut told),
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
        facts.extend(told.reviews.iter().map(|workspace| Fact::ReviewsChanged {
            workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
        }));
        facts.append(&mut told.steps);
        shared.tell(&facts);
        for recording in told.recordings {
            self.record_later(recording.workspace.clone(), ToRecord::TurnEnd(recording));
        }
        for round in told.rounds {
            self.record_later(round.workspace.clone(), ToRecord::Round(round));
        }
    }

    /// Works out one report: its events, given to the Switchboard, and what it waits for,
    /// answered by `delivery.rs`. Gives the answer for the piece: `Reply::Done` for a
    /// report that waits for nothing, and for one the Adapter cannot read, which is written
    /// down.
    fn report_taken(
        &self,
        kept: &Kept,
        report: &AgentReport,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        let (events, asked) = {
            let mut adapters = lock(&self.adapters);
            let Some(adapter) = adapters.iter_mut().find(|a| a.name() == report.agent) else {
                return Reply::Done;
            };
            let Some(folder) = self.folder_of(&report.agent) else {
                return Reply::Done;
            };
            let given = Report {
                terminal: report.terminal.as_ref().map(TerminalId::as_str),
                pid: report.pid,
                payload: &report.payload,
            };
            let events = match adapter.report(&given) {
                Ok(events) => events,
                // The CLI goes on, and so does the app.
                Err(e) => {
                    eprintln!("yaar-keeper: {}: {e}", report.agent);
                    return Reply::Done;
                }
            };
            // Asked of the same report after its events, as the Adapter wants.
            let asked = adapter.asked(folder, &given).unwrap_or_else(|e| {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                None
            });
            (events, asked)
        };
        self.follow(
            kept,
            &report.agent,
            events,
            asked.as_ref(),
            Cause::Told(at),
            told,
        );
        match asked {
            Some(asked) => self.answer_asked(kept, report, &asked, at, told),
            None => Reply::Done,
        }
    }

    /// The sessions the Keeper's stop cut off and nothing has reported since, each with the
    /// command its Adapter makes for bringing it back: what a Window is offered and what a
    /// resume runs, the one place the command is made.
    ///
    /// The Switchboard is asked with a command maker that only notes what it is given, and
    /// let go; the Adapters are locked after, to make each command. The command maker the
    /// Switchboard takes cannot borrow the Adapters, and holding the Switchboard's lock and
    /// the Adapters' together would fix an order of the two that code of this side which
    /// holds the Adapters while it reads the Switchboard would break.
    fn offers(&self) -> Vec<Resumable> {
        let names: Vec<&'static str> = self.described.iter().map(|d| d.name).collect();
        let given = Arc::new(Mutex::new(HashMap::new()));
        let noted = Arc::clone(&given);
        let offers = lock(&self.switchboard).resumable(&move |agent, cli, record| {
            names.contains(&agent).then(|| {
                lock(&noted).insert((agent.to_owned(), cli.clone()), record.map(Path::to_owned));
                String::new()
            })
        });
        let given: HashMap<(String, CliSession), Option<PathBuf>> =
            std::mem::take(&mut *lock(&given));
        let adapters = lock(&self.adapters);
        // Each offer finds what was noted for it by the session it is for: an agent's name
        // and its CLI's id name one session. An offer with nothing noted, or whose Adapter
        // is not held, is left out; the maker noted every offer the Switchboard gave and
        // the names are those of the Adapters, so none is.
        offers
            .into_iter()
            .filter_map(|mut offer| {
                let record = given.get(&(offer.agent.clone(), offer.cli.clone()))?;
                let adapter = adapters.iter().find(|a| a.name() == offer.agent)?;
                offer.command = adapter.resume(&offer.cli, record.as_deref());
                Some(offer)
            })
            .collect()
    }

    /// The folder of the Adapter named `agent`.
    fn folder_of(&self, agent: &str) -> Option<&Path> {
        self.described
            .iter()
            .find(|adapter| adapter.name == agent)
            .map(|adapter| adapter.folder.as_path())
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
                self.follow(kept, described.name, events, None, Cause::Told(at), told);
                return;
            }
            first += described.marks.len();
        }
    }

    /// The program of `terminal` ended: every session that runs in it is gone with it.
    fn terminal_gone(&self, kept: &Kept, terminal: &TerminalId, told: &mut Told) {
        // Held no more: what its CLI still reports is followed no further.
        lock(&self.terminals).remove(terminal.as_str());
        lock(&self.opened_for).remove(terminal.as_str());
        // A resume that ran in it is over: the session may be brought back again.
        lock(&self.resuming).retain(|hold| hold.terminal != terminal.as_str());
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

    /// What is attached of `review` changed: the sessions of its workspace are told again.
    fn attachment_told(&self, kept: &Kept, review: &ReviewId, told: &mut Told) {
        match kept
            .store
            .review(&yaar_store::ReviewId::new(review.as_str()))
        {
            Ok(Some(record)) => told.changed(&record.workspace),
            Ok(None) => {}
            Err(e) => eprintln!("yaar-keeper: a review could not be read: {e}"),
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
    /// `asked` is what the same report waits for, none for a mark: a turn it starts came
    /// from the user when it is `yaar_agent::Asks::Message`.
    fn follow(
        &self,
        kept: &Kept,
        agent: &str,
        events: Vec<Reported>,
        asked: Option<&Asked>,
        cause: Cause,
        told: &mut Told,
    ) {
        let from_user = asked.is_some_and(|asked| asked.asks == Asks::Message);
        for mut reported in events {
            // A terminal the Keeper does not hold counts for none: such a CLI is followed
            // by the folder it works in alone, and is in no terminal of the app.
            let of_terminal = reported
                .terminal
                .as_ref()
                .and_then(|terminal| lock(&self.terminals).get(terminal).cloned());
            if of_terminal.is_none() {
                reported.terminal = None;
            }
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
            let (cli, cli_record) = (reported.session.0.clone(), reported.cli_record.clone());
            let changes = lock(&self.switchboard).take(agent, reported, &mut random_id_text);
            let starting = Starting {
                workspace: &workspace,
                event: &event,
                from_user,
            };
            self.apply(kept, changes, Some(starting), cause, told);
            // After the changes: a session that starts is kept by them.
            if let Some(record) = cli_record {
                keep_cli_record(kept, agent, &cli, &record);
            }
        }
    }

    /// The workspace a session that is new belongs to: the one whose worktree holds the
    /// folder its CLI works in, the deepest if several do; else `of_terminal`, the one its
    /// terminal was opened in, none for a CLI in no terminal the Keeper holds.
    fn workspace_of(
        &self,
        kept: &Kept,
        folder: Option<&Path>,
        of_terminal: Option<WorkspaceId>,
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
        of_terminal
    }

    /// Keeps each change in the Store, then gives it to Workspaces and the engine, and
    /// notes what is to be told. `starting` says what the report that led to the changes
    /// was, none for a change the Keeper made itself; the workspace of any session but a
    /// new one is the one the Store keeps it in.
    fn apply(
        &self,
        kept: &Kept,
        changes: Vec<Change>,
        starting: Option<Starting<'_>>,
        cause: Cause,
        told: &mut Told,
    ) {
        for change in changes {
            let session = session_of(&change);
            let Some(workspace) = self.workspace_for(kept, &session, starting.map(|s| s.workspace))
            else {
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
                } => {
                    let beginning = Beginning {
                        agent: &agent,
                        cli: &cli,
                        terminal,
                    };
                    self.session_starts(kept, &workspace, &session, beginning, told);
                }
                Change::State { .. } => {}
                Change::TurnStarted { session, turn } => {
                    lock(&self.turns_from_user).insert(
                        session.0.clone(),
                        starting.is_some_and(|starting| starting.from_user),
                    );
                    let number = turns_of_session(kept, &workspace, &session) + 1;
                    let id = TurnId::new(turn.0);
                    let start = self.start_of(kept, &workspace, &id);
                    keep(kept.store.put_turn(&TurnRecord {
                        id,
                        session: SessionId::new(session.0),
                        number,
                        ended: None,
                        start,
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
                        starting.map(|starting| starting.event),
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
                    // In flight from here, not from the recording thread's first look: a
                    // turn that starts later in this same item has to see it.
                    let number = {
                        let mut starts = lock(&self.starts);
                        let starts = starts.entry(workspace.clone()).or_default();
                        starts.issued += 1;
                        starts.issued
                    };
                    told.recordings.push(Recording {
                        number,
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

    /// The state the change of the turn `turn`, which starts now in `workspace`, counts
    /// from: the newest state of the worktree. None while the state of the turn before is
    /// being recorded, which is newer than any the store holds: the turn waits for it, and
    /// [`Core::recorded`] gives it its start when the recording is kept. None too where the
    /// store holds no state, and the change then counts from the commit checked out.
    fn start_of(&self, kept: &Kept, workspace: &WorkspaceId, turn: &TurnId) -> Option<String> {
        {
            let mut starts = lock(&self.starts);
            if let Some(starts) = starts.get_mut(workspace)
                && starts.done < starts.issued
            {
                starts.waiting.push((turn.clone(), starts.issued));
                return None;
            }
        }
        newest_state(kept, workspace)
    }

    /// The recording `recording` is kept, or failed with `state` none: the turns that
    /// waited for it get their start. A recording that failed leaves them the newest state
    /// of the store, and none if it holds none.
    fn recorded(&self, kept: &Kept, recording: &Recording, state: Option<&str>) {
        let ready: Vec<TurnId> = {
            let mut starts = lock(&self.starts);
            let Some(starts) = starts.get_mut(&recording.workspace) else {
                return;
            };
            starts.done = starts.done.max(recording.number);
            let (ready, waiting) = std::mem::take(&mut starts.waiting)
                .into_iter()
                .partition(|(_, after)| *after <= recording.number);
            starts.waiting = waiting;
            ready.into_iter().map(|(turn, _)| turn).collect()
        };
        if ready.is_empty() {
            return;
        }
        let Some(start) = state
            .map(str::to_owned)
            .or_else(|| newest_state(kept, &recording.workspace))
        else {
            return;
        };
        for turn in ready {
            keep(kept.store.set_turn_start(&turn, &start));
        }
    }

    /// A session starts, or its CLI resumes it: it is kept as running, takes its review,
    /// and its worktree is watched while it runs. The review it takes is the one its
    /// terminal was opened for, when there is one; a session that took the review of
    /// another is the one that session's unreviewed mark goes with.
    fn session_starts(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        session: &yaar_switchboard::SessionId,
        beginning: Beginning<'_>,
        told: &mut Told,
    ) {
        let Beginning {
            agent,
            cli,
            terminal,
        } = beginning;
        let id = SessionId::new(session.0.clone());
        // Whichever session starts first in a terminal opened for a review is the one that
        // review waits for.
        let opened_for = terminal
            .as_ref()
            .and_then(|terminal| lock(&self.opened_for).remove(terminal));
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
            Ok(Some(record)) => self.takes_review(kept, &record, opened_for.as_ref(), told),
            Ok(None) => {}
            Err(e) => eprintln!("yaar-keeper: a session could not be read: {e}"),
        }
        self.watching
            .agent_runs(kept, &yaar_link::WorkspaceId::new(workspace.as_str()), true);
    }

    /// The session `record` that started takes its review, `opened_for` first if its
    /// terminal was opened for one. The session whose review it took over has nothing
    /// unreviewed any more: it holds no review to be unreviewed in.
    fn takes_review(
        &self,
        kept: &Kept,
        record: &SessionRecord,
        opened_for: Option<&yaar_store::ReviewId>,
        told: &mut Told,
    ) {
        let before = kept.store.reviews_of(&record.workspace).unwrap_or_default();
        let taken = match opened_for {
            Some(review) => kept.reviews.session_started_for(record, review),
            None => kept.reviews.session_started(record),
        };
        let review = match taken {
            Ok(review) => review,
            Err(e) => {
                eprintln!("yaar-keeper: {e}");
                return;
            }
        };
        // The review now names the session: a Window that holds it reads it again.
        told.reviews_changed(&record.workspace);
        let previous = before
            .iter()
            .find(|held| held.id == review)
            .and_then(|held| held.agent.clone())
            .filter(|holder| holder != record.id.as_str());
        if let Some(previous) = previous {
            let changes = lock(&self.switchboard).reviewed(&yaar_switchboard::SessionId(previous));
            self.apply(kept, changes, None, Cause::Own, told);
        }
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
            Feed::Wrote(path) => {
                kept.workspaces.agent_wrote(workspace, &session.0, path);
                // The round the review was sent in counts as edited from the first edit
                // its agent reports after it.
                match kept
                    .reviews
                    .agent_edited(&SessionId::new(session.0.clone()))
                {
                    Ok(Some(_)) => told.reviews_changed(workspace),
                    Ok(None) => {}
                    Err(e) => eprintln!("yaar-keeper: {e}"),
                }
            }
            Feed::Began => kept
                .workspaces
                .command_began(workspace, &session.0, cause.at()),
        }
        told.steps.push(Fact::Step {
            workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
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
        // Before the Switchboard is held: the offers take the Adapters first.
        let offers = self.offers();
        let board = lock(&self.switchboard);
        let mut shown = Vec::new();
        let mut sessions = Vec::new();
        for record in records {
            let known = board.session(&yaar_switchboard::SessionId(record.id.as_str().to_owned()));
            let state = known
                .as_ref()
                .map_or(State::NotRunning, |known| known.state);
            let held = reviews
                .iter()
                .find(|review| review.agent.as_deref() == Some(record.id.as_str()));
            let review = held.map(|review| ReviewId::new(review.id.as_str()));
            let offered = offers
                .iter()
                .any(|offer| offer.session.0 == record.id.as_str());
            // A session still offered stays listed: the Window draws its resume control
            // from the list, though another session has taken over its review.
            if state == State::NotRunning && review.is_none() && !offered {
                continue;
            }
            let title = self
                .described
                .iter()
                .find(|adapter| adapter.name == record.agent)
                .map_or(record.agent.as_str(), |adapter| adapter.title);
            shown.push(state);
            let turn = known.as_ref().and_then(|known| known.turn.clone());
            let turn_from_user = turn.is_some()
                && lock(&self.turns_from_user)
                    .get(record.id.as_str())
                    .copied()
                    .unwrap_or(false);
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
                turn: turn.map(|(turn, _)| yaar_link::TurnId::new(turn.0)),
                review,
                attached: held
                    .and_then(|review| review.attached.as_ref())
                    .map(attachment_of),
                turn_from_user,
                resumable: offered,
            });
        }
        Ok(Some(WorkspaceAgents {
            workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
            shown: yaar_switchboard::shown(shown).map(state_of),
            sessions,
        }))
    }

    /// Gives a turn that ended, or a round, to the thread of `workspace` that records
    /// states, begun when the first is to be recorded there. The thread records them in the
    /// order given.
    fn record_later(&self, workspace: WorkspaceId, item: ToRecord) {
        let mut recorders = lock(&self.recorders);
        let sender = recorders.entry(workspace).or_insert_with(|| {
            let (sender, received) = mpsc::channel::<ToRecord>();
            if let Some(core) = self.this.upgrade() {
                let thread = spawn("records the states of turns and rounds", move || {
                    for item in received {
                        match item {
                            ToRecord::TurnEnd(recording) => core.record(&recording),
                            ToRecord::Round(round) => core.record_round(&round),
                        }
                    }
                });
                lock(&self.recorder_threads).push(thread);
            }
            sender
        });
        // A thread that is gone is the Keeper ending.
        let _ = sender.send(item);
    }

    /// Records the state of the worktree for a round delivered with a message and keeps it
    /// as the round's state (`Reviews::record_round`), then tells every Window that the
    /// reviews of its workspace changed. The recording is taken with the parts held and no
    /// turn, as a turn end's is; a recording that fails leaves the round with no state, and
    /// is written down.
    fn record_round(&self, round: &RoundRecording) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        {
            let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
            let Some(kept) = parts.as_ref().and_then(Parts::kept) else {
                return;
            };
            let recorded = match kept.workspaces.worktree(&round.workspace) {
                Ok(Some(worktree)) => kept
                    .reviews
                    .record_round(&worktree, &round.review, round.round)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                Ok(None) => Err("the worktree is gone".to_owned()),
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = recorded {
                eprintln!("yaar-keeper: no state was recorded for a round: {e}");
                return;
            }
        }
        let _turn = lock(&shared.turn);
        shared.tell(&[Fact::ReviewsChanged {
            workspace: yaar_link::WorkspaceId::new(round.workspace.as_str()),
        }]);
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
            let recorded = record_state(kept, recording);
            // The credits go with the state once it is kept, off the agents' thread.
            if let Some(state) = &recorded.0 {
                keep_credits(kept, &recording.workspace, state);
            }
            recorded
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
        // Before the turn's end is told: a Window that hears it reads the turns that
        // waited, and finds their start.
        self.recorded(kept, recording, state.as_deref());
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

/// Keeps with the recorded state `state` who is credited with every uncommitted path of
/// the worktree of `workspace`. Written down when it cannot be done: the state stands
/// without credits, which over-shows and hides nothing.
fn keep_credits(kept: &Kept, workspace: &WorkspaceId, state: &str) {
    let credits = match kept
        .workspaces
        .credits(&yaar_workspaces::WorkspaceId::new(workspace.as_str()))
    {
        Ok(credits) => credits,
        Err(e) => {
            eprintln!("yaar-keeper: the credits of {workspace} could not be read: {e}");
            return;
        }
    };
    let credits: Vec<yaar_review_engine::Credited> = credits
        .into_iter()
        .map(|credit| yaar_review_engine::Credited {
            path: credit.path,
            by: credit.by,
        })
        .collect();
    keep(kept.reviews.keep_credits(workspace, state, &credits));
}

/// The newest state recorded for `workspace`: one `git for-each-ref` of its state store.
/// None where it holds none, and where it cannot be read, which is written down.
fn newest_state(kept: &Kept, workspace: &WorkspaceId) -> Option<String> {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    let store = match kept.reviews.states_of(&id) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("yaar-keeper: the states of {workspace} could not be read: {e}");
            return None;
        }
    };
    match store.states() {
        Ok(states) => states.last().map(|state| state.id.as_str().to_owned()),
        Err(e) => {
            eprintln!("yaar-keeper: the states of {workspace} could not be read: {e}");
            None
        }
    }
}

/// Tells the Switchboard of every session the Store holds, and marks those the Store holds
/// as running ended by `KeeperStopped`: a Keeper that starts runs none. Every session the
/// Keeper's stop cut off, now or in a start before this one that nothing brought back, is
/// offered to resume, with the CLI's own record of it the Store keeps.
fn remember_and_cut_off(kept: &Kept, switchboard: &mut Switchboard) {
    match kept.store.workspaces() {
        Ok(workspaces) => {
            for workspace in workspaces {
                for record in kept.store.sessions_of(&workspace.id).unwrap_or_default() {
                    let id = yaar_switchboard::SessionId(record.id.as_str().to_owned());
                    if record.running || record.ended_by == Some(EndedBy::KeeperStopped) {
                        let cli_record = kept.store.session_cli_record(&record.id).unwrap_or_else(
                            |e| {
                                eprintln!(
                                    "yaar-keeper: the record of a session could not be read: {e}"
                                );
                                None
                            },
                        );
                        switchboard.remember_resumable(
                            id,
                            &record.agent,
                            CliSession(record.cli_session),
                            cli_record,
                        );
                    } else {
                        switchboard.remember(id, &record.agent, CliSession(record.cli_session));
                    }
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

/// Keeps `record` as the CLI's own record of the session the CLI calls `cli`, when the
/// Store holds another or none: the last one a session's reports named is the one its
/// resume is given. A report that names none leaves what is kept.
fn keep_cli_record(kept: &Kept, agent: &str, cli: &str, record: &Path) {
    let session = match kept.store.session_of_cli(agent, cli) {
        Ok(Some(session)) => session.id,
        Ok(None) => return,
        Err(e) => {
            eprintln!("yaar-keeper: a session could not be read: {e}");
            return;
        }
    };
    match kept.store.session_cli_record(&session) {
        Ok(held) if held.as_deref() == Some(record) => {}
        Ok(_) => keep(kept.store.set_session_cli_record(&session, Some(record))),
        Err(e) => eprintln!("yaar-keeper: the record of a session could not be read: {e}"),
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
        ShellEntry::Variable { name, value } => yaar_dock::Entry::Variable { name, value },
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

/// What is attached of a review as the Store keeps it, in the Link's words.
fn attachment_of(attached: &yaar_store::AttachedRecord) -> yaar_link::Attachment {
    match attached {
        yaar_store::AttachedRecord::Review => yaar_link::Attachment::Review,
        yaar_store::AttachedRecord::Thread(thread) => {
            yaar_link::Attachment::Thread(yaar_link::ThreadId::new(thread.as_str()))
        }
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
