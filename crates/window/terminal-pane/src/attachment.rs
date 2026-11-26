//! The review attached to an agent's next message: a line over the terminal's slot with controls.
//! The keys act on `TerminalPane::focused`, wherever the focus is: on Linux an app key does nothing
//! while a terminal has the focus.

use std::collections::HashMap;

use gpui_kit::*;
use yaar_frame::{AskError, Wanted};
use yaar_link::{Attachment, Fact, Program, Reply, Request, ReviewId, SessionState, TerminalId};
use yaar_ui::metrics::{SPACE_3, SPACE_4, SPACE_5};
use yaar_ui::pieces::{Button, ButtonKind};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::{
    AddressEachComment, DetachAttachment, TerminalPane, ViewAttachment, WORKTREE_BEING_MADE,
};

pub const ADDRESS_EACH_COMMENT: &str = "Address each comment.";

const LINE: Pixels = px(28.);

struct View {
    text: Option<String>,
    asked_for: (ReviewId, Attachment),
    /// A newer question replaces this one, which drops the older answer.
    asking: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

#[derive(Default)]
pub(crate) struct Attachments {
    views: HashMap<TerminalId, View>,
}

impl Attachments {
    pub(crate) fn remove(&mut self, terminal: &TerminalId) {
        self.views.remove(terminal);
    }
}

impl TerminalPane {
    pub fn attachment(&self, terminal: &TerminalId, cx: &App) -> Option<Attachment> {
        self.models
            .agents
            .read(cx)
            .in_terminal(terminal)
            .and_then(|session| session.attached.clone())
    }

    /// Whether a review is attached and the session runs, not waiting at a permission prompt.
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

    pub fn viewed_attachment(&self, terminal: &TerminalId) -> Option<String> {
        self.attachments.views.get(terminal)?.text.clone()
    }

    fn attached_review(&self, terminal: &TerminalId, cx: &App) -> Option<(ReviewId, Attachment)> {
        let agents = self.models.agents.read(cx);
        let session = agents.in_terminal(terminal)?;
        Some((session.review.clone()?, session.attached.clone()?))
    }

    fn toggle_view(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        if self.attachments.views.remove(terminal).is_some() {
            cx.notify();
            return;
        }
        let Some(asked_for) = self.attached_review(terminal, cx) else {
            return;
        };
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

    fn type_sentence(&mut self, terminal: &TerminalId, cx: &mut Context<Self>) {
        if !self.offers_address(terminal, cx) {
            return;
        }
        self.models.link.update(cx, |link, _| {
            link.send_input(terminal, ADDRESS_EACH_COMMENT.as_bytes().to_vec())
        });
    }

    /// The line and the view text take the mouse, so a click on a control is no click in the
    /// program.
    pub(crate) fn render_attachment(
        &self,
        n: usize,
        terminal: &TerminalId,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let attached = self.attachment(terminal, cx)?;
        let palette = palette(cx);
        let words = match attached {
            Attachment::Review => "Review attached to your next message",
            Attachment::Thread(_) => "One thread attached to your next message",
        };
        let control = |name: &'static str, label: &'static str, primary: bool| {
            let selector = format!("terminal-attachment-{name}-{n}");
            Button::new(SharedString::from(selector.clone()), label)
                .kind(if primary {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                })
                .small()
                .selector(selector)
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
            .px(SPACE_5)
            .gap(SPACE_3)
            .flex()
            .flex_row()
            .items_center()
            .occlude()
            .bg(palette.window)
            .text_color(palette.text)
            .border_t_1()
            .border_color(palette.separator)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typed(TypeRole::Meta)
                    .child(words),
            )
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
                    .p(SPACE_4)
                    .bg(palette.pane)
                    .text_color(palette.text)
                    .typed(TypeRole::Body)
                    .border_t_1()
                    .border_color(palette.separator)
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

    pub(crate) fn worktree_wanted(&mut self, wanted: &Wanted, cx: &mut Context<Self>) {
        let Wanted::Worktree {
            repository,
            from,
            branch,
            agent,
        } = wanted
        else {
            return;
        };
        self.notice = Some(format!("{WORKTREE_BEING_MADE} {branch}"));
        let request = Request::CreateWorktree {
            repository: repository.clone(),
            from: from.clone(),
            branch: branch.clone(),
            agent: agent.clone(),
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        let program = Program::Agent {
            agent: agent.clone(),
        };
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.worktree_made(program, answer, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    /// The Frame is asked to show the new workspace: at once when its list holds it, else when a
    /// list that holds it arrives. The agent's terminal has the focus once it is on screen.
    fn worktree_made(
        &mut self,
        program: Program,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(Reply::WorktreeCreated {
                workspace,
                terminal,
            }) => {
                self.notice = None;
                self.focus_on_arrival = Some(workspace.clone());
                self.add_opened(workspace.clone(), program, terminal, cx);
                self.models
                    .workspaces
                    .update(cx, |list, cx| list.want_on_screen(workspace, cx));
            }
            Ok(_) => self.notice = Some(AskError::Unexpected.to_string()),
            Err(error) => self.notice = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn agent_wanted(
        &mut self,
        wanted: &Wanted,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (workspace, agent, review) = match wanted {
            Wanted::AgentFor {
                workspace,
                agent,
                review,
            } => (workspace, agent, review),
            Wanted::Worktree { .. } => return self.worktree_wanted(wanted, cx),
        };
        self.notice = None;
        let program = Program::AgentFor {
            agent: agent.clone(),
            review: review.clone(),
        };
        let request = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: program.clone(),
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        let workspace = workspace.clone();
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.opened(workspace, program, answer, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

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
