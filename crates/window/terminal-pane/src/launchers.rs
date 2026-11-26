//! The single-row terminal header bar: scrollable open tabs, split launcher, resumable-session
//! popover with dismissal, split toggle, and dock placement menu.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{
    AgentCli, Fact, Program, Reply, Request, Session, SessionId, SessionState, Setting, TerminalId,
    TerminalPlace,
};
use yaar_ui::metrics::{SPACE_1, SPACE_2, SPACE_3, SPACE_4};
use yaar_ui::pieces::{
    Button, ButtonKind, Divider, ListRow, MenuTrigger, Popover, SectionLabel, SplitButton, State,
    Tabs, Tone,
};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::{NewAgentTerminal, NextAgentCli, ResumeAgent, TerminalPane, ToggleSplit};

const HEADER_HEIGHT: Pixels = px(40.);

const NO_CLI: &str = "The Keeper names no agent CLI to start.";

const NOTHING_TO_RESUME: &str = "No session of this workspace is left to resume.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalPopover {
    Launcher,
    Resume,
    Dock,
}

struct Resume {
    session: SessionId,
    terminal: Option<TerminalId>,
}

/// The chosen CLI is the Frame's to keep: other panes read it too.
#[derive(Default)]
pub(crate) struct Launchers {
    resumed: Vec<Resume>,
    ended: Vec<TerminalId>,
    watching: Option<Subscription>,
    hearing: Option<Subscription>,
    popover: Option<TerminalPopover>,
    launcher_bounds: Rc<Cell<Bounds<Pixels>>>,
    resume_bounds: Rc<Cell<Bounds<Pixels>>>,
    dock_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl TerminalPane {
    pub fn launchers(&self, cx: &App) -> Vec<AgentCli> {
        self.models.agents.read(cx).clis().to_vec()
    }

    pub fn chosen_cli(&self, cx: &App) -> Option<AgentCli> {
        self.models.agents.read(cx).chosen_cli().cloned()
    }

    pub fn resumable(&self, cx: &App) -> Vec<SessionId> {
        self.resumable_sessions(cx)
            .into_iter()
            .map(|session| session.id)
            .collect()
    }

    pub(crate) fn close_popover(&mut self, cx: &mut Context<Self>) -> bool {
        if self.launchers.popover.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    fn toggle_popover(&mut self, which: TerminalPopover, cx: &mut Context<Self>) {
        self.launchers.popover = if self.launchers.popover == Some(which) {
            None
        } else {
            Some(which)
        };
        cx.notify();
    }

    fn resumable_sessions(&self, cx: &App) -> Vec<Session> {
        let Some(workspace) = self.models.workspaces.read(cx).selected() else {
            return Vec::new();
        };
        self.models
            .agents
            .read(cx)
            .of(workspace)
            .map(|agents| {
                agents
                    .sessions
                    .iter()
                    .filter(|session| session.resumable)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn forget_resumed(&mut self, cx: &App) {
        let agents = self.models.agents.read(cx);
        self.launchers.resumed.retain(|resume| {
            agents
                .session(&resume.session)
                .is_some_and(|session| session.resumable)
        });
    }

    fn holds(&self, session: &SessionId) -> bool {
        self.launchers
            .resumed
            .iter()
            .any(|resume| resume.session == *session)
    }

    /// Forgets held resumes on every change of the agents model, not every frame: an undrawn
    /// window would miss a session that stopped being resumable. Also when a terminal ends: the
    /// Keeper clears the resumable mark only when the resumed CLI reports.
    fn watch_agents(&mut self, cx: &mut Context<Self>) {
        if self.launchers.watching.is_none() {
            self.launchers.watching = Some(cx.observe(&self.models.agents, |pane, _, cx| {
                pane.forget_resumed(cx);
            }));
        }
        if self.launchers.hearing.is_none() {
            self.launchers.hearing =
                Some(cx.subscribe(&self.models.link, |pane, _, fact: &Fact, _| {
                    pane.resumed_ended(fact);
                }));
        }
    }

    fn resumed_ended(&mut self, fact: &Fact) {
        let Fact::TerminalEnded { terminal, .. } = fact else {
            return;
        };
        let resumed = &mut self.launchers.resumed;
        let held = resumed.len();
        resumed.retain(|resume| resume.terminal.as_ref() != Some(terminal));
        if resumed.len() == held && resumed.iter().any(|resume| resume.terminal.is_none()) {
            self.launchers.ended.push(terminal.clone());
        }
    }

    fn resume_answered(&mut self, session: &SessionId, answer: &Result<Reply, AskError>) {
        let opened = match answer {
            Ok(Reply::TerminalOpened { terminal }) => Some(terminal),
            _ => None,
        };
        let ended = opened.and_then(|terminal| {
            self.launchers
                .ended
                .iter()
                .position(|ended| ended == terminal)
        });
        match (opened, ended) {
            (Some(terminal), None) => {
                if let Some(resume) = self
                    .launchers
                    .resumed
                    .iter_mut()
                    .find(|resume| resume.session == *session)
                {
                    resume.terminal = Some(terminal.clone());
                }
            }
            _ => self
                .launchers
                .resumed
                .retain(|resume| resume.session != *session),
        }
        if !self
            .launchers
            .resumed
            .iter()
            .any(|resume| resume.terminal.is_none())
        {
            self.launchers.ended.clear();
        }
    }

    pub(crate) fn launch(&mut self, program: Program, cx: &mut Context<Self>) -> bool {
        let Some(workspace) = self.models.workspaces.read(cx).selected().cloned() else {
            return false;
        };
        self.notice = None;
        let request = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: program.clone(),
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        // Detached: a dropped task is cancelled.
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                if let Program::Resume { session } = &program {
                    pane.resume_answered(session, &answer);
                }
                pane.opened(workspace, program, answer, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
        true
    }

    fn resume(&mut self, session: SessionId, cx: &mut Context<Self>) {
        // The Keeper refuses a session only once it runs, later than the answer: a second ask
        // meanwhile would open a second terminal on one session.
        if self.holds(&session) {
            return;
        }
        let program = Program::Resume {
            session: session.clone(),
        };
        if self.launch(program, cx) {
            self.watch_agents(cx);
            self.launchers.resumed.push(Resume {
                session,
                terminal: None,
            });
        }
    }

    fn dismiss_resumable(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        let Some(workspace) = self.models.workspaces.read(cx).selected().cloned() else {
            return;
        };
        if session.is_none() {
            self.launchers.popover = None;
        }
        self.notice = None;
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::DismissResumable { workspace, session }, cx)
        });
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            if let Err(error) = answer {
                pane.update(cx, |pane, cx| {
                    pane.notice = Some(error.to_string());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn terminal_label(&self, id: &TerminalId, cx: &App) -> String {
        if let Some(session) = self.models.agents.read(cx).in_terminal(id) {
            return session.title.clone();
        }
        let agents = self.models.agents.read(cx);
        let mut shell_index = 0usize;
        for held in self.terminals_here() {
            if agents.in_terminal(&held).is_none() {
                shell_index += 1;
                if held == *id {
                    return if shell_index == 1 {
                        "Shell".to_owned()
                    } else {
                        format!("Shell {shell_index}")
                    };
                }
            }
        }
        "Shell".to_owned()
    }

    fn terminal_state_dot(&self, id: &TerminalId, cx: &App) -> Option<State> {
        let session = self.models.agents.read(cx).in_terminal(id)?;
        Some(match session.state {
            SessionState::Works => State::Working,
            SessionState::WaitsForPermission => State::Permission,
            SessionState::Unreviewed => State::NeedsYou,
            SessionState::NotRunning => State::Idle,
            SessionState::Idle => State::Ok,
        })
    }

    pub(crate) fn render_launchers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = palette(cx);
        let chosen = self.chosen_cli(cx);
        let here = self.terminals_here();
        let focused = self.focused();
        let pane_entity = cx.entity();
        let sessions = self.resumable_sessions(cx);
        let open_popover = self.launchers.popover;

        let (shell_key, agent_key, split_key, place_key, resume_key) = if cfg!(target_os = "macos")
        {
            ("Cmd-T", "Cmd-Shift-T", "Cmd-D", "Cmd-Alt-P", "Cmd-Shift-R")
        } else {
            (
                "Ctrl-T",
                "Ctrl-Shift-T",
                "Ctrl-D",
                "Ctrl-Alt-P",
                "Ctrl-Shift-R",
            )
        };

        let primary_tip = match &chosen {
            Some(cli) => format!("New {} ({agent_key})", cli.title),
            None => format!("New Shell ({shell_key})"),
        };
        let split_launcher = SplitButton::new("terminal-launcher-split", "+")
            .open(open_popover == Some(TerminalPopover::Launcher))
            .primary_tooltip(primary_tip)
            .menu_tooltip("Choose terminal or agent CLI")
            .selector("terminal-launcher-split")
            .primary_selector("terminal-launcher-add")
            .menu_selector("terminal-launcher-menu")
            .track_bounds(self.launchers.launcher_bounds.clone())
            .on_primary(cx.listener(|pane, _: &ClickEvent, _, cx| {
                pane.close_popover(cx);
                let program = match pane.chosen_cli(cx) {
                    Some(cli) => Program::Agent { agent: cli.agent },
                    None => Program::Shell,
                };
                pane.launch(program, cx);
            }))
            .on_menu(cx.listener(|pane, _: &ClickEvent, _, cx| {
                pane.toggle_popover(TerminalPopover::Launcher, cx);
            }));

        let left_group = if here.is_empty() {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex_none()
                        .typed(TypeRole::Strong)
                        .text_color(palette.text)
                        .child("Terminal"),
                )
                .child(split_launcher)
        } else {
            let mut row = Tabs::new("terminal-tabs")
                .mono()
                .scrollable()
                .selector("terminal-tab");
            let mut selected = None;
            for (index, id) in here.iter().enumerate() {
                if focused.as_ref() == Some(id) {
                    selected = Some(index);
                }
                row = row.tab(
                    id.as_str().to_owned(),
                    self.terminal_label(id, cx),
                    None,
                    self.terminal_state_dot(id, cx),
                );
            }
            if let Some(index) = selected {
                row = row.selected(index);
            }
            let ids_select = here.clone();
            let entity_select = pane_entity.clone();
            let ids_close = here.clone();
            let entity_close = pane_entity;
            let row = row
                .on_select(move |index, window, cx| {
                    if let Some(target) = ids_select.get(index).cloned() {
                        entity_select.update(cx, |pane, cx| pane.show(target, window, cx));
                    }
                })
                .on_close(move |index, window, cx| {
                    if let Some(target) = ids_close.get(index).cloned() {
                        entity_close.update(cx, |pane, cx| pane.close_terminal(target, window, cx));
                    }
                });
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(row)
                .child(split_launcher)
        };

        let resume_trigger = (!sessions.is_empty()).then(|| {
            let count = sessions.len();
            let noun = if count == 1 { "session" } else { "sessions" };
            MenuTrigger::new("terminal-resume", count.to_string())
                .leading_glyph("\u{21ba}")
                .tone(Tone::Warn)
                .open(open_popover == Some(TerminalPopover::Resume))
                .tooltip(format!("{count} cut-off {noun} to resume ({resume_key})"))
                .selector("terminal-resume")
                .track_bounds(self.launchers.resume_bounds.clone())
                .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                    pane.toggle_popover(TerminalPopover::Resume, cx);
                }))
        });

        let split_disabled = here.len() < 2;
        let split_tooltip = if split_disabled {
            "Open a second terminal to split".to_owned()
        } else {
            format!("Split terminal view ({split_key})")
        };
        let split_button = Button::new("terminal-split-toggle", "Split")
            .kind(ButtonKind::Ghost)
            .small()
            .selected(self.split())
            .disabled(split_disabled)
            .tooltip(split_tooltip)
            .selector("terminal-split-toggle")
            .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                pane.close_popover(cx);
                pane.toggle_split(&ToggleSplit, window, cx);
            }));

