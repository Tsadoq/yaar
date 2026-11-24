//! Tracks agent sessions from what hooks and extensions report and gives what changed as ordered
//! [`Change`]s. In memory only.

use std::fmt;
use std::path::{Path, PathBuf};

use yaar_agent::{CliSession, Event, Phase, Reported, TurnEnd};

/// Variant order is the display priority of [`shown`], first first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    WaitsForPermission,
    Unreviewed,
    Works,
    NotRunning,
    Idle,
}

pub fn shown(states: impl IntoIterator<Item = State>) -> Option<State> {
    states.into_iter().min()
}

/// 32 lowercase hex digits, drawn when the session first reports in; a CLI resume keeps it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub String);

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub id: SessionId,
    pub agent: String,
    pub cli: CliSession,
    pub terminal: Option<String>,
    pub folder: Option<PathBuf>,
    pub pid: Option<u32>,
    pub state: State,
    /// The open turn with the number of steps told in it so far; None between turns.
    pub turn: Option<(TurnId, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resumable {
    pub session: SessionId,
    pub agent: String,
    pub cli: CliSession,
    pub command: String,
}

/// None for an Adapter the caller has no command for.
pub type ResumeCommand = dyn Fn(&str, &CliSession, Option<&Path>) -> Option<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Started {
        session: SessionId,
        agent: String,
        cli: CliSession,
        terminal: Option<String>,
        folder: Option<PathBuf>,
        resumed: bool,
    },
    State {
        session: SessionId,
        state: State,
    },
    TurnStarted {
        session: SessionId,
        turn: TurnId,
    },
    Step {
        session: SessionId,
        turn: TurnId,
        number: u32,
        event: Event,
    },
    /// A shell command ended; it is no step. Also told for one that never reported its end,
    /// before its turn or session ends.
    CommandEnded {
        session: SessionId,
    },
    TurnEnded {
        session: SessionId,
        turn: TurnId,
        how: TurnEnd,
    },
    Ended {
        session: SessionId,
    },
}

#[derive(Debug)]
struct Entry {
    known: Known,
    waiting: bool,
    unreviewed: bool,
    command_open: bool,
    /// Some while the Keeper's stop cut the session off and no report followed; the inner value
    /// is the CLI's own record of the session, when it had one.
    offer: Option<Option<PathBuf>>,
}

impl Entry {
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
            offer: None,
        }
    }

    fn running(&self) -> bool {
        self.known.state != State::NotRunning
    }

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

    /// A step after a permission prompt is its answer: the wait is over.
    fn stop_waiting(&mut self, changes: &mut Vec<Change>) {
        if self.waiting {
            self.waiting = false;
            self.settle(changes);
        }
    }

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

    /// Ends the open turn and command first, then goes to not running with no state between.
    fn stop(&mut self, changes: &mut Vec<Change>) {
        self.close_turn(TurnEnd::Interrupted, changes);
        self.waiting = false;
        changes.push(Change::Ended {
            session: self.known.id.clone(),
        });
        self.set_state(State::NotRunning, changes);
    }
}

#[derive(Debug, Default)]
pub struct Switchboard {
    sessions: Vec<Entry>,
}

impl Switchboard {
    pub fn new() -> Switchboard {
        Switchboard::default()
    }

    pub fn remember(&mut self, session: SessionId, agent: &str, cli: CliSession) {
        if self.find(agent, &cli).is_none() {
            self.sessions.push(Entry::kept(session, agent, cli));
        }
    }

    /// Holds a session kept from an earlier run as not running, and offers it to resume until a
    /// report of it is heard; `cli_record` is its CLI's own record, which the command may need.
    /// A session already held is left as it is.
    pub fn remember_resumable(
        &mut self,
        session: SessionId,
        agent: &str,
        cli: CliSession,
        cli_record: Option<PathBuf>,
    ) {
        if self.find(agent, &cli).is_none() {
            let mut entry = Entry::kept(session, agent, cli);
            entry.offer = Some(cli_record);
            self.sessions.push(entry);
        }
    }

