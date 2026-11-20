//! The question an Adapter asks before it writes into the user's own settings of its CLI,
//! and the user's answer. Built by the package "the Window's question" of slice 7.
//!
//! While the shared model of the questions (`yaar_frame::ConsentList::asked`) holds one, a
//! line `terminal-consent` lies in the pane's top row, under the launchers and the resume
//! controls, above the notice and the terminals. For each question, in the order of the
//! Adapters, it shows the Adapter's own words as they are (`terminal-consent-question-<n>`,
//! n counted from 0), and under them, in the colour of a failure, what went wrong where the
//! CLI's settings could not be changed (`terminal-consent-problem-<n>`). After the last,
//! two controls answer every question shown: `terminal-consent-yes`, "Install", which is
//! [`crate::InstallAgentExtensions`], and `terminal-consent-no`, "No", which is
//! [`crate::KeepAgentExtensionsOut`]. The pane names no CLI: the words are the Adapter's.
//!
//! The question is not modal: every terminal, launcher and key works while it is up, and a
//! launcher of a CLI whose question is unanswered starts the CLI as any program. It is up
//! from the first Window's connect after the Keeper found the CLI installed until it is
//! answered; the Keeper tells every Window the answer, and the line goes in each.
//!
//! The two actions work anywhere, whether a question is shown or not, so that an answer can
//! be changed later: a yes after a no installs, a no after a yes takes the entry out. With
//! no question shown they answer every Adapter the Keeper lists as needing leave; when it
//! lists none, the notice says "No agent CLI asks to install anything." and nothing is sent.
//!
//! The line takes room from the terminals as the resume line does: they are resized when
//! it comes and goes, as when the window is made smaller.

use gpui_kit::*;

use crate::{InstallAgentExtensions, KeepAgentExtensionsOut, TerminalPane};

/// The notice when the user answers and no Adapter needs leave.
pub const NOTHING_TO_INSTALL: &str = "No agent CLI asks to install anything.";

impl TerminalPane {
    /// The line of the module's doc, while a question is shown. A hook of the contract that
    /// draws nothing.
    pub(crate) fn render_consent(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let _ = cx;
        None
    }

    /// What [`crate::InstallAgentExtensions`] does. A hook of the contract that does
    /// nothing.
    pub(crate) fn install_agent_extensions(
        &mut self,
        _: &InstallAgentExtensions,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// What [`crate::KeepAgentExtensionsOut`] does. A hook of the contract that does
    /// nothing.
    pub(crate) fn keep_agent_extensions_out(
        &mut self,
        _: &KeepAgentExtensionsOut,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
}
