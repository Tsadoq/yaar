//! Switchboard (`H-EVENTS, H-SESSION`).
//!
//! Owns: what hooks and extensions report, held while no window is attached; and for each
//! agent session its CLI, terminal, review and state.
//!
//! Surface:
//! - [`Switchboard`], which takes the events the Adapters made of what the CLIs reported,
//!   and what the Keeper saw itself (a process that is gone, a review marked reviewed),
//!   keeps every session with its state and its open turn, and gives what changed as
//!   [`Change`]s, in order;
//! - [`State`], what a session is doing, and [`shown`], the one state a workspace with
//!   several sessions shows;
//! - [`Known`], a session as the Switchboard holds it; [`SessionId`] and [`TurnId`], the
//!   app's names for a session and a turn.
//!
//! The Switchboard decides nothing for the user and answers nothing a CLI asks: it
//! listens. It keeps sessions in memory; the Keeper keeps what must outlive it in the
//! Store, from the changes this gives. It knows neither the Link nor the Store, and no CLI
//! by name: an Adapter's name is a text to it.

use std::fmt;
use std::path::PathBuf;

use yaar_agent::{CliSession, Event, Phase, Reported, TurnEnd};

/// What a session is doing.
///
/// The order of the variants is the order in which a workspace with several sessions
/// shows them, the first first: see [`shown`]. Idle is last, so that the four states the
/// sidebar marks keep their order among themselves whatever else runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// The agent waits at a permission prompt for the user's answer.
    WaitsForPermission,
    /// The agent finished a turn that the reviewer has not marked reviewed since.
    Unreviewed,
    /// The agent works on a turn.
    Works,
    /// The session ended, or its process is gone.
    NotRunning,
    /// The agent runs and waits for the user, with nothing unreviewed.
    Idle,
}

/// The state a workspace shows for its sessions: the first of these that holds for any
/// of them, waits for permission, finished a turn and not reviewed, works, not running;
/// idle only when every one is idle. None for no session.
pub fn shown(states: impl IntoIterator<Item = State>) -> Option<State> {
    // `State` is ordered as the sidebar shows it, so the least is the first that holds.
    states.into_iter().min()
}

/// The app's name for one agent session: 32 lowercase hexadecimal digits, drawn once when
/// the session first reported in. A session the CLI resumes keeps it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The app's name for one turn: 32 lowercase hexadecimal digits, drawn when it started.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub String);

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One session as the Switchboard holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The app's id of it.
    pub id: SessionId,
    /// The name of the Adapter of its CLI.
    pub agent: String,
    /// The CLI's own name for it.
    pub cli: CliSession,
    /// The terminal it runs in, by the id the app gave it.
    pub terminal: Option<String>,
    /// The folder its CLI works in, as the last report that named one said.
    pub folder: Option<PathBuf>,
    /// The process of the CLI; none when no report named one.
    pub pid: Option<u32>,
    /// What it is doing.
    pub state: State,
    /// Its open turn, with the number of steps told in it so far; none between turns.
    pub turn: Option<(TurnId, u32)>,
}

/// What changed, in the order it happened. The Keeper keeps it and tells the Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A session reported in for the first time, or a session that had ended was resumed
    /// by its CLI.
    Started {
        /// The session.
        session: SessionId,
        /// The name of the Adapter of its CLI.
        agent: String,
        /// The CLI's own name for it.
        cli: CliSession,
        /// The terminal it runs in.
        terminal: Option<String>,
        /// The folder its CLI works in, as its report named it.
        folder: Option<PathBuf>,
        /// Whether it is a session that ran before, resumed.
        resumed: bool,
    },
    /// A session's state is now this.
    State {
        /// The session.
        session: SessionId,
        /// Its state.
        state: State,
    },
    /// A turn started.
    TurnStarted {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
    },
    /// The agent took one more step in its open turn: a file read, a file written, or a
    /// shell command that began.
    Step {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// The number of the step in the turn, from 1.
        number: u32,
        /// The event that was the step: [`Event::FileRead`], [`Event::FileWritten`] or
        /// [`Event::Command`] that began.
        event: Event,
    },
    /// A shell command of the session ended; it is no step, but who wrote what during it
    /// is judged by when it ran. Also told for a command that began and was never
    /// reported ended, before the end of its turn or of its session: a command denied at
    /// its permission prompt, or interrupted, gets no report of its end.
    CommandEnded {
        /// The session.
        session: SessionId,
    },
    /// The open turn ended.
    TurnEnded {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// How.
        how: TurnEnd,
    },
    /// The session ended, as its CLI reported or as its process went.
    Ended {
        /// The session.
        session: SessionId,
    },
}

/// A session with what the Switchboard keeps beside what it tells.
#[derive(Debug)]
struct Entry {
    known: Known,
    /// A permission prompt is open.
    waiting: bool,
    /// The last turn ended and the review was not marked reviewed since.
    unreviewed: bool,
    /// A shell command began in the open turn and was not reported ended.
    command_open: bool,
}

impl Entry {
    /// A session as the Store kept it: not running, and no report of it heard yet.
    fn kept(id: SessionId, agent: &str, cli: CliSession) -> Entry {
        Entry {
            known: Known {
                id,
                agent: agent.to_string(),
                cli,
                terminal: None,
                folder: None,
                pid: None,
                state: State::NotRunning,
                turn: None,
            },
            waiting: false,
            unreviewed: false,
            command_open: false,
        }
    }