        let place = self.models.settings.read(cx).terminal_place();
        let place_name = match place {
            TerminalPlace::Left => "Left",
            TerminalPlace::Right => "Right",
            TerminalPlace::Below => "Bottom",
        };
        let dock_trigger = MenuTrigger::new("terminal-place", "Dock")
            .open(open_popover == Some(TerminalPopover::Dock))
            .tooltip(format!("Dock terminal ({place_name}) · {place_key}"))
            .selector("terminal-place")
            .track_bounds(self.launchers.dock_bounds.clone())
            .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                pane.toggle_popover(TerminalPopover::Dock, cx);
            }));

        let bar = div()
            .debug_selector(|| "terminal-launchers".to_owned())
            .flex_none()
            .w_full()
            .h(HEADER_HEIGHT)
            .px(SPACE_4)
            .gap(SPACE_2)
            .flex()
            .items_center()
            .overflow_hidden()
            .bg(palette.pane)
            .text_color(palette.text)
            .border_b_1()
            .border_color(palette.separator)
            .child(left_group)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .children(resume_trigger)
                    .child(split_button)
                    .child(dock_trigger),
            );
        Some(bar.into_any_element())
    }

    pub(crate) fn render_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let which = self.launchers.popover?;
        let palette = palette(cx);
        let entity = cx.entity();
        let dismiss = move |_: &mut Window, cx: &mut App| {
            entity.update(cx, |pane, cx| {
                pane.close_popover(cx);
            });
        };

        let (shell_key, agent_key) = if cfg!(target_os = "macos") {
            ("Cmd-T", "Cmd-Shift-T")
        } else {
            ("Ctrl-T", "Ctrl-Shift-T")
        };

        match which {
            TerminalPopover::Launcher => {
                let anchor = self.launchers.launcher_bounds.get();
                let chosen = self.chosen_cli(cx);
                let clis = self.launchers(cx);
                let mut rows = div().px(SPACE_2).pb(SPACE_2).flex().flex_col().gap(SPACE_1);
                for cli in clis {
                    let agent = cli.agent.clone();
                    let default_agent = cli.agent.clone();
                    let is_chosen = chosen.as_ref() == Some(&cli);
                    let mark = if is_chosen {
                        div()
                            .debug_selector({
                                let agent = cli.agent.clone();
                                move || format!("terminal-launcher-chosen-{agent}")
                            })
                            .flex_none()
                            .w(px(12.))
                            .typed(TypeRole::Meta)
                            .text_color(palette.accent_text)
                            .child("\u{2713}")
                    } else {
                        div().flex_none().w(px(12.))
                    };
                    let title_agent = cli.agent.clone();
                    let mut item = ListRow::new(SharedString::from(format!(
                        "terminal-launcher-{}",
                        cli.agent
                    )))
                    .compact()
                    .selected(is_chosen)
                    .selector(format!("terminal-launcher-{}", cli.agent))
                    .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                        pane.close_popover(cx);
                        pane.launch(
                            Program::Agent {
                                agent: agent.clone(),
                            },
                            cx,
                        );
                    }))
                    .child(mark)
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("terminal-launcher-title-{title_agent}")
                            })
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::Body)
                            .text_color(palette.text)
                            .child(cli.title),
                    )
                    .child(div().flex_1());
                    if is_chosen {
                        item = item.child(
                            div()
                                .flex_none()
                                .typed(TypeRole::Tiny)
                                .text_color(palette.text_dim)
                                .whitespace_nowrap()
                                .child(format!("Default · {agent_key}")),
                        );
                        rows = rows.child(item);
                    } else {
                        let set_default = Button::new(
                            SharedString::from(format!("terminal-launcher-default-{}", cli.agent)),
                            "Set default",
                        )
                        .kind(ButtonKind::Ghost)
                        .small()
                        .selector(format!("terminal-launcher-default-{}", cli.agent))
                        .on_click(cx.listener(
                            move |pane, _: &ClickEvent, _, cx| {
                                pane.models.agents.update(cx, |agents, cx| {
                                    agents.choose_cli(&default_agent, cx);
                                });
                            },
                        ));
                        rows = rows.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_1)
                                .child(div().flex_1().min_w_0().child(item))
                                .child(set_default),
                        );
                    }
                }
                let shell_row = ListRow::new("terminal-launcher-shell")
                    .compact()
                    .selector("terminal-launcher-shell")
                    .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                        pane.close_popover(cx);
                        pane.launch(Program::Shell, cx);
                    }))
                    .child(div().flex_none().w(px(12.)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .typed(TypeRole::Body)
                            .text_color(palette.text)
                            .child("Shell"),
                    )
                    .child(
                        div()
                            .flex_none()
                            .typed(TypeRole::Tiny)
                            .text_color(palette.text_dim)
                            .whitespace_nowrap()
                            .child(shell_key),
                    );
                Some(
                    Popover::new(anchor)
                        .min_width(px(260.))
                        .selector("terminal-launcher-popover")
                        .on_dismiss(dismiss)
                        .child(div().pt(SPACE_2).child(SectionLabel::new("New terminal")))
                        .child(rows)
                        .child(Divider::horizontal())
                        .child(div().p(SPACE_2).child(shell_row))
                        .into_any_element(),
                )
            }
            TerminalPopover::Resume => {
                let sessions = self.resumable_sessions(cx);
                if sessions.is_empty() {
                    return None;
                }
                let anchor = self.launchers.resume_bounds.get();
                let mut rows = div().px(SPACE_2).pb(SPACE_2).flex().flex_col().gap(SPACE_1);
                for (n, session) in sessions.into_iter().enumerate() {
                    let resume_id = session.id.clone();
                    let dismiss_id = session.id.clone();
                    let busy = self.holds(&session.id);
                    let selector = format!("terminal-resume-{n}");
                    let dismiss_selector = format!("terminal-resume-dismiss-{n}");
                    let resume_row = ListRow::new(SharedString::from(selector.clone()))
                        .compact()
                        .selector(selector)
                        .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                            pane.close_popover(cx);
                            pane.resume(resume_id.clone(), cx);
                        }))
                        .child(
                            div()
                                .debug_selector(move || format!("terminal-resume-title-{n}"))
                                .min_w_0()
                                .truncate()
                                .typed(TypeRole::Body)
                                .text_color(palette.text)
                                .child(session.title),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_none()
                                .typed(TypeRole::Tiny)
                                .text_color(palette.text_dim)
                                .whitespace_nowrap()
                                .child(if busy { "Opening…" } else { "Resume" }),
                        );
                    let dismiss_btn =
                        Button::new(SharedString::from(dismiss_selector.clone()), "\u{d7}")
                            .kind(ButtonKind::Ghost)
                            .small()
                            .tooltip("Dismiss resume offer")
                            .selector(dismiss_selector)
                            .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                                pane.dismiss_resumable(Some(dismiss_id.clone()), cx);
                            }));
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_1)
                            .child(div().flex_1().min_w_0().child(resume_row))
                            .child(dismiss_btn),
                    );
                }
                let footer = div()
                    .px(SPACE_3)
                    .py(SPACE_2)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .typed(TypeRole::Tiny)
                            .text_color(palette.text_dim)
                            .child("Reviews stay saved"),
                    )
                    .child(
                        Button::new("terminal-resume-dismiss-all", "Dismiss all")
                            .kind(ButtonKind::Ghost)
                            .small()
                            .selector("terminal-resume-dismiss-all")
                            .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                                pane.dismiss_resumable(None, cx);
                            })),
                    );
                Some(
                    Popover::new(anchor)
                        .width(px(300.))
                        .max_height(px(320.))
                        .align_right()
                        .selector("terminal-resume-popover")
                        .on_dismiss(dismiss)
                        .child(
                            div()
                                .pt(SPACE_2)
                                .child(SectionLabel::new("Resumable sessions")),
                        )
                        .child(rows)
                        .child(Divider::horizontal())
                        .child(footer)
                        .into_any_element(),
                )
            }
            TerminalPopover::Dock => {
                let anchor = self.launchers.dock_bounds.get();
                let place = self.models.settings.read(cx).terminal_place();
                let settings = self.models.settings.clone();
                let mut rows = div().px(SPACE_2).pb(SPACE_2).flex().flex_col().gap(SPACE_1);
                for (key, label, target) in [
                    ("left", "Dock Left", TerminalPlace::Left),
                    ("right", "Dock Right", TerminalPlace::Right),
                    ("bottom", "Dock Bottom", TerminalPlace::Below),
                ] {
                    let active = place == target;
                    let settings = settings.clone();
                    let mark = if active {
                        div()
                            .flex_none()
                            .w(px(12.))
                            .typed(TypeRole::Meta)
                            .text_color(palette.accent_text)
                            .child("\u{2713}")
                    } else {
                        div().flex_none().w(px(12.))
                    };
                    let row = ListRow::new(SharedString::from(format!("terminal-place-{key}")))
                        .compact()
                        .selected(active)
                        .selector(format!("terminal-place-{key}"))
                        .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                            pane.close_popover(cx);
                            settings.update(cx, |settings, cx| {
                                settings.set(Setting::TerminalPlace(target), cx);
                            });
                        }))
                        .child(mark)
                        .child(
                            div()
                                .typed(TypeRole::Body)
                                .text_color(palette.text)
                                .child(label),
                        );
                    rows = rows.child(row);
                }
                Some(
                    Popover::new(anchor)
                        .width(px(190.))
                        .align_right()
                        .selector("terminal-place-popover")
                        .on_dismiss(dismiss)
                        .child(div().pt(SPACE_2).child(SectionLabel::new("Dock terminal")))
                        .child(rows)
                        .into_any_element(),
                )
            }
        }
    }

    pub(crate) fn new_agent_terminal(
        &mut self,
        _: &NewAgentTerminal,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.chosen_cli(cx) {
            Some(cli) => {
                self.launch(Program::Agent { agent: cli.agent }, cx);
            }
            None => {
                self.notice = Some(NO_CLI.to_owned());
                cx.notify();
            }
        }
    }

    pub(crate) fn next_agent_cli(
        &mut self,
        _: &NextAgentCli,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.models
            .agents
            .update(cx, |agents, cx| agents.choose_next_cli(cx));
    }

    pub(crate) fn resume_agent(&mut self, _: &ResumeAgent, _: &mut Window, cx: &mut Context<Self>) {
        let next = self
            .resumable_sessions(cx)
            .into_iter()
            .find(|session| !self.holds(&session.id));
        match next {
            Some(session) => self.resume(session.id, cx),
            None => {
                self.notice = Some(NOTHING_TO_RESUME.to_owned());
                cx.notify();
            }
        }
    }
}
