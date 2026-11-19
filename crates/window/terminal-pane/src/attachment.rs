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

use std::collections::HashMap;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::{AskError, Wanted};
use yaar_link::{Attachment, Fact, Program, Reply, Request, ReviewId, SessionState, TerminalId};

use crate::{AddressEachComment, DetachAttachment, TerminalPane, ViewAttachment};

/// The sentence the one control types at the agent's prompt.
pub const ADDRESS_EACH_COMMENT: &str = "Address each comment.";

/// The height of the line the controls sit in, which the text of the view stops above.
const LINE: Pixels = px(28.);

/// The view of one terminal's attachment, while it is open.
struct View {
    /// What the Keeper last answered; none before the first answer.
    text: Option<String>,
    /// The review and what of it was attached when the text was last asked for, so that a
    /// change of either asks again.
    asked_for: (ReviewId, Attachment),
    /// The question in flight. A newer one takes its place, which drops the older answer:
    /// the text shown is always the answer to the last question.
    asking: Option<Task<()>>,
    /// What tells the view the reviews or the attachment changed. Dropped with the view.
    _subscriptions: [Subscription; 2],
}

/// What the pane keeps of the attachments it shows: the views that are open, by terminal.
#[derive(Default)]
pub(crate) struct Attachments {
    views: HashMap<TerminalId, View>,
}

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
        self.models
            .agents
            .read(cx)
            .in_terminal(terminal)
            .is_some_and(|session| {
                session.attached.is_some()
                    && !matches!(
                        session.state,
                        SessionState::WaitsForPermission | SessionState::NotRunning
                    )
            })
    }

    /// The text the view of the attachment of `terminal` shows while it is open, as the
    /// Keeper answered it; none while it is closed, or before the answer came.
    pub fn viewed_attachment(&self, terminal: &TerminalId) -> Option<String> {
        self.attachments.views.get(terminal)?.text.clone()
    }

    /// The review of the session that runs in `terminal`, and what of it is attached to
    /// that session's next message; none when nothing is attached.
    fn attached_review(&self, terminal: &TerminalId, cx: &App) -> Option<(ReviewId, Attachment)> {
        let agents = self.models.agents.read(cx);
        let session = agents.in_terminal(terminal)?;
        Some((session.review.clone()?, session.attached.clone()?))
    }

    /// Opens the view of the attachment of `terminal`, which asks the Keeper for its text,
    /// or closes it; nothing while nothing is attached.
    fn toggle_view(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        if self.attachments.views.remove(terminal).is_some() {
            cx.notify();
            return;
        }
        let Some(asked_for) = self.attached_review(terminal, cx) else {
            return;
        };
        // The view keeps what it shows up to date: it asks again when the reviews of the
        // terminal's workspace change, and when what is attached to the session does; it
        // closes when nothing is attached any more.
        let (for_reviews, for_agents) = (terminal.clone(), terminal.clone());
        let reviews = cx.subscribe(&self.models.link, move |pane, _, fact: &Fact, cx| {
            if let Fact::ReviewsChanged { workspace } = fact
                && pane.workspace(&for_reviews).as_ref() == Some(workspace)
                && pane.attachments.views.contains_key(&for_reviews)
            {
                pane.ask_text(&for_reviews, cx);
            }
        });
        let agents = cx.observe(&self.models.agents, move |pane, _, cx| {
            pane.attachment_moved(&for_agents, cx);
        });
        self.attachments.views.insert(
            terminal.clone(),
            View {
                text: None,
                asked_for,
                asking: None,
                _subscriptions: [reviews, agents],
            },
        );
        self.ask_text(terminal, cx);
        cx.notify();
    }

    /// The agents changed: the view of `terminal`, if open, follows what is attached now.
    fn attachment_moved(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        let Some(view) = self.attachments.views.get(terminal) else {
            return;
        };
        match self.attached_review(terminal, cx) {
            None => {
                self.attachments.views.remove(terminal);
                cx.notify();
            }
            Some(now) if now != view.asked_for => self.ask_text(terminal, cx),
            Some(_) => {}
        }
    }

    /// Asks the Keeper for what will be sent with the next message of `terminal`'s agent,
    /// for the open view of it.
    fn ask_text(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        let Some((review, attached)) = self.attached_review(terminal, cx) else {
            self.attachments.views.remove(terminal);
            cx.notify();
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::GetAttachment {
                    review: review.clone(),
                },
                cx,
            )
        });
        let named = terminal.clone();
        let task = cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.text_answered(&named, answer, cx))
                .ok();
        });
        if let Some(view) = self.attachments.views.get_mut(terminal) {
            view.asked_for = (review, attached);
            view.asking = Some(task);
        }
    }

    /// Takes the Keeper's answer to the question for the text of `terminal`'s attachment.
    /// A Keeper that has none to give, because nothing is attached any more or the review
    /// is gone, closes the view: there is nothing to show.
    fn text_answered(
        &mut self,
        terminal: &TerminalId,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.attachments.views.get_mut(terminal) else {
            return;
        };
        match answer {
            Ok(Reply::Attachment { text }) => view.text = Some(text),
            Ok(_) | Err(_) => {
                self.attachments.views.remove(terminal);
            }
        }
        cx.notify();
    }

    /// Sends the detach of the review attached to the next message of `terminal`'s agent;
    /// nothing while nothing is attached. The agents change when the Keeper has done it.
    fn detach(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        let Some((review, _)) = self.attached_review(terminal, cx) else {
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::DetachReview { review }, cx)
        });
        // Detached, not kept: a task that is dropped is cancelled, and its answer lost.
        cx.spawn(async move |_, _| {
            asked.await.ok();
        })
        .detach();
    }

    /// Types [`ADDRESS_EACH_COMMENT`] at the prompt of `terminal`'s program as the user's
    /// own keys would be: the bytes of the sentence and nothing more, so that the user
    /// words the rest and presses Enter. Nothing while the control is not offered.
    fn type_sentence(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        if !self.offers_address(terminal, cx) {
            return;
        }
        self.models.link.update(cx, |link, _| {
            link.send_input(terminal, ADDRESS_EACH_COMMENT.as_bytes().to_vec())
        });
    }

    /// The line of slot `n`, which shows `terminal`, while a review is attached to the
    /// next message of its agent; none otherwise.
    ///
    /// The line lies over the bottom of the slot and the text of the view over its lower
    /// part; both hold the mouse over them, so that a click on a control is no click in
    /// the program, and take the focus to the terminal, where the user presses Enter.
    pub(crate) fn render_attachment(
        &self,
        n: usize,
        terminal: &TerminalId,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let attached = self.attachment(terminal, cx)?;
        let theme = cx.theme();
        let words = match attached {
            Attachment::Review => "Review attached to your next message",
            Attachment::Thread(_) => "One thread attached to your next message",
        };
        let control = |name: &'static str, label: &'static str, primary: bool| {
            let button = div()
                .id(SharedString::from(format!(
                    "terminal-attachment-{name}-{n}"
                )))
                .debug_selector(move || format!("terminal-attachment-{name}-{n}"))
                .flex_none()
                .px_2()
                .rounded_sm()
                .cursor_pointer()
                .text_xs();
            if primary {
                button
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .child(label)
            } else {
                button
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .child(label)
            }
        };
        let click = |act: fn(&mut Self, &TerminalId, &mut Context<Self>)| {
            let terminal = terminal.clone();
            cx.listener(move |pane, _: &ClickEvent, window, cx| {
                pane.click_slot(n, window, cx);
                act(pane, &terminal, cx);
            })
        };
        let line = div()
            .debug_selector(move || format!("terminal-attachment-{n}"))
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(LINE)
            .px_2()
            .gap_2()
            .flex()
            .flex_row()
            .items_center()
            .occlude()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .border_t_1()
            .border_color(theme.border)
            .child(div().flex_1().min_w_0().truncate().text_xs().child(words))
            .child(control("view", "View", false).on_click(click(Self::toggle_view)))
            .child(control("detach", "Detach", false).on_click(click(Self::detach)))
            .children(self.offers_address(terminal, cx).then(|| {
                control("address", "Address each comment", true)
                    .on_click(click(Self::type_sentence))
            }));
        let text = self
            .attachments
            .views
            .get(terminal)
            .and_then(|view| view.text.clone())
            .map(|text| {
                div()
                    .id(SharedString::from(format!(
                        "terminal-attachment-scroll-{n}"
                    )))
                    .debug_selector(move || format!("terminal-attachment-text-{n}"))
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(LINE)
                    .max_h_3_4()
                    .overflow_y_scroll()
                    .occlude()
                    .p_2()
                    .bg(theme.background)
                    .text_color(theme.foreground)
                    .text_sm()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(text)
            });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .children(text)
                .child(line)
                .into_any_element(),
        )
    }

    /// A pane wants a terminal opened: for [`Wanted::AgentFor`], the launcher of that CLI
    /// in that workspace for that review, `yaar_link::Program::AgentFor`, as
    /// [`crate::NewTerminal`] opens a shell. It is shown and takes the focus as that
    /// terminal does; the Keeper's refusal is the pane's notice.
    pub(crate) fn agent_wanted(
        &mut self,
        wanted: &Wanted,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Wanted::AgentFor {
            workspace,
            agent,
            review,
        } = wanted;
        self.notice = None;
        let request = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::AgentFor {
                agent: agent.clone(),
                review: review.clone(),
            },
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        let workspace = workspace.clone();
        // Detached, as the question for a shell is: a task that is dropped is cancelled,
        // and its answer lost.
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.opened(workspace, answer, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    /// [`ViewAttachment`]: opens the view of the attachment of the focused terminal, which
    /// asks the Keeper for its text, or closes it.
    pub(crate) fn view_attachment(
        &mut self,
        _: &ViewAttachment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(terminal) = self.focused() {
            self.toggle_view(&terminal, cx);
        }
    }

    /// [`DetachAttachment`]: sends the detach of the review attached to the next message
    /// of the focused terminal's agent.
    pub(crate) fn detach_attachment(
        &mut self,
        _: &DetachAttachment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(terminal) = self.focused() {
            self.detach(&terminal, cx);
        }
    }

    /// [`AddressEachComment`]: while it is offered for the focused terminal, sends
    /// [`ADDRESS_EACH_COMMENT`] to its program, and nothing else; else does nothing.
    pub(crate) fn address_each_comment(
        &mut self,
        _: &AddressEachComment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(terminal) = self.focused() {
            self.type_sentence(&terminal, cx);
        }
    }
}
