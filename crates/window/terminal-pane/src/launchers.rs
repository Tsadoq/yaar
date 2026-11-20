//! The launchers next to the shell, and the controls that resume the sessions a restart cut
//! off. Built by the package "the terminal pane" of slice 7.
//!
//! A line at the top of the pane, `terminal-launchers`, offers a new shell
//! (`terminal-launcher-shell`, as [`crate::NewTerminal`] opens one) and, next to it, one
//! launcher for each agent CLI the Keeper names (`yaar_frame::AgentList::clis`, in that
//! order), `terminal-launcher-<agent>` with the CLI's title. A click on one opens a terminal
//! for the workspace on screen that runs `yaar_link::Program::Agent` of that CLI, as
//! [`crate::NewTerminal`] opens a shell: the same request, the same size, the same focus
//! and notice. One launcher is the chosen one, the first until the user chooses another,
//! drawn with `terminal-launcher-chosen-<agent>` over it: [`crate::NewAgentTerminal`]
//! opens the chosen CLI's, and [`crate::NextAgentCli`] makes the next one the chosen one,
//! after the last the first. With no CLI named, neither key opens anything, and the notice
//! says so.
//!
//! Under it, while the workspace on screen has sessions the agents model lists as
//! resumable (`yaar_link::Session::resumable`), a line `terminal-resume` offers a control
//! for each, in the order the model lists them, `terminal-resume-<n>`, n counted from 0,
//! with the words `Resume <the session's title>`. A click opens a terminal that runs
//! `yaar_link::Program::Resume` of that session. [`crate::ResumeAgent`] does the same for
//! the first resumable session of the workspace on screen the pane has not opened a resume
//! for since it was listed: a second press reaches the second session, not the first again.
//! The line goes with the last resumable session, as the Keeper tells the agents again
//! when a resumed session reports.
//!
//! Neither line takes the room of a terminal: they lie over the top of the pane's slots
//! when there is one and over the line that says how to open one when there is none, so
//! that the room the terminals are laid out in, and the sizes the Keeper is told, are
//! those of slice 6.

use gpui_kit::*;
use yaar_link::{AgentCli, SessionId};

use crate::{NewAgentTerminal, NextAgentCli, ResumeAgent, TerminalPane};

/// What the pane keeps of its launchers: which CLI is the chosen one, and the sessions it
/// opened a resume for.
#[derive(Default)]
pub(crate) struct Launchers {
    /// The chosen CLI, by its index in the CLIs the Keeper named.
    chosen: usize,
    /// The sessions the pane opened a resume for, until they are listed resumable no more.
    resumed: Vec<SessionId>,
}

impl TerminalPane {
    /// The launchers the pane offers next to the shell: one for each agent CLI the Keeper
    /// names, in that order.
    pub fn launchers(&self, cx: &App) -> Vec<AgentCli> {
        let _ = (cx, &self.launchers.chosen);
        todo!("the launchers next to the shell")
    }

    /// The CLI [`crate::NewAgentTerminal`] opens now; none while the Keeper names none.
    pub fn chosen_cli(&self, cx: &App) -> Option<AgentCli> {
        let _ = cx;
        todo!("the chosen launcher")
    }

    /// The sessions of the workspace on screen the pane offers to resume, in the order the
    /// agents model lists them.
    pub fn resumable(&self, cx: &App) -> Vec<SessionId> {
        let _ = (cx, &self.launchers.resumed);
        todo!("the sessions offered to resume")
    }

    /// The two lines of the module's doc, drawn over the top of the pane; none while
    /// neither has anything to offer. A hook of the contract that draws nothing.
    pub(crate) fn render_launchers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let _ = cx;
        None
    }

    /// What [`crate::NewAgentTerminal`] does. A hook of the contract that does nothing.
    pub(crate) fn new_agent_terminal(
        &mut self,
        _: &NewAgentTerminal,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// What [`crate::NextAgentCli`] does. A hook of the contract that does nothing.
    pub(crate) fn next_agent_cli(
        &mut self,
        _: &NextAgentCli,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// What [`crate::ResumeAgent`] does. A hook of the contract that does nothing.
    pub(crate) fn resume_agent(&mut self, _: &ResumeAgent, _: &mut Window, _: &mut Context<Self>) {}
}