    /// The offers to resume, in the order remembered. A session heard of since is not offered,
    /// nor one whose Adapter `command` gives no command for.
    pub fn resumable(&self, command: &ResumeCommand) -> Vec<Resumable> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let record = entry.offer.as_ref()?;
                let known = &entry.known;
                let command = command(&known.agent, &known.cli, record.as_deref())?;
                Some(Resumable {
                    session: known.id.clone(),
                    agent: known.agent.clone(),
                    cli: known.cli.clone(),
                    command,
                })
            })
            .collect()
    }

    /// `draw` gives a fresh id per call. An event of an ended session is a resume; a step outside a
    /// turn opens none. A turn start while one is open interrupts it.
    pub fn take(
        &mut self,
        agent: &str,
        reported: Reported,
        draw: &mut dyn FnMut() -> String,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut first_heard = false;
        let found = self.find(agent, &reported.session);
        // Any report ends the offer, even the end of a session that is not running.
        if let Some(at) = found {
            self.sessions[at].offer = None;
        }
        let at = match found {
            // No resume: its process went first, or the end came twice.
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

        // A resume keeps the unreviewed mark of a turn nobody reviewed; what it waited for is gone.
        if !entry.running() {
            let resumed = !first_heard;
            let state = if entry.unreviewed {
                State::Unreviewed
            } else {
                State::Idle
            };
            entry.known.state = state;
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
                    state,
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

    pub fn gone(&mut self, pid: u32) -> Vec<Change> {
        let mut changes = Vec::new();
        for entry in &mut self.sessions {
            if entry.running() && entry.known.pid == Some(pid) {
                entry.stop(&mut changes);
            }
        }
        changes
    }

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

    pub fn session(&self, session: &SessionId) -> Option<Known> {
        self.sessions
            .iter()
            .find(|entry| entry.known.id == *session)
            .map(|entry| entry.known.clone())
    }

    pub fn sessions(&self) -> Vec<Known> {
        self.sessions
            .iter()
            .map(|entry| entry.known.clone())
            .collect()
    }

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
mod steps;

#[cfg(test)]
mod tests {
    use super::*;
    use yaar_agent::{Start, WriteTool};

    const AGENT: &str = "claude";

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
            cli_record: None,
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

        fn state(&self, cli: &str) -> State {
            let id = self.id(cli);
            self.board.session(&id).expect("the session").state
        }
    }

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
        assert_eq!(desk.hear("a", started()), []);
        assert_eq!(desk.draws, 2);
    }

    #[test]
    fn the_state_follows_a_turn_from_its_start_to_the_mark_of_reviewed() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        let id = desk.id("a");
        assert_eq!(desk.state("a"), State::Idle);

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

        assert_eq!(
            desk.hear("a", permission(true)),
            [now(&id, State::WaitsForPermission)]
        );
        assert_eq!(desk.state("a"), State::WaitsForPermission);
        assert_eq!(desk.hear("a", permission(true)), []);

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

        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)),
            [
                ends(&id, &turn, TurnEnd::Finished),
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);
        assert_eq!(desk.known("a").turn, None);

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
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        assert_eq!(
            desk.hear("a", Event::SessionEnded),
            [ended(&id), now(&id, State::NotRunning)]
        );
        assert_eq!(desk.state("a"), State::NotRunning);
        assert_eq!(desk.known("a").turn, None);
        assert!(desk.board.pids().is_empty());

        let mut other = Desk::new();
        other.hear("b", started());
        other.hear("b", Event::TurnStarted);
        let id = other.id("b");
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
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)).last(),
            Some(&now(&id, State::Unreviewed))
        );
    }

    #[test]
    fn a_turn_started_while_the_session_waits_with_no_turn_open_works() {
        // The hook socket and the terminal's mark are separate channels: a permission prompt can be
        // heard with no turn open, after an interrupt already ended it.
        let mut desk = Desk::new();
        desk.hear("a", started());
        let id = desk.id("a");
        assert_eq!(
            desk.hear("a", permission(true)),
            [now(&id, State::WaitsForPermission)]
        );
        assert_eq!(desk.state("a"), State::WaitsForPermission);

        assert_eq!(
            desk.hear("a", Event::TurnStarted),
            [
                Change::TurnStarted {
                    session: id.clone(),
                    turn: turn_drawn(2),
                },
                now(&id, State::Works),
            ]
        );
        assert_eq!(desk.state("a"), State::Works);
    }

    #[test]
    fn a_step_heard_with_no_turn_open_ends_the_wait_it_answers() {
        let steps = [
            ("a write", written("p.txt")),
            ("a read", read("p.txt")),
            ("a command that began", command(Phase::Began)),
        ];
        for (what, step) in steps {
            let mut desk = Desk::new();
            desk.hear("a", started());
            let id = desk.id("a");
            assert_eq!(
                desk.hear("a", permission(true)),
                [now(&id, State::WaitsForPermission)],
                "{what}: the prompt"
            );
            assert_eq!(desk.hear("a", step), [now(&id, State::Idle)], "{what}");
            assert_eq!(desk.state("a"), State::Idle, "{what}");
            assert_eq!(desk.known("a").turn, None, "{what}");
        }
    }

    #[test]
    fn a_turn_ended_with_no_turn_open_changes_nothing_not_even_a_wait() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        let id = desk.id("a");
        desk.hear("a", permission(true));
        assert_eq!(desk.hear("a", turn_ended(TurnEnd::Finished)), []);
        assert_eq!(desk.hear("a", turn_ended(TurnEnd::Interrupted)), []);
        assert_eq!(desk.state("a"), State::WaitsForPermission);

        assert_eq!(desk.hear("a", written("p.txt")), [now(&id, State::Idle)]);
    }

    #[test]
    fn a_turn_ended_with_no_turn_open_gives_nothing() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        assert_eq!(desk.hear("a", turn_ended(TurnEnd::Finished)), []);
        assert_eq!(desk.state("a"), State::Idle);

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
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);
        assert_eq!(desk.known("a").pid, Some(200));
        assert_eq!(desk.known("a").terminal.as_deref(), Some("terminal-new"));
        assert_eq!(desk.board.pids(), [200]);
        assert_eq!(desk.draws, 2, "a resume draws no id for the session");
    }

    #[test]
    fn a_session_that_ended_at_a_permission_prompt_does_not_wait_when_it_resumes() {
        for ended_by_its_process in [false, true] {
            let (mut desk, id, turn) = in_a_turn("a");
            desk.hear("a", permission(true));
            assert_eq!(desk.state("a"), State::WaitsForPermission);
            let ending = if ended_by_its_process {
                desk.board.gone(100)
            } else {
                desk.hear("a", Event::SessionEnded)
            };
            assert_eq!(ending.last(), Some(&now(&id, State::NotRunning)));
            assert!(ending.contains(&ends(&id, &turn, TurnEnd::Interrupted)));

            assert_eq!(
                desk.hear("a", started()),
                [
                    Change::Started {
                        session: id.clone(),
                        agent: "claude".to_string(),
                        cli: CliSession("a".to_string()),
                        terminal: Some("terminal-a".to_string()),
                        folder: Some(PathBuf::from("/work/project")),
                        resumed: true,
                    },
                    now(&id, State::Unreviewed)
                ]
            );
            assert_eq!(desk.state("a"), State::Unreviewed);
            assert_eq!(desk.board.reviewed(&id), [now(&id, State::Idle)]);
            assert_eq!(desk.state("a"), State::Idle);
            let again = desk.hear("a", Event::TurnStarted);
            assert_eq!(again.last(), Some(&now(&id, State::Works)));
            assert_eq!(desk.board.reviewed(&id), []);
            assert_eq!(desk.state("a"), State::Works);
        }
    }

    #[test]
    fn a_session_resumed_after_an_unreviewed_turn_is_unreviewed_until_it_is_marked_reviewed() {
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.hear("a", Event::SessionEnded);
        assert_eq!(
            desk.hear("a", started()),
            [
                Change::Started {
                    session: id.clone(),
                    agent: "claude".to_string(),
                    cli: CliSession("a".to_string()),
                    terminal: Some("terminal-a".to_string()),
                    folder: Some(PathBuf::from("/work/project")),
                    resumed: true,
                },
                now(&id, State::Unreviewed)
            ]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);

        assert_eq!(
            desk.hear("a", permission(true)),
            [now(&id, State::WaitsForPermission)]
        );
        assert_eq!(
            desk.hear("a", permission(false)),
            [now(&id, State::Unreviewed)]
        );
        assert_eq!(desk.state("a"), State::Unreviewed);

        assert_eq!(desk.board.reviewed(&id), [now(&id, State::Idle)]);
        assert_eq!(desk.state("a"), State::Idle);
    }

    #[test]
    fn a_session_resumed_with_nothing_unreviewed_is_idle() {
        let mut desk = Desk::new();
        desk.hear("a", started());
        let id = desk.id("a");
        desk.hear("a", Event::SessionEnded);
        assert_eq!(
            desk.hear("a", started()).last(),
            Some(&now(&id, State::Idle))
        );
        assert_eq!(desk.state("a"), State::Idle);

        desk.hear("a", Event::TurnStarted);
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.board.reviewed(&id);
        desk.hear("a", Event::SessionEnded);
        assert_eq!(
            desk.hear("a", started()).last(),
            Some(&now(&id, State::Idle))
        );
        assert_eq!(desk.state("a"), State::Idle);
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
                now(&id, State::Unreviewed),
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

    fn resume_words(agent: &str, cli: &CliSession, record: Option<&Path>) -> Option<String> {
        if agent == "none" {
            return None;
        }
        let record = record.map_or("-".to_string(), |path| path.display().to_string());
        Some(format!("{agent} resume {} {record}", cli.0))
    }

    fn cli(id: &str) -> CliSession {
        CliSession(id.to_string())
    }

    fn offered(board: &Switchboard) -> Vec<Resumable> {
        board.resumable(&resume_words)
    }

    fn sessions_offered(board: &Switchboard) -> Vec<SessionId> {
        offered(board).into_iter().map(|o| o.session).collect()
    }

    /// A and B were running when the Keeper stopped; C ended by its CLI's word.
    fn after_a_restart() -> Switchboard {
        let mut board = Switchboard::new();
        board.remember_resumable(SessionId("sa".to_string()), "claude", cli("a-1"), None);
        board.remember_resumable(
            SessionId("sb".to_string()),
            "pi",
            cli("b-1"),
            Some(PathBuf::from("/s/b-1.jsonl")),
        );
        board.remember(SessionId("sc".to_string()), "claude", cli("c-1"));
        board
    }

    #[test]
    fn after_a_restart_each_session_that_was_running_is_offered_with_its_command_and_id() {
        let mut desk = Desk::new();
        desk.board = after_a_restart();
        let sa = SessionId("sa".to_string());
        let sb = SessionId("sb".to_string());

        assert_eq!(
            offered(&desk.board),
            [
                Resumable {
                    session: sa.clone(),
                    agent: "claude".to_string(),
                    cli: cli("a-1"),
                    command: "claude resume a-1 -".to_string(),
                },
                Resumable {
                    session: sb.clone(),
                    agent: "pi".to_string(),
                    cli: cli("b-1"),
                    command: "pi resume b-1 /s/b-1.jsonl".to_string(),
                },
            ]
        );

        desk.hear(
            "a-1",
            Event::SessionStarted {
                how: Start::Resumed,
            },
        );
        assert_eq!(sessions_offered(&desk.board), std::slice::from_ref(&sb));

        let changes = desk.board.gone(100);
        assert!(changes.contains(&ended(&sa)), "{changes:?}");
        assert_eq!(sessions_offered(&desk.board), std::slice::from_ref(&sb));

        desk.board
            .remember_resumable(SessionId("sd".to_string()), "none", cli("d-1"), None);
        assert_eq!(sessions_offered(&desk.board), [sb]);
    }

    #[test]
    fn a_session_heard_before_or_remembered_before_is_left_as_it_is() {
        let mut desk = Desk::new();
        desk.hear("h", started());
        let heard = desk.id("h");
        desk.board.gone(100);
        desk.board
            .remember(SessionId("sc".to_string()), "claude", cli("c-1"));

        desk.board
            .remember_resumable(SessionId("x".to_string()), "claude", cli("h"), None);
        desk.board
            .remember_resumable(SessionId("y".to_string()), "claude", cli("c-1"), None);
        assert!(offered(&desk.board).is_empty());
        assert_eq!(desk.board.session(&heard).expect("heard").cli, cli("h"));
        assert_eq!(desk.board.sessions().len(), 2);

        desk.board.remember_resumable(
            SessionId("sa".to_string()),
            "pi",
            cli("p"),
            Some(PathBuf::from("/s/one.jsonl")),
        );
        desk.board.remember_resumable(
            SessionId("other".to_string()),
            "pi",
            cli("p"),
            Some(PathBuf::from("/s/two.jsonl")),
        );
        let offers = offered(&desk.board);
        assert_eq!(offers.len(), 1, "{offers:?}");
        assert_eq!(offers[0].session, SessionId("sa".to_string()));
        assert_eq!(offers[0].command, "pi resume p /s/one.jsonl");

        desk.board
            .remember(SessionId("later".to_string()), "pi", cli("p"));
        assert_eq!(offered(&desk.board).len(), 1);
    }

    #[test]
    fn a_remembered_session_is_a_not_running_one_the_cli_resumes_under_its_id() {
        let mut desk = Desk::new();
        desk.board = after_a_restart();
        let sb = SessionId("sb".to_string());
        let known = desk.board.session(&sb).expect("the session");
        assert_eq!(known.state, State::NotRunning);
        assert_eq!(known.agent, "pi");
        assert!(desk.board.pids().is_empty());

        let changes = desk.take(
            "pi",
            reported(
                "b-1",
                7,
                Event::SessionStarted {
                    how: Start::Resumed,
                },
            ),
        );
        assert!(
            matches!(&changes[0], Change::Started { session, resumed: true, .. } if *session == sb),
            "{changes:?}"
        );
        assert_eq!(desk.draws, 0, "it keeps the id the Store gave it");
    }

    #[test]
    fn any_report_of_a_remembered_session_takes_its_offer_back_and_only_its_own() {
        let first = |event| {
            let mut desk = Desk::new();
            desk.board = after_a_restart();
            desk.take("claude", reported("a-1", 7, event));
            desk
        };
        let sb = SessionId("sb".to_string());

        for event in [Event::TurnStarted, read("/x"), permission(true)] {
            let desk = first(event);
            assert_eq!(sessions_offered(&desk.board), std::slice::from_ref(&sb));
        }
        let desk = first(Event::SessionEnded);
        assert_eq!(sessions_offered(&desk.board), std::slice::from_ref(&sb));

        let mut desk = Desk::new();
        desk.board = after_a_restart();
        desk.take("pi", reported("a-1", 7, started()));
        assert_eq!(desk.board.resumable(&resume_words).len(), 2);
        desk.take("claude", reported("b-1", 7, started()));
        assert_eq!(desk.board.resumable(&resume_words).len(), 2);
    }

    #[test]
    fn a_session_that_ran_again_and_ended_is_not_offered_and_asking_takes_nothing() {
        let mut desk = Desk::new();
        desk.board = after_a_restart();
        let sa = SessionId("sa".to_string());
        let sb = SessionId("sb".to_string());
        assert_eq!(sessions_offered(&desk.board), [sa.clone(), sb.clone()]);
        assert_eq!(sessions_offered(&desk.board), [sa.clone(), sb.clone()]);

        desk.hear("a-1", started());
        desk.hear("a-1", Event::SessionEnded);
        assert_eq!(desk.state("a-1"), State::NotRunning);
        assert_eq!(sessions_offered(&desk.board), [sb]);
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
        let (mut desk, id, _) = in_a_turn("a");
        desk.hear("a", turn_ended(TurnEnd::Finished));
        desk.hear("a", Event::TurnStarted);
        assert_eq!(desk.board.reviewed(&id), []);
        assert_eq!(desk.state("a"), State::Works);
        assert_eq!(
            desk.hear("a", turn_ended(TurnEnd::Finished)).last(),
            Some(&now(&id, State::Unreviewed))
        );
        desk.board.gone(100);
        assert_eq!(desk.board.reviewed(&id), []);
        assert_eq!(desk.state("a"), State::NotRunning);
        assert_eq!(desk.board.reviewed(&SessionId("unknown".to_string())), []);
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

    /// A random walk over three sessions: told states equal held states, none is told twice in a
    /// row, and a command ends before its turn.
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
        assert_eq!(seen.len(), ORDER.len(), "{seen:?}");
    }

    /// The display order of the states, written out so that the test does not lean on the
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
