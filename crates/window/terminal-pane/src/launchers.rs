//! The launchers next to the shell, one per agent CLI the Keeper names, and the controls that
//! resume the sessions a restart cut off.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{AgentCli, Fact, Program, Reply, Request, Session, SessionId, TerminalId};

use crate::{NewAgentTerminal, NextAgentCli, ResumeAgent, TerminalPane};

const LINE: Pixels = px(28.);

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

    fn launch(&mut self, program: Program, cx: &mut Context<Self>) -> bool {
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

    fn control(
        &self,
        selector: String,
        label: String,
        chosen: Option<String>,
        act: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        // The mark of the chosen launcher is a ring over the control, which takes no click from it.
        let mark = chosen.map(|selector| {
            div()
                .debug_selector(move || selector.clone())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded_sm()
                .border_1()
                .border_color(theme.primary)
        });
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .relative()
            .flex_none()
            .px_2()
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground)
            .child(label)
            .children(mark)
            .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| act(pane, cx)))
    }

    fn line(&self, selector: &'static str, cx: &App) -> Div {
        let theme = cx.theme();
        div()
            .debug_selector(move || selector.to_owned())
            .flex_none()
            .h(LINE)
            .px_2()
            .gap_2()
            .flex()
            .flex_row()
            .items_center()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .border_b_1()
            .border_color(theme.border)
    }

    pub(crate) fn render_launchers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chosen = self.chosen_cli(cx);
        let mut launchers = self.line("terminal-launchers", cx).child(self.control(
            "terminal-launcher-shell".to_owned(),
            "Shell".to_owned(),
            None,
            |pane, cx| {
                pane.launch(Program::Shell, cx);
            },
            cx,
        ));
        for cli in self.launchers(cx) {
            let agent = cli.agent.clone();
            let mark = (chosen.as_ref() == Some(&cli))
                .then(|| format!("terminal-launcher-chosen-{}", cli.agent));
            launchers = launchers.child(self.control(
                format!("terminal-launcher-{}", cli.agent),
                cli.title,
                mark,
                move |pane, cx| {
                    pane.launch(
                        Program::Agent {
                            agent: agent.clone(),
                        },
                        cx,
                    );
                },
                cx,
            ));
        }
        let sessions = self.resumable_sessions(cx);
        let resume = (!sessions.is_empty()).then(|| {
            let mut line = self.line("terminal-resume", cx);
            for (n, session) in sessions.into_iter().enumerate() {
                let id = session.id;
                line = line.child(self.control(
                    format!("terminal-resume-{n}"),
                    format!("Resume {}", session.title),
                    None,
                    move |pane, cx| pane.resume(id.clone(), cx),
                    cx,
                ));
            }
            line
        });
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .child(launchers)
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
