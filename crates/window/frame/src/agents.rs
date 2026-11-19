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
    Session, SessionId, SessionState, TerminalId, TerminalInfo, WorkspaceAgents, WorkspaceId,
};

use crate::link_model::KeeperLink;

/// The agent sessions of every workspace in which one reported, as the Keeper told them.
///
/// When the link connects it asks `Request::ListAgents` and takes the answer; each
/// `Fact::AgentsChanged` replaces the entry of its workspace, or adds it. Of two answers,
/// only that to the last question counts, and a fact heard while a question is on its way
/// has the question asked again, as [`crate::WorkspaceList`] does.
pub struct AgentList {
    link: Entity<KeeperLink>,
    agents: Vec<WorkspaceAgents>,
}

impl AgentList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            link,
            agents: Vec::new(),
        }
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
}

impl TerminalList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            link,
            terminals: Vec::new(),
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
        let _ = (terminal, cx);
        todo!("a terminal this Window opened joins the list")
    }
}
