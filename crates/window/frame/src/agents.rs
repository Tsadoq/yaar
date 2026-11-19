//! Two shared models of slice 4: the agents of every workspace, and the terminals the
//! Keeper holds.
//!
//! Each is filled from the Keeper when the link connects, and kept in step by the facts:
//! [`AgentList`] by `Fact::AgentsChanged`, [`TerminalList`] by `Fact::TerminalEnded`. The
//! sidebar reads the first for the state of each workspace, the terminal pane for the
//! waiting mark of an agent's terminal, and the review pane for whose reviews there are;
//! the terminal pane reads the second for the terminals to show again in a Window opened
//! after a plain quit.

use gpui_kit::*;
use yaar_link::{
    Fact, Reply, Request, Session, SessionId, SessionState, TerminalId, TerminalInfo,
    WorkspaceAgents, WorkspaceId,
};

use crate::link_model::{KeeperLink, LinkStatus};

/// The agent sessions of every workspace in which one reported, as the Keeper told them.
///
/// When the link connects it asks `Request::ListAgents` and takes the answer; each
/// `Fact::AgentsChanged` replaces the entry of its workspace, or adds it. Of two answers,
/// only that to the last question counts, and a fact heard while a question is on its way
/// has the question asked again, as [`crate::WorkspaceList`] does.
pub struct AgentList {
    link: Entity<KeeperLink>,
    agents: Vec<WorkspaceAgents>,
    /// The last question for the list. A new one takes its place, which drops the answer
    /// to the old one.
    listing: Option<Task<()>>,
    /// Whether the answer to that question is still awaited.
    awaits_list: bool,
}

impl AgentList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link tells of a change only when the outcome of reaching a Keeper is
        // recorded, so being connected then means a connection that was not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::AgentsChanged(agents) = fact {
                list.take_one(agents.clone(), cx);
                // An answer still on its way may have been made before this fact was.
                // It is not waited for; the question is asked again, and what answers
                // it is no older than this fact.
                if list.awaits_list {
                    list.refresh(cx);
                }
            }
        })
        .detach();

        let connected = matches!(link.read(cx).status(), LinkStatus::Connected(_));
        let mut list = Self {
            link,
            agents: Vec::new(),
            listing: None,
            awaits_list: false,
        };
        if connected {
            list.refresh(cx);
        }
        list
    }

    /// Asks the Keeper for every workspace's agents, and takes the answer unless a later
    /// question took its place.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListAgents, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                // A Keeper that refuses, as the one of the contract does, leaves the
                // list as it was: no workspace shows an agent.
                if let Ok(Reply::Agents(agents)) = answer {
                    list.take_all(agents, cx);
                }
            })
            .ok();
        }));
    }

    /// Takes the list as the Keeper answered it.
    fn take_all(&mut self, agents: Vec<WorkspaceAgents>, cx: &mut Context<Self>) {
        if agents != self.agents {
            self.agents = agents;
            cx.notify();
        }
    }

    /// Takes what the Keeper told of one workspace: it replaces that workspace's entry,
    /// or comes last.
    fn take_one(&mut self, told: WorkspaceAgents, cx: &mut Context<Self>) {
        match self
            .agents
            .iter_mut()
            .find(|agents| agents.workspace == told.workspace)
        {
            Some(held) if *held == told => return,
            Some(held) => *held = told,
            None => self.agents.push(told),
        }
        cx.notify();
    }

    /// The link this list is fed by.
    pub fn link(&self) -> &Entity<KeeperLink> {
        &self.link
    }

    /// Every workspace in which a session reported, with its sessions.
    pub fn all(&self) -> &[WorkspaceAgents] {
        &self.agents
    }

    /// The sessions of one workspace, and the state it shows; none when no session of a
    /// supported agent CLI ever reported in it.
    pub fn of(&self, workspace: &WorkspaceId) -> Option<&WorkspaceAgents> {
        self.agents
            .iter()
            .find(|agents| agents.workspace == *workspace)
    }

    /// The state the sidebar shows for `workspace`: none for a workspace with no session.
    pub fn shown(&self, workspace: &WorkspaceId) -> Option<SessionState> {
        self.of(workspace).and_then(|agents| agents.shown)
    }

    /// One session, wherever it works.
    pub fn session(&self, session: &SessionId) -> Option<&Session> {
        self.agents
            .iter()
            .flat_map(|agents| &agents.sessions)
            .find(|found| found.id == *session)
    }

    /// The session that runs in `terminal`, if one does.
    pub fn in_terminal(&self, terminal: &TerminalId) -> Option<&Session> {
        self.agents
            .iter()
            .flat_map(|agents| &agents.sessions)
            .find(|found| found.terminal.as_ref() == Some(terminal))
    }
}

/// The terminals the Keeper holds: those it listed when the link connected, as after a
/// plain quit, and those this Window opened since, in the order they were opened. A
/// terminal whose program ended leaves it when `Fact::TerminalEnded` is told.
pub struct TerminalList {
    link: Entity<KeeperLink>,
    terminals: Vec<TerminalInfo>,
    /// The last question for the list. A new one takes its place, which drops the answer
    /// to the old one.
    listing: Option<Task<()>>,
    /// Whether the answer to that question is still awaited.
    awaits_list: bool,
}

impl TerminalList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::TerminalEnded { terminal, .. } = fact {
                list.terminals.retain(|held| held.id != *terminal);
                cx.notify();
                list.ask_again_if_awaited(cx);
            }
        })
        .detach();

        let connected = matches!(link.read(cx).status(), LinkStatus::Connected(_));
        let mut list = Self {
            link,
            terminals: Vec::new(),
            listing: None,
            awaits_list: false,
        };
        if connected {
            list.refresh(cx);
        }
        list
    }

    /// Asks the Keeper for its terminals, and takes the answer unless a later question
    /// took its place.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListTerminals, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                // A Keeper that refuses, as the one of the contract does, leaves the
                // list as it was.
                if let Ok(Reply::Terminals(terminals)) = answer
                    && terminals != list.terminals
                {
                    list.terminals = terminals;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// A terminal ended or was opened, which an answer still on its way may not know:
    /// it is not waited for. The question is asked again, and what answers it is no
    /// older than the change.
    fn ask_again_if_awaited(&mut self, cx: &mut Context<Self>) {
        if self.awaits_list {
            self.refresh(cx);
        }
    }

    /// The link this list is fed by.
    pub fn link(&self) -> &Entity<KeeperLink> {
        &self.link
    }

    /// Every terminal listed, in the order they were opened.
    pub fn terminals(&self) -> &[TerminalInfo] {
        &self.terminals
    }

    /// The terminals of one workspace, in the order they were opened.
    pub fn of(&self, workspace: &WorkspaceId) -> Vec<&TerminalInfo> {
        self.terminals
            .iter()
            .filter(|terminal| terminal.workspace == *workspace)
            .collect()
    }

    /// This Window opened a terminal: it joins the list, at the end, once.
    pub fn opened(&mut self, terminal: TerminalInfo, cx: &mut Context<Self>) {
        if !self.terminals.iter().any(|held| held.id == terminal.id) {
            self.terminals.push(terminal);
            cx.notify();
        }
        self.ask_again_if_awaited(cx);
    }
}
