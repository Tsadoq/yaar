//! The agents of every workspace and the terminals the Keeper holds, kept by facts.

use gpui_kit::*;
use yaar_link::{
    AgentCli, Fact, Reply, Request, Session, SessionId, SessionState, TerminalId, TerminalInfo,
    WorkspaceAgents, WorkspaceId,
};

use crate::link_model::{KeeperLink, LinkStatus};

/// The agents of every workspace, and the CLIs the Keeper has an Adapter for, asked once per
/// connection since they never change while it runs. Only the answer to the last list question
/// counts; an `AgentsChanged` heard while one is on its way has the list asked again.
pub struct AgentList {
    link: Entity<KeeperLink>,
    agents: Vec<WorkspaceAgents>,
    clis: Vec<AgentCli>,
    listing_clis: Option<Task<()>>,
    listing: Option<Task<()>>,
    awaits_list: bool,
    chosen: Option<String>,
}

impl AgentList {
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link notifies only when an outcome is recorded, so connected here means a connection
        // not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
                list.ask_clis(cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::AgentsChanged(agents) = fact {
                list.take_one(agents.clone(), cx);
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
            clis: Vec::new(),
            listing_clis: None,
            listing: None,
            awaits_list: false,
            chosen: None,
        };
        if connected {
            list.refresh(cx);
            list.ask_clis(cx);
        }
        list
    }

    fn ask_clis(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListClis, cx));
        self.listing_clis = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                if let Ok(Reply::Clis(clis)) = answer
                    && clis != list.clis
                {
                    list.clis = clis;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub fn clis(&self) -> &[AgentCli] {
        &self.clis
    }

    /// Kept by the CLI's name: a list asked again keeps the choice while the CLI is in it, else the
    /// first.
    pub fn chosen_cli(&self) -> Option<&AgentCli> {
        self.chosen
            .as_ref()
            .and_then(|name| self.clis.iter().find(|cli| cli.agent == *name))
            .or_else(|| self.clis.first())
    }

    pub fn choose_next_cli(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.chosen_cli() else {
            return;
        };
        let at = self
            .clis
            .iter()
            .position(|cli| cli == current)
            .unwrap_or_default();
        let next = self.clis[(at + 1) % self.clis.len()].agent.clone();
        if self.chosen.as_ref() != Some(&next) {
            self.chosen = Some(next);
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListAgents, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                if let Ok(Reply::Agents(agents)) = answer {
                    list.take_all(agents, cx);
                }
            })
            .ok();
        }));
    }

    fn take_all(&mut self, agents: Vec<WorkspaceAgents>, cx: &mut Context<Self>) {
        if agents != self.agents {
            self.agents = agents;
            cx.notify();
        }
    }

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

    pub fn all(&self) -> &[WorkspaceAgents] {
        &self.agents
    }

    pub fn of(&self, workspace: &WorkspaceId) -> Option<&WorkspaceAgents> {
        self.agents
            .iter()
            .find(|agents| agents.workspace == *workspace)
    }

    pub fn shown(&self, workspace: &WorkspaceId) -> Option<SessionState> {
        self.of(workspace).and_then(|agents| agents.shown)
    }

    pub fn session(&self, session: &SessionId) -> Option<&Session> {
        self.agents
            .iter()
            .flat_map(|agents| &agents.sessions)
            .find(|found| found.id == *session)
    }

    pub fn in_terminal(&self, terminal: &TerminalId) -> Option<&Session> {
        self.agents
            .iter()
            .flat_map(|agents| &agents.sessions)
            .find(|found| found.terminal.as_ref() == Some(terminal))
    }
}

/// A terminal a pane wants opened. No pane calls another, so the wish goes through `TerminalList`,
/// which emits it to the terminal pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// The launcher of an agent CLI for a review the user attached, when the review's agent does
    /// not run or it belongs to no agent.
    AgentFor {
        workspace: yaar_link::WorkspaceId,
        agent: String,
        review: yaar_link::ReviewId,
    },
    Worktree {
        repository: yaar_link::RepositoryId,
        from: String,
        branch: String,
        agent: String,
    },
}

/// The terminals the Keeper holds: those listed when the link connected (as after a plain quit),
/// then those this Window opened. A terminal leaves at its `Fact::TerminalEnded`.
pub struct TerminalList {
    link: Entity<KeeperLink>,
    terminals: Vec<TerminalInfo>,
    listing: Option<Task<()>>,
    awaits_list: bool,
}

impl EventEmitter<Wanted> for TerminalList {}

impl TerminalList {
    pub fn want(&mut self, wanted: Wanted, cx: &mut Context<Self>) {
        cx.emit(wanted);
    }

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

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListTerminals, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
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

    /// A terminal ended or was opened: an answer still on its way may not know it, so the list is
    /// asked again.
    fn ask_again_if_awaited(&mut self, cx: &mut Context<Self>) {
        if self.awaits_list {
            self.refresh(cx);
        }
    }

    pub fn terminals(&self) -> &[TerminalInfo] {
        &self.terminals
    }

    pub fn of(&self, workspace: &WorkspaceId) -> Vec<&TerminalInfo> {
        self.terminals
            .iter()
            .filter(|terminal| terminal.workspace == *workspace)
            .collect()
    }

    pub fn opened(&mut self, terminal: TerminalInfo, cx: &mut Context<Self>) {
        if !self.terminals.iter().any(|held| held.id == terminal.id) {
            self.terminals.push(terminal);
            cx.notify();
        }
        self.ask_again_if_awaited(cx);
    }
}