    fn running(&self) -> bool {
        self.known.state != State::NotRunning
    }

    /// Tells the state the facts give, when it is not the one told last. For a session
    /// that runs.
    fn settle(&mut self, changes: &mut Vec<Change>) {
        let state = if self.waiting {
            State::WaitsForPermission
        } else if self.known.turn.is_some() {
            State::Works
        } else if self.unreviewed {
            State::Unreviewed
        } else {
            State::Idle
        };
        self.set_state(state, changes);
    }

    fn set_state(&mut self, state: State, changes: &mut Vec<Change>) {
        if self.known.state != state {
            self.known.state = state;
            changes.push(Change::State {
                session: self.known.id.clone(),
                state,
            });
        }
    }

    /// Whatever followed a permission prompt is its answer: the wait is over.
    fn stop_waiting(&mut self, changes: &mut Vec<Change>) {
        if self.waiting {
            self.waiting = false;
            self.settle(changes);
        }
    }

    /// Ends the open turn, if one is open, after the command it left open. Tells no state:
    /// the caller does, for it knows where the session goes from here.
    fn close_turn(&mut self, how: TurnEnd, changes: &mut Vec<Change>) {
        if self.command_open {
            self.command_open = false;
            changes.push(Change::CommandEnded {
                session: self.known.id.clone(),
            });
        }
        if let Some((turn, _)) = self.known.turn.take() {
            self.unreviewed = true;
            changes.push(Change::TurnEnded {
                session: self.known.id.clone(),
                turn,
                how,
            });
        }
    }

    /// The session is not running from here: its open turn and command end first, and it
    /// goes from whatever it was to not running with no state between.
    fn stop(&mut self, changes: &mut Vec<Change>) {
        self.close_turn(TurnEnd::Interrupted, changes);
        self.waiting = false;
        changes.push(Change::Ended {
            session: self.known.id.clone(),
        });
        self.set_state(State::NotRunning, changes);
    }
}

/// Every agent session the Keeper has heard of since it started, with its state.
#[derive(Debug, Default)]
pub struct Switchboard {
    sessions: Vec<Entry>,
}

impl Switchboard {
    /// A Switchboard that knows no session.
    pub fn new() -> Switchboard {
        Switchboard::default()
    }

    /// A session the Keeper kept from an earlier run, as it is in the Store: not running.
    /// Its CLI may resume it, and then it keeps its id.
    pub fn remember(&mut self, session: SessionId, agent: &str, cli: CliSession) {
        if self.find(agent, &cli).is_none() {
            self.sessions.push(Entry::kept(session, agent, cli));
        }
    }

    /// Takes one event of the Adapter named `agent`, and gives what it changed, in order.
    /// `draw` gives a new id, for a session or a turn, each time it is called.
    ///
    /// A session first heard of is [`Change::Started`], idle. A turn that starts makes it
    /// work; a permission it waits for, and its answer, turn waiting on and off; any step
    /// after a permission it waited for is the answer; a turn that ends, by the CLI's word
    /// or by the mark of an interrupt, leaves it finished and not reviewed; the session's
    /// end, not running. An event of a session that has ended is taken as its CLI resuming
    /// it. A step outside a turn opens none: it is told with no turn, as nothing.
    ///
    /// Ends that came unreported. A turn that starts while one is open ends the open one
    /// as interrupted first, as when the mark of an interrupt was missed. A turn's end with
    /// no turn open changes nothing. Before any end of a turn, and before the end of the
    /// session, a shell command that began and was not reported ended is
    /// [`Change::CommandEnded`].
    pub fn take(
        &mut self,
        agent: &str,
        reported: Reported,
        draw: &mut dyn FnMut() -> String,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut first_heard = false;
        let at = match self.find(agent, &reported.session) {
            // The end of a session that is not running tells nothing new, and is no
            // resume: its process went first, or the end came twice.
            Some(at) if reported.event == Event::SessionEnded && !self.sessions[at].running() => {
                return changes;
            }
            Some(at) => at,
            None => {
                first_heard = true;
                let id = SessionId(draw());
                self.sessions
                    .push(Entry::kept(id, agent, reported.session.clone()));
                self.sessions.len() - 1
            }
        };
        let entry = &mut self.sessions[at];

        // A session that does not run and is heard of starts: for the first time when the
        // Switchboard has not met it, as a resume when it has or the Store kept it, and idle
        // either way.
        if !entry.running() {
            let resumed = !first_heard;
            entry.known.state = State::Idle;
            entry.unreviewed = false;
            changes.push(Change::Started {
                session: entry.known.id.clone(),
                agent: entry.known.agent.clone(),
                cli: entry.known.cli.clone(),
                terminal: reported.terminal.clone(),
                folder: reported.folder.clone(),
                resumed,
            });
            if resumed {
                changes.push(Change::State {
                    session: entry.known.id.clone(),
                    state: State::Idle,
                });
            }
        }
        entry.known.pid = Some(reported.pid);
        if reported.terminal.is_some() {
            entry.known.terminal = reported.terminal;
        }
        if reported.folder.is_some() {
            entry.known.folder = reported.folder;
        }

        let id = entry.known.id.clone();
        match reported.event {
            Event::SessionStarted { .. } => {}
            Event::SessionEnded => entry.stop(&mut changes),
            Event::TurnStarted => {
                if entry.known.turn.is_some() {
                    entry.stop_waiting(&mut changes);
                    entry.close_turn(TurnEnd::Interrupted, &mut changes);
                    entry.settle(&mut changes);
                }
                entry.waiting = false;
                let turn = TurnId(draw());
                entry.known.turn = Some((turn.clone(), 0));
                changes.push(Change::TurnStarted { session: id, turn });
                entry.settle(&mut changes);
            }
            Event::TurnEnded { how } => {
                if entry.known.turn.is_some() {
                    entry.stop_waiting(&mut changes);
                    entry.close_turn(how, &mut changes);
                    entry.settle(&mut changes);
                }
            }
            Event::Permission { waiting } => {
                entry.waiting = waiting;
                entry.settle(&mut changes);
            }
            Event::Command {
                phase: Phase::Ended,
                ..
            } => {
                entry.command_open = false;
                changes.push(Change::CommandEnded { session: id });
            }
            step @ (Event::FileRead { .. }
            | Event::FileWritten { .. }
            | Event::Command {
                phase: Phase::Began,
                ..
            }) => {
                entry.stop_waiting(&mut changes);
                if let Some((turn, steps)) = &mut entry.known.turn {
                    *steps += 1;
                    if matches!(step, Event::Command { .. }) {
                        entry.command_open = true;
                    }
                    changes.push(Change::Step {
                        session: id,
                        turn: turn.clone(),
                        number: *steps,
                        event: step,
                    });
                }
            }
        }
        changes
    }

