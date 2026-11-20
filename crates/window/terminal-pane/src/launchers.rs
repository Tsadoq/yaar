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
//! `yaar_link::Program::Resume` of that session, once: while the pane holds a resume it opened
//! for the session, another click asks nothing. The pane lets go of it when the session is
//! listed resumable no more, and when the terminal it opened ends, which is all the pane
//! hears of a resumed program that fails before it reports. [`crate::ResumeAgent`] does the
//! same for
//! the first resumable session of the workspace on screen the pane has not opened a resume
//! for since it was listed: a second press reaches the second session, not the first again.
//! The line goes with the last resumable session, as the Keeper tells the agents again
//! when a resumed session reports.
//!
//! The two lines are a row of their own at the top of the pane, above the notice, the line
//! that says how to open a terminal and the terminals, so that no terminal's first rows lie
//! under them. The room the terminals are laid out in, which the pane's canvas measures and
//! a terminal attached before any is laid out is sized from, is what lies below that row:
//! every terminal has the height of the pane less the row, and the resume line, which comes
//! and goes, changes it as a window made smaller does.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{AgentCli, Fact, Program, Reply, Request, Session, SessionId, TerminalId};

use crate::{NewAgentTerminal, NextAgentCli, ResumeAgent, TerminalPane};

/// The height of each of the two lines.
const LINE: Pixels = px(28.);

/// What the notice says when New Agent Terminal finds no CLI to start.
const NO_CLI: &str = "The Keeper names no agent CLI to start.";

/// What the notice says when Resume Agent finds no session to resume.
const NOTHING_TO_RESUME: &str = "No session of this workspace is left to resume.";

/// A resume the pane asked for.
struct Resume {
    /// The session it brings back.
    session: SessionId,
    /// The terminal the Keeper opened for it, once the pane has the answer.
    terminal: Option<TerminalId>,
}

/// What the pane keeps of its launchers: which CLI is the chosen one, and the resumes it
/// asked for.
#[derive(Default)]
pub(crate) struct Launchers {
    /// The chosen CLI, by its index in the CLIs the Keeper named.
    chosen: usize,
    /// The resumes the pane asked for, until their session is listed resumable no more or
    /// their terminal ends.
    resumed: Vec<Resume>,
    /// Terminals that ended while a resume still waited for its answer: a program that
    /// fails at once can end before the pane takes the answer that names its terminal.
    ended: Vec<TerminalId>,
    /// Keeps `resumed` in step with the agents model, once there is something in it.
    watching: Option<Subscription>,
    /// Keeps `resumed` in step with the terminals that end, as long as `watching` does.
    hearing: Option<Subscription>,
}

impl TerminalPane {
    /// The launchers the pane offers next to the shell: one for each agent CLI the Keeper
    /// names, in that order.
    pub fn launchers(&self, cx: &App) -> Vec<AgentCli> {
        self.models.agents.read(cx).clis().to_vec()
    }

    /// The CLI [`crate::NewAgentTerminal`] opens now; none while the Keeper names none.
    pub fn chosen_cli(&self, cx: &App) -> Option<AgentCli> {
        let agents = self.models.agents.read(cx);
        agents
            .clis()
            .get(self.chosen_index(agents.clis().len()))
            .cloned()
    }

    /// The sessions of the workspace on screen the pane offers to resume, in the order the
    /// agents model lists them.
    pub fn resumable(&self, cx: &App) -> Vec<SessionId> {
        self.resumable_sessions(cx)
            .into_iter()
            .map(|session| session.id)
            .collect()
    }

    /// The index of the chosen CLI among `count`: the first when the one that was chosen
    /// is not among the CLIs the Keeper names now.
    fn chosen_index(&self, count: usize) -> usize {
        if self.launchers.chosen < count {
            self.launchers.chosen
        } else {
            0
        }
    }

    /// The sessions of the workspace on screen the agents model lists as resumable.
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

    /// Forgets the resumes opened for sessions the agents model lists resumable no more,
    /// so that a session listed again, as after another restart, is offered again.
    fn forget_resumed(&mut self, cx: &App) {
        let agents = self.models.agents.read(cx);
        self.launchers.resumed.retain(|resume| {
            agents
                .session(&resume.session)
                .is_some_and(|session| session.resumable)
        });
    }

    /// Whether the pane holds a resume it asked for `session`.
    fn holds(&self, session: &SessionId) -> bool {
        self.launchers
            .resumed
            .iter()
            .any(|resume| resume.session == *session)
    }

    /// Starts forgetting resumes with every change of the agents model, rather than with
    /// every frame: a window that is not drawn while the Keeper tells a session resumable
    /// no more, and then resumable again, would never see it not resumable. And with every
    /// terminal that ends: the Keeper clears a session's resumable mark only when the
    /// resumed CLI reports it, so a CLI that fails before that leaves the session listed
    /// and its control drawn, which has to ask again.
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

    /// Lets go of the resume whose terminal ended; a terminal that ends before its resume
    /// has an answer is remembered for the answer.
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

    /// Takes the Keeper's answer to the question for the resume of `session`: the terminal
    /// it opened is the one whose end lets the resume go, or none was opened and the
    /// session can be tried again.
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

    /// Asks the Keeper for a new terminal for the workspace on screen that runs `program`,
    /// as [`crate::NewTerminal`] asks for a shell: at the size the pane lays terminals out
    /// at, shown and focused once the Keeper answers, its refusal the notice. Whether a
    /// question was asked: with no workspace on screen there is nothing to open one in.
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
        // Detached, as the question for a shell is: a task that is dropped is cancelled,
        // and its answer lost. The pane is held weakly, so a pane that is gone takes no
        // answer.
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

    /// Asks for the terminal that resumes `session`, and remembers that it was asked for.
    fn resume(&mut self, session: SessionId, cx: &mut Context<Self>) {
        // The Keeper refuses a session only once it runs, which is later than the answer
        // to this question: a second ask while the first stands would open a second
        // terminal on one session. A refusal forgets it, so that a retry goes through, and
        // so does the end of the terminal it opened.
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

    /// One control of a line: a button that does `act` when it is clicked, under the
    /// selector `selector`.
    fn control(
        &self,
        selector: String,
        label: String,
        chosen: Option<String>,
        act: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        // The mark of the chosen launcher is a ring over the control, which takes no click
        // from it.
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

    /// A line of the top row, `selector` its name, with `controls` in it.
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

    /// The two lines of the module's doc, as the pane's top row (`terminal-launchers`, and
    /// `terminal-resume` under it while there is a session to resume).
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

    /// What [`crate::NewAgentTerminal`] does: opens the chosen CLI's launcher, or says
    /// that the Keeper names none.
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

    /// What [`crate::NextAgentCli`] does: the next CLI is the chosen one, the first after
    /// the last; nothing while the Keeper names none.
    pub(crate) fn next_agent_cli(
        &mut self,
        _: &NextAgentCli,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.models.agents.read(cx).clis().len();
        if count == 0 {
            return;
        }
        self.launchers.chosen = (self.chosen_index(count) + 1) % count;
        cx.notify();
    }

    /// What [`crate::ResumeAgent`] does: resumes the first session of the workspace on
    /// screen that no resume was opened for since it was listed, or says that none is
    /// left.
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
