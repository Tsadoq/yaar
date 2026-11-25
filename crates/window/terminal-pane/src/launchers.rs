//! The launchers next to the shell, one per agent CLI the Keeper names, and the controls that
//! resume the sessions a restart cut off.

use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{
    AgentCli, Fact, Program, Reply, Request, Session, SessionId, SessionState, Setting, TerminalId,
    TerminalPlace,
};
use yaar_ui::metrics::{SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use yaar_ui::pieces::{Banner, Button, ButtonKind, Segmented, State, Tabs};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::{NewAgentTerminal, NextAgentCli, ResumeAgent, TerminalPane, ToggleSplit};

pub(crate) const TABS_BAR_HEIGHT: Pixels = px(34.);

const NO_CLI: &str = "The Keeper names no agent CLI to start.";

const NOTHING_TO_RESUME: &str = "No session of this workspace is left to resume.";

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

        let mut launcher_buttons = div().flex_none().flex().items_center().gap(SPACE_1).child(
            Button::new("terminal-launcher-shell", "+ Shell")
                .kind(ButtonKind::Ghost)
                .small()
                .selector("terminal-launcher-shell")
                .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                    pane.launch(Program::Shell, cx);
                })),
        );
        for cli in self.launchers(cx) {
            let agent = cli.agent.clone();
            let is_chosen = chosen.as_ref() == Some(&cli);
            let mut button = Button::new(
                SharedString::from(format!("terminal-launcher-{}", cli.agent)),
                format!("+ {}", cli.title),
            )
            .kind(ButtonKind::Ghost)
            .small()
            .selected(is_chosen)
            .selector(format!("terminal-launcher-{}", cli.agent))
            .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                pane.launch(
                    Program::Agent {
                        agent: agent.clone(),
                    },
                    cx,
                );
            }));
            if is_chosen {
                button =
                    button.indicator_selector(format!("terminal-launcher-chosen-{}", cli.agent));
            }
            launcher_buttons = launcher_buttons.child(button);
        }

        let place = self.models.settings.read(cx).terminal_place();
        let place_index = match place {
            TerminalPlace::Left => 0,
            TerminalPlace::Right => 1,
            TerminalPlace::Below => 2,
        };
        let settings = self.models.settings.clone();
        let place_control = Segmented::new("terminal-place")
            .selector("terminal-place")
            .segment("left", "Left")
            .segment("right", "Right")
            .segment("bottom", "Bottom")
            .selected(place_index)
            .on_select(move |index, _, cx| {
                let target = match index {
                    0 => TerminalPlace::Left,
                    2 => TerminalPlace::Below,
                    _ => TerminalPlace::Right,
                };
                settings.update(cx, |settings, cx| {
                    settings.set(Setting::TerminalPlace(target), cx);
                });
            });

        let split_button = Button::new("terminal-split-toggle", "Split")
            .small()
            .selected(self.split())
            .disabled(here.len() < 2)
            .selector("terminal-split-toggle")
            .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                pane.toggle_split(&ToggleSplit, window, cx);
            }));

        let launchers = div()
            .debug_selector(|| "terminal-launchers".to_owned())
            .flex_none()
            .w_full()
            .min_h(px(40.))
            .px(SPACE_5)
            .py(SPACE_2)
            .gap(SPACE_3)
            .flex()
            .flex_wrap()
            .items_center()
            .bg(palette.pane)
            .text_color(palette.text)
            .border_b_1()
            .border_color(palette.separator)
            .child(
                div()
                    .flex_none()
                    .typed(TypeRole::Strong)
                    .text_color(palette.text)
                    .child("Terminal"),
            )
            .child(launcher_buttons)
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .child(split_button)
                    .child(place_control),
            );

        let tabs_row = (!here.is_empty()).then(|| {
            let mut row = Tabs::new("terminal-tabs").mono().selector("terminal-tab");
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
                .debug_selector(|| "terminal-tabs-bar".to_owned())
                .flex_none()
                .w_full()
                .min_h(TABS_BAR_HEIGHT)
                .px(SPACE_4)
                .py(SPACE_2)
                .flex()
                .flex_wrap()
                .items_center()
                .bg(palette.pane)
                .border_b_1()
                .border_color(palette.separator)
                .child(row)
        });

        let sessions = self.resumable_sessions(cx);
        let resume = (!sessions.is_empty()).then(|| {
            let mut row = div().flex().flex_wrap().items_center().gap(SPACE_3);
            for (n, session) in sessions.into_iter().enumerate() {
                let id = session.id;
                let selector = format!("terminal-resume-{n}");
                row = row.child(
                    Button::new(
                        SharedString::from(selector.clone()),
                        format!("Resume {}", session.title),
                    )
                    .small()
                    .selector(selector)
                    .on_click(cx.listener(
                        move |pane, _: &ClickEvent, _, cx| {
                            pane.resume(id.clone(), cx);
                        },
                    )),
                );
            }
            Banner::custom().selector("terminal-resume").child(row)
        });
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .child(launchers)
                .children(tabs_row)
                .children(resume)
                .into_any_element(),
        )
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