    /// The process `pid` of a CLI is gone, with no end reported: every session of it is
    /// not running, and its open turn, if any, ends as interrupted, after its open shell
    /// command, if any, as [`Change::CommandEnded`].
    pub fn gone(&mut self, pid: u32) -> Vec<Change> {
        let mut changes = Vec::new();
        for entry in &mut self.sessions {
            if entry.running() && entry.known.pid == Some(pid) {
                entry.stop(&mut changes);
            }
        }
        changes
    }

    /// The review of `session` was marked reviewed: a session that finished a turn not
    /// yet reviewed is idle now.
    pub fn reviewed(&mut self, session: &SessionId) -> Vec<Change> {
        let mut changes = Vec::new();
        if let Some(entry) = self.sessions.iter_mut().find(|e| e.known.id == *session) {
            entry.unreviewed = false;
            if entry.running() {
                entry.settle(&mut changes);
            }
        }
        changes
    }

    /// The session by its id.
    pub fn session(&self, session: &SessionId) -> Option<Known> {
        self.sessions
            .iter()
            .find(|entry| entry.known.id == *session)
            .map(|entry| entry.known.clone())
    }

    /// Every session, in the order they were first heard of.
    pub fn sessions(&self) -> Vec<Known> {
        self.sessions
            .iter()
            .map(|entry| entry.known.clone())
            .collect()
    }

    /// The processes of the sessions that run: those the Keeper watches for their end.
    pub fn pids(&self) -> Vec<u32> {
        let mut pids = Vec::new();
        for entry in self.sessions.iter().filter(|entry| entry.running()) {
            if let Some(pid) = entry.known.pid
                && !pids.contains(&pid)
            {
                pids.push(pid);
            }
        }
        pids
    }

