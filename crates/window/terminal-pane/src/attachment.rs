//! The review attached to an agent's next message, shown at the agent's terminal: a line
//! over the bottom of the terminal's slot that says what is attached, with a control that
//! views what will be sent, one that detaches it, and, while the agent shows no permission
//! prompt, the one control that types [`ADDRESS_EACH_COMMENT`] at its prompt. Built by the
//! package "the terminal pane" of slice 6.
//!
//! What is attached is the agents model's: `yaar_link::Session::attached` of the session
//! that runs in the terminal. The text the view shows is the Keeper's answer to
//! `yaar_link::Request::GetAttachment`, asked when the view opens and asked again at each
//! `yaar_link::Fact::ReviewsChanged` of the terminal's workspace while it is open, so that
//! an edited or a new draft shows in what will be sent; it closes when the attachment
//! goes. Detaching sends `yaar_link::Request::DetachReview`.
//!
//! The three keys act on the terminal of `TerminalPane::focused`, the one the keys go to
//! while the pane has the focus, wherever the focus is now: on Linux a key of the app does
//! nothing while a terminal has the focus, and the user presses them from another pane.
//! The sentence goes to the program as the bytes of the sentence alone, as keys the user
//! typed would: never a line end, never anything else; the user presses Enter.
//!
//! Drawn, for slot `n`, under the selectors `terminal-attachment-<n>` (the line),
//! `terminal-attachment-view-<n>`, `terminal-attachment-detach-<n>` and, while it is
//! offered, `terminal-attachment-address-<n>` (the controls), and, while the view is open,
//! `terminal-attachment-text-<n>`.

use gpui_kit::*;
use yaar_frame::Wanted;
use yaar_link::{Attachment, TerminalId};

use crate::{AddressEachComment, DetachAttachment, TerminalPane, ViewAttachment};

/// The sentence the one control types at the agent's prompt.
pub const ADDRESS_EACH_COMMENT: &str = "Address each comment.";

/// What the pane keeps of the attachments it shows: whose view is open, and the text the
/// Keeper answered for it. Its fields are the package's to choose.
#[derive(Default)]
pub(crate) struct Attachments {}

impl TerminalPane {
    /// What is attached to the next message of the agent session that runs in `terminal`,
    /// as the agents model says; none when nothing is, or no session runs there.
    pub fn attachment(&self, terminal: &TerminalId, cx: &App) -> Option<Attachment> {
        self.models
            .agents
            .read(cx)
            .in_terminal(terminal)
            .and_then(|session| session.attached.clone())
    }

    /// Whether the terminal offers the control that types [`ADDRESS_EACH_COMMENT`]: a
    /// review is attached to the next message of the session that runs in it, and that
    /// session runs and does not wait at a permission prompt.
    pub fn offers_address(&self, terminal: &TerminalId, cx: &App) -> bool {
        let _ = (terminal, cx);
        todo!()
    }

    /// The text the view of the attachment of `terminal` shows while it is open, as the
    /// Keeper answered it; none while it is closed, or before the answer came.
    pub fn viewed_attachment(&self, terminal: &TerminalId) -> Option<String> {
        let _ = terminal;
        todo!()
    }

    /// The line of slot `n`, which shows `terminal`, while a review is attached to the
    /// next message of its agent; none otherwise. Draws nothing until the package that
    /// builds it fills it in.
    pub(crate) fn render_attachment(
        &self,
        n: usize,
        terminal: &TerminalId,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let _ = (n, terminal, cx);
        None
    }

    /// A pane wants a terminal opened: for [`Wanted::AgentFor`], the launcher of that CLI
    /// in that workspace for that review, `yaar_link::Program::AgentFor`, as
    /// [`crate::NewTerminal`] opens a shell. Does nothing until the package that builds it
    /// fills it in.
    pub(crate) fn agent_wanted(
        &mut self,
        wanted: &Wanted,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (wanted, window, cx, &self.attachments);
    }

    /// [`ViewAttachment`]: opens the view of the attachment of the focused terminal, which
    /// asks the Keeper for its text, or closes it.
    pub(crate) fn view_attachment(
        &mut self,
        _: &ViewAttachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`DetachAttachment`]: sends the detach of the review attached to the next message
    /// of the focused terminal's agent.
    pub(crate) fn detach_attachment(
        &mut self,
        _: &DetachAttachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`AddressEachComment`]: while it is offered for the focused terminal, sends
    /// [`ADDRESS_EACH_COMMENT`] to its program, and nothing else; else does nothing.
    pub(crate) fn address_each_comment(
        &mut self,
        _: &AddressEachComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }
}