    fn find(&self, agent: &str, cli: &CliSession) -> Option<usize> {
        self.sessions
            .iter()
            .position(|entry| entry.known.agent == agent && entry.known.cli == *cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaar_agent::{Start, WriteTool};

    const AGENT: &str = "claude";

    /// The id the nth call of `draw` gives.
    fn drawn(n: u32) -> String {
        format!("{n:032x}")
    }

    fn session_drawn(n: u32) -> SessionId {
        SessionId(drawn(n))
    }

    fn turn_drawn(n: u32) -> TurnId {
        TurnId(drawn(n))
    }

    fn reported(cli: &str, pid: u32, event: Event) -> Reported {
        Reported {
            session: CliSession(cli.to_string()),
            terminal: Some(format!("terminal-{cli}")),
            pid,
            folder: Some(PathBuf::from("/work/project")),
            event,
        }
    }

    fn written(path: &str) -> Event {
        Event::FileWritten {
            path: PathBuf::from(path),
            tool: WriteTool::Write,
            lines: None,
        }
    }

    fn read(path: &str) -> Event {
        Event::FileRead {
            path: PathBuf::from(path),
        }
    }

    fn command(phase: Phase) -> Event {
        Event::Command {
            phase,
            command: "make".to_string(),
        }
    }

    fn started() -> Event {
        Event::SessionStarted { how: Start::New }
    }

    fn turn_ended(how: TurnEnd) -> Event {
        Event::TurnEnded { how }
    }

    fn permission(waiting: bool) -> Event {
        Event::Permission { waiting }
    }

    fn now(session: &SessionId, state: State) -> Change {
        Change::State {
            session: session.clone(),
            state,
        }
    }

    fn ends(session: &SessionId, turn: &TurnId, how: TurnEnd) -> Change {
        Change::TurnEnded {
            session: session.clone(),
            turn: turn.clone(),
            how,
        }
    }

    fn command_ended(session: &SessionId) -> Change {
        Change::CommandEnded {
            session: session.clone(),
        }
    }

    fn ended(session: &SessionId) -> Change {
        Change::Ended {
            session: session.clone(),
        }
    }

    /// A Switchboard with a `draw` that counts, so that a test knows each id it will see.
    struct Desk {
        board: Switchboard,
        draws: u32,
    }

    impl Desk {
        fn new() -> Desk {
            Desk {
                board: Switchboard::new(),
                draws: 0,
            }
        }

        fn take(&mut self, agent: &str, reported: Reported) -> Vec<Change> {
            let draws = &mut self.draws;
            self.board.take(agent, reported, &mut || {
                *draws += 1;
                drawn(*draws)
            })
        }

        /// One event of the session `cli`, from the process 100.
        fn hear(&mut self, cli: &str, event: Event) -> Vec<Change> {
            self.from(cli, 100, event)
        }

        fn from(&mut self, cli: &str, pid: u32, event: Event) -> Vec<Change> {
            self.take(AGENT, reported(cli, pid, event))
        }

        fn known(&self, cli: &str) -> Known {
            self.board
                .sessions()
                .into_iter()
                .find(|known| known.cli == CliSession(cli.to_string()))
                .unwrap_or_else(|| panic!("no session {cli}"))
        }

        fn id(&self, cli: &str) -> SessionId {
            self.known(cli).id
        }

        /// The state, read the way the Keeper reads it.
        fn state(&self, cli: &str) -> State {
            let id = self.id(cli);
            self.board.session(&id).expect("the session").state
        }
    }

    /// A desk with the session `cli` started and in its first turn, the ids 1 and 2.
    fn in_a_turn(cli: &str) -> (Desk, SessionId, TurnId) {
        let mut desk = Desk::new();
        desk.hear(cli, started());
        desk.hear(cli, Event::TurnStarted);
        let id = desk.id(cli);
        assert_eq!(id, session_drawn(1));
        (desk, id, turn_drawn(2))
    }

    #[test]
    fn a_session_first_heard_is_started_with_its_folder_and_is_idle() {
        let mut desk = Desk::new();
        let changes = desk.hear("a", started());
        let id = session_drawn(1);
        assert_eq!(
            changes,
            [Change::Started {
                session: id.clone(),
                agent: "claude".to_string(),
                cli: CliSession("a".to_string()),
                terminal: Some("terminal-a".to_string()),
                folder: Some(PathBuf::from("/work/project")),
                resumed: false,
            }]
        );
        assert_eq!(desk.state("a"), State::Idle);
        let known = desk.board.session(&id).expect("the session");
        assert_eq!(known.agent, "claude");
        assert_eq!(known.turn, None);
        assert_eq!(known.pid, Some(100));
        assert_eq!(known.terminal.as_deref(), Some("terminal-a"));
        assert_eq!(known.folder, Some(PathBuf::from("/work/project")));
    }

    #[test]
    fn any_event_of_an_unknown_session_starts_it_first() {
        let mut desk = Desk::new();
        let changes = desk.hear("a", Event::TurnStarted);
        let (id, turn) = (session_drawn(1), turn_drawn(2));
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(
            matches!(&changes[0], Change::Started { session, resumed: false, .. } if *session == id)
        );
        assert_eq!(
            changes[1..],
            [
                Change::TurnStarted {
                    session: id.clone(),
                    turn,
                },
                now(&id, State::Works)
            ]
        );
        // A session known to the Switchboard is not started twice, and draws no new id.
        assert_eq!(desk.hear("a", started()), []);
        assert_eq!(desk.draws, 2);
    }

    #[test]
    fn the_state_follows_a_turn_from_its_start_to_the_mark_of_reviewed() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        let id = desk.id("a");
        assert_eq!(desk.state("a"), State::Idle);

        // Works from the turn's start.
        let turn = turn_drawn(2);
        assert_eq!(
            desk.hear("a", Event::TurnStarted),
            [
                Change::TurnStarted {
                    session: id.clone(),
                    turn: turn.clone(),
                },
                now(&id, State::Works)
            ]
        );
        assert_eq!(desk.state("a"), State::Works);
        assert_eq!(desk.known("a").turn, Some((turn.clone(), 0)));

        // Waits at a permission prompt; a second report of it changes nothing.
        assert_eq!(
            desk.hear("a", permission(true)),
            [now(&id, State::WaitsForPermission)]
        );
        assert_eq!(desk.state("a"), State::WaitsForPermission);
        assert_eq!(desk.hear("a", permission(true)), []);

        // The answer yes is the write that follows: the wait ends before the step.
        assert_eq!(
            desk.hear("a", written("p.txt")),
            [
                now(&id, State::Works),
                Change::Step {
                    session: id.clone(),
                    turn: turn.clone(),
                    number: 1,
                    event: written("p.txt"),
                }
            ]
        );
        assert_eq!(desk.state("a"), State::Works);

        // Finished and not reviewed from the turn's end.
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                ends(&id, &turn, TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);
        assert_eq!(desk.known("a").turn, None);

        // Idle once the review is marked reviewed; marking it again changes nothing.
        assert_eq!(desk.board.reviewed(&id), [now(&id, State::Idle)]);
        assert_eq!(desk.state("a"), State::Idle);
        assert_eq!(desk.board.reviewed(&id), []);
    }

    #[test]
    fn a_permission_answered_with_a_report_of_its_own_ends_the_wait() {
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", permission(true));
        assert_eq!(desk.hear("a", permission(false)), [now(&id, State::Works)]);
        assert_eq!(desk.state("a"), State::Works);
        // An answer with no wait to end changes nothing.
        assert_eq!(desk.hear("a", permission(false)), []);
        assert_eq!(desk.state("a"), State::Works);
    }

    #[test]
    fn a_turn_ended_by_an_interrupt_also_leaves_the_session_unreviewed() {
        let (mut desk, id, turn) = in_a_turn("a");
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Interrupted)),
            [
                ends(&id, &turn, TurnEnd::Interrupted),
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);
        assert_eq!(desk.board.reviewed(&id), [now(&id, State::Idle)]);
    }

    #[test]
    fn an_interrupt_at_a_permission_prompt_ends_the_wait_before_the_turn() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", permission(true));
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Interrupted)),
            [
                now(&id, State::Works),
                ends(&id, &turn, TurnEnd::Interrupted),
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);
    }

    #[test]
    fn a_session_that_ends_is_not_running() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        assert_eq!(
            desk.hear("a", Event::SessionEnded),
            [ended(&id), now(&id, State::NotRunning)]
        );
        assert_eq!(desk.state("a"), State::NotRunning);
        assert_eq!(desk.known("a").turn, None);
        assert!(desk.board.pids().is_empty());

        // A session that ends in the middle of a turn ends the turn as interrupted first,
        // and goes from works to not running without a state between.
        let mut other = Desk::new();
        other.hear("b", started());
        other.hear("b", Event::TurnStarted);
        let (id, _) = (other.id("b"), turn);
        assert_eq!(
            other.hear("b", Event::SessionEnded),
            [
                ends(&id, &turn_drawn(2), TurnEnd::Interrupted),
                ended(&id),
                now(&id, State::NotRunning)
            ]
        );
        assert_eq!(other.state("b"), State::NotRunning);
        assert_eq!(other.known("b").turn, None);
    }

    #[test]
    fn a_report_of_an_end_after_the_end_changes_nothing() {
        let (mut desk, ..) = in_a_turn("a");
        desk.hear("a", Event::SessionEnded);
        assert_eq!(desk.hear("a", Event::SessionEnded), []);
        assert_eq!(desk.state("a"), State::NotRunning);
    }

    #[test]
    fn a_process_that_is_gone_leaves_its_sessions_not_running_and_no_other() {
        let mut desk = Desk::new();
        desk.from("a", 10, started());
        desk.from("b", 20, started());
        desk.from("b", 20, Event::TurnStarted);
        desk.from("c", 20, started());
        desk.from("d", 30, started());
        let (b, c) = (desk.id("b"), desk.id("c"));

        // Both sessions of the process go, in the order they were heard of; the one in a
        // turn ends it as interrupted first.
        assert_eq!(
            desk.board.gone(20),
            [
                ends(&b, &turn_drawn(3), TurnEnd::Interrupted),
                ended(&b),
                now(&b, State::NotRunning),
                ended(&c),
                now(&c, State::NotRunning),
            ]
        );
        assert_eq!(desk.state("b"), State::NotRunning);
        assert_eq!(desk.state("c"), State::NotRunning);
        assert_eq!(desk.state("a"), State::Idle);
        assert_eq!(desk.state("d"), State::Idle);

        // Nothing is left to say about it, nor about a process no session runs in.
        assert_eq!(desk.board.gone(20), []);
        assert_eq!(desk.board.gone(99), []);
    }

    #[test]
    fn a_command_left_open_ends_before_the_interrupt_that_ends_its_turn() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Interrupted)),
            [
                command_ended(&id),
                ends(&id, &turn, TurnEnd::Interrupted),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn a_command_left_open_ends_before_the_end_of_a_turn_the_cli_reports() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                command_ended(&id),
                ends(&id, &turn, TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn a_command_denied_at_its_prompt_ends_before_the_turn_the_mark_ends() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        desk.hear("a", permission(true));
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Interrupted)),
            [
                now(&id, State::Works),
                command_ended(&id),
                ends(&id, &turn, TurnEnd::Interrupted),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn a_command_that_ended_is_not_ended_again_by_the_end_of_its_turn() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        assert_eq!(desk.hear("a", command(Phase::Ended)), [command_ended(&id)]);
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                ends(&id, &turn, TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn a_command_left_open_ends_before_the_end_of_the_session() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        assert_eq!(
            desk.hear("a", Event::SessionEnded),
            [
                command_ended(&id),
                ends(&id, &turn, TurnEnd::Interrupted),
                ended(&id),
                now(&id, State::NotRunning)
            ]
        );
    }

    #[test]
    fn a_command_left_open_ends_before_the_end_of_a_process() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        assert_eq!(
            desk.board.gone(100),
            [
                command_ended(&id),
                ends(&id, &turn, TurnEnd::Interrupted),
                ended(&id),
                now(&id, State::NotRunning)
            ]
        );
        assert_eq!(desk.state("a"), State::NotRunning);
    }

    #[test]
    fn a_command_does_not_stay_open_into_the_next_turn() {
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", command(Phase::Began));
        desk.hear("a", turn_ended(TurnEnd::Interrupted));
        desk.hear("a", Event::TurnStarted);
        // No command is open any more: this end gives nothing before it.
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                ends(&id, &turn_drawn(3), TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn a_turn_started_while_one_is_open_ends_the_open_one_as_interrupted() {
        let (mut desk, id, first) = in_a_turn("a");
        desk.hear("a", read("a.txt"));
        desk.hear("a", command(Phase::Began));
        assert_eq!(
            desk.hear("a", Event::TurnStarted),
            [
                command_ended(&id),
                ends(&id, &first, TurnEnd::Interrupted),
                now(&id, State::Unreviewed),
                Change::TurnStarted {
                    session: id.clone(),
                    turn: turn_drawn(3),
                },
                now(&id, State::Works),
            ]
        );
        assert_eq!(desk.state("a"), State::Works);
        // The new turn counts its steps from the start.
        assert_eq!(desk.known("a").turn, Some((turn_drawn(3), 0)));
    }

    #[test]
    fn a_turn_started_while_the_session_waits_ends_the_wait_and_the_open_turn() {
        let (mut desk, id, first) = in_a_turn("a");
        desk.hear("a", permission(true));
        assert_eq!(
            desk.hear("a", Event::TurnStarted),
            [
                now(&id, State::Works),
                ends(&id, &first, TurnEnd::Interrupted),
                now(&id, State::Unreviewed),
                Change::TurnStarted {
                    session: id.clone(),
                    turn: turn_drawn(3),
                },
                now(&id, State::Works),
            ]
        );
        // The wait is over for good: a turn's end leaves it unreviewed.
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)).last(),
            Some(&now(&id, State::Unreviewed))
        );
    }

    #[test]
    fn a_turn_ended_with_no_turn_open_gives_nothing() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        assert_eq!(desk.hear("a", turn_ended(TurnEnd::Finished)), []);
        assert_eq!(desk.state("a"), State::Idle);

        // Nor does a second end of a turn already ended, or the mark after a `Stop`.
        desk.hear("a", Event::TurnStarted);
        desk.hear("a", turn_ended(TurnEnd::Finished));
        assert_eq!(desk.hear("a", turn_ended(TurnEnd::Interrupted)), []);
        assert_eq!(desk.state("a"), State::Unreviewed);
    }

    #[test]
    fn the_steps_of_a_turn_are_numbered_from_1_and_a_command_end_is_none() {
        let (mut desk, id, turn) = in_a_turn("a");
        let step = |number, event: Event| Change::Step {
            session: id.clone(),
            turn: turn.clone(),
            number,
            event,
        };
        assert_eq!(desk.hear("a", read("a.txt")), [step(1, read("a.txt"))]);
        assert_eq!(
            desk.hear("a", written("b.txt")),
            [step(2, written("b.txt"))]
        );
        assert_eq!(
            desk.hear("a", command(Phase::Began)),
            [step(3, command(Phase::Began))]
        );
        assert_eq!(desk.hear("a", command(Phase::Ended)), [command_ended(&id)]);
        assert_eq!(desk.known("a").turn, Some((turn.clone(), 3)));
        assert_eq!(desk.hear("a", read("c.txt")), [step(4, read("c.txt"))]);

        // The next turn starts again at 1.
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.hear("a", Event::TurnStarted);
        assert_eq!(
            desk.hear("a", read("d.txt")),
            [Change::Step {
                session: id.clone(),
                turn: turn_drawn(3),
                number: 1,
                event: read("d.txt"),
            }]
        );
    }

    #[test]
    fn a_step_outside_a_turn_is_told_as_nothing_and_opens_none() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        assert_eq!(desk.hear("a", read("a.txt")), []);
        assert_eq!(desk.hear("a", written("a.txt")), []);
        assert_eq!(desk.hear("a", command(Phase::Began)), []);
        assert_eq!(desk.state("a"), State::Idle);
        assert_eq!(desk.known("a").turn, None);

        // A command that began outside a turn was not told, so no end of it is made up.
        let id = desk.id("a");
        desk.hear("a", Event::TurnStarted);
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                ends(&id, &turn_drawn(2), TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
    }

    #[test]
    fn an_event_of_an_ended_session_is_a_resume_that_keeps_its_id() {
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.hear("a", Event::SessionEnded);
        assert_eq!(desk.state("a"), State::NotRunning);

        // The turn it left unreviewed does not come back with it: it is idle.
        let resumed = Reported {
            terminal: Some("terminal-new".to_string()),
            folder: Some(PathBuf::from("/work/other")),
            ..reported(
                "a",
                200,
                Event::SessionStarted {
                    how: Start::Resumed,
                },
            )
        };
        assert_eq!(
            desk.take(AGENT, resumed),
            [
                Change::Started {
                    session: id.clone(),
                    agent: "claude".to_string(),
                    cli: CliSession("a".to_string()),
                    terminal: Some("terminal-new".to_string()),
                    folder: Some(PathBuf::from("/work/other")),
                    resumed: true,
                },
                now(&id, State::Idle)
            ]
        );
        assert_eq!(desk.state("a"), State::Idle);
        assert_eq!(desk.known("a").pid, Some(200));
        assert_eq!(desk.known("a").terminal.as_deref(), Some("terminal-new"));
        assert_eq!(desk.board.pids(), [200]);
        assert_eq!(desk.draws, 2, "a resume draws no id for the session");
    }

    #[test]
    fn any_event_of_an_ended_session_resumes_it_before_it_is_taken() {
        let (mut desk, id, _) = in_a_turn("a");
        desk.board.gone(100);
        assert_eq!(
            desk.hear("a", Event::TurnStarted),
            [
                Change::Started {
                    session: id.clone(),
                    agent: "claude".to_string(),
                    cli: CliSession("a".to_string()),
                    terminal: Some("terminal-a".to_string()),
                    folder: Some(PathBuf::from("/work/project")),
                    resumed: true,
                },
                now(&id, State::Idle),
                Change::TurnStarted {
                    session: id.clone(),
                    turn: turn_drawn(3),
                },
                now(&id, State::Works),
            ]
        );
    }

    #[test]
    fn a_session_kept_by_the_store_is_not_running_and_its_cli_resumes_it_under_its_id() {
        let mut desk = Desk::new();
        let kept = SessionId("kept".to_string());
        desk.board
            .remember(kept.clone(), AGENT, CliSession("c".to_string()));
        let known = desk.board.session(&kept).expect("the session");
        assert_eq!(known.state, State::NotRunning);
        assert_eq!(
            (known.pid, known.terminal, known.folder, known.turn),
            (None, None, None, None)
        );
        assert!(desk.board.pids().is_empty());

        // Another Adapter's session of the same CLI name is another session.
        assert!(matches!(
            desk.take("other", reported("c", 5, started()))[0],
            Change::Started { resumed: false, .. }
        ));
        assert_eq!(desk.draws, 1);

        let changes = desk.hear(
            "c",
            Event::SessionStarted {
                how: Start::Resumed,
            },
        );
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(
            matches!(&changes[0], Change::Started { session, resumed: true, .. } if *session == kept),
            "{changes:?}"
        );
        assert_eq!(changes[1], now(&kept, State::Idle));
        assert_eq!(desk.draws, 1, "the session keeps the id the Store gave it");
        assert_eq!(desk.board.session(&kept).expect("kept").state, State::Idle);
        assert_eq!(desk.board.sessions().len(), 2);
    }

    #[test]
    fn the_pid_the_folder_and_the_terminal_follow_the_latest_report_that_names_them() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        let nameless = Reported {
            terminal: None,
            folder: None,
            ..reported("a", 200, read("a.txt"))
        };
        desk.take(AGENT, nameless);
        let known = desk.known("a");
        assert_eq!(known.pid, Some(200));
        assert_eq!(known.folder, Some(PathBuf::from("/work/project")));
        assert_eq!(known.terminal.as_deref(), Some("terminal-a"));

        let elsewhere = Reported {
            folder: Some(PathBuf::from("/work/elsewhere")),
            terminal: Some("terminal-b".to_string()),
            ..reported("a", 300, read("a.txt"))
        };
        desk.take(AGENT, elsewhere);
        let known = desk.known("a");
        assert_eq!(known.pid, Some(300));
        assert_eq!(known.folder, Some(PathBuf::from("/work/elsewhere")));
        assert_eq!(known.terminal.as_deref(), Some("terminal-b"));
    }

    #[test]
    fn the_processes_to_watch_are_those_of_the_running_sessions_once_each() {
        let mut desk = Desk::new();
        desk.from("a", 10, started());
        desk.from("b", 10, started());
        desk.from("c", 20, started());
        desk.from("d", 30, started());
        desk.from("d", 30, Event::SessionEnded);
        let mut pids = desk.board.pids();
        pids.sort_unstable();
        assert_eq!(pids, [10, 20]);
        desk.board.gone(10);
        assert_eq!(desk.board.pids(), [20]);
    }

    #[test]
    fn a_mark_of_reviewed_changes_nothing_while_a_turn_runs_or_for_a_session_that_is_gone() {
        let (mut desk, id, turn) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.hear("a", Event::TurnStarted);
        assert_eq!(desk.board.reviewed(&id), []);
        assert_eq!(desk.state("a"), State::Works);
        // The turn that follows leaves it unreviewed again.
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)).last(),
            Some(&now(&id, State::Unreviewed))
        );
        desk.board.gone(100);
        assert_eq!(desk.board.reviewed(&id), []);
        assert_eq!(desk.state("a"), State::NotRunning);
        assert_eq!(desk.board.reviewed(&SessionId("unknown".to_string())), []);
        let _ = turn;
    }

    #[test]
    fn sessions_come_in_the_order_they_were_first_heard_of() {
        let mut desk = Desk::new();
        for cli in ["c", "a", "b"] {
            desk.hear(cli, started());
        }
        desk.hear("c", Event::TurnStarted);
        let clis: Vec<_> = desk
            .board
            .sessions()
            .into_iter()
            .map(|known| known.cli.0)
            .collect();
        assert_eq!(clis, ["c", "a", "b"]);
        assert_eq!(desk.board.session(&SessionId("nobody".to_string())), None);
    }

    /// A long walk through events of three sessions, in an order a real CLI would not keep
    /// to, checks what must hold whatever arrives: the states told are the states held,
    /// no state is told twice in a row, a turn ends the turn that is open, and a command
    /// is ended before its turn is.
    #[test]
    fn whatever_arrives_the_states_told_are_the_states_held() {
        let events = [
            started(),
            Event::SessionStarted {
                how: Start::Resumed,
            },
            Event::TurnStarted,
            turn_ended(TurnEnd::Finished),
            turn_ended(TurnEnd::Interrupted),
            read("r"),
            written("w"),
            command(Phase::Began),
            command(Phase::Ended),
            permission(true),
            permission(false),
            Event::SessionEnded,
        ];
        let mut desk = Desk::new();
        let mut told: std::collections::HashMap<SessionId, State> = Default::default();
        let mut open_turn: std::collections::HashMap<SessionId, TurnId> = Default::default();
        let mut open_command: std::collections::HashMap<SessionId, bool> = Default::default();
        let mut seen: std::collections::HashSet<State> = Default::default();
        let mut seed = 7_u64;
        let mut check = |desk: &mut Desk, changes: Vec<Change>| {
            for change in changes {
                match change {
                    Change::Started {
                        session, resumed, ..
                    } => {
                        if resumed {
                            assert_eq!(told.get(&session), Some(&State::NotRunning));
                            told.insert(session.clone(), State::NotRunning);
                        } else {
                            told.insert(session.clone(), State::Idle);
                        }
                        open_turn.remove(&session);
                    }
                    Change::State { session, state } => {
                        seen.insert(state);
                        assert_ne!(told.insert(session, state), Some(state), "told twice");
                    }
                    Change::TurnStarted { session, turn } => {
                        assert_eq!(open_turn.insert(session, turn), None, "two turns open");
                    }
                    Change::Step {
                        session,
                        turn,
                        event,
                        ..
                    } => {
                        assert_eq!(open_turn.get(&session), Some(&turn));
                        if matches!(event, Event::Command { .. }) {
                            open_command.insert(session.clone(), true);
                        }
                    }
                    Change::CommandEnded { session } => {
                        open_command.insert(session, false);
                    }
                    Change::TurnEnded { session, turn, .. } => {
                        assert_eq!(open_turn.remove(&session), Some(turn));
                        assert_ne!(
                            open_command.get(&session),
                            Some(&true),
                            "command open at turn end"
                        );
                    }
                    Change::Ended { session } => {
                        assert!(!open_turn.contains_key(&session), "turn open at end");
                    }
                }
            }
            for known in desk.board.sessions() {
                assert_eq!(told.get(&known.id), Some(&known.state), "{known:?}");
            }
        };
        for _ in 0..3000 {
            // A small linear congruential generator keeps the walk the same every run.
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let pick = (seed >> 33) as usize;
            let cli = ["a", "b", "c"][pick % 3];
            let event = events[(pick / 3) % events.len()].clone();
            let changes = if pick.is_multiple_of(17) {
                desk.board.gone(100)
            } else if pick.is_multiple_of(19) {
                let id = desk.board.sessions().first().map(|known| known.id.clone());
                id.map(|id| desk.board.reviewed(&id)).unwrap_or_default()
            } else {
                desk.hear(cli, event)
            };
            check(&mut desk, changes);
        }
        // The walk is no use if it never got to a state.
        assert_eq!(seen.len(), ORDER.len(), "{seen:?}");
    }

    // SWB-02

    /// The order of the row's sentence, written out so that the test does not lean on the
    /// order the type derives.
    const ORDER: [State; 5] = [
        State::WaitsForPermission,
        State::Unreviewed,
        State::Works,
        State::NotRunning,
        State::Idle,
    ];

    fn first_of_the_order(states: &[State]) -> Option<State> {
        ORDER.into_iter().find(|state| states.contains(state))
    }

    #[test]
    fn the_state_shown_for_every_pair_and_triple_is_the_first_of_the_order() {
        for one in ORDER {
            assert_eq!(shown([one]), Some(one));
            for two in ORDER {
                let pair = [one, two];
                assert_eq!(shown(pair), first_of_the_order(&pair), "{pair:?}");
                for three in ORDER {
                    let triple = [one, two, three];
                    assert_eq!(shown(triple), first_of_the_order(&triple), "{triple:?}");
                }
            }
        }
    }

    #[test]
    fn the_order_is_waits_then_unreviewed_then_works_then_not_running() {
        let pairs = [
            (State::WaitsForPermission, State::Unreviewed),
            (State::Unreviewed, State::Works),
            (State::Works, State::NotRunning),
        ];
        for (first, second) in pairs {
            assert_eq!(shown([first, second]), Some(first));
            assert_eq!(shown([second, first]), Some(first));
        }
    }

    #[test]
    fn idle_is_shown_only_when_every_session_is_idle() {
        assert_eq!(
            shown([State::Idle, State::Idle, State::Idle]),
            Some(State::Idle)
        );
        for other in ORDER.into_iter().filter(|state| *state != State::Idle) {
            assert_eq!(shown([State::Idle, other]), Some(other));
            assert_eq!(shown([other, State::Idle, State::Idle]), Some(other));
        }
        assert_eq!(
            shown([State::Idle, State::NotRunning]),
            Some(State::NotRunning)
        );
    }

    #[test]
    fn no_session_shows_no_state() {
        assert_eq!(shown([]), None);
        assert_eq!(shown(Vec::<State>::new()), None);
    }
}
